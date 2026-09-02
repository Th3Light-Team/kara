//! The FreeDesktop thumbnail cache: reading it, and filling the gaps.
//!
//! Reference: `ground/spec/03-vistas.md`, "Miniaturas de imágenes, vídeos y
//! documentos", which explicitly asks for the shared `~/.cache/thumbnails`
//! store rather than a private one.
//!
//! # Design decisions
//!
//! - **The cache is shared, so the rules are not ours to bend.** The file name
//!   is the MD5 of the `file://` URI, the PNG carries `Thumb::URI` and
//!   `Thumb::MTime`, and a thumbnail is stale the moment the source's mtime
//!   moves. Get any of that wrong and Kara stops seeing what Dolphin generated
//!   while quietly filling the directory with duplicates.
//! - **Validation reads the head of the PNG, not all of it.** Checking a
//!   thumbnail means reading two text chunks; decoding a 256×256 image to throw
//!   it away would make browsing a folder cost more than drawing it.
//! - **Failures are recorded.** A file that cannot be decoded is remembered
//!   under `fail/`, so a folder full of broken images is expensive once instead
//!   of on every visit. That directory is also shared: other viewers honour it.
//! - **Nothing here spawns a thread.** Generation is synchronous and slow by
//!   nature; who runs it off the UI thread is the caller's problem, and keeping
//!   that decision out of this module is what makes it testable.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use image::ImageDecoder;
use md5::{Digest, Md5};

use crate::uri::file_uri;

/// The standard thumbnail sizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ThumbnailSize {
    Normal,
    Large,
    XLarge,
    XXLarge,
}

impl ThumbnailSize {
    /// Longest side, in pixels.
    #[must_use]
    pub fn pixels(self) -> u32 {
        match self {
            Self::Normal => 128,
            Self::Large => 256,
            Self::XLarge => 512,
            Self::XXLarge => 1024,
        }
    }

    /// The cache subdirectory the standard reserves for this size.
    #[must_use]
    pub fn directory(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Large => "large",
            Self::XLarge => "x-large",
            Self::XXLarge => "xx-large",
        }
    }

    /// The smallest standard size that can still be scaled down to `wanted`.
    ///
    /// Asking for the next size up and shrinking beats scaling a 128×128
    /// thumbnail up to 256: the pixels for that simply are not there.
    #[must_use]
    pub fn covering(wanted: u32) -> Self {
        [Self::Normal, Self::Large, Self::XLarge, Self::XXLarge]
            .into_iter()
            .find(|size| size.pixels() >= wanted)
            .unwrap_or(Self::XXLarge)
    }
}

/// Why a thumbnail could not be produced.
#[derive(Debug, thiserror::Error)]
pub enum ThumbnailError {
    #[error("no se pudo leer o escribir la miniatura: {0}")]
    Io(#[from] io::Error),
    #[error("el fichero no se pudo decodificar como imagen: {0}")]
    Decode(String),
    #[error("el fichero es demasiado grande para miniaturizarlo")]
    TooLarge,
    #[error("no hay una carpeta de caché utilizable")]
    NoCache,
}

/// Files above this are skipped.
///
/// The spec asks for a configurable limit; this is the default until there is
/// somewhere to keep settings. It is a guard against spending a second decoding
/// a camera raw nobody asked to see, not a correctness rule.
pub const DEFAULT_SIZE_LIMIT: u64 = 32 * 1024 * 1024;

/// The shared thumbnail store.
#[derive(Debug, Clone)]
pub struct Thumbnails {
    root: PathBuf,
    size: ThumbnailSize,
    /// Name used under `fail/`, which the standard scopes per application.
    application: String,
    limit: u64,
}

impl Thumbnails {
    /// Opens the store the desktop shares, for a given size.
    #[must_use]
    pub fn shared(size: ThumbnailSize) -> Option<Self> {
        Some(Self::at(cache_root()?, size))
    }

    /// Opens a store rooted anywhere. Exists so tests never touch the real one.
    #[must_use]
    pub fn at(root: impl Into<PathBuf>, size: ThumbnailSize) -> Self {
        Self {
            root: root.into(),
            size,
            application: "kara".to_string(),
            limit: DEFAULT_SIZE_LIMIT,
        }
    }

    #[must_use]
    pub fn with_size_limit(mut self, limit: u64) -> Self {
        self.limit = limit;
        self
    }

    #[must_use]
    pub fn size(&self) -> ThumbnailSize {
        self.size
    }

    /// Where this file's thumbnail lives, whether or not it has one yet.
    #[must_use]
    pub fn path_for(&self, file: &Path) -> PathBuf {
        self.root
            .join(self.size.directory())
            .join(cache_name(&file_uri(file)))
    }

    /// Where a failed attempt is recorded.
    #[must_use]
    pub fn failure_path_for(&self, file: &Path) -> PathBuf {
        self.root
            .join("fail")
            .join(&self.application)
            .join(cache_name(&file_uri(file)))
    }

    /// The cached thumbnail, if there is one and it still matches the file.
    ///
    /// `modified` is passed in rather than read here: the caller has just
    /// `stat`ed the entry to list it, and doing it twice per file is the kind
    /// of waste that shows up on a folder with thousands of entries.
    #[must_use]
    pub fn lookup(&self, file: &Path, modified: SystemTime) -> Option<PathBuf> {
        let path = self.path_for(file);
        let text = read_png_text(&path).ok()?;
        matches_source(&text, file, modified).then_some(path)
    }

    /// Whether generating this thumbnail already failed for this exact version
    /// of the file.
    #[must_use]
    pub fn failed_before(&self, file: &Path, modified: SystemTime) -> bool {
        let path = self.failure_path_for(file);
        read_png_text(&path).is_ok_and(|text| matches_source(&text, file, modified))
    }

    /// Generates the thumbnail and stores it. Returns where it landed.
    pub fn generate(&self, file: &Path, modified: SystemTime) -> Result<PathBuf, ThumbnailError> {
        match self.render(file) {
            Ok(image) => {
                let path = self.path_for(file);
                write_thumbnail(&path, &image, file, modified)?;
                Ok(path)
            }
            Err(error) => {
                // The marker is best effort: failing to record a failure must
                // not turn into a second, louder failure.
                let _ = write_failure_marker(&self.failure_path_for(file), file, modified);
                Err(error)
            }
        }
    }

    /// Decodes and scales, without touching the cache.
    fn render(&self, file: &Path) -> Result<image::RgbaImage, ThumbnailError> {
        let length = std::fs::metadata(file)?.len();
        if length > self.limit {
            return Err(ThumbnailError::TooLarge);
        }

        let mut reader = image::ImageReader::open(file)?
            .with_guessed_format()
            .map_err(|error| ThumbnailError::Decode(error.to_string()))?;
        // A malformed header can claim enormous dimensions. Without a ceiling,
        // browsing a folder could be made to exhaust memory by dropping one
        // crafted file into it.
        let mut limits = image::Limits::default();
        limits.max_alloc = Some(256 * 1024 * 1024);
        reader.limits(limits);

        let mut decoder = reader
            .into_decoder()
            .map_err(|error| ThumbnailError::Decode(error.to_string()))?;
        // Read the tag before consuming the decoder: a photo taken sideways is
        // stored sideways with a tag saying so, and the spec asks for the
        // thumbnail to come out upright.
        let orientation = decoder
            .orientation()
            .unwrap_or(image::metadata::Orientation::NoTransforms);
        let mut decoded = image::DynamicImage::from_decoder(decoder)
            .map_err(|error| ThumbnailError::Decode(error.to_string()))?;
        decoded.apply_orientation(orientation);

        let side = self.size.pixels();
        // Never upscale: a 32×32 icon blown up to 128 is not a better preview,
        // and the standard says a thumbnail is not larger than its source.
        let scaled = if decoded.width() <= side && decoded.height() <= side {
            decoded
        } else {
            decoded.thumbnail(side, side)
        };
        Ok(scaled.to_rgba8())
    }
}

/// Root of the shared cache.
fn cache_root() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))?;
    Some(base.join("thumbnails"))
}

/// The cache file name for a URI: its MD5 in lowercase hex, plus `.png`.
#[must_use]
pub fn cache_name(uri: &str) -> String {
    let digest = Md5::digest(uri.as_bytes());
    let mut name = String::with_capacity(36);
    for byte in digest {
        use std::fmt::Write;
        // Writing into a String cannot fail; the result is discarded knowingly.
        let _ = write!(name, "{byte:02x}");
    }
    name.push_str(".png");
    name
}

/// Whether a thumbnail's recorded provenance still matches the file on disk.
///
/// Both keys have to agree. The URI alone would let an MD5 collision through,
/// and the mtime alone would accept a thumbnail of a different file entirely.
fn matches_source(text: &HashMap<String, String>, file: &Path, modified: SystemTime) -> bool {
    let Some(uri) = text.get("Thumb::URI") else {
        return false;
    };
    if *uri != file_uri(file) {
        return false;
    }
    let Some(recorded) = text.get("Thumb::MTime").and_then(|v| v.parse::<i64>().ok()) else {
        return false;
    };
    seconds_since_epoch(modified).is_some_and(|actual| actual == recorded)
}

/// Seconds since the epoch, negative for anything older.
///
/// `duration_since` fails outright before 1970, and files that old do exist on
/// restored archives; treating them as unreadable would make their thumbnails
/// regenerate forever.
fn seconds_since_epoch(time: SystemTime) -> Option<i64> {
    match time.duration_since(UNIX_EPOCH) {
        Ok(after) => i64::try_from(after.as_secs()).ok(),
        Err(before) => i64::try_from(before.duration().as_secs()).ok().map(|s| -s),
    }
}

/// Reads the text chunks of a PNG without decoding the image.
///
/// All three flavours are read, and they all turn up in the wild: Ghostscript
/// writes `Thumb::URI` as a compressed `zTXt`, so a reader that only understands
/// plain `tEXt` sees a perfectly good thumbnail as unusable and regenerates it
/// on every visit.
///
/// Chunk payloads are skipped with a seek, and the walk stops at the first
/// `IDAT`: everything the thumbnail standard puts in a file is metadata that
/// precedes the pixels.
fn read_png_text(path: &Path) -> io::Result<HashMap<String, String>> {
    const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    /// A keyword is at most 79 bytes and the values here are URIs; anything
    /// larger is not thumbnail metadata and is not worth buffering.
    const MAX_CHUNK: u32 = 64 * 1024;

    let mut reader = BufReader::new(File::open(path)?);
    let mut signature = [0_u8; 8];
    reader.read_exact(&mut signature)?;
    if signature != SIGNATURE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "no es un PNG",
        ));
    }

    let mut text = HashMap::new();
    loop {
        let mut header = [0_u8; 8];
        if reader.read_exact(&mut header).is_err() {
            break;
        }
        let length = u32::from_be_bytes([header[0], header[1], header[2], header[3]]);
        let kind = &header[4..8];

        if kind == b"IDAT" || kind == b"IEND" {
            break;
        }

        let textual = matches!(kind, b"tEXt" | b"zTXt" | b"iTXt");
        if textual && length <= MAX_CHUNK {
            let mut payload = vec![0_u8; length as usize];
            reader.read_exact(&mut payload)?;
            let parsed = match kind {
                b"tEXt" => split_text_chunk(&payload),
                b"zTXt" => split_compressed_text_chunk(&payload),
                _ => split_international_text_chunk(&payload),
            };
            if let Some((keyword, value)) = parsed {
                text.insert(keyword, value);
            }
        } else {
            reader.seek(SeekFrom::Current(i64::from(length)))?;
        }
        // The four CRC bytes that close every chunk.
        reader.seek(SeekFrom::Current(4))?;
    }

    Ok(text)
}

/// A `tEXt` payload is `keyword\0value`, both Latin-1.
fn split_text_chunk(payload: &[u8]) -> Option<(String, String)> {
    let separator = payload.iter().position(|byte| *byte == 0)?;
    let keyword = latin1(&payload[..separator]);
    let value = latin1(&payload[separator + 1..]);
    Some((keyword, value))
}

/// A `zTXt` payload is `keyword\0method` followed by the deflated text. Only
/// method 0 (zlib) is defined.
fn split_compressed_text_chunk(payload: &[u8]) -> Option<(String, String)> {
    let separator = payload.iter().position(|byte| *byte == 0)?;
    let keyword = latin1(&payload[..separator]);
    let method = *payload.get(separator + 1)?;
    if method != 0 {
        return None;
    }
    let value = latin1(&inflate(payload.get(separator + 2..)?)?);
    Some((keyword, value))
}

/// An `iTXt` payload is `keyword\0flag method language\0translated\0text`,
/// with the text in UTF-8 and deflated when the flag is set.
fn split_international_text_chunk(payload: &[u8]) -> Option<(String, String)> {
    let separator = payload.iter().position(|byte| *byte == 0)?;
    let keyword = latin1(&payload[..separator]);
    let compressed = *payload.get(separator + 1)? != 0;
    let method = *payload.get(separator + 2)?;

    // Two more null-terminated fields —language tag and translated keyword—
    // stand between the header and the text.
    let mut rest = payload.get(separator + 3..)?;
    for _ in 0..2 {
        let end = rest.iter().position(|byte| *byte == 0)?;
        rest = rest.get(end + 1..)?;
    }

    let bytes = if compressed {
        if method != 0 {
            return None;
        }
        inflate(rest)?
    } else {
        rest.to_vec()
    };
    Some((keyword, String::from_utf8(bytes).ok()?))
}

/// Undoes zlib compression, refusing anything implausibly large.
///
/// The bound matters: a chunk of a few bytes can expand to gigabytes, and this
/// runs over files the user merely browsed past.
fn inflate(compressed: &[u8]) -> Option<Vec<u8>> {
    use std::io::Read as _;

    const MAX_DECOMPRESSED: u64 = 256 * 1024;
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(compressed)
        .take(MAX_DECOMPRESSED)
        .read_to_end(&mut out)
        .ok()?;
    Some(out)
}

/// PNG text is Latin-1, where every byte is its own code point.
fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| *byte as char).collect()
}

/// Writes a thumbnail with the provenance the standard requires.
fn write_thumbnail(
    path: &Path,
    image: &image::RgbaImage,
    source: &Path,
    modified: SystemTime,
) -> Result<(), ThumbnailError> {
    let mut encoded = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut encoded, image.width(), image.height());
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        // Without these two keys the file is just a PNG in a cache directory:
        // no other application, nor Kara on the next run, can tell what it is a
        // thumbnail of or whether it is still current.
        let uri = file_uri(source);
        encoder
            .add_text_chunk("Thumb::URI".to_string(), uri)
            .map_err(|error| ThumbnailError::Decode(error.to_string()))?;
        if let Some(seconds) = seconds_since_epoch(modified) {
            encoder
                .add_text_chunk("Thumb::MTime".to_string(), seconds.to_string())
                .map_err(|error| ThumbnailError::Decode(error.to_string()))?;
        }
        encoder
            .add_text_chunk("Software".to_string(), "Kara".to_string())
            .map_err(|error| ThumbnailError::Decode(error.to_string()))?;

        let mut writer = encoder
            .write_header()
            .map_err(|error| ThumbnailError::Decode(error.to_string()))?;
        writer
            .write_image_data(image.as_raw())
            .map_err(|error| ThumbnailError::Decode(error.to_string()))?;
    }

    write_private_file(path, &encoded)
}

/// A failure is recorded as a minimal PNG carrying the same provenance.
fn write_failure_marker(
    path: &Path,
    source: &Path,
    modified: SystemTime,
) -> Result<(), ThumbnailError> {
    let marker = image::RgbaImage::new(1, 1);
    write_thumbnail(path, &marker, source, modified)
}

/// Writes atomically and readable only by its owner.
///
/// Both matter and both come from the standard: a half-written thumbnail must
/// never be visible to another application mid-write, and the file may reveal
/// the contents of something private, so it is mode 0600 in a 0700 directory.
fn write_private_file(path: &Path, bytes: &[u8]) -> Result<(), ThumbnailError> {
    let Some(directory) = path.parent() else {
        return Err(ThumbnailError::NoCache);
    };
    std::fs::create_dir_all(directory)?;
    std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))?;

    let temporary = directory.join(format!(
        ".{}.{}.tmp",
        path.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id()
    ));

    {
        let mut file = File::create(&temporary)?;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }

    // Rename over the target: readers either see the old file or the new one.
    std::fs::rename(&temporary, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&temporary);
    })?;
    Ok(())
}
