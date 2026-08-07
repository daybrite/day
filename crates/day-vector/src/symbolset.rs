//! Plain SVG → generated SF Symbol template + `.symbolset` Contents.json (the plan's
//! "auto-symbolset on Apple" decision): a monochrome glyph gains native symbol behavior
//! (tinting, baseline alignment) by being wrapped into the template form Xcode accepts.
//!
//! Apple's minimum template is the `Ultralight-S` / `Regular-S` / `Black-S` trio — the other
//! 24 weight/scale variants are derived by the system. The generated document reproduces the
//! geometry every real template carries (measured from SF Symbols app exports): a 3300×2200
//! artboard, the S-row capline at y 625.541 and baseline at y 696 (cap height 70.459), the
//! `H-reference` glyph, and per-weight column centres. The input glyph is embedded three times
//! (identical art per weight — weight differentiation needs true per-weight sources, a
//! template-form master's job), scaled so its box spans the cap height and centred on each
//! column.

const ARTBOARD_W: f32 = 3300.0;
const ARTBOARD_H: f32 = 2200.0;
const CAPLINE_S: f32 = 625.541;
const BASELINE_S: f32 = 696.0;
const COL_ULTRALIGHT: f32 = 559.711;
const COL_REGULAR: f32 = 1449.84;
const COL_BLACK: f32 = 2933.4;
/// The `H-reference` letterform every template carries (copied from the SF export format).
const H_REFERENCE: &str = "M 54.9316 0 L 57.666 0 L 30.5664 -70.459 L 28.0762 -70.459 L 0.976562 0 L 3.66211 0 L 12.9395 -24.4629 L 45.7031 -24.4629 Z M 29.1992 -67.0898 L 29.4434 -67.0898 L 44.8242 -26.709 L 13.8184 -26.709 Z";

/// Wrap a standalone glyph SVG into `(template_svg, contents_json)` — the two files of a
/// `.symbolset` bundle (`<name>.svg` + `Contents.json`).
pub fn wrap_symbolset(glyph_svg: &str, name: &str) -> Result<(String, String), String> {
    let doc = roxmltree::Document::parse(glyph_svg).map_err(|e| format!("glyph parse: {e}"))?;
    let root = doc.root_element();
    // The glyph's coordinate box: viewBox, else width/height at origin.
    let (vbx, vby, vbw, vbh) = match root.attribute("viewBox") {
        Some(vb) => {
            let v: Vec<f32> = vb
                .split([' ', ','])
                .filter(|s| !s.is_empty())
                .filter_map(|s| s.parse().ok())
                .collect();
            if v.len() != 4 {
                return Err("glyph viewBox is not 4 numbers".into());
            }
            (v[0], v[1], v[2], v[3])
        }
        None => {
            let dim = |a: &str| {
                root.attribute(a)
                    .and_then(|s| s.trim_end_matches("px").parse::<f32>().ok())
            };
            match (dim("width"), dim("height")) {
                (Some(w), Some(h)) => (0.0, 0.0, w, h),
                _ => return Err("glyph has neither viewBox nor width/height".into()),
            }
        }
    };
    if vbw <= 0.0 || vbh <= 0.0 {
        return Err("glyph has a zero-sized box".into());
    }
    // Inner markup: everything between the root <svg …> tag and its close.
    let open_end = glyph_svg
        .find("<svg")
        .and_then(|at| glyph_svg[at..].find('>').map(|o| at + o + 1))
        .ok_or("no <svg> root")?;
    let close = glyph_svg.rfind("</svg>").ok_or("no </svg> close")?;
    let inner = &glyph_svg[open_end..close];

    let cap = BASELINE_S - CAPLINE_S;
    let scale = cap / vbh.max(vbw); // span the cap height, square glyphs exactly
    let variant = |id: &str, col: f32| {
        let tx = col - scale * vbw / 2.0;
        format!(
            "<g id=\"{id}\" transform=\"translate({tx} {CAPLINE_S}) scale({scale}) translate({nx} {ny})\">{inner}</g>",
            nx = -vbx,
            ny = -vby,
        )
    };
    let template = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" version=\"1.1\" width=\"{ARTBOARD_W}\" height=\"{ARTBOARD_H}\">\
         <g id=\"Notes\"><rect id=\"artboard\" fill=\"white\" width=\"{ARTBOARD_W}\" height=\"{ARTBOARD_H}\" x=\"0\" y=\"0\"/>\
         <text id=\"template-version\" style=\"text-anchor:end;font-size:13px\" transform=\"matrix(1 0 0 1 3036 1933)\">Template v.1.0</text></g>\
         <g id=\"Guides\">\
         <g id=\"H-reference\" style=\"fill:#27AAE1;stroke:none;\" transform=\"matrix(1 0 0 1 339 {BASELINE_S})\"><path d=\"{H_REFERENCE}\"/></g>\
         <line id=\"Baseline-S\" style=\"fill:none;stroke:#27AAE1;stroke-width:0.5\" x1=\"263\" x2=\"3036\" y1=\"{BASELINE_S}\" y2=\"{BASELINE_S}\"/>\
         <line id=\"Capline-S\" style=\"fill:none;stroke:#27AAE1;stroke-width:0.5\" x1=\"263\" x2=\"3036\" y1=\"{CAPLINE_S}\" y2=\"{CAPLINE_S}\"/>\
         </g>\
         <g id=\"Symbols\">{u}{r}{b}</g></svg>",
        u = variant("Ultralight-S", COL_ULTRALIGHT),
        r = variant("Regular-S", COL_REGULAR),
        b = variant("Black-S", COL_BLACK),
    );
    let contents = format!(
        "{{\n  \"info\" : {{ \"author\" : \"day\", \"version\" : 1 }},\n  \"symbols\" : [\n    {{ \"filename\" : \"{name}.svg\", \"idiom\" : \"universal\" }}\n  ]\n}}\n"
    );
    Ok((template, contents))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SourceKind;

    const GLYPH: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\"><path d=\"M4 4h16v16H4z\"/></svg>";

    #[test]
    fn wrapped_glyph_is_a_template_and_extracts_back() {
        let (template, contents) = wrap_symbolset(GLYPH, "square").unwrap();
        assert_eq!(crate::classify(&template), SourceKind::SfTemplate);
        assert!(contents.contains("\"square.svg\""));
        // Round-trip: the Regular-S variant extracts and renders non-blank.
        let glyph = crate::extract_variant(&template, "Regular", "S").unwrap();
        let tree = crate::parse(glyph.as_bytes()).unwrap();
        let png = crate::render_png(&tree, 32).unwrap();
        let pixmap = resvg::tiny_skia::Pixmap::decode_png(&png).unwrap();
        assert!(pixmap.pixels().iter().any(|p| p.alpha() > 128));
    }

    #[test]
    fn extracted_variant_fills_its_padded_box() {
        // The extracted glyph is measured in CANVAS coordinates (the viewBox maps to origin):
        // the content must sit inside the squared box and span most of it — proving the
        // wrap-then-extract geometry (cap-height scale, column centring, padding) holds.
        let (template, _) = wrap_symbolset(GLYPH, "square").unwrap();
        let glyph = crate::extract_variant(&template, "Regular", "S").unwrap();
        let t2 = crate::parse(glyph.as_bytes()).unwrap();
        let size = t2.size();
        let b = crate::content_bbox(&t2).unwrap();
        assert!(b.top() >= -1.0 && b.left() >= -1.0, "content outside box");
        assert!(
            b.bottom() <= size.height() + 1.0 && b.right() <= size.width() + 1.0,
            "content overflows box"
        );
        assert!(
            b.height() >= 0.8 * size.height(),
            "glyph does not fill its box: {} of {}",
            b.height(),
            size.height()
        );
    }
}
