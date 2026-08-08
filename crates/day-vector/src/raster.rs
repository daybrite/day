// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! SVG parsing + PNG rasterization — the render path both `day icon` and vector staging share.

use resvg::tiny_skia;
use resvg::usvg;

/// Parse SVG bytes into a usvg tree. Text is not compiled in (see the crate docs): a `<text>`
/// element parses but renders as nothing, so callers that care (day-cli) probe the raw XML for
/// `<text` and refuse with an "outline your text" message before calling this.
pub fn parse(data: &[u8]) -> Result<usvg::Tree, String> {
    usvg::Tree::from_data(data, &usvg::Options::default()).map_err(|e| e.to_string())
}

/// Render the tree into a `px`×`px` PNG, scaled uniformly to fit and centered. Transparent
/// background; the caller composites/flattens where a format demands opacity (e.g. the iOS
/// 1024 icon).
pub fn render_png(tree: &usvg::Tree, px: u32) -> Result<Vec<u8>, String> {
    render_png_padded(tree, px, 0.0)
}

/// Like [`render_png`], with `pad` (a fraction of the edge, e.g. `0.1` = 10 %) of transparent
/// margin on every side — the macOS icon convention (art inset on the 1024 canvas).
pub fn render_png_padded(tree: &usvg::Tree, px: u32, pad: f32) -> Result<Vec<u8>, String> {
    if px == 0 {
        return Err("zero-size render".into());
    }
    let mut pixmap =
        tiny_skia::Pixmap::new(px, px).ok_or_else(|| "pixmap allocation failed".to_string())?;
    let size = tree.size();
    let (w, h) = (size.width(), size.height());
    if w <= 0.0 || h <= 0.0 {
        return Err("SVG has a zero-sized viewport".into());
    }
    let inner = px as f32 * (1.0 - 2.0 * pad.clamp(0.0, 0.45));
    let scale = inner / w.max(h);
    let tx = (px as f32 - w * scale) / 2.0;
    let ty = (px as f32 - h * scale) / 2.0;
    let ts = tiny_skia::Transform::from_row(scale, 0.0, 0.0, scale, tx, ty);
    resvg::render(tree, ts, &mut pixmap.as_mut());
    pixmap.encode_png().map_err(|e| e.to_string())
}

/// The tree content's absolute bounding box (strokes included), or `None` for empty art — the
/// input for safe-zone validation (an Android adaptive foreground overflowing the 66/108 zone).
pub fn content_bbox(tree: &usvg::Tree) -> Option<tiny_skia::Rect> {
    let b = tree.root().abs_layer_bounding_box();
    tiny_skia::Rect::from_xywh(b.x(), b.y(), b.width(), b.height())
}
