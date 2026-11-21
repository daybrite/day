// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The canvas (§11): an `ARKUI_NODE_CUSTOM` node whose on-draw callback replays Day's encoded
//! display list with `OH_Drawing` (`ohos_sys::drawing`), inside a host node that holds focus for
//! it ([`create`]).
//!
//! Day records a display list in day points (vp); the custom node's draw canvas is in px, so a
//! density scale is pushed first. An exported bitmap ([`render_bitmap`]) replays the same list
//! through the same [`draw`], with the export scale in place of the density. The op encoding is `day_spec::encode_ops`'s: 9 doubles per op
//! `[kind, a, b, c, d, e, f, g, argb]`, with polygon points, paths, text and the like riding a
//! positional text channel.
//!
//! Three things `OH_Drawing` does not keep in the canvas are kept here (docs/canvas.md): the
//! shadow and the blend mode, which are graphics state to Day and properties of a pen or brush
//! to the platform, and the transform, which a shadow must not follow. A shadow is drawn as a
//! pass of its own before each record: the same geometry in the shadow's color under a blur
//! mask filter, moved by the offset worked back through the transform. A drawing that blends is
//! replayed inside a layer, so the canvas is its own compositing group.

// Node handles are opaque runtime tokens (see node.rs), never dereferenced here.
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::ptr;

use ohos_sys::arkui::native_node::{
    ArkUI_NodeAttributeType as Attr, ArkUI_NodeCustomEvent, ArkUI_NodeCustomEventType,
    ArkUI_NodeDirtyFlag, ArkUI_NodeEventType as Ev, ArkUI_NodeType,
    OH_ArkUI_NodeCustomEvent_GetDrawContextInDraw, OH_ArkUI_NodeCustomEvent_GetEventType,
    OH_ArkUI_NodeCustomEvent_GetUserData,
};
use ohos_sys::arkui::native_type::{
    ArkUI_AccessibilityMode, ArkUI_ButtonType, OH_ArkUI_DrawContext_GetCanvas,
    OH_ArkUI_DrawContext_GetSize,
};
use ohos_sys::drawing::bitmap::*;
use ohos_sys::drawing::brush::*;
use ohos_sys::drawing::canvas::*;
use ohos_sys::drawing::color_filter::*;
use ohos_sys::drawing::filter::*;
use ohos_sys::drawing::image_filter::*;
use ohos_sys::drawing::mask_filter::*;
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

use ohos_sys::multimedia::image_kit::native_image::common::ImageResult;
use ohos_sys::multimedia::image_kit::native_image::pixelmap::{
    OH_PixelmapInitializationOptions_Create, OH_PixelmapInitializationOptions_Release,
    OH_PixelmapInitializationOptions_SetAlphaType, OH_PixelmapInitializationOptions_SetHeight,
    OH_PixelmapInitializationOptions_SetPixelFormat,
    OH_PixelmapInitializationOptions_SetSrcPixelFormat, OH_PixelmapInitializationOptions_SetWidth,
    OH_PixelmapNative_CreatePixelmap, PIXEL_FORMAT, PIXELMAP_ALPHA_TYPE,
};

use crate::fonts::{self, FontReq};
use crate::images::OH_PixelmapNative;
use crate::node::{self, Handle};
use crate::paint;

/// A canvas node's display list.
#[derive(Default)]
struct Ops {
    nums: Vec<f64>,
    texts: Vec<String>,
}

day_reactive::tls_slots! {
    canvas;
    static CANVASES: RefCell<HashMap<usize, Ops>> = RefCell::new(HashMap::new());
    /// Decoded geometry, keyed by the encoder's content key (`day_spec::geometry_key`). Paths
    /// and polygons are the only ops whose geometry arrives as TEXT, and re-parsing every
    /// segment of a drawing that had not moved was this backend's per-frame cost. The key IS
    /// the content, so an entry can never go stale, and one entry is safely shared by every
    /// canvas: drawing and clipping only READ a path.
    static PATH_CACHE: RefCell<HashMap<i64, *mut OH_Drawing_Path>> = RefCell::new(HashMap::new());
    /// Day owns the Stack host; its drawing surface and focus button are private children.
    static SURFACES: RefCell<HashMap<usize, CanvasNodes>> = RefCell::new(HashMap::new());
}

struct CanvasNodes {
    surface: Handle,
    focus: Handle,
}

/// Build a canvas for day node `id`, returning its Stack host (null on allocation failure).
///
/// A custom drawing node cannot hold focus. A button with custom content becomes a focus
/// scope during layout, while a labeled button only measures its text child. Keep drawing
/// and focus as siblings: the Stack lays out both, and focus requests on its scope reach the
/// transparent button with an explicitly empty label. Only that button reports focus/keys;
/// accessibility and Day's gestures belong to the host.
pub fn create(id: u64) -> Handle {
    let host = node::create(node::STACK);
    if host.is_null() {
        return host;
    }
    let surface = node::create(node::CUSTOM);
    if surface.is_null() {
        node::dispose_raw(host);
        return surface;
    }
    let focus = node::create(ArkUI_NodeType::ARKUI_NODE_BUTTON);
    if focus.is_null() {
        node::dispose_raw(surface);
        node::dispose_raw(host);
        return focus;
    }
    node::set_i32(
        focus,
        Attr::NODE_BUTTON_TYPE,
        ArkUI_ButtonType::ARKUI_BUTTON_TYPE_NORMAL.0 as i32,
    );
    node::set_u32(focus, Attr::NODE_BACKGROUND_COLOR, 0);
    node::set_f32(focus, Attr::NODE_PADDING, 0.0);
    node::set_f32(focus, Attr::NODE_BORDER_RADIUS, 0.0);
    node::set_button_label(focus, "");
    node::set_i32(
        focus,
        Attr::NODE_ACCESSIBILITY_MODE,
        ArkUI_AccessibilityMode::ARKUI_ACCESSIBILITY_MODE_DISABLED_FOR_DESCENDANTS.0 as i32,
    );
    node::set_f32(surface, Attr::NODE_WIDTH_PERCENT, 1.0);
    node::set_f32(surface, Attr::NODE_HEIGHT_PERCENT, 1.0);
    node::set_f32(focus, Attr::NODE_WIDTH_PERCENT, 1.0);
    node::set_f32(focus, Attr::NODE_HEIGHT_PERCENT, 1.0);
    node::add_child(host, surface);
    node::add_child(host, focus);
    node::add_custom_event_receiver(surface, receiver);
    node::register_custom_event(
        surface,
        ArkUI_NodeCustomEventType::ARKUI_NODE_CUSTOM_EVENT_ON_DRAW,
        node::CANVAS_DRAW_TARGET,
        surface.cast(),
    );
    node::set_i32(host, Attr::NODE_FOCUSABLE, 1);
    node::set_i32(focus, Attr::NODE_FOCUSABLE, 1);
    node::register_event(focus, Ev::NODE_ON_FOCUS, id);
    node::register_event(focus, Ev::NODE_ON_BLUR, id);
    node::register_event(focus, Ev::NODE_ON_KEY_EVENT, id);
    // The focus button covers the canvas, so it takes no touches either until the canvas
    // takes input (docs/gestures.md "Pass-through"; [`accept_input`]). The drawing surface
    // never does: once the canvas takes input the button above it is what a touch hits.
    node::make_passive(focus);
    node::make_passive(surface);
    SURFACES.with(|s| {
        s.borrow_mut()
            .insert(host as usize, CanvasNodes { surface, focus })
    });
    host
}

/// Whether `host` is a canvas host (gesture.rs: a wheel pan over a canvas takes every tick,
/// over any other container only sideways movement).
/// Canvas `host` takes input now: its focus button joins hit testing again, as the press
/// path of the canvas's recognizers and its focus on touch.
pub(crate) fn accept_input(host: Handle) {
    let focus = SURFACES.with(|s| s.borrow().get(&(host as usize)).map(|nodes| nodes.focus));
    if let Some(focus) = focus {
        node::accept_input(focus);
    }
}

/// `Toolkit::set_focusable` on a canvas: a touch focuses its focus button, which reports focus
/// and keys already.
pub(crate) fn make_focusable(host: Handle) {
    node::accept_input(host);
    let focus = SURFACES.with(|s| s.borrow().get(&(host as usize)).map(|nodes| nodes.focus));
    if let Some(focus) = focus {
        node::set_i32(focus, Attr::NODE_FOCUS_ON_TOUCH, 1);
    }
}

pub(crate) fn is_canvas(host: Handle) -> bool {
    !surface_of(host).is_null()
}

/// The drawing node inside canvas host `host`, or null for a handle that is not a canvas.
fn surface_of(host: Handle) -> Handle {
    SURFACES.with(|s| {
        s.borrow()
            .get(&(host as usize))
            .map_or(ptr::null_mut(), |nodes| nodes.surface)
    })
}

/// Store the encoded display list for canvas `host` and request a repaint of its surface.
pub fn set_ops(host: Handle, nums: &[f64], texts: &[String]) {
    let n = surface_of(host);
    if n.is_null() {
        return;
    }
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

/// Drop the display list and both private children before the host is disposed.
pub fn forget(host: Handle) {
    let Some(nodes) = SURFACES.with(|s| s.borrow_mut().remove(&(host as usize))) else {
        return;
    };
    CANVASES.with(|c| c.borrow_mut().remove(&(nodes.surface as usize)));
    node::remove_child(host, nodes.focus);
    node::dispose_raw(nodes.focus);
    node::remove_child(host, nodes.surface);
    node::dispose_raw(nodes.surface);
}

/// Replay an encoded display list into a new pixelmap of `pw` × `ph` pixels at `scale` pixels
/// per point, transparent where nothing draws (docs/canvas.md "Exporting"). The drawing canvas
/// binds a CPU bitmap over a buffer owned here, painted by the same [`draw`] the node's callback
/// runs; the pixelmap copies the buffer, and the caller owns it. `Err` is the platform's refusal.
pub fn render_bitmap(
    nums: Vec<f64>,
    texts: Vec<String>,
    pw: u32,
    ph: u32,
    scale: f64,
) -> Result<*mut OH_PixelmapNative, String> {
    let ops = Ops { nums, texts };
    let row = pw as usize * 4;
    let len = row
        .checked_mul(ph as usize)
        .ok_or_else(|| format!("a {pw}x{ph} bitmap overflows memory"))?;
    let mut px: Vec<u8> = Vec::new();
    px.try_reserve_exact(len)
        .map_err(|e| format!("cannot allocate a {pw}x{ph} bitmap: {e}"))?;
    // Zeroed premultiplied pixels are fully transparent: the export's background.
    px.resize(len, 0);
    // BGRA is the byte order `OH_PixelmapNative_CreatePixelmap` documents for its buffer, so the
    // copy below never depends on a format conversion.
    let mut info = OH_Drawing_Image_Info {
        width: pw as i32,
        height: ph as i32,
        colorType: OH_Drawing_ColorFormat::COLOR_FORMAT_BGRA_8888,
        alphaType: OH_Drawing_AlphaFormat::ALPHA_FORMAT_PREMUL,
    };
    // SAFETY: the bitmap borrows `px`, which outlives it and the canvas bound to it; both are
    // destroyed before the buffer is read. The options object is released on every path, and
    // the pixelmap copies `px` rather than keeping it.
    unsafe {
        let bmp =
            OH_Drawing_BitmapCreateFromPixels(&raw mut info, px.as_mut_ptr().cast(), row as u32);
        if bmp.is_null() {
            return Err(format!(
                "OH_Drawing_BitmapCreateFromPixels refused {pw}x{ph}"
            ));
        }
        let cv = OH_Drawing_CanvasCreate();
        if cv.is_null() {
            OH_Drawing_BitmapDestroy(bmp);
            return Err("OH_Drawing_CanvasCreate failed".to_string());
        }
        OH_Drawing_CanvasBind(cv, bmp);
        let scale = scale as f32;
        draw(&ops, cv, scale, (pw as f32 / scale, ph as f32 / scale));
        OH_Drawing_CanvasDestroy(cv);
        OH_Drawing_BitmapDestroy(bmp);

        let mut opts = ptr::null_mut();
        if OH_PixelmapInitializationOptions_Create(&raw mut opts) != ImageResult::SUCCESS
            || opts.is_null()
        {
            return Err("OH_PixelmapInitializationOptions_Create failed".to_string());
        }
        let bgra = PIXEL_FORMAT::PIXEL_FORMAT_BGRA_8888.0 as i32;
        OH_PixelmapInitializationOptions_SetWidth(opts, pw);
        OH_PixelmapInitializationOptions_SetHeight(opts, ph);
        OH_PixelmapInitializationOptions_SetSrcPixelFormat(opts, bgra);
        OH_PixelmapInitializationOptions_SetPixelFormat(opts, bgra);
        OH_PixelmapInitializationOptions_SetAlphaType(
            opts,
            PIXELMAP_ALPHA_TYPE::PIXELMAP_ALPHA_TYPE_PREMULTIPLIED.0 as i32,
        );
        let mut pm = ptr::null_mut();
        let rc = OH_PixelmapNative_CreatePixelmap(px.as_mut_ptr(), px.len(), opts, &raw mut pm);
        OH_PixelmapInitializationOptions_Release(opts);
        if rc != ImageResult::SUCCESS || pm.is_null() {
            return Err(format!(
                "OH_PixelmapNative_CreatePixelmap failed with code {}",
                rc.0
            ));
        }
        Ok(pm)
    }
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
            let density = node::density() as f32;
            let px = OH_ArkUI_DrawContext_GetSize(dc);
            let size = (px.width as f32 / density, px.height as f32 / density);
            draw(&ops, cv, density, size);
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

/// A decoded gradient record (kind 14): type (0 linear, 1 radial, 2 conic) + unit geometry +
/// stops, applied as the shader effect of the brush or pen that draws the next shape record,
/// resolved against that shape's bounds.
#[derive(Default)]
struct Gradient {
    active: bool,
    kind: i32,
    // linear: start/end unit points; radial: sx,sy = center, ex = radius; conic: sx,sy =
    // center, ex = start angle in degrees
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
    fn num<'a>(tok: &mut impl Iterator<Item = &'a str>) -> f32 {
        tok.next().and_then(|s| s.parse().ok()).unwrap_or(0.0)
    }
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

/// Build the pending gradient as a shader for a shape whose bounding box is (`x`, `y`, `w`,
/// `h`). The caller owns the effect; null when the platform refuses it.
unsafe fn gradient_shader(
    g: &Gradient,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
) -> *mut OH_Drawing_ShaderEffect {
    // SAFETY: every OH_Drawing object created here is destroyed here, except the effect returned.
    unsafe {
        match g.kind {
            1 => {
                // Radial, elliptical-to-bounds: circular in unit space, stretched onto the bounds
                // by the shader's local matrix (the same rule as every other backend).
                let center = OH_Drawing_Point2D { x: g.sx, y: g.sy };
                let m = OH_Drawing_MatrixCreate();
                OH_Drawing_MatrixSetMatrix(m, w, 0.0, x, 0.0, h, y, 0.0, 0.0, 1.0);
                let fx = OH_Drawing_ShaderEffectCreateRadialGradientWithLocalMatrix(
                    &raw const center,
                    g.ex.max(1e-4),
                    g.colors.as_ptr(),
                    g.offsets.as_ptr(),
                    g.colors.len() as u32,
                    OH_Drawing_TileMode::CLAMP,
                    m,
                );
                OH_Drawing_MatrixDestroy(m);
                fx
            }
            2 => {
                // Conic: the native sweep gradient, which turns clockwise from +x as Day's
                // does. It always starts at 0°, so the start angle (slot c) goes into the stops
                // instead (`paint::sweep_stops`). The center is a unit point of the bounds and
                // the angle stays in canvas space, so no local matrix is involved.
                let stops: Vec<(f32, u32)> = g
                    .offsets
                    .iter()
                    .copied()
                    .zip(g.colors.iter().copied())
                    .collect();
                let (offsets, colors): (Vec<f32>, Vec<u32>) =
                    paint::sweep_stops(g.ex, &stops).into_iter().unzip();
                let center = OH_Drawing_PointCreate(x + g.sx * w, y + g.sy * h);
                let fx = OH_Drawing_ShaderEffectCreateSweepGradient(
                    center,
                    colors.as_ptr(),
                    offsets.as_ptr(),
                    colors.len() as u32,
                    OH_Drawing_TileMode::CLAMP,
                );
                OH_Drawing_PointDestroy(center);
                fx
            }
            _ => {
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
            }
        }
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

/// What a shape or text record draws, decoded once and drawn up to twice per copy: as its
/// shadow, then as itself. Rectangles are `[left, top, right, bottom]`.
enum Geo {
    Rect([f32; 4]),
    RoundRect([f32; 4], f32),
    Oval([f32; 4]),
    /// Start and sweep, in degrees.
    Arc([f32; 4], f32, f32),
    /// `[x1, y1, x2, y2]`.
    Line([f32; 4]),
    /// A path and whether this record owns it (a cached one belongs to the cache).
    Path(*mut OH_Drawing_Path, bool),
    /// A text blob this record owns, and its baseline origin.
    Text(*mut OH_Drawing_TextBlob, f32, f32),
}

impl Geo {
    /// The bounding box a gradient's unit points resolve against, as (x, y, w, h): the
    /// geometry's, as `day_spec::Shape::bounds` gives it, without the stroke's width. Text
    /// takes no gradient.
    unsafe fn bounds(&self) -> Option<(f32, f32, f32, f32)> {
        match *self {
            Geo::Rect(r) | Geo::RoundRect(r, _) | Geo::Oval(r) | Geo::Arc(r, _, _) => {
                Some((r[0], r[1], r[2] - r[0], r[3] - r[1]))
            }
            Geo::Line(l) => Some((
                l[0].min(l[2]),
                l[1].min(l[3]),
                (l[2] - l[0]).abs(),
                (l[3] - l[1]).abs(),
            )),
            // SAFETY: a live path for the length of the record.
            Geo::Path(path, _) => Some(unsafe { path_bounds(path) }),
            Geo::Text(..) => None,
        }
    }

    /// Draw with whatever pen or brush is attached to `cv`.
    unsafe fn draw(&self, cv: *mut OH_Drawing_Canvas) {
        // SAFETY: a live canvas; scratch rects are destroyed here, and the path or blob is
        // alive until `release`.
        unsafe {
            match *self {
                Geo::Rect(r) => {
                    let rect = OH_Drawing_RectCreate(r[0], r[1], r[2], r[3]);
                    OH_Drawing_CanvasDrawRect(cv, rect);
                    OH_Drawing_RectDestroy(rect);
                }
                Geo::RoundRect(r, radius) => {
                    let rect = OH_Drawing_RectCreate(r[0], r[1], r[2], r[3]);
                    let rr = OH_Drawing_RoundRectCreate(rect, radius, radius);
                    OH_Drawing_CanvasDrawRoundRect(cv, rr);
                    OH_Drawing_RoundRectDestroy(rr);
                    OH_Drawing_RectDestroy(rect);
                }
                Geo::Oval(r) => {
                    let rect = OH_Drawing_RectCreate(r[0], r[1], r[2], r[3]);
                    OH_Drawing_CanvasDrawOval(cv, rect);
                    OH_Drawing_RectDestroy(rect);
                }
                Geo::Arc(r, start, sweep) => {
                    let rect = OH_Drawing_RectCreate(r[0], r[1], r[2], r[3]);
                    OH_Drawing_CanvasDrawArc(cv, rect, start, sweep);
                    OH_Drawing_RectDestroy(rect);
                }
                Geo::Line(l) => OH_Drawing_CanvasDrawLine(cv, l[0], l[1], l[2], l[3]),
                Geo::Path(path, _) => OH_Drawing_CanvasDrawPath(cv, path),
                Geo::Text(blob, x, y) => OH_Drawing_CanvasDrawTextBlob(cv, blob, x, y),
            }
        }
    }

    unsafe fn release(self) {
        // SAFETY: only what this record owns is destroyed.
        unsafe {
            match self {
                Geo::Path(path, true) => OH_Drawing_PathDestroy(path),
                Geo::Text(blob, _, _) => OH_Drawing_TextBlobDestroy(blob),
                _ => {}
            }
        }
    }
}

/// The pen or the brush a record draws with: the four things the shadow pass changes and puts
/// back, spelled once for both.
#[derive(Clone, Copy)]
enum Tool {
    Pen(*mut OH_Drawing_Pen),
    Brush(*mut OH_Drawing_Brush),
}

impl Tool {
    unsafe fn color(self, argb: u32) {
        // SAFETY: a live pen or brush.
        unsafe {
            match self {
                Tool::Pen(p) => OH_Drawing_PenSetColor(p, argb),
                Tool::Brush(b) => OH_Drawing_BrushSetColor(b, argb),
            }
        }
    }
    unsafe fn shader(self, fx: *mut OH_Drawing_ShaderEffect) {
        // SAFETY: a live pen or brush; a null effect clears it.
        unsafe {
            match self {
                Tool::Pen(p) => OH_Drawing_PenSetShaderEffect(p, fx),
                Tool::Brush(b) => OH_Drawing_BrushSetShaderEffect(b, fx),
            }
        }
    }
    unsafe fn filter(self, filter: *mut OH_Drawing_Filter) {
        // SAFETY: a live pen or brush and a live filter.
        unsafe {
            match self {
                Tool::Pen(p) => OH_Drawing_PenSetFilter(p, filter),
                Tool::Brush(b) => OH_Drawing_BrushSetFilter(b, filter),
            }
        }
    }
    unsafe fn attach(self, cv: *mut OH_Drawing_Canvas) {
        // SAFETY: a live canvas and a live pen or brush.
        unsafe {
            match self {
                Tool::Pen(p) => OH_Drawing_CanvasAttachPen(cv, p),
                Tool::Brush(b) => OH_Drawing_CanvasAttachBrush(cv, b),
            }
        }
    }
    unsafe fn detach(self, cv: *mut OH_Drawing_Canvas) {
        // SAFETY: a live canvas.
        unsafe {
            match self {
                Tool::Pen(_) => OH_Drawing_CanvasDetachPen(cv),
                Tool::Brush(_) => OH_Drawing_CanvasDetachBrush(cv),
            }
        }
    }
}

/// A canvas shadow as its record set it (kind 24), in points.
#[derive(Clone, Copy)]
struct CastShadow {
    dx: f32,
    dy: f32,
    blur: f32,
    color: u32,
}

/// A shadow resolved for the current transform: where to move the drawing in the current
/// coordinates so that it lands the shadow's offset away in points, and the Gaussian's standard
/// deviation in those coordinates, which the transform then scales back to half the blur.
#[derive(Clone, Copy)]
struct ShadowPass {
    vx: f32,
    vy: f32,
    sigma: f32,
    color: u32,
}

/// The graphics state Day scopes with save and restore that `OH_Drawing` keeps on the pen and
/// brush, not in the canvas: the shadow and the blend mode. The transform is tracked beside them
/// because a shadow is not transformed and has to be worked back through it.
#[derive(Clone, Copy)]
struct GState {
    shadow: Option<CastShadow>,
    /// [`day_spec::BlendMode::wire`]; a number this decoder does not know is stored as 0.
    blend: i32,
    lin: paint::Linear,
}

impl GState {
    fn shadow_pass(&self) -> Option<ShadowPass> {
        let s = self.shadow?;
        let (vx, vy) = self.lin.unmap(s.dx, s.dy)?;
        Some(ShadowPass {
            vx,
            vy,
            sigma: s.blur / 2.0 / self.lin.radius_scale(),
            color: s.color,
        })
    }
}

/// The `OH_Drawing` mode for a [`day_spec::BlendMode::wire`] number; an unknown one is normal.
fn blend_mode(wire: i32) -> OH_Drawing_BlendMode {
    match wire {
        1 => OH_Drawing_BlendMode::BLEND_MODE_MULTIPLY,
        2 => OH_Drawing_BlendMode::BLEND_MODE_SCREEN,
        3 => OH_Drawing_BlendMode::BLEND_MODE_OVERLAY,
        4 => OH_Drawing_BlendMode::BLEND_MODE_DARKEN,
        5 => OH_Drawing_BlendMode::BLEND_MODE_LIGHTEN,
        6 => OH_Drawing_BlendMode::BLEND_MODE_PLUS,
        // Erase: destination-out removes what was drawn by the source's alpha.
        7 => OH_Drawing_BlendMode::BLEND_MODE_DST_OUT,
        _ => OH_Drawing_BlendMode::BLEND_MODE_SRC_OVER,
    }
}

/// The filter objects a replay makes for its shadows. A pen or brush refers to its filter, and
/// the filter to its parts, so all of them live until the last record has been drawn, like the
/// dash effects.
struct Filters {
    /// An empty filter: what takes a blur off a pen or brush again.
    none: *mut OH_Drawing_Filter,
    /// The last blur made and its sigma. Consecutive records under one shadow share it.
    blur: Option<(f32, *mut OH_Drawing_Filter)>,
    made: Vec<*mut OH_Drawing_Filter>,
    masks: Vec<*mut OH_Drawing_MaskFilter>,
    images: Vec<*mut OH_Drawing_ImageFilter>,
    colors: Vec<*mut OH_Drawing_ColorFilter>,
}

impl Filters {
    unsafe fn new() -> Self {
        Filters {
            // SAFETY: a fresh filter, destroyed in `destroy`.
            none: unsafe { OH_Drawing_FilterCreate() },
            blur: None,
            made: Vec::new(),
            masks: Vec::new(),
            images: Vec::new(),
            colors: Vec::new(),
        }
    }

    /// A filter that blurs the shape a pen or brush draws by `sigma`, in the coordinates current
    /// when it is drawn: the shadow of a fill, a stroke or text. The empty filter for a hard
    /// edge.
    unsafe fn blur(&mut self, sigma: f32) -> *mut OH_Drawing_Filter {
        if !sigma.is_finite() || sigma <= 0.0 {
            return self.none;
        }
        if let Some((s, filter)) = self.blur
            && s.to_bits() == sigma.to_bits()
        {
            return filter;
        }
        // SAFETY: fresh objects, kept alive in the lists until `destroy`.
        unsafe {
            let mask = OH_Drawing_MaskFilterCreateBlur(OH_Drawing_BlurType::NORMAL, sigma, true);
            let filter = OH_Drawing_FilterCreate();
            OH_Drawing_FilterSetMaskFilter(filter, mask);
            self.masks.push(mask);
            self.made.push(filter);
            self.blur = Some((sigma, filter));
            filter
        }
    }

    /// A filter for a layer's brush that turns whatever the layer holds into its shadow: every
    /// pixel recolored to `color` by its own alpha, then blurred by `sigma`. The shadow of an
    /// image with transparency, whose silhouette is its pixels and not a shape.
    unsafe fn silhouette(&mut self, sigma: f32, color: u32) -> *mut OH_Drawing_Filter {
        // SAFETY: fresh objects, kept alive in the lists until `destroy`.
        unsafe {
            let filter = OH_Drawing_FilterCreate();
            let recolor = OH_Drawing_ColorFilterCreateBlendMode(
                color,
                OH_Drawing_BlendMode::BLEND_MODE_SRC_IN,
            );
            OH_Drawing_FilterSetColorFilter(filter, recolor);
            self.colors.push(recolor);
            if sigma.is_finite() && sigma > 0.0 {
                let blur = OH_Drawing_ImageFilterCreateBlur(
                    sigma,
                    sigma,
                    OH_Drawing_TileMode::DECAL,
                    ptr::null_mut(),
                );
                OH_Drawing_FilterSetImageFilter(filter, blur);
                self.images.push(blur);
            }
            self.made.push(filter);
            filter
        }
    }

    unsafe fn destroy(self) {
        // SAFETY: objects this replay created; nothing attached refers to them any more.
        unsafe {
            OH_Drawing_FilterDestroy(self.none);
            for f in self.made {
                OH_Drawing_FilterDestroy(f);
            }
            for m in self.masks {
                OH_Drawing_MaskFilterDestroy(m);
            }
            for i in self.images {
                OH_Drawing_ImageFilterDestroy(i);
            }
            for c in self.colors {
                OH_Drawing_ColorFilterDestroy(c);
            }
        }
    }
}

/// The next entry of the positional text channel.
fn take_text<'a>(texts: &'a [String], at: &mut usize) -> &'a str {
    let s = texts.get(*at).map_or("", String::as_str);
    *at += 1;
    s
}

/// Draw a pixel map region into `dst` with nothing attached to the canvas.
unsafe fn draw_pixel_map(
    cv: *mut OH_Drawing_Canvas,
    dpm: *mut OH_Drawing_PixelMap,
    src: *mut OH_Drawing_Rect,
    dst: *mut OH_Drawing_Rect,
) {
    // SAFETY: a live canvas, pixel map and rects; the scratch brush and options die here.
    unsafe {
        let so = OH_Drawing_SamplingOptionsCreate(
            OH_Drawing_FilterMode::FILTER_MODE_LINEAR,
            OH_Drawing_MipmapMode::MIPMAP_MODE_NONE,
        );
        // A fully transparent rect first. A recording whose only content is pixel map draws
        // renders nothing; the clear fill gives the recording content without changing a pixel.
        let clear = OH_Drawing_BrushCreate();
        OH_Drawing_BrushSetColor(clear, 0x0000_0000);
        OH_Drawing_CanvasAttachBrush(cv, clear);
        OH_Drawing_CanvasDrawRect(cv, dst);
        OH_Drawing_CanvasDetachBrush(cv);
        OH_Drawing_BrushDestroy(clear);
        OH_Drawing_CanvasDrawPixelMapRect(cv, dpm, src, dst, so);
        OH_Drawing_SamplingOptionsDestroy(so);
    }
}

/// Replay `ops` onto `cv` at `px_per_vp` pixels per day point: the node's density on screen,
/// the export scale for [`render_bitmap`]. The one painter both paths share. `size` is the
/// canvas in points, the bounds of the layer a drawing that blends is isolated in.
unsafe fn draw(ops: &Ops, cv: *mut OH_Drawing_Canvas, px_per_vp: f32, size: (f32, f32)) {
    let n = &ops.nums;
    let texts = &ops.texts;
    // SAFETY: every OH_Drawing object created in this replay is destroyed before it returns;
    // the canvas is the caller's (ArkUI's draw callback, or an export's bitmap) for the call.
    unsafe {
        let pen = OH_Drawing_PenCreate();
        OH_Drawing_PenSetAntiAlias(pen, true);
        let brush = OH_Drawing_BrushCreate();
        OH_Drawing_BrushSetAntiAlias(brush, true);

        // Base transform: scale vp → px so Day's point-space ops land correctly.
        OH_Drawing_CanvasSave(cv);
        let scale = OH_Drawing_MatrixCreate();
        let d = px_per_vp;
        OH_Drawing_MatrixSetMatrix(scale, d, 0.0, 0.0, 0.0, d, 0.0, 0.0, 0.0, 1.0);
        OH_Drawing_CanvasConcatMatrix(cv, scale);

        // A canvas is its own compositing group (docs/canvas.md "Blend modes"): a blend mode
        // combines with what this canvas has drawn, never with the views behind it, and Erase
        // leaves a hole those views show through. The node's drawing is recorded and replayed
        // into whatever surface its ancestors share, so a drawing that blends goes into a layer
        // of its own, transparent to start with and composited normally when it ends. A
        // drawing with no blend record pays nothing.
        let isolated = n
            .as_chunks::<9>()
            .0
            .iter()
            .any(|r| r[0] as i32 == 25 && (1..=7).contains(&(r[1] as i32)));
        // The layer's brush: a plain one composites the group normally. Alive until the layer
        // has been restored.
        let group = OH_Drawing_BrushCreate();
        if isolated {
            let bounds = if size.0 > 0.0 && size.1 > 0.0 {
                OH_Drawing_RectCreate(0.0, 0.0, size.0, size.1)
            } else {
                ptr::null_mut()
            };
            OH_Drawing_CanvasSaveLayer(cv, bounds, group);
            if !bounds.is_null() {
                OH_Drawing_RectDestroy(bounds);
            }
        }

        let mut text_i = 0usize;
        let mut grad = Gradient::default();
        let mut style = StrokeStyle::default();
        let mut fontp: Option<FontReq> = None;
        // A decoded kind-23 record (image source): the region, in the bitmap's pixels, the
        // next image record draws instead of the whole bitmap.
        let mut srcp: Option<day_spec::Rect> = None;
        // Dash effects created during this replay, destroyed once the last op has been drawn.
        let mut dash_effects: Vec<*mut OH_Drawing_PathEffect> = Vec::new();
        let mut filters = Filters::new();
        // The shadow, the blend mode and the transform, with the stack that scopes them: one
        // entry per save record still open.
        let mut gs = GState {
            shadow: None,
            blend: 0,
            lin: paint::Linear::IDENTITY,
        };
        let mut saved: Vec<GState> = Vec::new();
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
            match kind {
                // Stamp prefix and its coordinate records: collected, never drawn on their own.
                20 => {
                    stamp_at.clear();
                    stamp_n = a.max(0.0) as usize;
                    continue;
                }
                21 => {
                    // Four points per record, in slot pairs (1,2) (3,4) (5,6) (7,8): the fourth
                    // point's y rides the slot other records use for their color. The last
                    // record of a run is padded with zeros, so the header's count says where
                    // the real ones stop.
                    let xs = [a, c, e, g];
                    let ys = [b, dd, f, n[i - 1] as f32];
                    for q in 0..4 {
                        if stamp_at.len() < stamp_n {
                            stamp_at.push((xs[q], ys[q]));
                        }
                    }
                    continue;
                }
                0..=7 | 11..=13 | 15 | 16 => {
                    let stroke = matches!(kind, 1 | 4 | 5 | 6 | 12 | 13 | 16);
                    let rect = [a, b, a + c, b + dd];
                    let geo = match kind {
                        0 | 1 => Geo::Rect(rect),
                        // rounded rect fill / stroke (radius = e)
                        2 | 13 => Geo::RoundRect(rect, e),
                        3 | 4 => Geo::Oval(rect),
                        // arc (start = e, sweep = f)
                        5 => Geo::Arc(rect, e, f),
                        6 => Geo::Line([a, b, c, dd]),
                        7 => {
                            // text: size = e, anchor = f packed as h*4+v (TextAnchor::pack);
                            // the string on the text channel
                            let s = take_text(texts, &mut text_i);
                            let req = fontp.clone().unwrap_or_default();
                            // The cache's font: resolved once, drawn every frame.
                            let font = fonts::font(e, &req);
                            let cs = node::cstr(s);
                            let blob = OH_Drawing_TextBlobCreateFromString(
                                cs.as_ptr(),
                                font,
                                OH_Drawing_TextEncoding::TEXT_ENCODING_UTF8,
                            );
                            // DrawTextBlob takes the BASELINE; the anchor positions the line
                            // box from the font metrics (Skia-style: ascent negative, descent
                            // positive), the same formula as TextAnchor::offset and the Android
                            // canvas backend, so glyphs land in the same place on every
                            // platform.
                            let m = fonts::metrics(font);
                            let (ah, av) = (f as i32 / 4, f as i32 % 4);
                            let line_h = m.descent - m.ascent;
                            let mut x = a;
                            let mut y = b - m.ascent;
                            if ah != 0 {
                                let w = fonts::text_width(font, s, e);
                                x = a + if ah == 1 { -w / 2.0 } else { -w };
                            }
                            if av == 1 {
                                y = b - f32::midpoint(m.ascent, m.descent);
                            } else if av == 2 {
                                y = b; // `at` IS the baseline
                            } else if av == 3 {
                                y = b - line_h - m.ascent;
                            }
                            Geo::Text(blob, x, y)
                        }
                        11 | 12 => {
                            // polygon fill / stroke: points ride the text channel as
                            // "x,y x,y …". The text entry is consumed either way: that channel
                            // is positional, so a cache hit that skipped it would desynchronize
                            // every record after this one.
                            let pts = take_text(texts, &mut text_i);
                            let (path, owned) = cached_polygon(pts, f64::from(a));
                            Geo::Path(path, owned)
                        }
                        _ => {
                            // path fill / stroke: segments ride the text channel; f = fill rule
                            let spec = take_text(texts, &mut text_i);
                            let (path, owned) = cached_path(spec, f as i32, f64::from(a));
                            Geo::Path(path, owned)
                        }
                    };
                    let tool = if stroke {
                        OH_Drawing_PenSetColor(pen, col);
                        OH_Drawing_PenSetWidth(pen, if g > 0.0 { g } else { 1.0 });
                        // A kind-18 record styles the next stroke only; otherwise Day's
                        // defaults apply.
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
                            OH_Drawing_PenSetJoin(
                                pen,
                                OH_Drawing_PenLineJoinStyle::LINE_MITER_JOIN,
                            );
                            OH_Drawing_PenSetMiterLimit(pen, 10.0);
                            OH_Drawing_PenSetPathEffect(pen, ptr::null_mut());
                        }
                        Tool::Pen(pen)
                    } else {
                        OH_Drawing_BrushSetColor(brush, col);
                        Tool::Brush(brush)
                    };
                    // A pending gradient (kind 14) paints the next shape record, fill or
                    // stroke, as the shader of whichever of the two draws it, resolved against
                    // the shape's bounds.
                    let mut fx = ptr::null_mut();
                    if grad.active
                        && let Some((bx, by, bw, bh)) = geo.bounds()
                    {
                        fx = gradient_shader(&grad, bx, by, bw, bh);
                        grad.active = false;
                    }
                    tool.shader(fx);
                    // The shadow takes the strength of what casts it: a half-transparent fill
                    // casts half a shadow. A gradient's record color is opaque.
                    let shadow = gs.shadow_pass().map(|s| ShadowPass {
                        color: paint::scale_alpha(s.color, (col >> 24) as f32 / 255.0),
                        ..s
                    });
                    // The template is drawn once per position under a translated canvas.
                    let single = [(0.0, 0.0)];
                    let stamped = !stamp_at.is_empty();
                    let places: &[(f32, f32)] = if stamped { &stamp_at } else { &single };
                    for &(px, py) in places {
                        if stamped {
                            OH_Drawing_CanvasSave(cv);
                            OH_Drawing_CanvasTranslate(cv, px, py);
                        }
                        if let Some(s) = shadow {
                            // The same geometry through the same pen or brush, in the shadow's
                            // color, blurred, and moved by the offset. The blur is a mask
                            // filter, a true Gaussian over the shape's coverage, so a stroke's
                            // shadow is the stroke's and text's is its glyphs'.
                            tool.color(s.color);
                            tool.shader(ptr::null_mut());
                            tool.filter(filters.blur(s.sigma));
                            OH_Drawing_CanvasSave(cv);
                            OH_Drawing_CanvasTranslate(cv, s.vx, s.vy);
                            tool.attach(cv);
                            geo.draw(cv);
                            tool.detach(cv);
                            OH_Drawing_CanvasRestore(cv);
                            tool.color(col);
                            tool.shader(fx);
                            tool.filter(filters.none);
                        }
                        tool.attach(cv);
                        geo.draw(cv);
                        tool.detach(cv);
                        if stamped {
                            OH_Drawing_CanvasRestore(cv);
                        }
                    }
                    tool.shader(ptr::null_mut());
                    if !fx.is_null() {
                        OH_Drawing_ShaderEffectDestroy(fx);
                    }
                    geo.release();
                }
                8 => {
                    OH_Drawing_CanvasSave(cv);
                    saved.push(gs);
                }
                9 => {
                    // A restore with no save open would pop this replay's own base state.
                    if let Some(outer) = saved.pop() {
                        OH_Drawing_CanvasRestore(cv);
                        gs = outer;
                        OH_Drawing_PenSetBlendMode(pen, blend_mode(gs.blend));
                        OH_Drawing_BrushSetBlendMode(brush, blend_mode(gs.blend));
                    }
                }
                10 => {
                    // concat affine [a b c d tx ty] (day_geometry::Affine, column vectors)
                    let m = OH_Drawing_MatrixCreate();
                    OH_Drawing_MatrixSetMatrix(m, a, c, e, b, dd, f, 0.0, 0.0, 1.0);
                    OH_Drawing_CanvasConcatMatrix(cv, m);
                    OH_Drawing_MatrixDestroy(m);
                    gs.lin = gs.lin.concat(paint::Linear { a, b, c, d: dd });
                }
                17 => {
                    // clip: f names the shape, a..dd geometry, e radius or fill rule. The
                    // three fixed-size kinds build a path of their own and always own it;
                    // only the two payload kinds can come from the cache.
                    let (clip, owned) = match f as i32 {
                        3 => {
                            let spec = take_text(texts, &mut text_i);
                            cached_path(spec, e as i32, f64::from(a))
                        }
                        4 => {
                            let pts = take_text(texts, &mut text_i);
                            cached_polygon(pts, f64::from(a))
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
                    let dashes = take_text(texts, &mut text_i);
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
                19 => {
                    // font for the next text: a weight (0 default), b italic; family on
                    // the text channel
                    let family = take_text(texts, &mut text_i).to_owned();
                    fontp = Some(FontReq {
                        weight: a as i32,
                        italic: b > 0.5,
                        family,
                    });
                }
                14 => {
                    // set-gradient (f = type): stops ride the text channel as
                    // "offset,aarrggbb offset,aarrggbb ..."
                    let stops = take_text(texts, &mut text_i);
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
                    let region = srcp.take();
                    if !pm.is_null() && c > 0.0 && dd > 0.0 {
                        let dpm = OH_Drawing_PixelMapGetFromOhPixelMapNative(pm);
                        if !dpm.is_null() {
                            let (iw, ih, alpha_type) = crate::images::info(pm);
                            // The whole pixel map, or the pending region (kind 23) clamped
                            // to it with its mapping into the rect kept; `DrawPixelMapRect`
                            // takes the source in pixels, exactly the space the op names.
                            let rects = if iw > 0 && ih > 0 {
                                let whole =
                                    day_spec::Rect::new(0.0, 0.0, f64::from(iw), f64::from(ih));
                                let dest = day_spec::Rect::new(
                                    f64::from(a),
                                    f64::from(b),
                                    f64::from(c),
                                    f64::from(dd),
                                );
                                match region {
                                    None => Some((whole, dest)),
                                    Some(src) => {
                                        day_spec::clamp_image_source(src, dest, whole.size)
                                    }
                                }
                            } else {
                                None
                            };
                            if let Some((sr, dr)) = rects {
                                let src = OH_Drawing_RectCreate(
                                    sr.origin.x as f32,
                                    sr.origin.y as f32,
                                    (sr.origin.x + sr.size.width) as f32,
                                    (sr.origin.y + sr.size.height) as f32,
                                );
                                let (dl, dt, dr_, db) = (
                                    dr.origin.x as f32,
                                    dr.origin.y as f32,
                                    (dr.origin.x + dr.size.width) as f32,
                                    (dr.origin.y + dr.size.height) as f32,
                                );
                                let dst = OH_Drawing_RectCreate(dl, dt, dr_, db);
                                let opacity = f.clamp(0.0, 1.0);
                                if let Some(s) = gs.shadow_pass() {
                                    let color = paint::scale_alpha(s.color, opacity);
                                    let opaque = alpha_type
                                        == PIXELMAP_ALPHA_TYPE::PIXELMAP_ALPHA_TYPE_OPAQUE.0 as i32;
                                    OH_Drawing_CanvasSave(cv);
                                    if opaque {
                                        // An image with no transparency casts its rectangle's
                                        // shadow, drawn like any filled shape's.
                                        OH_Drawing_BrushSetColor(brush, color);
                                        OH_Drawing_BrushSetShaderEffect(brush, ptr::null_mut());
                                        OH_Drawing_BrushSetFilter(brush, filters.blur(s.sigma));
                                        OH_Drawing_CanvasTranslate(cv, s.vx, s.vy);
                                        OH_Drawing_CanvasAttachBrush(cv, brush);
                                        OH_Drawing_CanvasDrawRect(cv, dst);
                                        OH_Drawing_CanvasDetachBrush(cv);
                                        OH_Drawing_BrushSetFilter(brush, filters.none);
                                    } else {
                                        // One with transparency casts the shadow of its own
                                        // pixels: it is drawn into a layer whose brush recolors
                                        // and blurs the layer as it is composited. The layer
                                        // reaches three sigma past the image, where a Gaussian
                                        // has faded out.
                                        let cast = OH_Drawing_BrushCreate();
                                        OH_Drawing_BrushSetBlendMode(cast, blend_mode(gs.blend));
                                        OH_Drawing_BrushSetFilter(
                                            cast,
                                            filters.silhouette(s.sigma, color),
                                        );
                                        let pad = if s.sigma.is_finite() {
                                            s.sigma.max(0.0) * 3.0 + 1.0
                                        } else {
                                            1.0
                                        };
                                        let bounds = OH_Drawing_RectCreate(
                                            dl + s.vx - pad,
                                            dt + s.vy - pad,
                                            dr_ + s.vx + pad,
                                            db + s.vy + pad,
                                        );
                                        OH_Drawing_CanvasSaveLayer(cv, bounds, cast);
                                        OH_Drawing_CanvasTranslate(cv, s.vx, s.vy);
                                        draw_pixel_map(cv, dpm, src, dst);
                                        OH_Drawing_CanvasRestore(cv);
                                        OH_Drawing_RectDestroy(bounds);
                                        OH_Drawing_BrushDestroy(cast);
                                    }
                                    OH_Drawing_CanvasRestore(cv);
                                }
                                // Opacity and the blend mode ride a layer: `DrawPixelMapRect`
                                // takes no brush, so a brush on a saved layer is the only way
                                // to compose the image below full strength or in another mode.
                                let layered = opacity < 0.999 || gs.blend != 0;
                                let layer = OH_Drawing_BrushCreate();
                                if layered {
                                    OH_Drawing_BrushSetAlpha(layer, (opacity * 255.0 + 0.5) as u8);
                                    OH_Drawing_BrushSetBlendMode(layer, blend_mode(gs.blend));
                                    OH_Drawing_CanvasSaveLayer(cv, dst, layer);
                                }
                                draw_pixel_map(cv, dpm, src, dst);
                                if layered {
                                    OH_Drawing_CanvasRestore(cv);
                                }
                                OH_Drawing_BrushDestroy(layer);
                                OH_Drawing_RectDestroy(dst);
                                OH_Drawing_RectDestroy(src);
                            }
                            OH_Drawing_PixelMapDissolve(dpm);
                        }
                    }
                }
                // Image source (23): the region (a,b origin · c,dd size, in the bitmap's
                // pixels) the next image record draws.
                23 => {
                    srcp = Some(day_spec::Rect::new(
                        f64::from(a),
                        f64::from(b),
                        f64::from(c),
                        f64::from(dd),
                    ));
                }
                // Shadow (24): a,b the offset and c the blur, in points; the color slot is the
                // shadow's, and a fully transparent one clears it. State, until the restore
                // that matches the save it was set under.
                24 => {
                    gs.shadow = (col >> 24 != 0).then_some(CastShadow {
                        dx: a,
                        dy: b,
                        blur: c.max(0.0),
                        color: col,
                    });
                }
                // Blend mode (25): a = the mode's wire number. State, scoped like the shadow.
                25 => {
                    let wire = a as i32;
                    gs.blend = if (1..=7).contains(&wire) { wire } else { 0 };
                    OH_Drawing_PenSetBlendMode(pen, blend_mode(gs.blend));
                    OH_Drawing_BrushSetBlendMode(brush, blend_mode(gs.blend));
                }
                _ => {}
            }
            stamp_at.clear();
            // A style record applies to one stroke; anything else clears it.
            if kind != 18 {
                style.active = false;
            }
            if kind != 19 {
                fontp = None;
            }
            if kind != 23 {
                srcp = None;
            }
        }
        // Saves the drawing left open, then the blend group, then the base transform.
        for _ in saved.drain(..) {
            OH_Drawing_CanvasRestore(cv);
        }
        if isolated {
            OH_Drawing_CanvasRestore(cv);
        }
        OH_Drawing_BrushDestroy(group);
        OH_Drawing_CanvasRestore(cv);
        OH_Drawing_MatrixDestroy(scale);
        // Detach before destroying: the pen still holds whichever effect was set last.
        OH_Drawing_PenSetPathEffect(pen, ptr::null_mut());
        for fx in dash_effects {
            OH_Drawing_PathEffectDestroy(fx);
        }
        OH_Drawing_PenSetFilter(pen, filters.none);
        OH_Drawing_BrushSetFilter(brush, filters.none);
        OH_Drawing_PenDestroy(pen);
        OH_Drawing_BrushDestroy(brush);
        filters.destroy();
    }
}
