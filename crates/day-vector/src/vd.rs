// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! SVG → Android VectorDrawable (docs/vectors.md §per-backend emission).
//!
//! The usvg tree after normalization — paths with absolute transforms, resolved paints — maps
//! almost 1:1 onto VectorDrawable's model, which is why this is a ~200-line emitter and not a
//! project. The supported subset is deliberate: solid fills and strokes, both fill rules,
//! nested plain groups. Anything VD cannot express faithfully (gradients-without-aapt-attrs,
//! clips, masks, filters, embedded rasters, partial group opacity) returns [`Unsupported`] and
//! the caller stages a rasterized PNG ladder instead — a *loud* fallback, never wrong art.

use resvg::tiny_skia;
use resvg::usvg;

/// Why an SVG cannot become a VectorDrawable (the caller's fallback reason, shown in `day
/// build`'s status line).
#[derive(Debug)]
pub struct Unsupported(pub String);

impl std::fmt::Display for Unsupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

struct VdPath {
    data: String,
    fill: Option<(String, bool)>, // (#AARRGGBB, even_odd)
    stroke: Option<VdStroke>,
}

struct VdStroke {
    color: String,
    width: f32,
    cap: &'static str,
    join: &'static str,
}

/// Emit the tree as VectorDrawable XML, or say why it can't be one.
pub fn to_vector_drawable(tree: &usvg::Tree) -> Result<String, Unsupported> {
    let mut paths = Vec::new();
    collect(tree.root(), &mut paths)?;
    if paths.is_empty() {
        return Err(Unsupported("no drawable paths".into()));
    }
    let size = tree.size();
    let (vw, vh) = (size.width(), size.height());
    let mut xml = String::with_capacity(1024);
    xml.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
    xml.push_str("<vector xmlns:android=\"http://schemas.android.com/apk/res/android\"\n");
    // 24 dp intrinsic size — the Material icon convention; Day layouts size the view anyway.
    xml.push_str("    android:width=\"24dp\"\n    android:height=\"24dp\"\n");
    xml.push_str(&format!(
        "    android:viewportWidth=\"{}\"\n    android:viewportHeight=\"{}\">\n",
        fnum(vw),
        fnum(vh)
    ));
    for p in &paths {
        xml.push_str("  <path\n");
        xml.push_str(&format!("      android:pathData=\"{}\"", p.data));
        if let Some((color, even_odd)) = &p.fill {
            xml.push_str(&format!("\n      android:fillColor=\"{color}\""));
            if *even_odd {
                xml.push_str("\n      android:fillType=\"evenOdd\"");
            }
        }
        if let Some(s) = &p.stroke {
            xml.push_str(&format!(
                "\n      android:strokeColor=\"{}\"\n      android:strokeWidth=\"{}\"\n      android:strokeLineCap=\"{}\"\n      android:strokeLineJoin=\"{}\"",
                s.color,
                fnum(s.width),
                s.cap,
                s.join
            ));
        }
        xml.push_str("/>\n");
    }
    xml.push_str("</vector>\n");
    Ok(xml)
}

fn collect(group: &usvg::Group, out: &mut Vec<VdPath>) -> Result<(), Unsupported> {
    if group.clip_path().is_some() {
        return Err(Unsupported("clip-path".into()));
    }
    if group.mask().is_some() {
        return Err(Unsupported("mask".into()));
    }
    if !group.filters().is_empty() {
        return Err(Unsupported("filter".into()));
    }
    // Partial group opacity composites the GROUP, not each path; folding it per-path is wrong
    // for overlapping art, so it is out of the subset.
    if group.opacity().get() < 1.0 {
        return Err(Unsupported("group opacity".into()));
    }
    for node in group.children() {
        match node {
            usvg::Node::Group(g) => collect(g, out)?,
            usvg::Node::Path(p) => {
                if let Some(vp) = vd_path(p)? {
                    out.push(vp);
                }
            }
            usvg::Node::Image(_) => return Err(Unsupported("embedded raster image".into())),
            usvg::Node::Text(_) => return Err(Unsupported("text (outline it)".into())),
        }
    }
    Ok(())
}

fn vd_path(p: &usvg::Path) -> Result<Option<VdPath>, Unsupported> {
    let fill = match p.fill() {
        None => None,
        Some(f) => Some((
            solid(f.paint(), f.opacity().get())?,
            matches!(f.rule(), usvg::FillRule::EvenOdd),
        )),
    };
    let stroke = match p.stroke() {
        None => None,
        Some(s) => {
            let ts = p.abs_transform();
            // Uniform-ish scale for the stroke width (glyph transforms are translate+scale).
            let scale = ((ts.sx * ts.sy - ts.kx * ts.ky).abs()).sqrt();
            Some(VdStroke {
                color: solid(s.paint(), s.opacity().get())?,
                width: s.width().get() * scale,
                cap: match s.linecap() {
                    usvg::LineCap::Butt => "butt",
                    usvg::LineCap::Round => "round",
                    usvg::LineCap::Square => "square",
                },
                join: match s.linejoin() {
                    usvg::LineJoin::Round => "round",
                    usvg::LineJoin::Bevel => "bevel",
                    _ => "miter",
                },
            })
        }
    };
    if fill.is_none() && stroke.is_none() {
        return Ok(None);
    }
    let data = p
        .data()
        .clone()
        .transform(p.abs_transform())
        .ok_or_else(|| Unsupported("degenerate transform".into()))?;
    Ok(Some(VdPath {
        data: path_data(&data),
        fill,
        stroke,
    }))
}

/// A solid `#AARRGGBB`, or Unsupported for gradients/patterns (VD gradients need aapt inline
/// attrs — out of the subset; the raster fallback covers them).
fn solid(paint: &usvg::Paint, opacity: f32) -> Result<String, Unsupported> {
    match paint {
        usvg::Paint::Color(c) => {
            let a = (opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
            Ok(format!(
                "#{:02X}{:02X}{:02X}{:02X}",
                a, c.red, c.green, c.blue
            ))
        }
        usvg::Paint::LinearGradient(_) | usvg::Paint::RadialGradient(_) => {
            Err(Unsupported("gradient".into()))
        }
        usvg::Paint::Pattern(_) => Err(Unsupported("pattern".into())),
    }
}

/// Serialize a (transformed) tiny-skia path as SVG path grammar — exactly what
/// `android:pathData` accepts.
fn path_data(path: &tiny_skia::Path) -> String {
    use tiny_skia::PathSegment;
    let mut d = String::new();
    for seg in path.segments() {
        match seg {
            PathSegment::MoveTo(p) => d.push_str(&format!("M{},{}", fnum(p.x), fnum(p.y))),
            PathSegment::LineTo(p) => d.push_str(&format!("L{},{}", fnum(p.x), fnum(p.y))),
            PathSegment::QuadTo(c, p) => d.push_str(&format!(
                "Q{},{} {},{}",
                fnum(c.x),
                fnum(c.y),
                fnum(p.x),
                fnum(p.y)
            )),
            PathSegment::CubicTo(c1, c2, p) => d.push_str(&format!(
                "C{},{} {},{} {},{}",
                fnum(c1.x),
                fnum(c1.y),
                fnum(c2.x),
                fnum(c2.y),
                fnum(p.x),
                fnum(p.y)
            )),
            PathSegment::Close => d.push('Z'),
        }
    }
    d
}

/// Compact float: up to 3 decimals, trailing zeros trimmed (`24`, `1.5`, `0.125`).
fn fnum(v: f32) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(svg: &str) -> usvg::Tree {
        crate::parse(svg.as_bytes()).unwrap()
    }

    #[test]
    fn solid_glyph_converts() {
        let t = tree(
            "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24'>\
             <path d='M4 4h16v16H4z' fill='#102030' fill-rule='evenodd'/></svg>",
        );
        let xml = to_vector_drawable(&t).unwrap();
        assert!(xml.contains("android:viewportWidth=\"24\""));
        assert!(xml.contains("android:fillColor=\"#FF102030\""));
        assert!(xml.contains("android:fillType=\"evenOdd\""));
        assert!(xml.contains("android:pathData=\"M"));
        // Well-formed XML (roxmltree accepts it — same parser class aapt2 uses).
        roxmltree::Document::parse(&xml).unwrap();
    }

    #[test]
    fn gradient_is_refused_with_a_reason() {
        let t = tree(
            "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 10 10'>\
             <defs><linearGradient id='g'><stop offset='0' stop-color='red'/>\
             <stop offset='1' stop-color='blue'/></linearGradient></defs>\
             <rect width='10' height='10' fill='url(#g)'/></svg>",
        );
        let err = to_vector_drawable(&t).unwrap_err();
        assert!(err.0.contains("gradient"));
    }

    #[test]
    fn material_home_regular_m_converts() {
        // The real pipeline: template → extract → VectorDrawable.
        let glyph = crate::extract_variant(
            include_str!("../fixtures/home-symbol-template.svg"),
            "Regular",
            "M",
        )
        .unwrap();
        let xml = to_vector_drawable(&tree(&glyph)).unwrap();
        roxmltree::Document::parse(&xml).unwrap();
        assert!(xml.contains("fillColor"));
    }
}
