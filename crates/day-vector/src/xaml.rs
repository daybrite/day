// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! SVG → XAML geometry (docs/vectors.md §per-backend emission).
//!
//! Windows' `Path`/`PathIcon` take a `Geometry`, and XAML's geometry mini-language IS the SVG
//! path grammar — so the usvg tree, whose paths already carry absolute transforms and resolved
//! paints, converts about as directly as it does to a VectorDrawable next door in `vd.rs`. The
//! supported subset matches that emitter's for the same reason: solid fills and strokes, both
//! fill rules, nested plain groups; anything XAML cannot express as one flat shape list returns
//! [`Unsupported`] and the caller stages only the raster, a loud fallback and never wrong art.
//!
//! Emitted at BUILD time rather than parsed in the backend: day-xaml would otherwise carry
//! resvg/usvg into every Windows binary to read a handful of icons, and Android already sets
//! the precedent of converting once, in the CLI, into the form the toolkit loads natively.
//!
//! Keeping geometry (not a rasterized or pre-colored image) is what lets the backend compose a
//! tint at runtime: the color is a brush on the shape, applied when the glyph is realized, so
//! one staged glyph serves every tint at every size without a second asset.

use resvg::tiny_skia;
use resvg::usvg;

use crate::vd::Unsupported;

/// One flat shape: geometry plus the paints it was authored with. `fill`/`stroke` are
/// `#AARRGGBB`; a backend asked for a tint substitutes its own brush and ignores both.
pub struct XamlShape {
    /// SVG path grammar, which XAML's `Geometry` mini-language parses unchanged.
    pub data: String,
    pub fill: Option<String>,
    pub even_odd: bool,
    pub stroke: Option<String>,
    pub stroke_width: f32,
    pub cap: &'static str,
    pub join: &'static str,
}

/// A glyph as XAML geometry: the shapes plus the viewport they are drawn in, which the backend
/// needs to scale them into whatever box the layout gives (a `Viewbox`, or a scale transform on
/// the geometry for the fixed icon slots).
pub struct XamlGeometry {
    pub width: f32,
    pub height: f32,
    pub shapes: Vec<XamlShape>,
}

/// The line-oriented form staged under `build/day/vectors/xaml/`, parsed by day-xaml.
///
/// ```text
/// V <width> <height>
/// P <fill|-> <evenOdd 0|1> <stroke|-> <strokeWidth> <cap> <join> \t <path data>
/// ```
///
/// Tab-separated payload so the path data — which contains spaces and commas — needs no
/// escaping, and one line per shape so a reader can stop at the first line it does not know.
impl XamlGeometry {
    pub fn to_spec(&self) -> String {
        let mut s = format!("V {} {}\n", fnum(self.width), fnum(self.height));
        for sh in &self.shapes {
            s.push_str(&format!(
                "P {} {} {} {} {} {}\t{}\n",
                sh.fill.as_deref().unwrap_or("-"),
                u8::from(sh.even_odd),
                sh.stroke.as_deref().unwrap_or("-"),
                fnum(sh.stroke_width),
                sh.cap,
                sh.join,
                sh.data
            ));
        }
        s
    }
}

/// Convert the tree to XAML geometry, or say why it can't be.
pub fn to_xaml_geometry(tree: &usvg::Tree) -> Result<XamlGeometry, Unsupported> {
    let mut shapes = Vec::new();
    collect(tree.root(), &mut shapes)?;
    if shapes.is_empty() {
        return Err(Unsupported("no drawable paths".into()));
    }
    let size = tree.size();
    Ok(XamlGeometry {
        width: size.width(),
        height: size.height(),
        shapes,
    })
}

fn collect(group: &usvg::Group, out: &mut Vec<XamlShape>) -> Result<(), Unsupported> {
    if group.clip_path().is_some() {
        return Err(Unsupported("clip-path".into()));
    }
    if group.mask().is_some() {
        return Err(Unsupported("mask".into()));
    }
    if !group.filters().is_empty() {
        return Err(Unsupported("filter".into()));
    }
    // Partial group opacity composites the GROUP, not each shape — folding it into per-shape
    // alpha is wrong wherever the art overlaps itself, so it stays out of the subset.
    if group.opacity().get() < 1.0 {
        return Err(Unsupported("group opacity".into()));
    }
    for node in group.children() {
        match node {
            usvg::Node::Group(g) => collect(g, out)?,
            usvg::Node::Path(p) => {
                if let Some(shape) = shape(p)? {
                    out.push(shape);
                }
            }
            usvg::Node::Image(_) => return Err(Unsupported("embedded raster image".into())),
            usvg::Node::Text(_) => return Err(Unsupported("text (outline it)".into())),
        }
    }
    Ok(())
}

fn shape(p: &usvg::Path) -> Result<Option<XamlShape>, Unsupported> {
    let (fill, even_odd) = match p.fill() {
        None => (None, false),
        Some(f) => (
            Some(solid(f.paint(), f.opacity().get())?),
            matches!(f.rule(), usvg::FillRule::EvenOdd),
        ),
    };
    let mut stroke_width = 0.0;
    let mut cap = "Flat";
    let mut join = "Miter";
    let stroke = match p.stroke() {
        None => None,
        Some(s) => {
            let ts = p.abs_transform();
            // Uniform-ish scale for the stroke width (glyph transforms are translate+scale) —
            // the geometry below is baked into absolute coordinates, so the width must follow.
            let scale = ((ts.sx * ts.sy - ts.kx * ts.ky).abs()).sqrt();
            stroke_width = s.width().get() * scale;
            // XAML's PenLineCap/PenLineJoin names, not SVG's.
            cap = match s.linecap() {
                usvg::LineCap::Butt => "Flat",
                usvg::LineCap::Round => "Round",
                usvg::LineCap::Square => "Square",
            };
            join = match s.linejoin() {
                usvg::LineJoin::Round => "Round",
                usvg::LineJoin::Bevel => "Bevel",
                _ => "Miter",
            };
            Some(solid(s.paint(), s.opacity().get())?)
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
    Ok(Some(XamlShape {
        data: path_data(&data),
        fill,
        even_odd,
        stroke,
        stroke_width,
        cap,
        join,
    }))
}

/// A solid `#AARRGGBB`. Gradients and patterns are out of the subset — a `Path` could carry a
/// gradient brush, but the staged spec is one flat shape list and stop data does not belong in
/// it; the raster fallback covers that art.
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

/// Serialize a (transformed) tiny-skia path as XAML's geometry mini-language.
///
/// Only `M`/`L`/`C`/`Z` are emitted. The grammar is SVG's, but not every SVG command survives
/// XAML's parser, and a command it rejects fails the WHOLE geometry — the glyph then silently
/// falls back to the raster, which is the least debuggable outcome available. Quadratics are the
/// case that matters (Material's glyphs are full of them), so they are ELEVATED to cubics rather
/// than emitted as `Q`: exact, not an approximation, since a quadratic is the cubic with
/// C1 = P0 + ⅔(Q−P0) and C2 = P2 + ⅔(Q−P2).
fn path_data(path: &tiny_skia::Path) -> String {
    use tiny_skia::PathSegment;
    let mut d = String::new();
    // The quadratic elevation needs the segment's start, which the segment itself doesn't carry.
    let (mut cx, mut cy) = (0.0f32, 0.0f32);
    // `Z` returns to the subpath's start, so that has to be tracked too or the point after a
    // close is wrong for any curve following it.
    let (mut sx, mut sy) = (0.0f32, 0.0f32);
    for seg in path.segments() {
        match seg {
            PathSegment::MoveTo(p) => {
                d.push_str(&format!("M{},{}", fnum(p.x), fnum(p.y)));
                (cx, cy) = (p.x, p.y);
                (sx, sy) = (p.x, p.y);
            }
            PathSegment::LineTo(p) => {
                d.push_str(&format!("L{},{}", fnum(p.x), fnum(p.y)));
                (cx, cy) = (p.x, p.y);
            }
            PathSegment::QuadTo(c, p) => {
                let c1 = (cx + 2.0 / 3.0 * (c.x - cx), cy + 2.0 / 3.0 * (c.y - cy));
                let c2 = (p.x + 2.0 / 3.0 * (c.x - p.x), p.y + 2.0 / 3.0 * (c.y - p.y));
                d.push_str(&format!(
                    "C{},{} {},{} {},{}",
                    fnum(c1.0),
                    fnum(c1.1),
                    fnum(c2.0),
                    fnum(c2.1),
                    fnum(p.x),
                    fnum(p.y)
                ));
                (cx, cy) = (p.x, p.y);
            }
            PathSegment::CubicTo(c1, c2, p) => {
                d.push_str(&format!(
                    "C{},{} {},{} {},{}",
                    fnum(c1.x),
                    fnum(c1.y),
                    fnum(c2.x),
                    fnum(c2.y),
                    fnum(p.x),
                    fnum(p.y)
                ));
                (cx, cy) = (p.x, p.y);
            }
            PathSegment::Close => {
                d.push('Z');
                (cx, cy) = (sx, sy);
            }
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
        crate::parse(svg.as_bytes()).expect("parse")
    }

    /// The shape of a staged glyph: one filled path, no stroke, viewport carried through so the
    /// backend can scale it into any box.
    #[test]
    fn filled_glyph_becomes_one_shape() {
        let g = to_xaml_geometry(&tree(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="48" height="48" viewBox="0 0 48 48"><path d="M0 0h48v48H0z"/></svg>"#,
        ))
        .expect("geometry");
        assert_eq!((g.width, g.height), (48.0, 48.0));
        assert_eq!(g.shapes.len(), 1);
        let s = &g.shapes[0];
        assert_eq!(s.fill.as_deref(), Some("#FF000000"));
        assert!(s.stroke.is_none());
        assert!(s.data.starts_with('M'), "path grammar: {}", s.data);
    }

    /// A negative-origin viewBox (the Material convention, `0 -960 960 960`) must land in the
    /// viewport's own coordinates — otherwise every glyph draws outside the box it is scaled to.
    #[test]
    fn negative_origin_viewbox_is_normalized() {
        let g = to_xaml_geometry(&tree(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="48" height="48" viewBox="0 -960 960 960"><path d="M100-800h300v300h-300z"/></svg>"#,
        ))
        .expect("geometry");
        for seg in g.data_numbers() {
            assert!(
                (0.0..=g.width.max(g.height) as f64 + 0.01).contains(&seg.abs()),
                "coordinate {seg} outside the {}x{} viewport",
                g.width,
                g.height
            );
        }
    }

    /// Only M/L/C/Z reach XAML. A `Q` slipping through fails the whole geometry in the parser
    /// and drops the glyph to its raster — the regression this guards is invisible on screen
    /// (the icon still draws, just not from geometry), so it is asserted on the data instead.
    #[test]
    fn quadratics_are_elevated_to_cubics() {
        // `a` is an arc and `q` a quadratic — the shapes Material's glyphs are built from.
        let g = to_xaml_geometry(&tree(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="48" height="48" viewBox="0 0 48 48"><path d="M4 24q4-20 20-20a20 20 0 0 1 20 20z"/></svg>"#,
        ))
        .expect("geometry");
        let data = &g.shapes[0].data;
        assert!(
            !data.contains(['Q', 'q', 'T', 't', 'S', 's', 'A', 'a', 'H', 'h', 'V', 'v']),
            "only M/L/C/Z may be emitted: {data}"
        );
        assert!(
            data.contains('C'),
            "the curve must survive as a cubic: {data}"
        );
    }

    /// Elevation must be exact, not a redraw: the cubic has to pass through the quadratic's own
    /// endpoints, or every curved glyph shifts a little.
    #[test]
    fn elevated_curve_keeps_its_endpoints() {
        let g = to_xaml_geometry(&tree(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100" viewBox="0 0 100 100"><path d="M10 10Q50 90 90 10Z"/></svg>"#,
        ))
        .expect("geometry");
        let d = &g.shapes[0].data;
        assert!(d.starts_with("M10,10C"), "starts at the move: {d}");
        // Cubic control points for M10,10 Q50,90 90,10 are (36.667,63.333) and (63.333,63.333).
        assert!(
            d.contains("C36.667,63.333 63.333,63.333 90,10"),
            "exact elevation: {d}"
        );
    }

    /// Art outside the subset must FAIL rather than silently drop the part XAML can't draw.
    #[test]
    fn gradient_is_unsupported() {
        let err = to_xaml_geometry(&tree(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10" viewBox="0 0 10 10"><defs><linearGradient id="g"><stop offset="0" stop-color="#000"/><stop offset="1" stop-color="#fff"/></linearGradient></defs><path d="M0 0h10v10H0z" fill="url(#g)"/></svg>"##,
        ));
        assert!(err.is_err(), "gradient must not convert");
    }

    /// The staged spec round-trips through one line per shape, with the data last so it can
    /// hold spaces and commas without escaping.
    #[test]
    fn spec_puts_data_after_a_tab() {
        let g = to_xaml_geometry(&tree(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8" viewBox="0 0 8 8"><path d="M0 0h8v8H0z"/></svg>"#,
        ))
        .expect("geometry");
        let spec = g.to_spec();
        let mut lines = spec.lines();
        assert_eq!(lines.next().unwrap(), "V 8 8");
        let p = lines.next().unwrap();
        let (head, data) = p.split_once('\t').expect("tab-separated data");
        assert!(head.starts_with("P #FF000000 0 - "), "head: {head}");
        assert!(data.starts_with('M'), "data: {data}");
    }

    impl XamlGeometry {
        /// Every numeric literal in the emitted path data — a coordinate-range assertion helper.
        fn data_numbers(&self) -> Vec<f64> {
            self.shapes
                .iter()
                .flat_map(|s| {
                    s.data
                        .split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
                        .filter(|t| !t.is_empty())
                        .filter_map(|t| t.parse::<f64>().ok())
                        .collect::<Vec<_>>()
                })
                .collect()
        }
    }
}
