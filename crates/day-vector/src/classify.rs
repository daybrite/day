// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Source classification + SF Symbol template variant extraction (docs/vectors.md).
//!
//! An SF Symbol template (the SF Symbols app's export format, matched by third-party
//! generators) is one big annotated SVG: a `#Notes` group of documentation, a `#Guides` group
//! of caplines/baselines/margins, and a `#Symbols` group holding one child group per
//! `Weight-Scale` variant (`Ultralight-S` … `Black-L`). Day's canonical glyph for non-Apple
//! targets is ONE variant (Regular-M by default — the plan's "Regular only" decision), cut out
//! textually: usvg normalizes document structure away, so the variant's markup is sliced from
//! the original XML by byte range and re-wrapped with a tight, squared viewBox measured by a
//! probe parse. Textual slicing keeps whatever the variant contains (paths, primitives,
//! transforms) byte-for-byte.

/// What a `resource/vectors/` SVG file is. (`.symbolset` bundles are a directory form the
/// caller unpacks — their inner template SVG classifies here as [`SourceKind::SfTemplate`].)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceKind {
    /// Any ordinary SVG: the whole document is the glyph.
    Plain,
    /// An SF Symbol template: `#Symbols` with `Weight-Scale` variant groups.
    SfTemplate,
}

const WEIGHTS: [&str; 9] = [
    "Ultralight",
    "Thin",
    "Light",
    "Regular",
    "Medium",
    "Semibold",
    "Bold",
    "Heavy",
    "Black",
];

fn is_variant_id(id: &str) -> bool {
    let Some((weight, scale)) = id.rsplit_once('-') else {
        return false;
    };
    WEIGHTS.contains(&weight) && matches!(scale, "S" | "M" | "L")
}

/// Classify SVG text: an SF Symbol template, or plain art.
pub fn classify(xml: &str) -> SourceKind {
    let Ok(doc) = roxmltree::Document::parse(xml) else {
        return SourceKind::Plain; // unparseable is reported by the real parse later
    };
    let template = doc.descendants().any(|n| {
        n.has_tag_name("g")
            && n.attribute("id") == Some("Symbols")
            && n.children()
                .any(|c| c.attribute("id").is_some_and(is_variant_id))
    });
    if template {
        SourceKind::SfTemplate
    } else {
        SourceKind::Plain
    }
}

/// Cut one `Weight-Scale` variant out of an SF Symbol template as a standalone glyph SVG.
///
/// Preference order: the exact `weight-scale` requested, then the same weight at any scale,
/// then any variant at all — a sparse third-party template still yields a glyph. The output
/// viewBox is the variant's measured bounding box, padded ~6 % and squared (icon slots are
/// square; centering the short axis keeps optical alignment).
pub fn extract_variant(xml: &str, weight: &str, scale: &str) -> Result<String, String> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| format!("parse: {e}"))?;
    let symbols = doc
        .descendants()
        .find(|n| n.has_tag_name("g") && n.attribute("id") == Some("Symbols"))
        .ok_or("no #Symbols group — not an SF Symbol template")?;
    let variants: Vec<roxmltree::Node> = symbols
        .children()
        .filter(|c| c.attribute("id").is_some_and(is_variant_id))
        .collect();
    let wanted = format!("{weight}-{scale}");
    let variant = variants
        .iter()
        .find(|n| n.attribute("id") == Some(wanted.as_str()))
        .or_else(|| {
            variants
                .iter()
                .find(|n| n.attribute("id").is_some_and(|id| id.starts_with(weight)))
        })
        .or_else(|| variants.first())
        .ok_or("the #Symbols group has no variant children")?;

    let slice = &xml[variant.range()];

    // Probe parse: the slice alone, in the original viewport, measured for its bbox. The
    // original root's viewBox/width/height keep the coordinate system the slice's transforms
    // assume.
    let root = doc.root_element();
    let mut dims = String::new();
    for attr in ["viewBox", "width", "height"] {
        if let Some(v) = root.attribute(attr) {
            dims.push_str(&format!(" {attr}=\"{v}\""));
        }
    }
    let probe = format!("<svg xmlns=\"http://www.w3.org/2000/svg\"{dims}>{slice}</svg>");
    let tree = crate::parse(probe.as_bytes())?;
    let b = crate::content_bbox(&tree).ok_or("the variant renders no content")?;

    // Pad, then square around the center.
    let pad = 0.06 * b.width().max(b.height());
    let (w, h) = (b.width() + 2.0 * pad, b.height() + 2.0 * pad);
    let edge = w.max(h);
    let x = b.x() - pad - (edge - w) / 2.0;
    let y = b.y() - pad - (edge - h) / 2.0;
    Ok(format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{x} {y} {edge} {edge}\">{slice}</svg>"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEMPLATE: &str = include_str!("../fixtures/home-symbol-template.svg");

    #[test]
    fn template_is_detected_and_plain_is_not() {
        assert_eq!(classify(TEMPLATE), SourceKind::SfTemplate);
        assert_eq!(
            classify("<svg xmlns='http://www.w3.org/2000/svg'><path d='M0 0h4v4z'/></svg>"),
            SourceKind::Plain
        );
    }

    #[test]
    fn regular_m_extracts_parses_and_renders() {
        let glyph = extract_variant(TEMPLATE, "Regular", "M").unwrap();
        let tree = crate::parse(glyph.as_bytes()).unwrap();
        let png = crate::render_png(&tree, 64).unwrap();
        assert!(png.starts_with(&[0x89, b'P', b'N', b'G']));
        // Non-blank: some pixel must be opaque-ish. Decode via tiny-skia.
        let pixmap = resvg::tiny_skia::Pixmap::decode_png(&png).unwrap();
        assert!(pixmap.pixels().iter().any(|p| p.alpha() > 128));
    }

    #[test]
    fn missing_variant_falls_back_to_a_sibling() {
        // The fixture template has no Heavy/Black columns in some generators; asking for an
        // absent combination still yields a glyph via the fallback ladder.
        let glyph = extract_variant(TEMPLATE, "Black", "L");
        assert!(glyph.is_ok());
    }
}
