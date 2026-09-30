// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The canvas (§11): an `ARKUI_NODE_CUSTOM` node whose on-draw callback replays Day's encoded
//! display list with OH_Drawing (`ohos_sys::drawing`).
//!
//! Day records a display list in day points (vp); the custom node's draw canvas is in px, so a
//! density scale is pushed first. The op encoding is `day_spec::encode_ops`'s: 9 doubles per op
//! `[kind, a, b, c, d, e, f, g, argb]`, with polygon points, paths, text and the like riding a
//! positional text channel.

// Node handles are opaque runtime tokens (see node.rs), never dereferenced here.
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::ptr;

use ohos_sys::arkui::native_node::{
    ArkUI_NodeAttributeType as Attr, ArkUI_NodeCustomEvent, ArkUI_NodeCustomEventType,
    ArkUI_NodeDirtyFlag, ArkUI_NodeEventType as Ev, OH_ArkUI_NodeCustomEvent_GetDrawContextInDraw,
    OH_ArkUI_NodeCustomEvent_GetEventType, OH_ArkUI_NodeCustomEvent_GetUserData,
};
use ohos_sys::arkui::native_type::OH_ArkUI_DrawContext_GetCanvas;
use ohos_sys::drawing::brush::*;
use ohos_sys::drawing::canvas::*;
use ohos_sys::drawing::matrix::*;
use ohos_sys::drawing::path::*;
use ohos_sys::drawing::path_effect::*;
use ohos_sys::drawing::pen::*;
use ohos_sys::drawing::pixel_map::*;
use ohos_sys::drawing::point::*;
use ohos_sys::drawing::rect::*;
use ohos_sys::drawing::round_rect::*;
use ohos_sys::drawing::sampling_options::*;
use ohos_sys::drawing::shader_effect::*;
use ohos_sys::drawing::text_blob::*;
use ohos_sys::drawing::types::*;

use crate::fonts::{self, FontReq};
use crate::node::{self, Handle};

/// A canvas node's display list.
#[derive(Default)]
struct Ops {
    nums: Vec<f64>,
    texts: Vec<String>,
}

thread_local! {
    static CANVASES: RefCell<HashMap<usize, Ops>> = RefCell::new(HashMap::new());
    /// Decoded geometry, keyed by the encoder's content key (`day_spec::geometry_key`). Paths
    /// and polygons are the only ops whose geometry arrives as TEXT, and re-parsing every
    /// segment of a drawing that had not moved was this backend's per-frame cost. The key IS
    /// the content, so an entry can never go stale, and one entry is safely shared by every
    /// canvas: drawing and clipping only READ a path.
    static PATH_CACHE: RefCell<HashMap<i64, *mut OH_Drawing_Path>> = RefCell::new(HashMap::new());
}

/// Register the on-draw receiver for a canvas node, and make it focusable with the focus pair
/// and the key event (docs/menus.md, docs/focus.md): a custom-drawn node is not focusable by
/// default, and nothing an app DRAWS could otherwise hold the keys.
pub fn init(n: Handle, id: u64) {
    if n.is_null() {
        return;
    }
    node::add_custom_event_receiver(n, receiver);
    node::register_custom_event(
        n,
        ArkUI_NodeCustomEventType::ARKUI_NODE_CUSTOM_EVENT_ON_DRAW,
        node::CANVAS_DRAW_TARGET,
        n.cast(),
    );
    node::set_i32(n, Attr::NODE_FOCUSABLE, 1);
    node::register_event(n, Ev::NODE_ON_FOCUS, id);
    node::register_event(n, Ev::NODE_ON_BLUR, id);
    node::register_event(n, Ev::NODE_ON_KEY_EVENT, id);
}

/// Store the encoded display list for `n` and request a repaint.
pub fn set_ops(n: Handle, nums: &[f64], texts: &[String]) {
    CANVASES.with(|c| {
        c.borrow_mut().insert(
            n as usize,
            Ops {
                nums: nums.to_vec(),
                texts: texts.to_vec(),
            },
        )
    });
    node::mark_dirty(n, ArkUI_NodeDirtyFlag::NODE_NEED_RENDER);
}

/// A canvas node is being disposed: drop its display list so the entry cannot alias a
/// recycled node address.
pub fn forget(n: Handle) {
    CANVASES.with(|c| c.borrow_mut().remove(&(n as usize)));
}

unsafe extern "C" fn receiver(ev: *mut ArkUI_NodeCustomEvent) {
    if ev.is_null() {
        return;
    }
    day_spec::ffi_guard::contain((), || {
        // SAFETY: a live custom event; the draw context and canvas are ArkUI's for the call.
        unsafe {
            if OH_ArkUI_NodeCustomEvent_GetEventType(ev)
                != ArkUI_NodeCustomEventType::ARKUI_NODE_CUSTOM_EVENT_ON_DRAW
            {
                return;
            }
            let n = OH_ArkUI_NodeCustomEvent_GetUserData(ev) as usize;
            let dc = OH_ArkUI_NodeCustomEvent_GetDrawContextInDraw(ev);
            if dc.is_null() {
                return;
            }
            let cv = OH_ArkUI_DrawContext_GetCanvas(dc).cast::<OH_Drawing_Canvas>();
            if cv.is_null() {
                return;
            }
            // The list is taken out for the replay and put back, so a draw callback that
            // re-enters (it should not) finds no borrow held.
            let Some(ops) = CANVASES.with(|c| c.borrow_mut().remove(&n)) else {
                return;
            };
            draw(&ops, cv);
            CANVASES.with(|c| {
                c.borrow_mut().entry(n).or_insert(ops);
            });
        }
    });
}

/// A decoded stroke-style record (kind 18), applied to the next stroke record only.
#[derive(Default)]
struct StrokeStyle {
    active: bool,
    cap: i32,
    join: i32,
    miter: f32,
    phase: f32,
    dash: Vec<f32>,
}

/// A decoded gradient record (kind 14): type (0 linear, 1 radial) + unit geometry + stops,
/// applied as the brush's shader effect for the next fill record, resolved against that
/// shape's bounds.
#[derive(Default)]
struct Gradient {
    active: bool,
    kind: i32,
    // linear: start/end unit points; radial: sx,sy = center, ex = radius
    sx: f32,
    sy: f32,
    ex: f32,
    ey: f32,
    colors: Vec<u32>,
    offsets: Vec<f32>,
}

/// Parse "M x y L x y Q .. C .. Z" (`day_spec::encode_path`) into a path. Tolerant: a
/// malformed token stops the walk rather than aborting the frame.
unsafe fn parse_path(spec: &str, rule: i32) -> *mut OH_Drawing_Path {
    // SAFETY: a fresh path the caller owns.
    unsafe {
        let path = OH_Drawing_PathCreate();
        OH_Drawing_PathSetFillType(
            path,
            if rule == 1 {
                OH_Drawing_PathFillType::PATH_FILL_TYPE_EVEN_ODD
            } else {
                OH_Drawing_PathFillType::PATH_FILL_TYPE_WINDING
            },
        );
        let mut tok = spec.split_whitespace();
        fn num<'a>(tok: &mut impl Iterator<Item = &'a str>) -> f32 {
            tok.next().and_then(|s| s.parse().ok()).unwrap_or(0.0)
        }
        while let Some(op) = tok.next() {
            match op {
                "M" => {
                    let (x, y) = (num(&mut tok), num(&mut tok));
                    OH_Drawing_PathMoveTo(path, x, y);
                }
                "L" => {
                    let (x, y) = (num(&mut tok), num(&mut tok));
                    OH_Drawing_PathLineTo(path, x, y);
                }
                "Q" => {
                    let (cx, cy, x, y) =
                        (num(&mut tok), num(&mut tok), num(&mut tok), num(&mut tok));
                    OH_Drawing_PathQuadTo(path, cx, cy, x, y);
                }
                "C" => {
                    let (ax, ay, bx, by) =
                        (num(&mut tok), num(&mut tok), num(&mut tok), num(&mut tok));
                    let (x, y) = (num(&mut tok), num(&mut tok));
                    OH_Drawing_PathCubicTo(path, ax, ay, bx, by, x, y);
                }
                "Z" => OH_Drawing_PathClose(path),
                _ => {}
            }
        }
        path
    }
}

/// "x,y x,y …" as one closed path.
unsafe fn parse_polygon(pts: &str) -> *mut OH_Drawing_Path {
    // SAFETY: a fresh path the caller owns.
    unsafe {
        let path = OH_Drawing_PathCreate();
        let mut first = true;
        for tok in pts.split_whitespace() {
            let Some((x, y)) = tok.split_once(',') else {
                continue;
            };
            let (px, py) = (
                x.parse::<f32>().unwrap_or(0.0),
                y.parse::<f32>().unwrap_or(0.0),
            );
            if first {
                OH_Drawing_PathMoveTo(path, px, py);
                first = false;
            } else {
                OH_Drawing_PathLineTo(path, px, py);
            }
        }
        if !first {
            OH_Drawing_PathClose(path);
        }
        path
    }
}

fn cache_trim() {
    PATH_CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() <= 512 {
            return;
        }
        for (_, p) in c.drain() {
            // SAFETY: paths this cache owns.
            unsafe { OH_Drawing_PathDestroy(p) };
        }
    });
}

/// The path for `spec`, from the cache when the encoder offered a key. Ownership moves with
/// it: a cached path belongs to the cache, so the second element says whether the caller
/// destroys it, which is true exactly when there was no key.
unsafe fn cached_path(spec: &str, rule: i32, key: f64) -> (*mut OH_Drawing_Path, bool) {
    let k = key as i64;
    if k == 0 {
        // SAFETY: as parse_path.
        return (unsafe { parse_path(spec, rule) }, true);
    }
    if let Some(p) = PATH_CACHE.with(|c| c.borrow().get(&k).copied()) {
        return (p, false);
    }
    // SAFETY: as parse_path.
    let built = unsafe { parse_path(spec, rule) };
    cache_trim();
    PATH_CACHE.with(|c| c.borrow_mut().insert(k, built));
    (built, false)
}

unsafe fn cached_polygon(pts: &str, key: f64) -> (*mut OH_Drawing_Path, bool) {
    let k = key as i64;
    if k != 0
        && let Some(p) = PATH_CACHE.with(|c| c.borrow().get(&k).copied())
    {
        return (p, false);
    }
    // SAFETY: as parse_polygon.
    let built = unsafe { parse_polygon(pts) };
    if k == 0 {
        return (built, true);
    }
    cache_trim();
    PATH_CACHE.with(|c| c.borrow_mut().insert(k, built));
    (built, false)
}

/// Install a dash pattern on the pen, or clear it when the style has none. The created effect
/// is pushed onto `owned` rather than destroyed here: the effect must stay alive while the pen
/// references it and be reclaimed once the frame is drawn.
unsafe fn apply_dash(
    pen: *mut OH_Drawing_Pen,
    style: &StrokeStyle,
    owned: &mut Vec<*mut OH_Drawing_PathEffect>,
) {
    // SAFETY: a live pen; the effect outlives the pen's use of it.
    unsafe {
        if style.dash.is_empty() {
            OH_Drawing_PenSetPathEffect(pen, ptr::null_mut());
            return;
        }
        // OH_Drawing wants an even count; an odd pattern repeats to become even, matching the
        // other backends.
        let mut d = style.dash.clone();
        if d.len() % 2 == 1 {
            d.extend_from_slice(&style.dash);
        }
        let fx = OH_Drawing_CreateDashPathEffect(d.as_mut_ptr(), d.len() as i32, style.phase);
        OH_Drawing_PenSetPathEffect(pen, fx);
        owned.push(fx);
    }
}

unsafe fn apply_gradient(
    brush: *mut OH_Drawing_Brush,
    g: &mut Gradient,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
) {
    // SAFETY: a live brush; every OH_Drawing object created here is destroyed here.
    unsafe {
        let fx = if g.kind == 1 {
            // Radial, elliptical-to-bounds: circular in unit space, stretched onto the bounds
            // by the shader's local matrix (the same rule as every other backend).
            let center = OH_Drawing_Point2D { x: g.sx, y: g.sy };
            let m = OH_Drawing_MatrixCreate();
            OH_Drawing_MatrixSetMatrix(m, w, 0.0, x, 0.0, h, y, 0.0, 0.0, 1.0);
            let fx = OH_Drawing_ShaderEffectCreateRadialGradientWithLocalMatrix(
                &center,
                g.ex.max(1e-4),
                g.colors.as_ptr(),
                g.offsets.as_ptr(),
                g.colors.len() as u32,
                OH_Drawing_TileMode::CLAMP,
                m,
            );
            OH_Drawing_MatrixDestroy(m);
            fx
        } else {
            let start = OH_Drawing_PointCreate(x + g.sx * w, y + g.sy * h);
            let end = OH_Drawing_PointCreate(x + g.ex * w, y + g.ey * h);
            let fx = OH_Drawing_ShaderEffectCreateLinearGradient(
                start,
                end,
                g.colors.as_ptr(),
                g.offsets.as_ptr(),
                g.colors.len() as u32,
                OH_Drawing_TileMode::CLAMP,
            );
            OH_Drawing_PointDestroy(start);
            OH_Drawing_PointDestroy(end);
            fx
        };
        OH_Drawing_BrushSetShaderEffect(brush, fx);
        OH_Drawing_ShaderEffectDestroy(fx);
        g.active = false;
    }
}

/// A path's bounds: (x, y, w, h).
unsafe fn path_bounds(path: *mut OH_Drawing_Path) -> (f32, f32, f32, f32) {
    // SAFETY: a scratch rect filled from a live path.
    unsafe {
        let r = OH_Drawing_RectCreate(0.0, 0.0, 0.0, 0.0);
        OH_Drawing_PathGetBounds(path, r);
        let b = (
            OH_Drawing_RectGetLeft(r),
            OH_Drawing_RectGetTop(r),
            OH_Drawing_RectGetWidth(r),
            OH_Drawing_RectGetHeight(r),
        );
        OH_Drawing_RectDestroy(r);
        b
    }
}

unsafe fn draw(ops: &Ops, cv: *mut OH_Drawing_Canvas) {
    let n = &ops.nums;
    let texts = &ops.texts;
    // SAFETY: every OH_Drawing object created in this replay is destroyed before it returns;
    // the canvas is ArkUI's for the callback's duration.
    unsafe {
        let pen = OH_Drawing_PenCreate();
        OH_Drawing_PenSetAntiAlias(pen, true);
        let brush = OH_Drawing_BrushCreate();
        OH_Drawing_BrushSetAntiAlias(brush, true);

        // Base transform: scale vp → px so Day's point-space ops land correctly.
        OH_Drawing_CanvasSave(cv);
        let scale = OH_Drawing_MatrixCreate();
        let d = node::density() as f32;
        OH_Drawing_MatrixSetMatrix(scale, d, 0.0, 0.0, 0.0, d, 0.0, 0.0, 0.0, 1.0);
        OH_Drawing_CanvasConcatMatrix(cv, scale);

        let mut text_i = 0usize;
        let mut grad = Gradient::default();
        let mut style = StrokeStyle::default();
        let mut fontp: Option<FontReq> = None;
        // Dash effects created during this replay, destroyed once the last op has been drawn.
        let mut dash_effects: Vec<*mut OH_Drawing_PathEffect> = Vec::new();
        // A decoded kind-20 record (stamp): the positions the next shape record is drawn at,
        // once each. Empty means the ordinary one-shape-one-record case (docs/canvas.md
        // "Stamping").
        let mut stamp_at: Vec<(f32, f32)> = Vec::new();
        let mut stamp_n = 0usize;
        let mut i = 0;
        while i + 8 < n.len() {
            let kind = n[i] as i32;
            let (a, b, c, dd) = (
                n[i + 1] as f32,
                n[i + 2] as f32,
                n[i + 3] as f32,
                n[i + 4] as f32,
            );
            let (e, f, g) = (n[i + 5] as f32, n[i + 6] as f32, n[i + 7] as f32);
            let col = n[i + 8] as i64 as u32; // already 0xAARRGGBB
            i += 9;
            OH_Drawing_PenSetColor(pen, col);
            OH_Drawing_PenSetWidth(pen, if g > 0.0 { g } else { 1.0 });
            OH_Drawing_BrushSetColor(brush, col);
            let stroke = matches!(kind, 1 | 4 | 5 | 6 | 12 | 13 | 16);
            // A kind-18 record styles the next stroke only; otherwise Day's defaults apply.
            if stroke {
                if style.active {
                    OH_Drawing_PenSetCap(
                        pen,
                        match style.cap {
                            1 => OH_Drawing_PenLineCapStyle::LINE_ROUND_CAP,
                            2 => OH_Drawing_PenLineCapStyle::LINE_SQUARE_CAP,
                            _ => OH_Drawing_PenLineCapStyle::LINE_FLAT_CAP,
                        },
                    );
                    OH_Drawing_PenSetJoin(
                        pen,
                        match style.join {
                            1 => OH_Drawing_PenLineJoinStyle::LINE_ROUND_JOIN,
                            2 => OH_Drawing_PenLineJoinStyle::LINE_BEVEL_JOIN,
                            _ => OH_Drawing_PenLineJoinStyle::LINE_MITER_JOIN,
                        },
                    );
                    OH_Drawing_PenSetMiterLimit(pen, style.miter);
                    apply_dash(pen, &style, &mut dash_effects);
                } else {
                    OH_Drawing_PenSetCap(pen, OH_Drawing_PenLineCapStyle::LINE_FLAT_CAP);
                    OH_Drawing_PenSetJoin(pen, OH_Drawing_PenLineJoinStyle::LINE_MITER_JOIN);
                    OH_Drawing_PenSetMiterLimit(pen, 10.0);
                    OH_Drawing_PenSetPathEffect(pen, ptr::null_mut());
                }
            }
            // Fill kinds consume a pending gradient (kind 14) as the brush's shader effect.
            if grad.active {
                if matches!(kind, 0 | 2 | 3) {
                    apply_gradient(brush, &mut grad, a, b, c, dd);
                }
                // kinds 11/15 resolve after parsing (bounds unknown here)
            } else {
                OH_Drawing_BrushSetShaderEffect(brush, ptr::null_mut());
            }
            if stroke {
                OH_Drawing_CanvasAttachPen(cv, pen);
            } else {
                OH_Drawing_CanvasAttachBrush(cv, brush);
            }
            // Stamp prefix and its coordinate records: collected, never drawn on their own.
            if kind == 20 {
                stamp_at.clear();
                stamp_n = a.max(0.0) as usize;
                if stroke {
                    OH_Drawing_CanvasDetachPen(cv);
                } else {
                    OH_Drawing_CanvasDetachBrush(cv);
                }
                continue;
            }
            if kind == 21 {
                // Four points per record, in slot pairs (1,2) (3,4) (5,6) (7,8): the fourth
                // point's y rides the slot other records use for their color. The last record
                // of a run is padded with zeros, so the header's count says where the real
                // ones stop.
                let xs = [a, c, e, g];
                let ys = [b, dd, f, n[i - 1] as f32];
                for q in 0..4 {
                    if stamp_at.len() < stamp_n {
                        stamp_at.push((xs[q], ys[q]));
                    }
                }
                OH_Drawing_CanvasDetachBrush(cv);
                continue;
            }
            // The template is replayed once per position under a translated canvas. `text_i`
            // is rewound each time so a template with a texts payload reads the same entry
            // every repetition and consumes it exactly once overall.
            let reps = if stamp_at.is_empty() {
                1
            } else {
                stamp_at.len()
            };
            let ti_start = text_i;
            let next_text = |text_i: &mut usize| -> String {
                let s = texts.get(*text_i).cloned().unwrap_or_default();
                *text_i += 1;
                s
            };
            for rep in 0..reps {
                text_i = ti_start;
                if !stamp_at.is_empty() {
                    OH_Drawing_CanvasSave(cv);
                    OH_Drawing_CanvasTranslate(cv, stamp_at[rep].0, stamp_at[rep].1);
                }
                match kind {
                    0 | 1 => {
                        // rect fill / stroke
                        let r = OH_Drawing_RectCreate(a, b, a + c, b + dd);
                        OH_Drawing_CanvasDrawRect(cv, r);
                        OH_Drawing_RectDestroy(r);
                    }
                    2 | 13 => {
                        // rounded rect fill / stroke (radius = e)
                        let r = OH_Drawing_RectCreate(a, b, a + c, b + dd);
                        let rr = OH_Drawing_RoundRectCreate(r, e, e);
                        OH_Drawing_CanvasDrawRoundRect(cv, rr);
                        OH_Drawing_RoundRectDestroy(rr);
                        OH_Drawing_RectDestroy(r);
                    }
                    3 | 4 => {
                        // ellipse fill / stroke
                        let r = OH_Drawing_RectCreate(a, b, a + c, b + dd);
                        OH_Drawing_CanvasDrawOval(cv, r);
                        OH_Drawing_RectDestroy(r);
                    }
                    5 => {
                        // arc (start = e, sweep = f)
                        let r = OH_Drawing_RectCreate(a, b, a + c, b + dd);
                        OH_Drawing_CanvasDrawArc(cv, r, e, f);
                        OH_Drawing_RectDestroy(r);
                    }
                    6 => OH_Drawing_CanvasDrawLine(cv, a, b, c, dd),
                    7 => {
                        // text: size = e, anchor = f packed as h*4+v (TextAnchor::pack); the
                        // string on the text channel
                        let s = next_text(&mut text_i);
                        let req = fontp.clone().unwrap_or_default();
                        let (font, tf) = fonts::make_font(e, &req);
                        let cs = node::cstr(&s);
                        let blob = OH_Drawing_TextBlobCreateFromString(
                            cs.as_ptr(),
                            font,
                            OH_Drawing_TextEncoding::TEXT_ENCODING_UTF8,
                        );
                        // DrawTextBlob takes the BASELINE; the anchor positions the line box
                        // from the font metrics (Skia-style: ascent negative, descent
                        // positive), the same formula as TextAnchor::offset and the Android
                        // canvas backend, so glyphs land in the same place on every platform.
                        let m = fonts::metrics(font);
                        let (ah, av) = (f as i32 / 4, f as i32 % 4);
                        let line_h = m.descent - m.ascent;
                        let mut x = a;
                        let mut y = b - m.ascent;
                        if ah != 0 {
                            let w = fonts::text_width(font, &s, e);
                            x = a + if ah == 1 { -w / 2.0 } else { -w };
                        }
                        if av == 1 {
                            y = b - (m.ascent + m.descent) / 2.0;
                        } else if av == 2 {
                            y = b; // `at` IS the baseline
                        } else if av == 3 {
                            y = b - line_h - m.ascent;
                        }
                        OH_Drawing_CanvasDrawTextBlob(cv, blob, x, y);
                        OH_Drawing_TextBlobDestroy(blob);
                        fonts::destroy_font(font, tf);
                    }
                    19 => {
                        // font for the next text: a weight (0 default), b italic; family on
                        // the text channel
                        let family = next_text(&mut text_i);
                        fontp = Some(FontReq {
                            weight: a as i32,
                            italic: b > 0.5,
                            family,
                        });
                    }
                    8 => OH_Drawing_CanvasSave(cv),
                    9 => OH_Drawing_CanvasRestore(cv),
                    10 => {
                        // concat affine [a b c d tx ty] (day_geometry::Affine, column vectors)
                        let m = OH_Drawing_MatrixCreate();
                        OH_Drawing_MatrixSetMatrix(m, a, c, e, b, dd, f, 0.0, 0.0, 1.0);
                        OH_Drawing_CanvasConcatMatrix(cv, m);
                        OH_Drawing_MatrixDestroy(m);
                    }
                    11 | 12 => {
                        // polygon fill / stroke: points ride the text channel as "x,y x,y …".
                        // The text entry is consumed either way: that channel is positional,
                        // so a cache hit that skipped it would desynchronize every record
                        // after this one.
                        let pts = next_text(&mut text_i);
                        let (path, owned) = cached_polygon(&pts, f64::from(a));
                        if kind == 11 && grad.active {
                            let (bx, by, bw, bh) = path_bounds(path);
                            OH_Drawing_CanvasDetachBrush(cv);
                            apply_gradient(brush, &mut grad, bx, by, bw, bh);
                            OH_Drawing_CanvasAttachBrush(cv, brush);
                        }
                        OH_Drawing_CanvasDrawPath(cv, path);
                        if owned {
                            OH_Drawing_PathDestroy(path);
                        }
                    }
                    15 | 16 => {
                        // path fill / stroke: segments ride the text channel; f = fill rule
                        let spec = next_text(&mut text_i);
                        let (path, owned) = cached_path(&spec, f as i32, f64::from(a));
                        if kind == 15 && grad.active {
                            let (bx, by, bw, bh) = path_bounds(path);
                            OH_Drawing_CanvasDetachBrush(cv);
                            apply_gradient(brush, &mut grad, bx, by, bw, bh);
                            OH_Drawing_CanvasAttachBrush(cv, brush);
                        }
                        OH_Drawing_CanvasDrawPath(cv, path);
                        if owned {
                            OH_Drawing_PathDestroy(path);
                        }
                    }
                    17 => {
                        // clip: f names the shape, a..dd geometry, e radius or fill rule. The
                        // three fixed-size kinds build a path of their own and always own it;
                        // only the two payload kinds can come from the cache.
                        let (clip, owned) = match f as i32 {
                            3 => {
                                let spec = next_text(&mut text_i);
                                cached_path(&spec, e as i32, f64::from(a))
                            }
                            4 => {
                                let pts = next_text(&mut text_i);
                                cached_polygon(&pts, f64::from(a))
                            }
                            2 => {
                                let clip = OH_Drawing_PathCreate();
                                let r = OH_Drawing_RectCreate(a, b, a + c, b + dd);
                                OH_Drawing_PathAddOval(
                                    clip,
                                    r,
                                    OH_Drawing_PathDirection::PATH_DIRECTION_CW,
                                );
                                OH_Drawing_RectDestroy(r);
                                (clip, true)
                            }
                            1 => {
                                let clip = OH_Drawing_PathCreate();
                                let r = OH_Drawing_RectCreate(a, b, a + c, b + dd);
                                let rr = OH_Drawing_RoundRectCreate(r, e, e);
                                OH_Drawing_PathAddRoundRect(
                                    clip,
                                    rr,
                                    OH_Drawing_PathDirection::PATH_DIRECTION_CW,
                                );
                                OH_Drawing_RoundRectDestroy(rr);
                                OH_Drawing_RectDestroy(r);
                                (clip, true)
                            }
                            _ => {
                                let clip = OH_Drawing_PathCreate();
                                OH_Drawing_PathAddRect(
                                    clip,
                                    a,
                                    b,
                                    a + c,
                                    b + dd,
                                    OH_Drawing_PathDirection::PATH_DIRECTION_CW,
                                );
                                (clip, true)
                            }
                        };
                        if !clip.is_null() {
                            // INTERSECT: a clip only ever narrows until the matching restore.
                            OH_Drawing_CanvasClipPath(
                                cv,
                                clip,
                                OH_Drawing_CanvasClipOp::INTERSECT,
                                true,
                            );
                            if owned {
                                OH_Drawing_PathDestroy(clip);
                            }
                        }
                    }
                    18 => {
                        // stroke style for the next stroke: a cap, b join, c miter, dd phase;
                        // the dashes on the text channel
                        let dashes = next_text(&mut text_i);
                        style = StrokeStyle {
                            active: true,
                            cap: a as i32,
                            join: b as i32,
                            miter: c,
                            phase: dd,
                            dash: dashes
                                .split_whitespace()
                                .filter_map(|t| t.parse().ok())
                                .collect(),
                        };
                    }
                    14 => {
                        // set-gradient (f = type): stops ride the text channel as
                        // "offset,aarrggbb offset,aarrggbb ..."
                        let stops = next_text(&mut text_i);
                        grad.kind = f as i32;
                        grad.colors.clear();
                        grad.offsets.clear();
                        for tok in stops.split_whitespace() {
                            let Some((off, color)) = tok.split_once(',') else {
                                continue;
                            };
                            if off.is_empty() {
                                continue;
                            }
                            grad.offsets.push(off.parse().unwrap_or(0.0));
                            grad.colors
                                .push(u32::from_str_radix(color, 16).unwrap_or(0));
                        }
                        grad.sx = a;
                        grad.sy = b;
                        grad.ex = c;
                        grad.ey = dd;
                        grad.active = grad.colors.len() >= 2;
                    }
                    22 => {
                        // image: a,b origin · c,dd size · e the BitmapId · f opacity
                        // (docs/images.md). A released bitmap draws nothing rather than a
                        // placeholder: the canvas re-records on every tracked read, so a
                        // handle can be dropped between record and replay.
                        let pm = crate::images::bitmap(e as u64);
                        if !pm.is_null() && c > 0.0 && dd > 0.0 {
                            let dpm = OH_Drawing_PixelMapGetFromOhPixelMapNative(pm);
                            if !dpm.is_null() {
                                let (iw, ih, _) = crate::images::info(pm);
                                if iw > 0 && ih > 0 {
                                    let src = OH_Drawing_RectCreate(0.0, 0.0, iw as f32, ih as f32);
                                    let dst = OH_Drawing_RectCreate(a, b, a + c, b + dd);
                                    let so = OH_Drawing_SamplingOptionsCreate(
                                        OH_Drawing_FilterMode::FILTER_MODE_LINEAR,
                                        OH_Drawing_MipmapMode::MIPMAP_MODE_NONE,
                                    );
                                    // Opacity rides a layer: `DrawPixelMapRect` takes no brush,
                                    // so an alpha brush on a saved layer is the only way to
                                    // compose the image below full strength.
                                    let faded = f < 0.999;
                                    let mut alpha = ptr::null_mut();
                                    if faded {
                                        alpha = OH_Drawing_BrushCreate();
                                        OH_Drawing_BrushSetAlpha(
                                            alpha,
                                            (f.clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
                                        );
                                        OH_Drawing_CanvasSaveLayer(cv, dst, alpha);
                                    }
                                    // A fully transparent rect first. A recording whose only
                                    // content is pixel map draws renders nothing; the clear fill
                                    // gives the recording content without changing a pixel.
                                    let clear = OH_Drawing_BrushCreate();
                                    OH_Drawing_BrushSetColor(clear, 0x0000_0000);
                                    OH_Drawing_CanvasAttachBrush(cv, clear);
                                    OH_Drawing_CanvasDrawRect(cv, dst);
                                    OH_Drawing_CanvasDetachBrush(cv);
                                    OH_Drawing_BrushDestroy(clear);
                                    OH_Drawing_CanvasDrawPixelMapRect(cv, dpm, src, dst, so);
                                    if faded {
                                        OH_Drawing_CanvasRestore(cv);
                                        OH_Drawing_BrushDestroy(alpha);
                                    }
                                    OH_Drawing_SamplingOptionsDestroy(so);
                                    OH_Drawing_RectDestroy(dst);
                                    OH_Drawing_RectDestroy(src);
                                }
                                OH_Drawing_PixelMapDissolve(dpm);
                            }
                        }
                    }
                    _ => {}
                }
                if !stamp_at.is_empty() {
                    OH_Drawing_CanvasRestore(cv);
                }
            }
            stamp_at.clear();
            if stroke {
                OH_Drawing_CanvasDetachPen(cv);
            } else {
                OH_Drawing_CanvasDetachBrush(cv);
            }
            // A style record applies to one stroke; anything else clears it.
            if kind != 18 {
                style.active = false;
            }
            if kind != 19 {
                fontp = None;
            }
        }
        OH_Drawing_CanvasRestore(cv);
        OH_Drawing_MatrixDestroy(scale);
        // Detach before destroying: the pen still holds whichever effect was set last.
        OH_Drawing_PenSetPathEffect(pen, ptr::null_mut());
        for fx in dash_effects {
            OH_Drawing_PathEffectDestroy(fx);
        }
        OH_Drawing_PenDestroy(pen);
        OH_Drawing_BrushDestroy(brush);
    }
}
