// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// Qt: this crate's OWN shim (src/lib-qt-shim.cpp) — a `QTextEdit` driven through a QTextCursor.
//
// Qt document positions are QChar counts, which are UTF-16 code units, so this arm shares the
// Apple arms' offset conversion rather than GTK's character one.
//
// Everything about how the attributes are applied — cursor rather than `setHtml`, one edit block
// per sweep, formatting shortcuts swallowed — is in the shim's header comment, which is where the
// reasoning belongs since it is C++ that has to hold it.
// ---------------------------------------------------------------------------

use super::*;
use std::collections::HashMap;
use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_void};

use day_qt::{Qt, QtHandle};
use day_spec::sidetable::SideTable;
use day_spec::{ListStyle, NodeId, ParagraphAlign, Proposal, Size, Underline};

unsafe extern "C" {
    fn day_texteditor_new(
        id: u64,
        editable: c_int,
        base_pt: f64,
        placeholder: *const c_char,
        text_cb: extern "C" fn(u64, *const c_char),
        sel_cb: extern "C" fn(u64, u64, u64),
    ) -> *mut c_void;
    fn day_texteditor_set_text(h: *mut c_void, utf8: *const c_char);
    fn day_texteditor_begin_attrs(h: *mut c_void);
    fn day_texteditor_apply_run(
        h: *mut c_void,
        start: c_int,
        len: c_int,
        pt: f64,
        weight: c_int,
        italic: c_int,
        mono: c_int,
        underline: c_int,
        strike: c_int,
        has_fg: c_int,
        fg: u32,
        has_bg: c_int,
        bg: u32,
    );
    fn day_texteditor_apply_paragraph(
        h: *mut c_void,
        start: c_int,
        len: c_int,
        align: c_int,
        indent: f64,
        space_before: f64,
        space_after: f64,
        marker: c_int,
    );
    fn day_texteditor_end_attrs(h: *mut c_void);
    fn day_texteditor_set_selection(h: *mut c_void, start: c_int, len: c_int);
    fn day_texteditor_set_typing(
        h: *mut c_void,
        pt: f64,
        weight: c_int,
        italic: c_int,
        mono: c_int,
        underline: c_int,
        strike: c_int,
        has_fg: c_int,
        fg: u32,
        has_bg: c_int,
        bg: u32,
    );
    fn day_texteditor_set_editable(h: *mut c_void, editable: c_int);
    fn day_texteditor_measure(
        h: *mut c_void,
        avail_w: f64,
        min_lines: u32,
        max_lines: u32,
        out_w: *mut f64,
        out_h: *mut f64,
    );
}

/// Per-view state: the measure band the shim needs on every call, plus the node id, which is what
/// ties a released widget to its entry in [`TEXT`].
struct EdState {
    node: u64,
    base_points: f64,
    min_lines: u32,
    max_lines: u32,
}

day_core::tls_group! {
    static STATE: SideTable<EdState> = SideTable::new();
    /// The text each editor currently holds, by node.
    ///
    /// Qt reports selections in UTF-16 units and offers no way to read the document from the
    /// callback, so the conversion back to bytes needs the string — and the C callback has only
    /// the node id to find it by. `on_text` keeps this current, which is exactly when it changes.
    static TEXT: RefCell<HashMap<u64, String>> = RefCell::new(HashMap::new());

}

fn text_of(node: u64) -> String {
    TEXT.with(|m| m.borrow().get(&node).cloned().unwrap_or_default())
}

fn remember_text(node: u64, text: &str) {
    TEXT.with(|m| {
        m.borrow_mut().insert(node, text.to_string());
    });
}

extern "C" fn on_text(id: u64, text: *const c_char) {
    // The pointer is a `QByteArray` alive only for this call, so the string is copied here.
    let s = if text.is_null() {
        String::new()
    } else {
        // SAFETY: the shim passes a NUL-terminated UTF-8 buffer that outlives the call.
        unsafe { std::ffi::CStr::from_ptr(text) }
            .to_string_lossy()
            .into_owned()
    };
    remember_text(id, &s);
    day_qt::emit(NodeId(id), Event::TextChanged(s));
}

extern "C" fn on_sel(id: u64, start: u64, end: u64) {
    // Qt counts QChars (UTF-16 units); the piece speaks bytes.
    let text = text_of(id);
    let a = byte_of_utf16(&text, start as usize);
    let b = byte_of_utf16(&text, end as usize);
    day_qt::emit(
        NodeId(id),
        Event::custom("texteditor:sel", selection_payload(a, b)),
    );
}

fn pack(c: day_spec::Color) -> u32 {
    let q = |v: f64| ((v.clamp(0.0, 1.0) * 255.0).round() as u32) & 0xFF;
    (q(c.a) << 24) | (q(c.r) << 16) | (q(c.g) << 8) | q(c.b)
}

/// Day's `FontWeight` as a Qt/CSS numeric weight (Qt 6 uses the same 100–900 scale).
fn qt_weight(spec: day_spec::FontSpec) -> c_int {
    use day_spec::FontWeight as W;
    match spec.weight {
        Some(W::UltraLight) => 100,
        Some(W::Thin) => 200,
        Some(W::Light) => 300,
        Some(W::Regular) => 400,
        Some(W::Medium) => 500,
        Some(W::Semibold) => 600,
        Some(W::Bold) => 700,
        Some(W::Heavy) => 800,
        Some(W::Black) => 900,
        None => 400,
    }
}

fn underline_code(u: Underline) -> c_int {
    match u {
        Underline::None => 0,
        Underline::Single => 1,
        Underline::Double => 2,
        Underline::Dotted => 3,
        Underline::Wavy => 4,
    }
}

/// Push one run into the widget, in UTF-16 units.
fn apply_run(h: *mut c_void, text: &str, r: &TextRun, base_points: f64) {
    let Some((start, len)) = utf16_range(text, &r.range) else {
        return;
    };
    let (has_fg, fg) = r.color.map(|c| (1, pack(c))).unwrap_or((0, 0));
    let (has_bg, bg) = r.background.map(|c| (1, pack(c))).unwrap_or((0, 0));
    unsafe {
        day_texteditor_apply_run(
            h,
            start as c_int,
            len as c_int,
            r.font.resolved_points(base_points),
            qt_weight(r.font),
            r.font.italic as c_int,
            r.font.monospace as c_int,
            underline_code(r.underline),
            r.strikethrough as c_int,
            has_fg,
            fg,
            has_bg,
            bg,
        )
    };
}

fn apply_attributes(
    h: *mut c_void,
    text: &str,
    runs: &[TextRun],
    paragraphs: &[ParagraphRun],
    base_points: f64,
) {
    unsafe { day_texteditor_begin_attrs(h) };
    for p in paragraphs {
        let Some((start, len)) = utf16_range(text, &p.range) else {
            continue;
        };
        let s = p.style();
        unsafe {
            day_texteditor_apply_paragraph(
                h,
                start as c_int,
                len as c_int,
                match s.align {
                    ParagraphAlign::Natural => 0,
                    ParagraphAlign::Center => 1,
                    ParagraphAlign::Trailing => 2,
                    ParagraphAlign::Justified => 3,
                },
                s.indent + f64::from(s.list_level) * 24.0,
                s.space_before,
                s.space_after,
                (s.list != ListStyle::None) as c_int,
            )
        };
    }
    for r in runs {
        apply_run(h, text, r, base_points);
    }
    unsafe { day_texteditor_end_attrs(h) };
}

fn make(_backend: &mut Qt, p: &EditorProps, id: NodeId) -> QtHandle {
    let placeholder = CString::new(p.placeholder.as_str()).unwrap_or_default();
    let base_points = day_qt::qt_style(p.base).0;
    let h = unsafe {
        day_texteditor_new(
            id.0,
            p.editable as c_int,
            base_points,
            placeholder.as_ptr(),
            on_text,
            on_sel,
        )
    };
    remember_text(id.0, &p.doc.text);
    if !p.doc.is_empty() {
        let text = CString::new(p.doc.text.as_str()).unwrap_or_default();
        unsafe { day_texteditor_set_text(h, text.as_ptr()) };
        apply_attributes(h, &p.doc.text, &p.doc.runs, &p.doc.paragraphs, base_points);
    }
    STATE.with(|t| {
        t.insert(
            h as usize,
            EdState {
                node: id.0,
                base_points,
                min_lines: p.min_lines,
                max_lines: p.max_lines,
            },
        )
    });
    QtHandle(h)
}

fn update(_backend: &mut Qt, h: &QtHandle, patch: &EditorPatch) {
    let Some((node, base_points)) =
        STATE.with(|t| t.with(h.0 as usize, |st| (st.node, st.base_points)))
    else {
        return;
    };
    match patch {
        EditorPatch::SetDocument(doc) => {
            let text = CString::new(doc.text.as_str()).unwrap_or_default();
            unsafe { day_texteditor_set_text(h.0, text.as_ptr()) };
            remember_text(node, &doc.text);
            apply_attributes(h.0, &doc.text, &doc.runs, &doc.paragraphs, base_points);
        }
        EditorPatch::SetAttributes(attrs) => {
            // The patch carries the text as well as the attributes, which is what keeps the
            // remembered copy from going one keystroke stale under a live highlighter.
            remember_text(node, &attrs.text);
            apply_attributes(
                h.0,
                &attrs.text,
                &attrs.runs,
                &attrs.paragraphs,
                base_points,
            );
        }
        EditorPatch::SetSelection(r) => {
            let text = text_of(node);
            if let Some((start, len)) = utf16_range(&text, r) {
                unsafe { day_texteditor_set_selection(h.0, start as c_int, len as c_int) };
            }
        }
        EditorPatch::SetTypingStyle(s) => {
            let (has_fg, fg) = s.color.map(|c| (1, pack(c))).unwrap_or((0, 0));
            let (has_bg, bg) = s.background.map(|c| (1, pack(c))).unwrap_or((0, 0));
            unsafe {
                day_texteditor_set_typing(
                    h.0,
                    s.font.resolved_points(base_points),
                    qt_weight(s.font),
                    s.font.italic as c_int,
                    s.font.monospace as c_int,
                    underline_code(s.underline),
                    s.strikethrough as c_int,
                    has_fg,
                    fg,
                    has_bg,
                    bg,
                )
            };
        }
        EditorPatch::SetEditable(v) => unsafe { day_texteditor_set_editable(h.0, *v as c_int) },
    }
}

fn measure(_backend: &mut Qt, h: &QtHandle, p: Proposal) -> Size {
    let avail_w = p.width.unwrap_or(320.0).max(120.0);
    let (min_lines, max_lines) = STATE
        .with(|t| t.with(h.0 as usize, |st| (st.min_lines, st.max_lines)))
        .unwrap_or((3, 0));
    let (mut w, mut hh) = (0.0, 0.0);
    unsafe { day_texteditor_measure(h.0, avail_w, min_lines, max_lines, &mut w, &mut hh) };
    Size::new(w.max(120.0), hh.max(24.0))
}

fn release(_backend: &mut Qt, h: &QtHandle) {
    STATE.with(|t| {
        if let Some(node) = t.with(h.0 as usize, |st| st.node) {
            TEXT.with(|m| {
                m.borrow_mut().remove(&node);
            });
        }
        t.remove(h.0 as usize);
    });
}

day_pieces::renderer!(day_qt::RENDERERS, Qt,
    kind: KIND, props: EditorProps, patch: EditorPatch,
    make: make, update: update, measure: measure, release: release);
