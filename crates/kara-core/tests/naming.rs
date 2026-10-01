//! Splitting a name into body and extension, and finding a free name for
//! «Conservar ambos». Both sit under copy/paste, so a mistake here shows up as a
//! file overwritten or a name mangled.

use std::collections::HashSet;

use kara_core::{split_name, unique_name};

#[test]
fn a_plain_extension_is_split_off() {
    assert_eq!(split_name("report.pdf"), ("report", Some("pdf")));
    assert_eq!(split_name("a.b.c"), ("a.b", Some("c")));
}

#[test]
fn compound_extensions_stay_together_whatever_the_case() {
    assert_eq!(split_name("backup.tar.gz"), ("backup", Some("tar.gz")));
    assert_eq!(split_name("BACKUP.TAR.GZ"), ("BACKUP", Some("TAR.GZ")));
    assert_eq!(split_name("src.tar.xz"), ("src", Some("tar.xz")));
    // `.v2.pdf` is not compound: nothing in the name says so, and the list is explicit.
    assert_eq!(split_name("report.v2.pdf"), ("report.v2", Some("pdf")));
}

#[test]
fn a_name_that_is_only_a_compound_extension_has_a_body_of_its_own() {
    // `.tar.gz` by itself is a hidden file called "tar" + "gz", not an empty body.
    assert_eq!(split_name(".tar.gz"), (".tar", Some("gz")));
}

#[test]
fn dotfiles_and_trailing_dots_have_no_extension() {
    assert_eq!(split_name(".bashrc"), (".bashrc", None));
    assert_eq!(split_name("archive."), ("archive.", None));
    assert_eq!(split_name("README"), ("README", None));
    assert_eq!(split_name(""), ("", None));
}

#[test]
fn multibyte_names_split_on_character_boundaries() {
    assert_eq!(split_name("ñandú.tar.gz"), ("ñandú", Some("tar.gz")));
    assert_eq!(split_name("日本語.txt"), ("日本語", Some("txt")));
}

fn taken(names: &[&str]) -> impl Fn(&str) -> bool {
    let set: HashSet<String> = names.iter().map(|n| (*n).to_string()).collect();
    move |candidate| set.contains(candidate)
}

#[test]
fn a_free_name_is_returned_untouched() {
    assert_eq!(unique_name("a.txt", taken(&[])), "a.txt");
    assert_eq!(unique_name("a.txt", taken(&["b.txt"])), "a.txt");
}

#[test]
fn the_number_goes_before_the_extension_and_counts_from_two() {
    assert_eq!(unique_name("a.txt", taken(&["a.txt"])), "a (2).txt");
    assert_eq!(unique_name("a.txt", taken(&["a.txt", "a (2).txt"])), "a (3).txt");
}

#[test]
fn it_skips_over_gaps_it_finds_taken() {
    let busy = ["a.txt", "a (2).txt", "a (3).txt", "a (4).txt"];
    assert_eq!(unique_name("a.txt", taken(&busy)), "a (5).txt");
    // A free slot below a taken one is still found first.
    assert_eq!(unique_name("a.txt", taken(&["a.txt", "a (3).txt"])), "a (2).txt");
}

#[test]
fn compound_extensions_and_extensionless_names_get_the_number_in_the_right_place() {
    assert_eq!(unique_name("x.tar.gz", taken(&["x.tar.gz"])), "x (2).tar.gz");
    assert_eq!(unique_name("Makefile", taken(&["Makefile"])), "Makefile (2)");
    assert_eq!(unique_name(".bashrc", taken(&[".bashrc"])), ".bashrc (2)");
}

#[test]
fn an_already_numbered_name_is_numbered_again_not_reinterpreted() {
    // Documented decision: the suffix is always added to the original.
    assert_eq!(unique_name("a (2).txt", taken(&["a (2).txt"])), "a (2) (2).txt");
}

#[test]
fn a_very_long_name_is_shortened_in_the_body_never_in_the_number_or_extension() {
    let long = format!("{}.pdf", "x".repeat(255 - 4));
    assert_eq!(long.len(), 255);
    let out = unique_name(&long, taken(&[&long]));
    assert!(out.len() <= 255, "{} bytes", out.len());
    assert!(out.ends_with(" (2).pdf"), "{out}");
}

#[test]
fn shortening_never_cuts_a_multibyte_character_in_half() {
    // 3-byte characters: the cut point will land inside one unless it is moved.
    let long = format!("{}.txt", "日".repeat(84)); // 252 + 4 = 256 bytes before the number
    let out = unique_name(&long, taken(&[&long]));
    assert!(out.len() <= 255);
    assert!(out.ends_with(" (2).txt"));
    // A name that was cut mid-character would not even be a valid `String`, so
    // the real check is that every remaining character is whole.
    assert!(out.trim_end_matches(" (2).txt").chars().all(|c| c == '日'));
}
