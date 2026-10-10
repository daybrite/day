// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The ArkUI node API (`arkui/native_node.h`) through `ohos_sys::arkui` (docs/harmonyos.md).
//!
//! `ArkUI_NativeNodeAPI_1` is a table of function pointers the runtime hands out by name; this
//! module resolves it once per thread and wraps the calls Day makes: creating and parenting
//! nodes, the attribute setters (every attribute is a `NumberValue` array, a string, or an
//! object), measurement, and event registration. The semantic setters below (`set_text`,
//! `set_frame`, the label runs, the picker range) are the attribute recipes the backend needs,
//! one per ArkUI quirk, so `lib.rs` reads as toolkit logic rather than attribute plumbing.
//!
//! Everything here runs on the JS thread, which is Day's UI thread on this backend.

// A `Handle` is an opaque token the runtime owns and only the runtime dereferences: the wrappers
// hand it back to ArkUI's own functions, which check it, so they are not `unsafe` to call.
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::{CStr, CString, c_void};
use std::ptr::{self, NonNull};

use ohos_sys::arkui::native_interface::{
    ArkUI_NativeAPIVariantKind, OH_ArkUI_QueryModuleInterfaceByName,
};
use ohos_sys::arkui::native_interface_focus::{OH_ArkUI_FocusClear, OH_ArkUI_FocusRequest};
use ohos_sys::arkui::native_node::{
    ArkUI_AttributeItem, ArkUI_NativeNodeAPI_1, ArkUI_NodeAttributeType as Attr,
    ArkUI_NodeCustomEvent, ArkUI_NodeCustomEventType, ArkUI_NodeDirtyFlag, ArkUI_NodeEvent,
    ArkUI_NodeEventType, ArkUI_NodeType, OH_ArkUI_GetContextByNode, OH_ArkUI_NodeContent_AddNode,
    OH_ArkUI_NodeUtils_GetLayoutPositionInWindow, OH_ArkUI_NodeUtils_GetLayoutSize,
    OH_ArkUI_NodeUtils_GetNodeType,
};
use ohos_sys::arkui::native_type::*;

/// An `ArkUI_NodeHandle`: the opaque node pointer the API deals in.
pub use ohos_sys_opaque_types::ArkUI_NodeHandle as Handle;

// The node kinds this backend realizes, by their ArkUI names.
pub const STACK: ArkUI_NodeType = ArkUI_NodeType::ARKUI_NODE_STACK;
pub const TEXT: ArkUI_NodeType = ArkUI_NodeType::ARKUI_NODE_TEXT;
pub const SPAN: ArkUI_NodeType = ArkUI_NodeType::ARKUI_NODE_SPAN;
pub const BUTTON: ArkUI_NodeType = ArkUI_NodeType::ARKUI_NODE_BUTTON;
pub const TEXT_INPUT: ArkUI_NodeType = ArkUI_NodeType::ARKUI_NODE_TEXT_INPUT;
pub const TEXT_AREA: ArkUI_NodeType = ArkUI_NodeType::ARKUI_NODE_TEXT_AREA;
pub const TEXT_PICKER: ArkUI_NodeType = ArkUI_NodeType::ARKUI_NODE_TEXT_PICKER;
pub const TOGGLE: ArkUI_NodeType = ArkUI_NodeType::ARKUI_NODE_TOGGLE;
pub const SLIDER: ArkUI_NodeType = ArkUI_NodeType::ARKUI_NODE_SLIDER;
pub const SCROLL: ArkUI_NodeType = ArkUI_NodeType::ARKUI_NODE_SCROLL;
pub const COLUMN: ArkUI_NodeType = ArkUI_NodeType::ARKUI_NODE_COLUMN;
pub const ROW: ArkUI_NodeType = ArkUI_NodeType::ARKUI_NODE_ROW;
pub const LOADING: ArkUI_NodeType = ArkUI_NodeType::ARKUI_NODE_LOADING_PROGRESS;
pub const PROGRESS: ArkUI_NodeType = ArkUI_NodeType::ARKUI_NODE_PROGRESS;
pub const IMAGE: ArkUI_NodeType = ArkUI_NodeType::ARKUI_NODE_IMAGE;
pub const CUSTOM: ArkUI_NodeType = ArkUI_NodeType::ARKUI_NODE_CUSTOM;
pub const SWIPER: ArkUI_NodeType = ArkUI_NodeType::ARKUI_NODE_SWIPER;
pub const LIST: ArkUI_NodeType = ArkUI_NodeType::ARKUI_NODE_LIST;
pub const LIST_ITEM: ArkUI_NodeType = ArkUI_NodeType::ARKUI_NODE_LIST_ITEM;

// The node events the built-in pieces register, by their ArkUI names.
pub const EV_CLICK: ArkUI_NodeEventType = ArkUI_NodeEventType::NODE_ON_CLICK;
pub const EV_TEXT_INPUT_CHANGE: ArkUI_NodeEventType =
    ArkUI_NodeEventType::NODE_TEXT_INPUT_ON_CHANGE;
pub const EV_TEXT_AREA_CHANGE: ArkUI_NodeEventType = ArkUI_NodeEventType::NODE_TEXT_AREA_ON_CHANGE;
pub const EV_TEXT_PICKER_CHANGE: ArkUI_NodeEventType =
    ArkUI_NodeEventType::NODE_TEXT_PICKER_EVENT_ON_CHANGE;
pub const EV_TOGGLE_CHANGE: ArkUI_NodeEventType = ArkUI_NodeEventType::NODE_TOGGLE_ON_CHANGE;
pub const EV_TOUCH: ArkUI_NodeEventType = ArkUI_NodeEventType::NODE_TOUCH_EVENT;
pub const EV_KEY: ArkUI_NodeEventType = ArkUI_NodeEventType::NODE_ON_KEY_EVENT;
pub const EV_SLIDER_CHANGE: ArkUI_NodeEventType = ArkUI_NodeEventType::NODE_SLIDER_EVENT_ON_CHANGE;
pub const EV_SWIPER_CHANGE: ArkUI_NodeEventType = ArkUI_NodeEventType::NODE_SWIPER_EVENT_ON_CHANGE;

thread_local! {
    static API: Cell<Option<NonNull<ArkUI_NativeNodeAPI_1>>> = const { Cell::new(None) };
    /// Display density (px per vp): ArkUI attributes are vp, measure and layout are px.
    static DENSITY: Cell<f64> = const { Cell::new(1.0) };
    /// The `NODE_ACCESSIBILITY_VALUE` object each node was last given (see `set_a11y`).
    static A11Y_VALUES: RefCell<HashMap<usize, *mut ArkUI_AccessibilityValue>> =
        RefCell::new(HashMap::new());
}

/// The node API table, resolved on first use. `None` only before ArkUI's native module is
/// loadable, in which case every operation below is a no-op.
pub fn api() -> Option<&'static ArkUI_NativeNodeAPI_1> {
    API.with(|slot| {
        if let Some(p) = slot.get() {
            // SAFETY: the table is a process-lifetime static inside ArkUI.
            return Some(unsafe { &*p.as_ptr() });
        }
        // SAFETY: a name lookup with a valid C string.
        let raw = unsafe {
            OH_ArkUI_QueryModuleInterfaceByName(
                ArkUI_NativeAPIVariantKind::ARKUI_NATIVE_NODE,
                c"ArkUI_NativeNodeAPI_1".as_ptr(),
            )
        };
        let p = NonNull::new(raw.cast::<ArkUI_NativeNodeAPI_1>())?;
        slot.set(Some(p));
        // SAFETY: as above.
        Some(unsafe { &*p.as_ptr() })
    })
}

/// Call one entry of the node API table, `None` when the table or the entry is missing.
macro_rules! api_call {
    ($f:ident ( $($a:expr),* )) => {
        match $crate::node::api().and_then(|a| a.$f) {
            // SAFETY: the table's entries are the runtime's own functions, called with the
            // argument types the header declares.
            Some(f) => Some(unsafe { f($($a),*) }),
            None => None,
        }
    };
}

pub fn density() -> f64 {
    DENSITY.with(|d| d.get())
}

pub fn set_density(density: f64) {
    DENSITY.with(|d| d.set(if density > 0.0 { density } else { 1.0 }));
}

/// A C string for an attribute, with interior NULs stripped rather than the text blanked.
pub(crate) fn cstr(s: &str) -> CString {
    CString::new(s).unwrap_or_else(|_| {
        let stripped: Vec<u8> = s.bytes().filter(|b| *b != 0).collect();
        CString::new(stripped).unwrap_or_default()
    })
}

pub fn f32v(v: f32) -> ArkUI_NumberValue {
    ArkUI_NumberValue { f32_: v }
}
pub fn i32v(v: i32) -> ArkUI_NumberValue {
    ArkUI_NumberValue { i32_: v }
}
pub fn u32v(v: u32) -> ArkUI_NumberValue {
    ArkUI_NumberValue { u32_: v }
}

// ---- tree -------------------------------------------------------------------------------

pub fn create(kind: ArkUI_NodeType) -> Handle {
    api_call!(createNode(kind)).unwrap_or(ptr::null_mut())
}

/// Dispose a node, first erasing the per-node side state this backend keeps (canvas display
/// lists, list adapters, drag registrations): `createNode` can recycle the address, and a stale
/// entry would then alias the new node.
pub fn dispose(n: Handle) {
    if n.is_null() {
        return;
    }
    crate::transfer::forget(n);
    crate::canvas::forget(n);
    crate::list::forget(n);
    forget_a11y(n);
    label_runs_clear(n);
    api_call!(disposeNode(n));
}

/// Dispose `n` and everything under it, deepest first. Only for a subtree this backend built
/// whole for its own chrome (a nav menu's rows, a tab-bar cell): [`dispose`] frees one node, and
/// freeing just the root of such a subtree left every row, label and icon in it allocated. Never
/// for a container holding day's nodes, which day releases one by one through `release`.
pub fn dispose_tree(n: Handle) {
    if n.is_null() {
        return;
    }
    let children: Vec<Handle> = (0..child_count(n) as i32)
        .map(|i| child_at(n, i))
        .filter(|c| !c.is_null())
        .collect();
    for c in children {
        remove_child(n, c);
        dispose_tree(c);
    }
    dispose(n);
}

/// Dispose a node this backend built for its own chrome (a list cell, a probe): no side state
/// to erase, unlike [`dispose`].
pub fn dispose_raw(n: Handle) {
    api_call!(disposeNode(n));
}

pub fn add_child(parent: Handle, child: Handle) {
    api_call!(addChild(parent, child));
}

pub fn insert_child(parent: Handle, child: Handle, position: i32) {
    api_call!(insertChildAt(parent, child, position));
}

pub fn remove_child(parent: Handle, child: Handle) {
    api_call!(removeChild(parent, child));
}

pub fn child_count(n: Handle) -> u32 {
    api_call!(getTotalChildCount(n)).unwrap_or(0)
}

pub fn child_at(n: Handle, index: i32) -> Handle {
    api_call!(getChildAt(n, index)).unwrap_or(ptr::null_mut())
}

/// The node's parent in the C-API tree, or null at the top (a node mounted in an ArkTS
/// `NodeContent`, or a detached one).
pub fn parent(n: Handle) -> Handle {
    api_call!(getParent(n)).unwrap_or(ptr::null_mut())
}

/// The node's kind as ArkUI reports it, `None` for a kind the C API does not name.
pub fn node_type(n: Handle) -> Option<ArkUI_NodeType> {
    // SAFETY: a query on a live node; -1 answers a kind with no C-API name.
    let t = unsafe { OH_ArkUI_NodeUtils_GetNodeType(n) };
    u32::try_from(t).ok().map(ArkUI_NodeType)
}

/// The node's laid-out frame in window coordinates, in px: the layout box, without the
/// translate/scale channels. `None` when ArkUI refuses either query.
pub fn layout_frame_px(n: Handle) -> Option<(f64, f64, f64, f64)> {
    let mut pos = ArkUI_IntOffset { x: 0, y: 0 };
    let mut size = ArkUI_IntSize {
        width: 0,
        height: 0,
    };
    // SAFETY: queries on a live node into out-params that live across the calls.
    let ok = unsafe {
        OH_ArkUI_NodeUtils_GetLayoutPositionInWindow(n, &mut pos) == 0
            && OH_ArkUI_NodeUtils_GetLayoutSize(n, &mut size) == 0
    };
    ok.then(|| {
        (
            f64::from(pos.x),
            f64::from(pos.y),
            f64::from(size.width),
            f64::from(size.height),
        )
    })
}

pub fn mark_dirty(n: Handle, flag: ArkUI_NodeDirtyFlag) {
    api_call!(markDirty(n, flag));
}

/// Mount `node` into an ArkTS `NodeContent` slot. Returns 0 on success.
pub fn content_add(content: ArkUI_NodeContentHandle, node: Handle) -> i32 {
    // SAFETY: both handles are live objects the host and this backend own.
    unsafe { OH_ArkUI_NodeContent_AddNode(content, node) }
}

// ---- attributes -------------------------------------------------------------------------

pub fn set_values(n: Handle, attr: Attr, values: &[ArkUI_NumberValue]) -> i32 {
    let item = ArkUI_AttributeItem {
        value: values.as_ptr(),
        size: values.len() as i32,
        string: ptr::null(),
        object: ptr::null_mut(),
    };
    api_call!(setAttribute(n, attr, &item)).unwrap_or(-1)
}

pub fn set_str(n: Handle, attr: Attr, s: &str) -> i32 {
    set_str_values(n, attr, s, &[])
}

/// A string attribute that also carries numbers (the picker range needs its type beside the
/// options).
pub fn set_str_values(n: Handle, attr: Attr, s: &str, values: &[ArkUI_NumberValue]) -> i32 {
    let s = cstr(s);
    let item = ArkUI_AttributeItem {
        value: if values.is_empty() {
            ptr::null()
        } else {
            values.as_ptr()
        },
        size: values.len() as i32,
        string: s.as_ptr(),
        object: ptr::null_mut(),
    };
    api_call!(setAttribute(n, attr, &item)).unwrap_or(-1)
}

pub fn set_object(n: Handle, attr: Attr, object: *mut c_void) -> i32 {
    let item = ArkUI_AttributeItem {
        value: ptr::null(),
        size: 0,
        string: ptr::null(),
        object,
    };
    api_call!(setAttribute(n, attr, &item)).unwrap_or(-1)
}

/// Put an attribute back to ArkUI's own default.
pub fn reset(n: Handle, attr: Attr) {
    api_call!(resetAttribute(n, attr));
}

pub fn set_f32(n: Handle, attr: Attr, v: f32) -> i32 {
    set_values(n, attr, &[f32v(v)])
}
pub fn set_i32(n: Handle, attr: Attr, v: i32) -> i32 {
    set_values(n, attr, &[i32v(v)])
}
pub fn set_u32(n: Handle, attr: Attr, v: u32) -> i32 {
    set_values(n, attr, &[u32v(v)])
}

/// The attribute as ArkUI holds it. The item is ArkUI's, valid until the next attribute call.
pub fn get_item(n: Handle, attr: Attr) -> Option<&'static ArkUI_AttributeItem> {
    let p = api_call!(getAttribute(n, attr))?;
    // SAFETY: a non-null item ArkUI owns; callers read it before the next API call.
    unsafe { p.as_ref() }
}

pub fn get_values(n: Handle, attr: Attr) -> Vec<ArkUI_NumberValue> {
    match get_item(n, attr) {
        Some(it) if !it.value.is_null() && it.size > 0 => {
            // SAFETY: `size` values live at `value` for the item's lifetime.
            unsafe { std::slice::from_raw_parts(it.value, it.size as usize) }.to_vec()
        }
        _ => Vec::new(),
    }
}

pub fn get_f32(n: Handle, attr: Attr, index: usize) -> Option<f32> {
    // SAFETY: the attribute's slots are floats.
    get_values(n, attr).get(index).map(|v| unsafe { v.f32_ })
}

pub fn get_i32(n: Handle, attr: Attr, index: usize) -> Option<i32> {
    // SAFETY: the attribute's slots are integers.
    get_values(n, attr).get(index).map(|v| unsafe { v.i32_ })
}

/// Copy one attribute from `src` to `dst`, as ArkUI reports it.
pub fn copy_attr(src: Handle, dst: Handle, attr: Attr) {
    if let Some(it) = get_item(src, attr) {
        api_call!(setAttribute(dst, attr, it));
    }
}

// ---- events -----------------------------------------------------------------------------

/// Register a node event; `id` returns as the event's user data.
pub fn register_event(n: Handle, event: ArkUI_NodeEventType, id: u64) {
    api_call!(registerNodeEvent(n, event, 0, id as usize as *mut c_void));
}

/// The global event receiver every registered node event reaches.
pub fn register_event_receiver(receiver: unsafe extern "C" fn(*mut ArkUI_NodeEvent)) {
    api_call!(registerNodeEventReceiver(Some(receiver)));
}

/// An additive per-node receiver, for a piece that observes its own node's events without
/// touching the global one (docs/extending.md).
pub fn add_event_receiver(n: Handle, receiver: unsafe extern "C" fn(*mut ArkUI_NodeEvent)) {
    api_call!(addNodeEventReceiver(n, Some(receiver)));
}

pub fn register_custom_event(
    n: Handle,
    event: ArkUI_NodeCustomEventType,
    target: i32,
    user_data: *mut c_void,
) {
    api_call!(registerNodeCustomEvent(n, event, target, user_data));
}

pub fn add_custom_event_receiver(
    n: Handle,
    receiver: unsafe extern "C" fn(*mut ArkUI_NodeCustomEvent),
) {
    api_call!(addNodeCustomEventReceiver(n, Some(receiver)));
}

// ---- measurement ------------------------------------------------------------------------

/// Measure `n` under a proposal (`<= 0` = unbounded), in vp.
pub fn measure(n: Handle, max_w: f64, max_h: f64) -> (f64, f64) {
    if n.is_null() || api().is_none() {
        return (0.0, 0.0);
    }
    let d = density();
    let mw = if max_w > 0.0 {
        (max_w * d) as i32
    } else {
        1_000_000
    };
    let mh = if max_h > 0.0 {
        (max_h * d) as i32
    } else {
        1_000_000
    };
    // SAFETY: a constraint object created here and disposed below.
    let c = unsafe {
        let c = OH_ArkUI_LayoutConstraint_Create();
        OH_ArkUI_LayoutConstraint_SetMaxWidth(c, mw);
        OH_ArkUI_LayoutConstraint_SetMaxHeight(c, mh);
        OH_ArkUI_LayoutConstraint_SetMinWidth(c, 0);
        OH_ArkUI_LayoutConstraint_SetMinHeight(c, 0);
        c
    };
    api_call!(measureNode(n, c));
    let size = api_call!(getMeasuredSize(n)).unwrap_or(ArkUI_IntSize {
        width: 0,
        height: 0,
    });
    // SAFETY: the constraint created above, no longer referenced.
    unsafe { OH_ArkUI_LayoutConstraint_Dispose(c) };
    (f64::from(size.width) / d, f64::from(size.height) / d)
}

/// The text attributes a label's measuring copy carries.
const TEXT_MEASURE_ATTRS: [Attr; 10] = [
    Attr::NODE_TEXT_CONTENT,
    Attr::NODE_FONT_SIZE,
    Attr::NODE_FONT_WEIGHT,
    Attr::NODE_FONT_STYLE,
    Attr::NODE_FONT_FAMILY,
    Attr::NODE_FONT_FEATURE,
    Attr::NODE_TEXT_MAX_LINES,
    Attr::NODE_TEXT_LETTER_SPACING,
    Attr::NODE_TEXT_LINE_HEIGHT,
    Attr::NODE_TEXT_BASELINE_OFFSET,
];

/// Measure a label (a TEXT node) on a detached copy. After a Text's content changes,
/// `measureNode` on that node keeps answering the previous text's size until ArkUI's own layout
/// pass has run (`markDirty` and a different constraint don't clear it), so a label whose text
/// grows is laid out at its old width and wraps or clips. A TEXT node outside the tree carrying
/// the same text and font attributes has no such cache once its attributes are reset and it is
/// marked for measurement (see the probe below). A styled label (SPAN children) can't be copied this
/// simply, so it keeps the direct measure, as does anything the copy sizes to nothing.
pub fn measure_label(n: Handle, max_w: f64, max_h: f64) -> (f64, f64) {
    if n.is_null() || api().is_none() {
        return (0.0, 0.0);
    }
    if child_count(n) > 0 {
        return measure(n, max_w, max_h);
    }
    // One probe for the thread's life, not a fresh node per measurement. Creating and disposing
    // a Text leaks inside ArkUI (OpenHarmony 6.1): measured on the emulator, a Showcase grid
    // visit (about 1,750 measurements) grew the process by 26 MB with a fresh probe each time
    // and by 11 MB with this one, though every probe was disposed. The probe is never attached
    // to a tree. Each use resets every attribute it copies, so nothing carries over from the
    // previous label (`copy_attr` skips an attribute the source leaves unset), and marks it for
    // measurement, which is what makes `measureNode` answer for the new attributes rather than
    // the old ones, the reason this used a fresh copy.
    thread_local! {
        static PROBE: Cell<Handle> = const { Cell::new(ptr::null_mut()) };
    }
    let probe = PROBE.with(|p| {
        if p.get().is_null() {
            p.set(create(TEXT));
        }
        p.get()
    });
    if probe.is_null() {
        return measure(n, max_w, max_h);
    }
    for attr in TEXT_MEASURE_ATTRS {
        api_call!(resetAttribute(probe, attr));
        copy_attr(n, probe, attr);
    }
    mark_dirty(probe, ArkUI_NodeDirtyFlag::NODE_NEED_MEASURE);
    let size = measure(probe, max_w, max_h);
    // Nothing to size on the copy: a styled label keeps its text in SPAN children (which
    // getTotalChildCount doesn't count) and an empty content, so measure it directly.
    if size.0 <= 0.0 {
        return measure(n, max_w, max_h);
    }
    size
}

thread_local! {
    /// The capsule around one line of button text: (pad_w, pad_h, min_h), calibrated once.
    static BUTTON_CHROME: Cell<Option<(f64, f64, f64)>> = const { Cell::new(None) };
}

/// Measure a button. A title button measures a fresh copy, for the reason a label does: after
/// the label attribute changes, `measureNode` answers with the old title's size. A button with
/// custom `content` (the icon + title Row the backend inserts) is sized from that Row plus the
/// button's own padding, because ArkUI measures the Button at its 32 vp minimum without regard
/// to the Row, which then spilled out of the capsule.
pub fn measure_button(n: Handle, content: Handle, max_w: f64, max_h: f64) -> (f64, f64) {
    if n.is_null() || api().is_none() {
        return (0.0, 0.0);
    }
    if content.is_null() {
        // Only the title is copied: Day sets no font on a button (just its title and colors),
        // and the NODE_FONT_* getters are Text-model accessors that crash on a Button node.
        let probe = create(BUTTON);
        let mut size = (0.0, 0.0);
        if !probe.is_null() {
            copy_attr(n, probe, Attr::NODE_BUTTON_LABEL);
            size = measure(probe, max_w, max_h);
            api_call!(disposeNode(probe));
        }
        if size.0 <= 0.0 {
            size = measure(n, max_w, max_h);
        }
        return size;
    }
    // The capsule's padding: a fresh "M" button against a fresh "M" text at the button's
    // default font size. Their difference is the padding; the button's height is its minimum.
    // Measured once, since the theme doesn't change while the app runs.
    let (pad_w, pad_h, min_h) = BUTTON_CHROME.with(|c| {
        if let Some(v) = c.get() {
            return v;
        }
        let b = create(BUTTON);
        let t = create(TEXT);
        let mut v = (0.0, 0.0, 0.0);
        if !b.is_null() && !t.is_null() {
            set_str(b, Attr::NODE_BUTTON_LABEL, "M");
            set_str(t, Attr::NODE_TEXT_CONTENT, "M");
            // ArkUI's button title is 16 fp by default; the text gets the same explicitly.
            set_f32(t, Attr::NODE_FONT_SIZE, 16.0);
            let (bw, bh) = measure(b, -1.0, -1.0);
            let (tw, th) = measure(t, -1.0, -1.0);
            v = ((bw - tw).max(0.0), (bh - th).max(0.0), bh);
            c.set(Some(v));
        }
        if !b.is_null() {
            api_call!(disposeNode(b));
        }
        if !t.is_null() {
            api_call!(disposeNode(t));
        }
        v
    });
    let inner = |max: f64, pad: f64| {
        if max > 0.0 {
            if max > pad { max - pad } else { 1.0 }
        } else {
            -1.0
        }
    };
    let (cw, ch) = measure(content, inner(max_w, pad_w), inner(max_h, pad_h));
    (cw + pad_w, (ch + pad_h).max(min_h))
}

/// First text baseline from the node's top, in vp, for a box `box_h` tall (docs/baseline.md).
/// The ArkUI C API exposes no baseline: NODE_FONT_SIZE is the only type metric a native node
/// will answer, so this centers one line box of that font in the node's height and puts the
/// baseline an ascent below its top, the same model the Qt and XAML backends use. `None` ⇒ the
/// node carries no font attribute, which is how a container or an image opts out.
pub fn baseline(n: Handle, box_h: f64) -> Option<f64> {
    let size = f64::from(get_f32(n, Attr::NODE_FONT_SIZE, 0)?);
    if size <= 0.0 || size.is_nan() {
        return None;
    }
    // HarmonyOS Sans metrics, matching the ratios the other font-derived backends use.
    let ascent = size * 0.86;
    let line = size * 1.33;
    let top = if box_h > line {
        (box_h - line) / 2.0
    } else {
        0.0
    };
    Some(top + ascent)
}

// ---- semantic setters -------------------------------------------------------------------

pub fn set_text(n: Handle, s: &str) {
    set_str(n, Attr::NODE_TEXT_CONTENT, s);
}

pub fn label_single_line(n: Handle) {
    set_i32(n, Attr::NODE_TEXT_MAX_LINES, 1);
    set_i32(
        n,
        Attr::NODE_TEXT_OVERFLOW,
        ArkUI_TextOverflow::ARKUI_TEXT_OVERFLOW_ELLIPSIS.0 as i32,
    );
}

/// Make a Text node's text user-selectable (the `.selectable()` modifier, docs/text.md). A
/// non-Text node ignores the attribute (setAttribute returns an error, no crash).
pub fn label_set_selectable(n: Handle, on: bool) {
    let opt = if on {
        ArkUI_CopyOptions::ARKUI_COPY_OPTIONS_LOCAL_DEVICE
    } else {
        ArkUI_CopyOptions::ARKUI_COPY_OPTIONS_NONE
    };
    set_i32(n, Attr::NODE_TEXT_COPY_OPTION, opt.0 as i32);
}

/// Start replacing a label's children with styled RUNS (docs/text-runs.md).
///
/// ArkUI models runs as SPAN child nodes of a Text, not as attributes on one widget, so a
/// styled label is a small subtree here, unlike every other backend. The Text's own content is
/// cleared first: a Text with both content and spans renders the content and ignores the spans.
pub fn label_runs_begin(n: Handle) {
    set_text(n, "");
    label_runs_clear(n);
}

thread_local! {
    /// Each styled Text's SPAN children, in order (docs/text-runs.md). ArkUI takes a span
    /// through `addChild` but never hands it back: on OpenHarmony 6.1 (API 18)
    /// `getTotalChildCount` answers 0 for a Text holding spans and `getChildAt` returns null,
    /// while the spans render. So the backend keeps the handles it attached. Clearing a label's
    /// runs reaches the old spans through this list (a child walk found none, so a changed run
    /// list stacked its spans after the old ones), and `native_text` reads each span's own
    /// NODE_SPAN_CONTENT through it.
    static LABEL_SPANS: RefCell<HashMap<usize, Vec<Handle>>> = RefCell::new(HashMap::new());
}

/// Remove and dispose a label's spans, last to first: before new runs, and when the label goes
/// back to plain text, which renders only once its spans are gone.
pub fn label_runs_clear(n: Handle) {
    let spans = LABEL_SPANS.with(|m| m.borrow_mut().remove(&(n as usize)));
    for c in spans.unwrap_or_default().into_iter().rev() {
        remove_child(n, c);
        api_call!(disposeNode(c));
    }
}

/// The spans [`label_runs_add`] attached to `n`, in order (empty for a plain label).
pub fn label_spans(n: Handle) -> Vec<Handle> {
    LABEL_SPANS.with(|m| m.borrow().get(&(n as usize)).cloned().unwrap_or_default())
}

/// One run's styling flags for [`label_runs_add`].
#[derive(Clone, Copy, Default)]
pub struct RunStyle {
    pub bold: bool,
    pub italic: bool,
    pub monospace: bool,
    pub strikethrough: bool,
    pub underline: bool,
    pub color: Option<u32>,
    pub background: Option<u32>,
    /// The run's own size in fp, where its scale differs from the label's.
    pub size_fp: Option<f64>,
}

/// Append one run as a SPAN child.
pub fn label_runs_add(n: Handle, text: &str, style: RunStyle) {
    let span = create(SPAN);
    if span.is_null() {
        return;
    }
    set_str(span, Attr::NODE_SPAN_CONTENT, text);
    if style.bold {
        set_i32(
            span,
            Attr::NODE_FONT_WEIGHT,
            ArkUI_FontWeight::ARKUI_FONT_WEIGHT_BOLD.0 as i32,
        );
    }
    if style.italic {
        set_i32(
            span,
            Attr::NODE_FONT_STYLE,
            ArkUI_FontStyle::ARKUI_FONT_STYLE_ITALIC.0 as i32,
        );
    }
    if style.monospace {
        set_str(
            span,
            Attr::NODE_FONT_FAMILY,
            "HarmonyOS Sans Mono, monospace",
        );
    }
    // ArkUI has one decoration attribute per span, so a run that is both underlined and struck
    // through can only have one line. Strikethrough wins: it is the one that changes what the
    // text MEANS, and an underline that goes missing is cosmetic (docs/text-runs.md).
    if style.strikethrough || style.underline {
        let kind = if style.strikethrough {
            ArkUI_TextDecorationType::ARKUI_TEXT_DECORATION_TYPE_LINE_THROUGH
        } else {
            ArkUI_TextDecorationType::ARKUI_TEXT_DECORATION_TYPE_UNDERLINE
        };
        // Decoration takes {type, color, style}; the color slot repeats the text color so the
        // line matches the glyphs.
        let color = style.color.unwrap_or(0xFF00_0000);
        set_values(
            span,
            Attr::NODE_TEXT_DECORATION,
            &[i32v(kind.0 as i32), u32v(color)],
        );
    }
    if let Some(bg) = style.background {
        // A span's own background (docs/text-runs.md): {color, then optional corner radii}.
        set_values(span, Attr::NODE_SPAN_TEXT_BACKGROUND_STYLE, &[u32v(bg)]);
    }
    if let Some(fp) = style.size_fp {
        set_f32(span, Attr::NODE_FONT_SIZE, fp as f32);
    }
    if let Some(color) = style.color {
        set_u32(span, Attr::NODE_FONT_COLOR, color);
    }
    add_child(n, span);
    LABEL_SPANS.with(|m| m.borrow_mut().entry(n as usize).or_default().push(span));
}

pub fn set_enabled(n: Handle, enabled: bool) {
    set_i32(n, Attr::NODE_ENABLED, i32::from(enabled));
}

pub fn set_button_border(n: Handle, width: f64, color: u32) {
    set_f32(n, Attr::NODE_BORDER_WIDTH, width as f32);
    set_u32(n, Attr::NODE_BORDER_COLOR, color);
}

pub fn set_button_label(n: Handle, s: &str) {
    set_str(n, Attr::NODE_BUTTON_LABEL, s);
}

pub fn set_input_text(n: Handle, s: &str) {
    set_str(n, Attr::NODE_TEXT_INPUT_TEXT, s);
}

pub fn set_placeholder(n: Handle, s: &str) {
    set_str(n, Attr::NODE_TEXT_INPUT_PLACEHOLDER, s);
}

/// The text a TextInput holds now.
pub fn input_text(n: Handle) -> String {
    get_item(n, Attr::NODE_TEXT_INPUT_TEXT).map_or_else(String::new, |it| text_of(it.string))
}

/// Text entry traits on a TextInput (docs/textfield.md): the members ArkUI has an attribute
/// for. Every one is written on every call, so a member back at its default leaves nothing
/// behind. Read-only has no attribute; `lib.rs` holds it through [`watch_input_edits`].
pub fn set_input_traits(n: Handle, traits: &day_spec::InputTraits) {
    use ArkUI_EnterKeyType as Key;
    use ArkUI_TextInputContentType as Content;
    use ArkUI_TextInputType as Type;
    use day_spec::{InputPurpose as P, SubmitLabel as L};
    // ArkUI folds hidden characters into the input type, so the type comes from both members.
    // Its numeric password takes digits only, which fits a PIN and nothing else: a hidden
    // decimal or phone number takes the general password type, which accepts every character.
    let ty = match (traits.secure, traits.purpose) {
        (true, P::Number) => Type::ARKUI_TEXTINPUT_TYPE_NUMBER_PASSWORD,
        (true, P::NewPassword) => Type::ARKUI_TEXTINPUT_TYPE_NEW_PASSWORD,
        (true, _) => Type::ARKUI_TEXTINPUT_TYPE_PASSWORD,
        (false, P::Email) => Type::ARKUI_TEXTINPUT_TYPE_EMAIL,
        (false, P::Phone) => Type::ARKUI_TEXTINPUT_TYPE_PHONE_NUMBER,
        (false, P::Number) => Type::ARKUI_TEXTINPUT_TYPE_NUMBER,
        (false, P::Decimal) => Type::ARKUI_TEXTINPUT_TYPE_NUMBER_DECIMAL,
        (false, P::Username) => Type::ARKUI_TEXTINPUT_TYPE_USER_NAME,
        // A URL and a one-time code have types from API 20 on only; a name and a password
        // shown in the clear are ordinary text.
        (false, _) => Type::ARKUI_TEXTINPUT_TYPE_NORMAL,
    };
    if get_i32(n, Attr::NODE_TEXT_INPUT_TYPE, 0) != Some(ty.0 as i32) {
        // The text survives a type change; the caret is put back where the user had it.
        let caret = get_i32(n, Attr::NODE_TEXT_INPUT_CARET_OFFSET, 0).filter(|c| *c > 0);
        set_i32(n, Attr::NODE_TEXT_INPUT_TYPE, ty.0 as i32);
        if let Some(caret) = caret {
            set_i32(n, Attr::NODE_TEXT_INPUT_CARET_OFFSET, caret);
        }
    }
    // The password types draw their own reveal icon, which would show the characters behind
    // the app's back: `secure` is the one switch, and the app draws its own control for it.
    set_i32(n, Attr::NODE_TEXT_INPUT_SHOW_PASSWORD_ICON, 0);
    // What the system offers to fill in, where the purpose names something it keeps.
    let content = match traits.purpose {
        P::Name => Some(Content::ARKUI_TEXTINPUT_CONTENT_TYPE_PERSON_FULL_NAME),
        P::Email => Some(Content::ARKUI_TEXTINPUT_CONTENT_EMAIL_ADDRESS),
        P::Phone => Some(Content::ARKUI_TEXTINPUT_CONTENT_TYPE_FULL_PHONE_NUMBER),
        P::Username => Some(Content::ARKUI_TEXTINPUT_CONTENT_TYPE_USER_NAME),
        P::Password => Some(Content::ARKUI_TEXTINPUT_CONTENT_TYPE_PASSWORD),
        P::NewPassword => Some(Content::ARKUI_TEXTINPUT_CONTENT_TYPE_NEW_PASSWORD),
        P::Text | P::Url | P::Number | P::Decimal | P::OneTimeCode => None,
    };
    match content {
        Some(c) => {
            set_i32(n, Attr::NODE_TEXT_INPUT_CONTENT_TYPE, c.0 as i32);
        }
        None => reset(n, Attr::NODE_TEXT_INPUT_CONTENT_TYPE),
    }
    // ArkUI's own key is Done, so that is what Return leaves in place.
    let key = match traits.submit_label {
        L::Return => None,
        L::Done => Some(Key::ARKUI_ENTER_KEY_TYPE_DONE),
        L::Go => Some(Key::ARKUI_ENTER_KEY_TYPE_GO),
        L::Next => Some(Key::ARKUI_ENTER_KEY_TYPE_NEXT),
        L::Search => Some(Key::ARKUI_ENTER_KEY_TYPE_SEARCH),
        L::Send => Some(Key::ARKUI_ENTER_KEY_TYPE_SEND),
    };
    match key {
        Some(k) => {
            set_i32(n, Attr::NODE_TEXT_INPUT_ENTER_KEY_TYPE, k.0 as i32);
        }
        None => reset(n, Attr::NODE_TEXT_INPUT_ENTER_KEY_TYPE),
    }
    // A field that takes no edits raises no keyboard when it is focused.
    set_i32(
        n,
        Attr::NODE_TEXT_INPUT_ENABLE_KEYBOARD_ON_FOCUS,
        i32::from(!traits.read_only),
    );
    // `max_length` takes no attribute: NODE_TEXT_INPUT_MAX_LENGTH does not say what it counts,
    // and the bound is in characters. day-pieces holds it.
}

/// Ask a TextInput before every insertion and deletion (docs/textfield.md): the receiver
/// answers each one, which is how a read-only field refuses edits while it stays focusable,
/// selectable and copyable.
pub fn watch_input_edits(n: Handle, id: u64) {
    register_event(n, ArkUI_NodeEventType::NODE_TEXT_INPUT_ON_WILL_INSERT, id);
    register_event(n, ArkUI_NodeEventType::NODE_TEXT_INPUT_ON_WILL_DELETE, id);
}

pub fn set_textarea_text(n: Handle, s: &str) {
    set_str(n, Attr::NODE_TEXT_AREA_TEXT, s);
}

pub fn set_textarea_placeholder(n: Handle, s: &str) {
    set_str(n, Attr::NODE_TEXT_AREA_PLACEHOLDER, s);
}

/// The picker wheel's options (docs/picker.md): a ';'-joined range, whose type must ride in
/// the item's value slot beside the strings, or ArkUI keeps no options and draws only its
/// divider lines; then the selected index.
pub fn set_picker(n: Handle, options_semi: &str, selected: u32) {
    set_str_values(
        n,
        Attr::NODE_TEXT_PICKER_OPTION_RANGE,
        options_semi,
        &[i32v(
            ArkUI_TextPickerRangeType::ARKUI_TEXTPICKER_RANGETYPE_SINGLE.0 as i32,
        )],
    );
    set_u32(n, Attr::NODE_TEXT_PICKER_OPTION_SELECTED, selected);
}

pub fn set_picker_selected(n: Handle, selected: u32) {
    set_u32(n, Attr::NODE_TEXT_PICKER_OPTION_SELECTED, selected);
}

pub fn set_toggle(n: Handle, on: bool) {
    set_i32(n, Attr::NODE_TOGGLE_VALUE, i32::from(on));
}

pub fn set_slider(n: Handle, v: f64) {
    set_f32(n, Attr::NODE_SLIDER_VALUE, v as f32);
}

/// An image node's source: a `resource://RAWFILE/<path>` URI, a file path, a URL or base64.
pub fn set_image_src(n: Handle, s: &str) {
    set_str(n, Attr::NODE_IMAGE_SRC, s);
}

/// Scaling (§18.3): `ArkUI_ObjectFit` CONTAIN=0 / COVER=1 / FILL=3.
pub fn set_image_fit(n: Handle, fit: i32) {
    set_i32(n, Attr::NODE_IMAGE_OBJECT_FIT, fit);
}

/// SVG-only recolor: repaints every path of an SVG src (raster sources ignore it), which is
/// how nav-row vector icons tint (docs/vectors.md).
pub fn set_image_fill(n: Handle, argb: u32) {
    set_u32(n, Attr::NODE_IMAGE_FILL_COLOR, argb);
}

/// One margin (vp) on all four sides; symmetric, so RTL needs no flip.
pub fn set_margin(n: Handle, vp: f64) {
    set_f32(n, Attr::NODE_MARGIN, vp as f32);
}

/// Absolute frame (Day owns layout): position + explicit size, in vp.
pub fn set_frame(n: Handle, x: f64, y: f64, w: f64, h: f64) {
    set_values(n, Attr::NODE_POSITION, &[f32v(x as f32), f32v(y as f32)]);
    set_size(n, w, h);
}

/// Explicit size only (no position), for children whose parent places them.
pub fn set_size(n: Handle, w: f64, h: f64) {
    set_f32(n, Attr::NODE_WIDTH, w as f32);
    set_f32(n, Attr::NODE_HEIGHT, h as f32);
}

pub fn set_bg_color(n: Handle, argb: u32) {
    set_u32(n, Attr::NODE_BACKGROUND_COLOR, argb);
}

pub fn set_font_size(n: Handle, vp: f64) {
    set_f32(n, Attr::NODE_FONT_SIZE, vp as f32);
}

pub fn set_font_color(n: Handle, argb: u32) {
    set_u32(n, Attr::NODE_FONT_COLOR, argb);
}

/// A bundled custom font family (§18.4), registered by the ArkTS EntryAbility before the
/// native UI loads; ArkUI falls back to the default family when the name isn't registered.
pub fn set_font_family(n: Handle, family: &str) {
    set_str(n, Attr::NODE_FONT_FAMILY, family);
}

/// An OpenType feature string (`"tnum 1"` for tabular figures). A font without the feature,
/// or an SDK predating the attribute, ignores it.
pub fn set_font_feature(n: Handle, feature: &str) {
    set_str(n, Attr::NODE_FONT_FEATURE, feature);
}

pub fn set_corner_radius(n: Handle, vp: f64) {
    set_f32(n, Attr::NODE_BORDER_RADIUS, vp as f32);
}

/// Weight (CSS 100..900, snapped to ArkUI's W100..W900 rungs) and italic for a text node. Both
/// are set every time, so a label that stops asking for bold or italic goes back to regular
/// upright.
pub fn set_font_weight_style(n: Handle, css_weight: i32, italic: bool) {
    let rung = ((css_weight + 50) / 100).clamp(1, 9);
    let weight = ArkUI_FontWeight::ARKUI_FONT_WEIGHT_W100.0 as i32 + (rung - 1);
    set_i32(n, Attr::NODE_FONT_WEIGHT, weight);
    let style = if italic {
        ArkUI_FontStyle::ARKUI_FONT_STYLE_ITALIC
    } else {
        ArkUI_FontStyle::ARKUI_FONT_STYLE_NORMAL
    };
    set_i32(n, Attr::NODE_FONT_STYLE, style.0 as i32);
}

/// Clip children to the node's (rounded) bounds. NODE_BORDER_RADIUS rounds only the node's own
/// background; `.corner_radius` puts the fill on an inner node, so the outer one must clip.
pub fn set_clip(n: Handle, on: bool) {
    set_i32(n, Attr::NODE_CLIP, i32::from(on));
}

// The animatable visual channels (§8.4): plain setters, animated by running them inside
// `anim::animate`'s apply closure, where ArkUI interpolates every attribute change.
pub fn set_opacity(n: Handle, opacity: f64) {
    set_f32(n, Attr::NODE_OPACITY, opacity as f32);
}

/// The pivot for scale + rotate, as fractions of the node's size (NODE_TRANSFORM_CENTER's
/// percentage slots [3]/[4] override the vp slots [0]/[1]).
pub fn set_transform_center(n: Handle, ax: f64, ay: f64) {
    set_values(
        n,
        Attr::NODE_TRANSFORM_CENTER,
        &[
            f32v(0.0),
            f32v(0.0),
            f32v(0.0),
            f32v(ax as f32),
            f32v(ay as f32),
        ],
    );
}

/// Translate (vp) + scale + z-rotation (degrees) about the transform center.
pub fn set_transform(n: Handle, tx: f64, ty: f64, sx: f64, sy: f64, deg: f64) {
    set_values(
        n,
        Attr::NODE_TRANSLATE,
        &[f32v(tx as f32), f32v(ty as f32), f32v(0.0)],
    );
    set_values(n, Attr::NODE_SCALE, &[f32v(sx as f32), f32v(sy as f32)]);
    // Rotation about the z axis (the screen normal); [4] = perspective, 0 = none.
    set_values(
        n,
        Attr::NODE_ROTATE,
        &[f32v(0.0), f32v(0.0), f32v(1.0), f32v(deg as f32), f32v(0.0)],
    );
}

/// A determinate progress bar: ArkUI uses a value in [0, total]; Day passes the 0..1 fraction,
/// scaled onto a fixed 0..1000 range (like day-android's LinearProgressIndicator ticks).
pub fn set_progress(n: Handle, fraction: f64) {
    set_f32(n, Attr::NODE_PROGRESS_TOTAL, 1000.0);
    set_f32(
        n,
        Attr::NODE_PROGRESS_VALUE,
        (fraction.clamp(0.0, 1.0) * 1000.0) as f32,
    );
}

/// Visibility: VISIBLE, else NONE (removed from layout; one TABS page shown at a time).
pub fn set_visibility(n: Handle, visible: bool) {
    let v = if visible {
        ArkUI_Visibility::ARKUI_VISIBILITY_VISIBLE
    } else {
        ArkUI_Visibility::ARKUI_VISIBILITY_NONE
    };
    set_i32(n, Attr::NODE_VISIBILITY, v.0 as i32);
}

/// Start or stop an indeterminate spinner (a LOADING_PROGRESS node). A stopped one is HIDDEN,
/// which keeps its layout box (so the row around it does not jump), as on Android and iOS.
pub fn set_loading(n: Handle, on: bool) {
    set_i32(n, Attr::NODE_LOADING_PROGRESS_ENABLE_LOADING, i32::from(on));
    let v = if on {
        ArkUI_Visibility::ARKUI_VISIBILITY_VISIBLE
    } else {
        ArkUI_Visibility::ARKUI_VISIBILITY_HIDDEN
    };
    set_i32(n, Attr::NODE_VISIBILITY, v.0 as i32);
}

/// The active page index of a Swiper.
pub fn set_swiper_index(n: Handle, i: i32) {
    set_i32(n, Attr::NODE_SWIPER_INDEX, i);
}

/// Configure a Swiper used as a tab pager: show the dot indicator, don't loop.
pub fn swiper_setup(n: Handle) {
    set_i32(n, Attr::NODE_SWIPER_SHOW_INDICATOR, 1);
    set_i32(n, Attr::NODE_SWIPER_LOOP, 0);
}

/// The label a screen reader announces for a node (a button's title, by default).
pub fn set_a11y_text(n: Handle, label: &str) {
    if !label.is_empty() {
        set_str(n, Attr::NODE_ACCESSIBILITY_TEXT, label);
    }
}

/// Accessibility (§13, docs/accessibility.md): every annotation the node carries, each written
/// only when set so an unset member leaves the component's own default alone.
///
/// - label → `NODE_ACCESSIBILITY_TEXT`; hint → `NODE_ACCESSIBILITY_DESCRIPTION`.
/// - value → `NODE_ACCESSIBILITY_VALUE`'s text member. The object is a native struct the
///   attribute call reads; it is kept per node until the next value or the node's disposal
///   rather than released on the spot, so the node never points at freed text.
/// - role → `NODE_ACCESSIBILITY_ROLE`, which takes an `ArkUI_NodeType`: a heading can only be
///   retyped as TEXT (its level has no attribute), a meter as PROGRESS, a group as STACK, and
///   a tree as LIST / LIST_ITEM. `NODE_ACCESSIBILITY_GROUP` is deliberately NOT set for a group:
///   it collapses the subtree into one focusable unit, which is the opposite of a container
///   whose children stay reachable.
/// - hidden → `NODE_ACCESSIBILITY_MODE` DISABLED_FOR_DESCENDANTS. Hidden is sticky, so the mode
///   is written only when set: nothing here ever puts a node back to AUTO.
/// - identifier → `NODE_ID`, the component id the ArkTS inspector addresses nodes by.
pub fn set_a11y(n: Handle, a11y: &day_spec::A11yProps) {
    use day_spec::Role;
    if let Some(label) = &a11y.label {
        set_a11y_text(n, label);
    }
    if let Some(hint) = &a11y.hint {
        set_str(n, Attr::NODE_ACCESSIBILITY_DESCRIPTION, hint);
    }
    if let Some(value) = &a11y.value {
        let text = cstr(value);
        // SAFETY: a value object created here and given a NUL-terminated string that outlives
        // the call; `forget_a11y` releases the object once the node no longer needs it.
        let obj = unsafe { OH_ArkUI_AccessibilityValue_Create() };
        if !obj.is_null() {
            // SAFETY: as above.
            unsafe { OH_ArkUI_AccessibilityValue_SetText(obj, text.as_ptr()) };
            set_object(n, Attr::NODE_ACCESSIBILITY_VALUE, obj.cast());
            let previous = A11Y_VALUES.with(|m| m.borrow_mut().insert(n as usize, obj));
            if let Some(previous) = previous {
                // SAFETY: the node now holds the new object; the old one is ours to release.
                unsafe { OH_ArkUI_AccessibilityValue_Dispose(previous) };
            }
        }
    }
    let role = match a11y.role {
        Role::None => None,
        Role::Button => Some(BUTTON),
        Role::Toggle => Some(TOGGLE),
        Role::Slider => Some(SLIDER),
        Role::TextInput => Some(TEXT_INPUT),
        Role::Heading(_) => Some(TEXT),
        Role::Image => Some(IMAGE),
        Role::Meter => Some(PROGRESS),
        Role::Group => Some(STACK),
        Role::Tree => Some(LIST),
        Role::TreeItem => Some(LIST_ITEM),
        // A tab reads as a button; the strip as a plain container. The selected state has no
        // attribute in the NDK this crate binds (`NODE_ACCESSIBILITY_STATE` arrives later), so
        // `selected` is not applied here.
        Role::Tab => Some(BUTTON),
        Role::TabList => Some(STACK),
    };
    if let Some(role) = role {
        set_u32(n, Attr::NODE_ACCESSIBILITY_ROLE, role.0);
    }
    if a11y.hidden || a11y.decorative {
        set_i32(
            n,
            Attr::NODE_ACCESSIBILITY_MODE,
            ArkUI_AccessibilityMode::ARKUI_ACCESSIBILITY_MODE_DISABLED_FOR_DESCENDANTS.0 as i32,
        );
    }
    if let Some(id) = &a11y.identifier {
        set_str(n, Attr::NODE_ID, id);
    }
}

/// Release the accessibility value object a node was given, before the node goes away
/// (`dispose`): the address may be recycled, and a stale entry would then alias the new node.
pub(crate) fn forget_a11y(n: Handle) {
    if let Some(obj) = A11Y_VALUES.with(|m| m.borrow_mut().remove(&(n as usize))) {
        // SAFETY: an object created in `set_a11y` that nothing else owns.
        unsafe { OH_ArkUI_AccessibilityValue_Dispose(obj) };
    }
}

/// Flex-grow within a Row/Column (the menu label grows so the chevron hugs the trailing edge).
pub fn set_flex_grow(n: Handle, grow: f64) {
    set_f32(n, Attr::NODE_FLEX_GROW, grow as f32);
}

/// A conventional list separator: a full-width hairline in the caller's theme-aware color.
pub fn menu_separator(n: Handle, argb: u32) {
    set_f32(n, Attr::NODE_WIDTH_PERCENT, 1.0);
    set_f32(n, Attr::NODE_HEIGHT, 0.7);
    set_bg_color(n, argb);
}

/// A NAV_MENU / tab-bar row: full width, `height_vp` tall, left-aligned padded text.
pub fn style_row(n: Handle, height_vp: f64) {
    set_f32(n, Attr::NODE_WIDTH_PERCENT, 1.0);
    set_f32(n, Attr::NODE_HEIGHT, height_vp as f32);
    set_f32(n, Attr::NODE_PADDING, 16.0);
    // START (0), which follows the layout direction.
    set_i32(n, Attr::NODE_TEXT_ALIGN, 0);
}

/// A NAV_MENU section title (docs/navigation.md): full width, medium weight, padded with more
/// room above than below. `first` trims the top padding for the list's head.
pub fn style_nav_heading(n: Handle, first: bool) {
    set_f32(n, Attr::NODE_WIDTH_PERCENT, 1.0);
    // Four padding values run top, right, bottom, left.
    let top = if first { 8.0 } else { 20.0 };
    set_values(
        n,
        Attr::NODE_PADDING,
        &[f32v(top), f32v(16.0), f32v(6.0), f32v(16.0)],
    );
    set_i32(
        n,
        Attr::NODE_FONT_WEIGHT,
        ArkUI_FontWeight::ARKUI_FONT_WEIGHT_MEDIUM.0 as i32,
    );
    set_i32(n, Attr::NODE_TEXT_ALIGN, 0);
}

/// The scroll axis of a SCROLL node (docs/shapes.md h-scroll).
pub fn scroll_direction(n: Handle, horizontal: bool) {
    let d = if horizontal {
        ArkUI_ScrollDirection::ARKUI_SCROLL_DIRECTION_HORIZONTAL
    } else {
        ArkUI_ScrollDirection::ARKUI_SCROLL_DIRECTION_VERTICAL
    };
    set_i32(n, Attr::NODE_SCROLL_SCROLL_DIRECTION, d.0 as i32);
}

/// The minimal scroll that reveals [x, y, w, h] (content vp): ArkUI positions by absolute
/// offset, so read the current offset and the node's own size, compute the reveal, and write
/// the offset back (clamped by the node to its scrollable range).
pub fn scroll_to_rect(n: Handle, x: f32, y: f32, w: f32, h: f32, animated: bool) {
    if n.is_null() || api().is_none() {
        return;
    }
    let offset = get_values(n, Attr::NODE_SCROLL_OFFSET);
    // SAFETY: the offset attribute's two slots are floats.
    let (ox, oy) = if offset.len() >= 2 {
        unsafe { (offset[0].f32_, offset[1].f32_) }
    } else {
        (0.0, 0.0)
    };
    let pw = get_f32(n, Attr::NODE_WIDTH, 0).unwrap_or(0.0);
    let ph = get_f32(n, Attr::NODE_HEIGHT, 0).unwrap_or(0.0);
    let (mut nx, mut ny) = (ox, oy);
    if x + w > nx + pw {
        nx = x + w - pw;
    }
    if x < nx {
        nx = x;
    }
    if y + h > ny + ph {
        ny = y + h - ph;
    }
    if y < ny {
        ny = y;
    }
    // The third slot is the animation duration in ms (0 = jump).
    set_values(
        n,
        Attr::NODE_SCROLL_OFFSET,
        &[f32v(nx), f32v(ny), i32v(if animated { 300 } else { 0 })],
    );
}

/// Focus (docs/focus.md): observe gain/blur (+ the text-input submit action) on the node.
pub fn enable_focus(n: Handle, id: u64, is_text_input: bool) {
    register_event(n, ArkUI_NodeEventType::NODE_ON_FOCUS, id);
    register_event(n, ArkUI_NodeEventType::NODE_ON_BLUR, id);
    if is_text_input {
        register_event(n, ArkUI_NodeEventType::NODE_TEXT_INPUT_ON_SUBMIT, id);
    }
}

/// Drive focus: request it (typed errors for non-focusable targets are deliberately ignored:
/// no event means the signal snaps back, docs/focus.md rule 2), or clear the UI context's focus,
/// only while this node still owns it, so a stale release can't blur a sibling.
pub fn focus(n: Handle, focused: bool) {
    if n.is_null() || api().is_none() {
        return;
    }
    if focused {
        // SAFETY: a live node.
        let _ = unsafe { OH_ArkUI_FocusRequest(n) };
        return;
    }
    let owns = get_i32(n, Attr::NODE_FOCUS_STATUS, 0).unwrap_or(0) != 0;
    if !owns {
        return;
    }
    // SAFETY: the node's UI context, cleared only when non-null.
    unsafe {
        let ctx = OH_ArkUI_GetContextByNode(n);
        if !ctx.is_null() {
            OH_ArkUI_FocusClear(ctx);
        }
    }
}

/// The custom-event target id a canvas registers its draw callback under.
pub const CANVAS_DRAW_TARGET: i32 = 77;

/// A C string from a native event's payload, or empty.
pub(crate) fn text_of(p: *const std::ffi::c_char) -> String {
    if p.is_null() {
        String::new()
    } else {
        // SAFETY: a NUL-terminated string the runtime owns for the callback's duration.
        unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
    }
}
