// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// XAML: this crate's OWN C++/WinRT shim (src/lib-xaml-shim.cpp) — a `RichEditBox` driven through
// its Text Object Model document. Positions are UTF-16 code units, so this arm shares the Apple
// conversion; colors cross packed as 0xAARRGGBB, as they do to the Qt shim.
//
// Windows-only, built in CI, NOT verified locally. docs/texteditor.md lists what a check on
// Windows has to confirm.
// ---------------------------------------------------------------------------

use super::*;
use std::collections::HashMap;
use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_void};

use day_spec::sidetable::SideTable;
use day_spec::{ListStyle, NodeId, ParagraphAlign, Proposal, Size, Underline};
use day_xaml::{WinHandle, Xaml};

unsafe extern "C" {
    fn day_texteditor_xaml_new(
        id: u64,
        editable: c_int,
        spellcheck: c_int,
        base_pt: f64,
        placeholder: *const c_char,
        initial: *const c_char,
        text_cb: extern "C" fn(u64, *const c_char),
        sel_cb: extern "C" fn(u64, u64, u64),
    ) -> *mut c_void;
    fn day_texteditor_xaml_set_text(h: *mut c_void, utf8: *const c_char);
    fn day_texteditor_xaml_begin_attrs(h: *mut c_void);
    fn day_texteditor_xaml_apply_run(
        h: *mut c_void,
        start: c_int,
        end: c_int,
        pt: f64,
        bold: c_int,
        italic: c_int,
        mono: c_int,
        underline: c_int,
        strike: c_int,
        has_fg: c_int,
        fg: u32,
        has_bg: c_int,
        bg: u32,
    );
    fn day_texteditor_xaml_apply_paragraph(
        h: *mut c_void,
        start: c_int,
        end: c_int,
        align: c_int,
        indent: f64,
        space_before: f64,
        space_after: f64,
        marker: c_int,
    );
    fn day_texteditor_xaml_end_attrs(h: *mut c_void);
    fn day_texteditor_xaml_set_selection(h: *mut c_void, start: c_int, end: c_int);
    fn day_texteditor_xaml_set_typing(
        h: *mut c_void,
        pt: f64,
        bold: c_int,
        italic: c_int,
        mono: c_int,
        underline: c_int,
        strike: c_int,
        has_fg: c_int,
        fg: u32,
        has_bg: c_int,
        bg: u32,
    );
    fn day_texteditor_xaml_set_editable(h: *mut c_void, on: c_int);
    fn day_texteditor_xaml_release(h: *mut c_void);
    // Generic size hint from day-xaml-sys (already linked).
    fn day_xaml_measure(
        w: *mut c_void,
        avail_w: f64,
        avail_h: f64,
        out_w: *mut f64,
        out_h: *mut f64,
    );
}

const BASE_POINTS: f64 = 15.0;
const LEVEL_INDENT: f64 = 24.0;

struct EdState {
    node: u64,
    min_lines: u32,
    max_lines: u32,
}

day_core::tls_group! {
    static STATE: SideTable<EdState> = SideTable::new();
    /// The text each editor holds, by node — the selection callback has only the node id, and
    /// UTF-16 offsets cannot be turned back into bytes without the string.
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
    let s = if text.is_null() {
        String::new()
    } else {
        // SAFETY: the shim passes a NUL-terminated UTF-8 buffer that outlives the call.
        unsafe { std::ffi::CStr::from_ptr(text) }
            .to_string_lossy()
            .into_owned()
    };
    remember_text(id, &s);
    day_xaml::emit(NodeId(id), Event::TextChanged(s));
}

extern "C" fn on_sel(id: u64, start: u64, end: u64) {
    let text = text_of(id);
    let a = byte_of_utf16(&text, start as usize);
    let b = byte_of_utf16(&text, end as usize);
    day_xaml::emit(
        NodeId(id),
        Event::custom("texteditor:sel", selection_payload(a, b)),
    );
}

fn pack(c: day_spec::Color) -> u32 {
    let q = |v: f64| ((v.clamp(0.0, 1.0) * 255.0).round() as u32) & 0xFF;
    (q(c.a) << 24) | (q(c.r) << 16) | (q(c.g) << 8) | q(c.b)
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

fn apply_attributes(h: *mut c_void, text: &str, runs: &[TextRun], paragraphs: &[ParagraphRun]) {
    unsafe { day_texteditor_xaml_begin_attrs(h) };
    for p in paragraphs {
        let Some((start, len)) = utf16_range(text, &p.range) else {
            continue;
        };
        let s = p.style();
        unsafe {
            day_texteditor_xaml_apply_paragraph(
                h,
                start as c_int,
                (start + len) as c_int,
                match s.align {
                    ParagraphAlign::Natural => 0,
                    ParagraphAlign::Center => 1,
                    ParagraphAlign::Trailing => 2,
                    ParagraphAlign::Justified => 3,
                },
                s.indent + f64::from(s.list_level) * LEVEL_INDENT,
                s.space_before,
                s.space_after,
                c_int::from(s.list != ListStyle::None),
            )
        };
    }
    for r in runs {
        let Some((start, len)) = utf16_range(text, &r.range) else {
            continue;
        };
        let (has_fg, fg) = r.color.map(|c| (1, pack(c))).unwrap_or((0, 0));
        let (has_bg, bg) = r.background.map(|c| (1, pack(c))).unwrap_or((0, 0));
        let bold = r
            .font
            .weight
            .is_some_and(|w| w >= day_spec::FontWeight::Semibold);
        unsafe {
            day_texteditor_xaml_apply_run(
                h,
                start as c_int,
                (start + len) as c_int,
                r.font.resolved_points(BASE_POINTS),
                c_int::from(bold),
                c_int::from(r.font.italic),
                c_int::from(r.font.monospace),
                underline_code(r.underline),
                c_int::from(r.strikethrough),
                has_fg,
                fg,
                has_bg,
                bg,
            )
        };
    }
    unsafe { day_texteditor_xaml_end_attrs(h) };
}

fn make(_backend: &mut Xaml, p: &EditorProps, id: NodeId) -> WinHandle {
    let placeholder = CString::new(p.placeholder.as_str()).unwrap_or_default();
    let initial = CString::new(p.doc.text.as_str()).unwrap_or_default();
    let h = unsafe {
        day_texteditor_xaml_new(
            id.0,
            c_int::from(p.editable),
            c_int::from(p.spellcheck),
            BASE_POINTS,
            placeholder.as_ptr(),
            initial.as_ptr(),
            on_text,
            on_sel,
        )
    };
    remember_text(id.0, &p.doc.text);
    if !p.doc.runs.is_empty() || !p.doc.paragraphs.is_empty() {
        apply_attributes(h, &p.doc.text, &p.doc.runs, &p.doc.paragraphs);
    }
    STATE.with(|t| {
        t.insert(
            h as usize,
            EdState {
                node: id.0,
                min_lines: p.min_lines,
                max_lines: p.max_lines,
            },
        )
    });
    WinHandle(h)
}

fn update(_backend: &mut Xaml, h: &WinHandle, patch: &EditorPatch) {
    let Some(node) = STATE.with(|t| t.with(h.0 as usize, |st| st.node)) else {
        return;
    };
    match patch {
        EditorPatch::SetDocument(doc) => {
            let text = CString::new(doc.text.as_str()).unwrap_or_default();
            unsafe { day_texteditor_xaml_set_text(h.0, text.as_ptr()) };
            remember_text(node, &doc.text);
            apply_attributes(h.0, &doc.text, &doc.runs, &doc.paragraphs);
        }
        EditorPatch::SetAttributes(attrs) => {
            remember_text(node, &attrs.text);
            apply_attributes(h.0, &attrs.text, &attrs.runs, &attrs.paragraphs);
        }
        EditorPatch::SetSelection(r) => {
            let text = text_of(node);
            if let Some((start, len)) = utf16_range(&text, r) {
                unsafe {
                    day_texteditor_xaml_set_selection(h.0, start as c_int, (start + len) as c_int)
                };
            }
        }
        EditorPatch::SetTypingStyle(s) => {
            let (has_fg, fg) = s.color.map(|c| (1, pack(c))).unwrap_or((0, 0));
            let (has_bg, bg) = s.background.map(|c| (1, pack(c))).unwrap_or((0, 0));
            let bold = s
                .font
                .weight
                .is_some_and(|w| w >= day_spec::FontWeight::Semibold);
            unsafe {
                day_texteditor_xaml_set_typing(
                    h.0,
                    s.font.resolved_points(BASE_POINTS),
                    c_int::from(bold),
                    c_int::from(s.font.italic),
                    c_int::from(s.font.monospace),
                    underline_code(s.underline),
                    c_int::from(s.strikethrough),
                    has_fg,
                    fg,
                    has_bg,
                    bg,
                )
            };
        }
        EditorPatch::SetEditable(v) => unsafe {
            day_texteditor_xaml_set_editable(h.0, c_int::from(*v))
        },
    }
}

fn measure(_backend: &mut Xaml, h: &WinHandle, p: Proposal) -> Size {
    let avail_w = p.width.unwrap_or(320.0).max(120.0);
    let (min_lines, max_lines) = STATE
        .with(|t| t.with(h.0 as usize, |st| (st.min_lines, st.max_lines)))
        .unwrap_or((3, 0));
    let (mut w, mut hh) = (0.0, 0.0);
    unsafe { day_xaml_measure(h.0, avail_w, f64::MAX, &mut w, &mut hh) };
    // The control reports its content height; the band is the piece's own promise, and past
    // `max_lines` the RichEditBox scrolls inside it.
    let line_h = BASE_POINTS * 1.4;
    let min_h = f64::from(min_lines) * line_h + 12.0;
    let max_h = if max_lines > 0 {
        f64::from(max_lines) * line_h + 12.0
    } else {
        f64::MAX
    };
    Size::new(avail_w, hh.clamp(min_h, max_h).ceil())
}

fn release(_backend: &mut Xaml, h: &WinHandle) {
    STATE.with(|t| {
        if let Some(node) = t.with(h.0 as usize, |st| st.node) {
            TEXT.with(|m| {
                m.borrow_mut().remove(&node);
            });
        }
        t.remove(h.0 as usize);
    });
    unsafe { day_texteditor_xaml_release(h.0) };
}

day_pieces::renderer!(day_xaml::RENDERERS, Xaml,
    kind: KIND, props: EditorProps, patch: EditorPatch,
    make: make, update: update, measure: measure, release: release);
