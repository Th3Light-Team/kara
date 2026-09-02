//! The FreeDesktop thumbnail cache.
//!
//! Every test works against a cache root inside a temporary directory: the real
//! one is shared with the rest of the desktop and no test gets to write there.

mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use kara_fs::thumbnails::{ThumbnailSize, Thumbnails, cache_name};

use common::TempTree;

/// A small solid-colour PNG, written where the test asks for it.
fn write_png(path: &Path, width: u32, height: u32) {
    let image = image::RgbaImage::from_pixel(width, height, image::Rgba([12, 90, 200, 255]));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("cannot create the source directory");
    }
    image.save(path).expect("cannot write the test image");
}

fn modified(path: &Path) -> SystemTime {
    std::fs::metadata(path)
        .expect("the file must exist")
        .modified()
        .expect("this filesystem must report mtime")
}

fn store(root: &Path) -> Thumbnails {
    Thumbnails::at(root, ThumbnailSize::Normal)
}

#[test]
fn the_cache_name_is_the_md5_of_the_uri() {
    // Fixed vectors: this is the whole contract with every other file manager
    // on the machine. If it drifts, Kara stops seeing thumbnails that are
    // already there and starts writing duplicates beside them.
    assert_eq!(
        cache_name("file:///home/ana/foto.png"),
        "8faecfd4ef179b3641fcd0fcacf36522.png"
    );
    assert_eq!(
        cache_name("file:///home/ana/Mis%20cosas/a.png"),
        "8570610e8e69e29c17f4b8d6177ec1fc.png"
    );
}

#[test]
fn each_size_lives_in_the_directory_the_standard_reserves() {
    assert_eq!(ThumbnailSize::Normal.directory(), "normal");
    assert_eq!(ThumbnailSize::Large.directory(), "large");
    assert_eq!(ThumbnailSize::XLarge.directory(), "x-large");
    assert_eq!(ThumbnailSize::XXLarge.directory(), "xx-large");

    assert_eq!(ThumbnailSize::Normal.pixels(), 128);
    assert_eq!(ThumbnailSize::XXLarge.pixels(), 1024);
}

#[test]
fn the_covering_size_is_the_smallest_that_still_fits() {
    assert_eq!(ThumbnailSize::covering(16), ThumbnailSize::Normal);
    assert_eq!(ThumbnailSize::covering(128), ThumbnailSize::Normal);
    assert_eq!(ThumbnailSize::covering(129), ThumbnailSize::Large);
    // Beyond the largest standard size there is nothing bigger to ask for.
    assert_eq!(ThumbnailSize::covering(4000), ThumbnailSize::XXLarge);
}

#[test]
fn a_generated_thumbnail_is_found_again() {
    let tree = TempTree::on_tmpfs("thumb-roundtrip");
    let source = tree.path("foto.png");
    write_png(&source, 400, 300);
    let cache = tree.mkdir("cache");
    let thumbnails = store(&cache);

    let when = modified(&source);
    assert!(
        thumbnails.lookup(&source, when).is_none(),
        "nothing is cached before generating"
    );

    let written = thumbnails
        .generate(&source, when)
        .expect("a plain PNG must be thumbnailable");
    assert!(written.exists());
    assert_eq!(thumbnails.lookup(&source, when), Some(written));
}

#[test]
fn a_thumbnail_of_an_older_version_is_not_used() {
    // This is the rule that keeps a stale preview off the screen after the file
    // is edited: the mtime recorded inside the PNG has to match.
    let tree = TempTree::on_tmpfs("thumb-stale");
    let source = tree.path("foto.png");
    write_png(&source, 400, 300);
    let thumbnails = store(&tree.mkdir("cache"));

    let when = modified(&source);
    thumbnails.generate(&source, when).expect("must generate");

    let later = when + Duration::from_secs(60);
    assert_eq!(thumbnails.lookup(&source, later), None);
}

#[test]
fn the_thumbnail_is_scaled_down_to_the_requested_size() {
    let tree = TempTree::on_tmpfs("thumb-scale");
    let source = tree.path("grande.png");
    write_png(&source, 800, 400);
    let thumbnails = store(&tree.mkdir("cache"));

    let path = thumbnails
        .generate(&source, modified(&source))
        .expect("must generate");
    let decoded = image::open(&path).expect("the thumbnail must be a readable PNG");
    assert_eq!(decoded.width(), 128, "the long side becomes the target");
    assert_eq!(decoded.height(), 64, "the aspect ratio is kept");
}

#[test]
fn something_smaller_than_the_target_is_not_blown_up() {
    // Upscaling invents pixels that are not there, and the standard says a
    // thumbnail is never larger than its source.
    let tree = TempTree::on_tmpfs("thumb-small");
    let source = tree.path("icono.png");
    write_png(&source, 32, 32);
    let thumbnails = store(&tree.mkdir("cache"));

    let path = thumbnails
        .generate(&source, modified(&source))
        .expect("must generate");
    let decoded = image::open(&path).expect("must be readable");
    assert_eq!((decoded.width(), decoded.height()), (32, 32));
}

#[test]
fn a_file_that_is_not_an_image_fails_and_is_remembered() {
    let tree = TempTree::on_tmpfs("thumb-fail");
    let source = tree.write("apuntes.txt", b"esto no es una imagen");
    let thumbnails = store(&tree.mkdir("cache"));

    let when = modified(&source);
    assert!(!thumbnails.failed_before(&source, when));

    assert!(thumbnails.generate(&source, when).is_err());
    assert!(
        thumbnails.failed_before(&source, when),
        "a folder full of broken files must cost once, not every visit"
    );
    assert!(
        thumbnails.lookup(&source, when).is_none(),
        "a failure marker is not a thumbnail"
    );
}

#[test]
fn a_recorded_failure_does_not_apply_to_a_newer_version() {
    let tree = TempTree::on_tmpfs("thumb-fail-stale");
    let source = tree.write("roto.png", b"cabecera invalida");
    let thumbnails = store(&tree.mkdir("cache"));

    let when = modified(&source);
    let _ = thumbnails.generate(&source, when);

    let later = when + Duration::from_secs(60);
    assert!(
        !thumbnails.failed_before(&source, later),
        "replacing the file has to earn a fresh attempt"
    );
}

#[test]
fn something_over_the_limit_is_left_alone() {
    let tree = TempTree::on_tmpfs("thumb-limit");
    let source = tree.path("grande.png");
    write_png(&source, 400, 300);
    let thumbnails = Thumbnails::at(tree.mkdir("cache"), ThumbnailSize::Normal).with_size_limit(16);

    assert!(thumbnails.generate(&source, modified(&source)).is_err());
}

#[test]
fn the_thumbnail_is_readable_only_by_its_owner() {
    // It can reveal the contents of a private file, so the standard puts it at
    // 0600 inside a 0700 directory.
    let tree = TempTree::on_tmpfs("thumb-modes");
    let source = tree.path("foto.png");
    write_png(&source, 64, 64);
    let cache = tree.mkdir("cache");
    let thumbnails = store(&cache);

    let path = thumbnails
        .generate(&source, modified(&source))
        .expect("must generate");

    assert_eq!(common::mode_of(&path) & 0o777, 0o600);
    let directory = path.parent().expect("the thumbnail lives somewhere");
    assert_eq!(common::mode_of(directory) & 0o777, 0o700);
}

/// Writes a 1x1 PNG carrying provenance in the chunk flavour asked for.
fn write_thumbnail_with_chunk(path: &Path, uri: &str, mtime: i64, flavour: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("cannot create the cache directory");
    }
    let file = std::fs::File::create(path).expect("cannot create the thumbnail");
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), 1, 1);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let add = |encoder: &mut png::Encoder<_>, key: &str, value: String| match flavour {
        "zTXt" => encoder.add_ztxt_chunk(key.to_string(), value),
        "iTXt" => encoder.add_itxt_chunk(key.to_string(), value),
        _ => encoder.add_text_chunk(key.to_string(), value),
    }
    .expect("the chunk must be writable");
    add(&mut encoder, "Thumb::URI", uri.to_string());
    add(&mut encoder, "Thumb::MTime", mtime.to_string());
    let mut writer = encoder.write_header().expect("header");
    writer.write_image_data(&[0, 0, 0, 255]).expect("pixels");
}

#[test]
fn provenance_is_read_from_compressed_chunks_too() {
    // Ghostscript writes `Thumb::URI` as a compressed `zTXt`. A reader that only
    // understands plain `tEXt` treats a perfectly good thumbnail as missing and
    // regenerates it on every single visit.
    for flavour in ["tEXt", "zTXt", "iTXt"] {
        let tree = TempTree::on_tmpfs("thumb-chunks");
        let source = tree.path("foto.png");
        write_png(&source, 64, 64);
        let thumbnails = store(&tree.mkdir("cache"));

        let when = modified(&source);
        let seconds = when
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("after the epoch")
            .as_secs() as i64;
        write_thumbnail_with_chunk(
            &thumbnails.path_for(&source),
            &kara_fs::file_uri(&source),
            seconds,
            flavour,
        );

        assert!(
            thumbnails.lookup(&source, when).is_some(),
            "{flavour} chunks must be understood"
        );
    }
}

#[test]
fn a_thumbnail_that_claims_another_file_is_rejected() {
    // The name is an MD5, so a collision is conceivable; the recorded URI is
    // what actually ties a thumbnail to its file.
    let tree = TempTree::on_tmpfs("thumb-wrong-uri");
    let source = tree.path("foto.png");
    write_png(&source, 64, 64);
    let thumbnails = store(&tree.mkdir("cache"));

    let when = modified(&source);
    let seconds = when
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("after the epoch")
        .as_secs() as i64;
    write_thumbnail_with_chunk(
        &thumbnails.path_for(&source),
        "file:///otro/fichero.png",
        seconds,
        "tEXt",
    );

    assert_eq!(thumbnails.lookup(&source, when), None);
}

#[test]
fn a_png_without_provenance_is_not_taken_for_a_thumbnail() {
    // A stray PNG that happens to sit under the cache name must not be shown:
    // it carries no claim about which file it depicts.
    let tree = TempTree::on_tmpfs("thumb-bare");
    let source = tree.path("foto.png");
    write_png(&source, 64, 64);
    let cache = tree.mkdir("cache");
    let thumbnails = store(&cache);

    let target = thumbnails.path_for(&source);
    write_png(&target, 16, 16);

    assert_eq!(thumbnails.lookup(&source, modified(&source)), None);
}

#[test]
fn no_leftovers_are_kept_next_to_the_thumbnail() {
    // The write goes through a temporary file so nobody ever sees half a PNG;
    // that temporary must not survive the rename.
    let tree = TempTree::on_tmpfs("thumb-temp");
    let source = tree.path("foto.png");
    write_png(&source, 64, 64);
    let cache = tree.mkdir("cache");
    let thumbnails = store(&cache);

    let path = thumbnails
        .generate(&source, modified(&source))
        .expect("must generate");

    let directory: PathBuf = path.parent().expect("has a parent").to_path_buf();
    let leftovers: Vec<String> = std::fs::read_dir(&directory)
        .expect("the directory must be readable")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "left behind: {leftovers:?}");
}
