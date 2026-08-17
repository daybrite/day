// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! SVG → Android VectorDrawable (docs/vectors.md §per-backend emission).
//!
//! The usvg tree after normalization — paths with absolute transforms, resolved paints — maps
//! almost 1:1 onto VectorDrawable's model, which is why this is a small emitter and not a
//! project. The supported subset: solid fills and strokes, both fill rules, nested plain groups,
//! and linear/radial gradients on fill AND stroke. Anything VD cannot express faithfully (clips,
//! masks, filters, embedded rasters, partial group opacity) returns [`Unsupported`] and the
//! caller stages a rasterized PNG ladder instead — a *loud* fallback, never wrong art.
//!
//! Gradients ride `aapt:attr`, which VectorDrawable has understood since **API 24** — exactly the
//! `minSdk` the Android scaffold sets, so they cost no compatibility. usvg does the work that
//! makes this tractable: by the time the tree exists, `gradientUnits` is resolved (coordinates are
//! in user space) and `href` stop inheritance is flattened, which is the fiddly half of Android's
//! own converter.
//!
//! One limit is geometric rather than incidental. A VD gradient carries no transform: a linear one
//! is two points with its bands perpendicular to them, a radial one is a true circle. An SVG
//! `gradientTransform` (or an enclosing non-uniform transform) can shear a gradient into bands
//! that are not perpendicular, or a radial into an ellipse — neither of which VD can say. So the
//! transform is baked into the emitted coordinates only when it is a SIMILARITY (rotation, uniform
//! scale, translation); skew and non-uniform scale fall back to the raster. Android Studio's own
//! `SvgGradientNode` carries a standing TODO for the same case and emits wrong art instead.

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
    fill: Option<(VdPaint, bool)>, // (paint, even_odd)
    stroke: Option<VdStroke>,
}

struct VdStroke {
    paint: VdPaint,
    width: f32,
    cap: &'static str,
    join: &'static str,
    /// Emitted only when it differs from VD's own default of 4.
    miterlimit: Option<f32>,
}

/// What paints a fill or stroke: a flat `#AARRGGBB`, or a gradient that becomes an `aapt:attr`
/// child element rather than an attribute value.
enum VdPaint {
    Solid(String),
    Gradient(VdGradient),
}

struct VdGradient {
    /// `linear` or `radial`.
    kind: &'static str,
    /// Positional attributes in emission order — `startX`/`endY` for linear, `centerX`/
    /// `gradientRadius` for radial. Kept as a list so both shapes share one writer.
    coords: Vec<(&'static str, f32)>,
    tile: &'static str,
    /// (offset, `#AARRGGBB`).
    stops: Vec<(f32, String)>,
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
    // The aapt namespace is declared only when something uses it: a gradient is an `aapt:attr`
    // CHILD of the path, not an attribute value, and an unused namespace on every icon is noise.
    let has_gradient = paths.iter().any(|p| {
        matches!(&p.fill, Some((VdPaint::Gradient(_), _)))
            || matches!(&p.stroke, Some(s) if matches!(s.paint, VdPaint::Gradient(_)))
    });
    let mut xml = String::with_capacity(1024);
    xml.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
    xml.push_str("<vector xmlns:android=\"http://schemas.android.com/apk/res/android\"\n");
    if has_gradient {
        xml.push_str("    xmlns:aapt=\"http://schemas.android.com/aapt\"\n");
    }
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
        if let Some((paint, even_odd)) = &p.fill {
            if let VdPaint::Solid(color) = paint {
                xml.push_str(&format!("\n      android:fillColor=\"{color}\""));
            }
            if *even_odd {
                xml.push_str("\n      android:fillType=\"evenOdd\"");
            }
        }
        if let Some(s) = &p.stroke {
            if let VdPaint::Solid(color) = &s.paint {
                xml.push_str(&format!("\n      android:strokeColor=\"{color}\""));
            }
            xml.push_str(&format!(
                "\n      android:strokeWidth=\"{}\"\n      android:strokeLineCap=\"{}\"\n      android:strokeLineJoin=\"{}\"",
                fnum(s.width),
                s.cap,
                s.join
            ));
            if let Some(m) = s.miterlimit {
                xml.push_str(&format!("\n      android:strokeMiterLimit=\"{}\"", fnum(m)));
            }
        }
        // Gradients are children, so a path carrying one cannot self-close.
        let fill_grad = match &p.fill {
            Some((VdPaint::Gradient(g), _)) => Some(g),
            _ => None,
        };
        let stroke_grad = match &p.stroke {
            Some(VdStroke {
                paint: VdPaint::Gradient(g),
                ..
            }) => Some(g),
            _ => None,
        };
        if fill_grad.is_none() && stroke_grad.is_none() {
            xml.push_str("/>\n");
            continue;
        }
        xml.push_str(">\n");
        if let Some(g) = fill_grad {
            write_gradient(&mut xml, "android:fillColor", g);
        }
        if let Some(g) = stroke_grad {
            write_gradient(&mut xml, "android:strokeColor", g);
        }
        xml.push_str("  </path>\n");
    }
    xml.push_str("</vector>\n");
    Ok(xml)
}

/// One `aapt:attr` gradient child, targeting `attr` (`android:fillColor` or `android:strokeColor`).
fn write_gradient(xml: &mut String, attr: &str, g: &VdGradient) {
    xml.push_str(&format!("    <aapt:attr name=\"{attr}\">\n"));
    xml.push_str(&format!(
        "      <gradient\n          android:type=\"{}\"",
        g.kind
    ));
    for (name, v) in &g.coords {
        xml.push_str(&format!("\n          android:{name}=\"{}\"", fnum(*v)));
    }
    xml.push_str(&format!("\n          android:tileMode=\"{}\">\n", g.tile));
    for (offset, color) in &g.stops {
        xml.push_str(&format!(
            "        <item android:offset=\"{}\" android:color=\"{color}\"/>\n",
            fnum(*offset)
        ));
    }
    xml.push_str("      </gradient>\n");
    xml.push_str("    </aapt:attr>\n");
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
            paint_of(f.paint(), f.opacity().get(), p.abs_transform())?,
            matches!(f.rule(), usvg::FillRule::EvenOdd),
        )),
    };
    let stroke = match p.stroke() {
        None => None,
        Some(s) => {
            let ts = p.abs_transform();
            // Uniform-ish scale for the stroke width (glyph transforms are translate+scale).
            let scale = ((ts.sx * ts.sy - ts.kx * ts.ky).abs()).sqrt();
            let miter = s.miterlimit().get();
            Some(VdStroke {
                paint: paint_of(s.paint(), s.opacity().get(), ts)?,
                width: s.width().get() * scale,
                miterlimit: (miter - 4.0).abs().gt(&1e-4).then_some(miter),
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

/// `#AARRGGBB` for a color, a [`VdGradient`] for a gradient VD can express, else Unsupported.
///
/// `ts` is the path's absolute transform — already baked into the path data, so the gradient's
/// coordinates must travel through it too, combined with the gradient's own `gradientTransform`.
fn paint_of(
    paint: &usvg::Paint,
    opacity: f32,
    ts: tiny_skia::Transform,
) -> Result<VdPaint, Unsupported> {
    match paint {
        usvg::Paint::Color(c) => Ok(VdPaint::Solid(argb(*c, opacity))),
        usvg::Paint::LinearGradient(g) => {
            let m = ts.pre_concat(g.transform());
            if !keeps_bands_square(&m, g.x2() - g.x1(), g.y2() - g.y1()) {
                return Err(Unsupported(
                    "linear gradient sheared out of VD's perpendicular bands".into(),
                ));
            }
            let [x1, y1, x2, y2] = map_pts(m, [(g.x1(), g.y1()), (g.x2(), g.y2())]);
            Ok(VdPaint::Gradient(VdGradient {
                kind: "linear",
                coords: vec![("startX", x1), ("startY", y1), ("endX", x2), ("endY", y2)],
                tile: tile_mode(g.spread_method()),
                stops: stops_of(g.stops(), opacity)?,
            }))
        }
        usvg::Paint::RadialGradient(g) => {
            // VD's radial is a circle around one center. SVG's focal point (fx, fy) offsets the
            // color origin and has no VD equivalent, so a real focal gradient rasterizes.
            if (g.fx() - g.cx()).abs() > 1e-4 || (g.fy() - g.cy()).abs() > 1e-4 {
                return Err(Unsupported("radial gradient with a focal point".into()));
            }
            let m = ts.pre_concat(g.transform());
            // A radial IS stricter than a linear: VD can only say a circle, so anything that
            // turns one into an ellipse (non-uniform scale as much as skew) has to rasterize.
            let scale = similarity_scale(&m)
                .ok_or_else(|| Unsupported("radial gradient stretched into an ellipse".into()))?;
            let [cx, cy] = map_pts(m, [(g.cx(), g.cy()), (g.cx(), g.cy())])[..2]
                .try_into()
                .unwrap_or([0.0, 0.0]);
            Ok(VdPaint::Gradient(VdGradient {
                kind: "radial",
                coords: vec![
                    ("centerX", cx),
                    ("centerY", cy),
                    ("gradientRadius", g.r().get() * scale),
                ],
                tile: tile_mode(g.spread_method()),
                stops: stops_of(g.stops(), opacity)?,
            }))
        }
        usvg::Paint::Pattern(_) => Err(Unsupported("pattern".into())),
    }
}

fn argb(c: usvg::Color, opacity: f32) -> String {
    let a = (opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02X}{:02X}{:02X}{:02X}", a, c.red, c.green, c.blue)
}

/// Whether a linear gradient survives `m` intact: VD's color bands are always PERPENDICULAR to
/// the start→end vector, so the transform must keep them that way. The bands run along `rot90(d)`
/// in gradient space, so the test is that the two stay perpendicular after `m`.
///
/// This is deliberately weaker than [`similarity_scale`], and the difference matters: usvg
/// expresses `gradientUnits="objectBoundingBox"` AS a scale by the bounding box, so every ordinary
/// `x1/y1/x2/y2` gradient on a non-square shape arrives carrying a non-uniform transform. Judging
/// those by the similarity test would refuse the single most common gradient in existence —
/// which is exactly what it did to `day_mark` on the first run of this code.
fn keeps_bands_square(m: &tiny_skia::Transform, dx: f32, dy: f32) -> bool {
    let lin = |x: f32, y: f32| (m.sx * x + m.kx * y, m.ky * x + m.sy * y);
    let axis = lin(dx, dy);
    let band = lin(-dy, dx);
    let la = (axis.0 * axis.0 + axis.1 * axis.1).sqrt();
    let lb = (band.0 * band.0 + band.1 * band.1).sqrt();
    if la <= f32::EPSILON || lb <= f32::EPSILON {
        return false;
    }
    ((axis.0 * band.0 + axis.1 * band.1) / (la * lb)).abs() <= 1e-3
}

/// The uniform scale of `m`, or `None` when it is not a similarity (rotation + uniform scale +
/// translation). The two column vectors of a similarity are equal in length and perpendicular.
fn similarity_scale(m: &tiny_skia::Transform) -> Option<f32> {
    let (ax, ay) = (m.sx, m.ky);
    let (bx, by) = (m.kx, m.sy);
    let la = (ax * ax + ay * ay).sqrt();
    let lb = (bx * bx + by * by).sqrt();
    if la <= f32::EPSILON || lb <= f32::EPSILON {
        return None;
    }
    if ((la - lb) / la.max(lb)).abs() > 1e-3 {
        return None;
    }
    if ((ax * bx + ay * by) / (la * lb)).abs() > 1e-3 {
        return None;
    }
    Some(la)
}

fn map_pts(m: tiny_skia::Transform, pts: [(f32, f32); 2]) -> [f32; 4] {
    let mut p = [
        tiny_skia::Point::from_xy(pts[0].0, pts[0].1),
        tiny_skia::Point::from_xy(pts[1].0, pts[1].1),
    ];
    m.map_points(&mut p);
    [p[0].x, p[0].y, p[1].x, p[1].y]
}

fn tile_mode(s: usvg::SpreadMethod) -> &'static str {
    match s {
        usvg::SpreadMethod::Pad => "clamp",
        usvg::SpreadMethod::Reflect => "mirror",
        usvg::SpreadMethod::Repeat => "repeat",
    }
}

/// Gradient stops, with the fill/stroke opacity folded into each stop's alpha — VD has no
/// per-gradient alpha, and this is what Android's own converter does.
fn stops_of(stops: &[usvg::Stop], opacity: f32) -> Result<Vec<(f32, String)>, Unsupported> {
    if stops.is_empty() {
        return Err(Unsupported("gradient with no stops".into()));
    }
    let mut out: Vec<(f32, String)> = stops
        .iter()
        .map(|s| {
            (
                s.offset().get(),
                argb(s.color(), s.opacity().get() * opacity),
            )
        })
        .collect();
    // A single stop is a flat fill; VD wants a range, so give it one (Android's converter does
    // the same, warning "Gradient has only one color stop").
    if out.len() == 1 {
        out.push((1.0, out[0].1.clone()));
    }
    Ok(out)
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
    fn linear_gradient_becomes_an_aapt_attr() {
        // day_mark.svg's shape: a two-stop vertical linear gradient in objectBoundingBox units,
        // which usvg resolves to user space before we ever see it.
        let t = tree(
            "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 10 10'>\
             <defs><linearGradient id='g' x1='0' y1='1' x2='0' y2='0'>\
             <stop offset='0' stop-color='#B7410E'/>\
             <stop offset='1' stop-color='#EFA94A'/></linearGradient></defs>\
             <rect width='10' height='10' fill='url(#g)'/></svg>",
        );
        let xml = to_vector_drawable(&t).unwrap();
        assert!(xml.contains("xmlns:aapt=\"http://schemas.android.com/aapt\""));
        assert!(xml.contains("<aapt:attr name=\"android:fillColor\">"));
        assert!(xml.contains("android:type=\"linear\""));
        assert!(xml.contains("android:color=\"#FFB7410E\""));
        assert!(xml.contains("android:color=\"#FFEFA94A\""));
        assert!(xml.contains("android:tileMode=\"clamp\""));
        // A gradient path carries children, so it must NOT self-close.
        assert!(xml.contains("</path>"));
        roxmltree::Document::parse(&xml).unwrap();
    }

    #[test]
    fn radial_gradient_and_spread_method_convert() {
        let t = tree(
            "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 10 10'>\
             <defs><radialGradient id='g' spreadMethod='reflect'>\
             <stop offset='0' stop-color='#FFFFFF'/>\
             <stop offset='1' stop-color='#000000'/></radialGradient></defs>\
             <rect width='10' height='10' fill='url(#g)'/></svg>",
        );
        let xml = to_vector_drawable(&t).unwrap();
        assert!(xml.contains("android:type=\"radial\""));
        assert!(xml.contains("android:gradientRadius="));
        assert!(xml.contains("android:tileMode=\"mirror\""));
        roxmltree::Document::parse(&xml).unwrap();
    }

    #[test]
    fn stroke_gradients_convert_too() {
        let t = tree(
            "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 10 10'>\
             <defs><linearGradient id='g'><stop offset='0' stop-color='red'/>\
             <stop offset='1' stop-color='blue'/></linearGradient></defs>\
             <rect x='1' y='1' width='8' height='8' fill='none' \
             stroke='url(#g)' stroke-width='2'/></svg>",
        );
        let xml = to_vector_drawable(&t).unwrap();
        assert!(xml.contains("<aapt:attr name=\"android:strokeColor\">"));
        roxmltree::Document::parse(&xml).unwrap();
    }

    #[test]
    fn skewed_gradient_still_rasterizes() {
        // VD bands are perpendicular to start→end; a skew cannot be said, so it must refuse
        // rather than emit art that is subtly wrong.
        let t = tree(
            "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 10 10'>\
             <defs><linearGradient id='g' gradientTransform='matrix(1,0,0.7,1,0,0)'>\
             <stop offset='0' stop-color='red'/>\
             <stop offset='1' stop-color='blue'/></linearGradient></defs>\
             <rect width='10' height='10' fill='url(#g)'/></svg>",
        );
        let err = to_vector_drawable(&t).unwrap_err();
        assert!(err.0.contains("sheared"), "got {}", err.0);
    }

    #[test]
    fn a_rotated_gradient_is_baked_rather_than_refused() {
        // A rotation IS a similarity, so the endpoints travel through it and the gradient stays
        // a vector — this is the case that would be lost by refusing every gradientTransform.
        let t = tree(
            "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 10 10'>\
             <defs><linearGradient id='g' gradientTransform='rotate(45 5 5)'>\
             <stop offset='0' stop-color='red'/>\
             <stop offset='1' stop-color='blue'/></linearGradient></defs>\
             <rect width='10' height='10' fill='url(#g)'/></svg>",
        );
        let xml = to_vector_drawable(&t).unwrap();
        assert!(xml.contains("android:type=\"linear\""));
        roxmltree::Document::parse(&xml).unwrap();
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
