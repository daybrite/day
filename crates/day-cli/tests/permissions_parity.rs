//! The declaration table in `day_build::permissions` names each permission's Rust variant so
//! `day lint` can map `Permission::Camera` in an app's source back to a `Day.toml` declaration.
//! That spelling is a duplicate of the real enum in `day-part-permissions`, and a rename on either
//! side would silently break the lint rather than fail a build — so this test pins them together.
//!
//! It reads the part's source rather than `include_str!`ing it: an `include_str!` across package
//! boundaries breaks `cargo publish` (the file is not in day-cli's package), and a checkout that
//! doesn't contain the part — a published crate's own test run — simply skips.

use std::path::PathBuf;

fn part_lib_rs() -> Option<String> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../parts/day-part-permissions/src/lib.rs");
    std::fs::read_to_string(path).ok()
}

/// Extract the `Permission` enum's variant names from the part's source.
fn declared_variants(src: &str) -> Vec<String> {
    let Some(start) = src.find("pub enum Permission {") else {
        return Vec::new();
    };
    let body = &src[start..];
    let Some(end) = body.find("\n}") else {
        return Vec::new();
    };
    body[..end]
        .lines()
        .skip(1)
        .map(str::trim)
        // Skip doc comments, attributes, and the `Raw(&'static str)` escape hatch, which has no
        // declaration to map to.
        .filter(|l| {
            !l.is_empty() && !l.starts_with("//") && !l.starts_with('#') && !l.starts_with("Raw(")
        })
        .filter_map(|l| l.strip_suffix(','))
        .map(str::to_string)
        .collect()
}

#[test]
fn every_table_variant_exists_in_the_part() {
    let Some(src) = part_lib_rs() else {
        return; // not a full workspace checkout
    };
    let variants = declared_variants(&src);
    assert!(
        !variants.is_empty(),
        "could not parse the Permission enum — has its shape changed?"
    );
    for spec in day_build::permissions::ALL {
        assert!(
            variants.iter().any(|v| v == spec.variant),
            "day_build::permissions names variant {:?} for {:?}, but day-part-permissions has no \
             such Permission variant (it has: {:?})",
            spec.variant,
            spec.name,
            variants
        );
    }
}

/// The other direction: a portable permission the runtime can ask for, but that the CLI cannot
/// declare, is the iOS-crash trap this whole pipeline exists to close.
#[test]
fn every_part_variant_has_a_declaration_row() {
    let Some(src) = part_lib_rs() else {
        return;
    };
    for variant in declared_variants(&src) {
        assert!(
            day_build::permissions::find_variant(&variant).is_some(),
            "day-part-permissions can request Permission::{variant}, but day_build::permissions \
             has no row for it — an app declaring it in Day.toml would get no manifest entry, and \
             on iOS that is a crash on first use"
        );
    }
}
