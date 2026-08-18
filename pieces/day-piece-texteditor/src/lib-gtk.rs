// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// GTK: a `GtkTextView` over a `GtkTextBuffer`, in a scrolled window with the same placeholder
// overlay the built-in text area uses.
//
// GTK styles text with TAGS, not attributes: a tag is an object in the buffer's tag table, applied
// over an iter range. Two consequences shape this arm:
//
// - **Tags are interned.** A syntax highlighter re-applies attributes on every keystroke, and a
//   fresh `GtkTextTag` per run per keystroke would grow the tag table without bound. Each distinct
//   `RunStyle` becomes one tag, keyed by a canonical string, and is reused for the buffer's life.
// - **Offsets are CHARACTERS.** `TextIter` counts characters, not bytes and not UTF-16 units, so
//   every range crosses through `char_range` / `byte_of_char`.
//
// GTK has no typing-attributes concept, and this arm needs none: the piece applies a pending
// typing style to the inserted characters in its own model and patches the result straight back,
// so a keystroke is styled by the same code on every backend.
// ---------------------------------------------------------------------------

use super::*;
use std::cell::Cell;
use std::collections::HashMap;

use day_gtk::Gtk;
use day_spec::sidetable::SideTable;
use day_spec::{ListStyle, NodeId, ParagraphAlign, Proposal, Size, Underline, ffi_guard};
use gtk4::glib::translate::IntoGlib;
use gtk4::prelude::*;

const MARGIN_V: i32 = 4;
const MARGIN_H: i32 = 6;
const PAD: f64 = (2 * MARGIN_V) as f64;
/// The pixels one list level and one list marker indent by — the AppKit arm's figures, in px.
const LEVEL_INDENT: f64 = 24.0;
const MARKER_INDENT: f64 = 18.0;

struct EdState {
    textview: gtk4::TextView,
    buffer: gtk4::TextBuffer,
    placeholder: gtk4::Label,
    /// Guards the "changed" signal while Day itself writes the buffer: `set_text` fires it exactly
    /// as a keystroke would, and reporting it back would loop.
    suppress: Rc<Cell<bool>>,
    /// Interned tags, keyed by [`style_key`] / [`paragraph_key`].
    tags: Rc<RefCell<HashMap<String, gtk4::TextTag>>>,
    /// The point size a run's relative scale multiplies — the base font resolved once.
    base_points: f64,
    line_h: f64,
    min_lines: u32,
    max_lines: u32,
}

thread_local! {
    static STATE: SideTable<EdState> = SideTable::new();
}

fn key(w: &gtk4::Widget) -> usize {
    w.as_ptr() as usize
}

fn buffer_text(b: &gtk4::TextBuffer) -> String {
    b.text(&b.start_iter(), &b.end_iter(), false).to_string()
}

fn rgba(c: day_spec::Color) -> gtk4::gdk::RGBA {
    gtk4::gdk::RGBA::new(c.r as f32, c.g as f32, c.b as f32, c.a as f32)
}

/// Pango has no dotted underline and no plain wavy one: `Error` is the wavy line (the spell-check
/// squiggle), and dotted falls back to a single rule. Both are stated in docs/texteditor.md as the
/// GTK approximations.
fn pango_underline(u: Underline) -> gtk4::pango::Underline {
    use gtk4::pango::Underline as U;
    match u {
        Underline::None => U::None,
        Underline::Single | Underline::Dotted => U::Single,
        Underline::Double => U::Double,
        Underline::Wavy => U::Error,
    }
}

/// A canonical key for a run style, so identical styles share one tag.
fn style_key(s: &RunStyle, base_points: f64) -> String {
    let c = |c: Option<day_spec::Color>| c.map(|c| c.to_hex_string()).unwrap_or_default();
    format!(
        "r|{:.2}|{}|{}|{}|{}|{}|{}|{}",
        s.font.resolved_points(base_points),
        s.font.weight.map(|w| w as u8).unwrap_or(255),
        s.font.italic as u8,
        s.font.monospace as u8,
        s.underline as u8,
        s.strikethrough as u8,
        c(s.color),
        c(s.background),
    )
}

fn paragraph_key(p: &day_spec::ParagraphStyle) -> String {
    // Only whether there IS a marker matters to the layout — the bullet or number itself is the
    // app's text, not a tag property.
    format!(
        "p|{}|{}|{:.1}|{:.1}|{:.1}|{}",
        p.align as u8, p.list_level, p.indent, p.space_before, p.space_after, p.list != ListStyle::None,
    )
}

/// Get (or create and intern) the tag for a run style.
fn run_tag(st: &EdState, s: &RunStyle) -> gtk4::TextTag {
    let k = style_key(s, st.base_points);
    if let Some(t) = st.tags.borrow().get(&k) {
        return t.clone();
    }
    let tag = gtk4::TextTag::new(None);
    tag.set_size_points(s.font.resolved_points(st.base_points));
    let (_, inherent) = day_gtk::gtk_style(s.font.style);
    let weight = s.font.weight.unwrap_or(inherent);
    tag.set_weight(day_gtk::pango_weight(weight).into_glib());
    if s.font.italic {
        tag.set_style(gtk4::pango::Style::Italic);
    }
    if s.font.monospace {
        tag.set_family(Some("monospace"));
    }
    if s.underline.is_on() {
        tag.set_underline(pango_underline(s.underline));
    }
    tag.set_strikethrough(s.strikethrough);
    if let Some(c) = s.color {
        tag.set_foreground_rgba(Some(&rgba(c)));
    }
    if let Some(c) = s.background {
        tag.set_background_rgba(Some(&rgba(c)));
    }
    st.buffer.tag_table().add(&tag);
    st.tags.borrow_mut().insert(k, tag.clone());
    tag
}

/// Get (or create and intern) the tag for a paragraph style. GTK carries paragraph attributes on
/// the same tags text attributes ride on, applied over the paragraph's range.
fn para_tag(st: &EdState, p: &day_spec::ParagraphStyle) -> gtk4::TextTag {
    let k = paragraph_key(p);
    if let Some(t) = st.tags.borrow().get(&k) {
        return t.clone();
    }
    let tag = gtk4::TextTag::new(None);
    tag.set_justification(match p.align {
        ParagraphAlign::Natural => gtk4::Justification::Left,
        ParagraphAlign::Center => gtk4::Justification::Center,
        ParagraphAlign::Trailing => gtk4::Justification::Right,
        ParagraphAlign::Justified => gtk4::Justification::Fill,
    });
    let indent = p.indent + f64::from(p.list_level) * LEVEL_INDENT;
    let marker = if p.list == ListStyle::None {
        0.0
    } else {
        MARKER_INDENT
    };
    // GTK's `indent` is the FIRST line's offset relative to the left margin, so the marker's
    // hanging indent is a negative first-line offset against a wider margin — the inverse of the
    // way Apple spells the same layout.
    tag.set_left_margin(MARGIN_H + (indent + marker) as i32);
    tag.set_indent(-(marker as i32));
    tag.set_pixels_above_lines(p.space_before as i32);
    tag.set_pixels_below_lines(p.space_after as i32);
    st.buffer.tag_table().add(&tag);
    st.tags.borrow_mut().insert(k, tag.clone());
    tag
}

/// Re-tag the whole buffer from `runs` / `paragraphs`, over text the buffer already holds.
fn apply_attributes(st: &EdState, text: &str, runs: &[TextRun], paragraphs: &[ParagraphRun]) {
    let b = &st.buffer;
    let (start, end) = (b.start_iter(), b.end_iter());
    b.remove_all_tags(&start, &end);
    for p in paragraphs {
        let Some((cs, cl)) = char_range(text, &p.range) else {
            continue;
        };
        let tag = para_tag(st, &p.style());
        b.apply_tag(
            &tag,
            &b.iter_at_offset(cs as i32),
            &b.iter_at_offset((cs + cl) as i32),
        );
    }
    for r in runs {
        let Some((cs, cl)) = char_range(text, &r.range) else {
            continue;
        };
        let tag = run_tag(st, &r.style());
        b.apply_tag(
            &tag,
            &b.iter_at_offset(cs as i32),
            &b.iter_at_offset((cs + cl) as i32),
        );
    }
}

/// Replace the buffer's text and re-tag it, keeping the caret where the user left it.
fn set_document(st: &EdState, doc: &StyledText) {
    let caret = st.buffer.cursor_position();
    st.suppress.set(true);
    st.buffer.set_text(&doc.text);
    apply_attributes(st, &doc.text, &doc.runs, &doc.paragraphs);
    st.suppress.set(false);
    let at = caret.min(st.buffer.char_count());
    st.buffer.place_cursor(&st.buffer.iter_at_offset(at));
    st.placeholder.set_visible(doc.text.is_empty());
}

fn make(_backend: &mut Gtk, p: &EditorProps, id: NodeId) -> gtk4::Widget {
    let textview = gtk4::TextView::new();
    textview.set_wrap_mode(gtk4::WrapMode::WordChar);
    textview.set_editable(p.editable);
    textview.set_top_margin(MARGIN_V);
    textview.set_bottom_margin(MARGIN_V);
    textview.set_left_margin(MARGIN_H);
    textview.set_right_margin(MARGIN_H);
    // GtkTextView ships no spell-checker at all (that is gspell, a separate library), so the prop
    // has nothing to drive here — `Cap::TextSpellCheck` already answers `Unsupported` on GTK.
    let buffer = textview.buffer();

    let ctx = textview.pango_context();
    let layout = gtk4::pango::Layout::new(&ctx);
    layout.set_text("Ag");
    let line_h = layout.pixel_size().1 as f64;

    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    scroll.set_hexpand(true);
    scroll.set_vexpand(true);
    scroll.set_child(Some(&textview));

    let overlay = gtk4::Overlay::new();
    overlay.set_child(Some(&scroll));

    let placeholder = gtk4::Label::new(Some(&p.placeholder));
    placeholder.add_css_class("dim-label");
    placeholder.set_halign(gtk4::Align::Start);
    placeholder.set_valign(gtk4::Align::Start);
    placeholder.set_margin_start(MARGIN_H);
    placeholder.set_margin_top(MARGIN_V);
    placeholder.set_can_target(false);
    placeholder.set_visible(!p.placeholder.is_empty() && p.doc.is_empty());
    overlay.add_overlay(&placeholder);

    let suppress = Rc::new(Cell::new(false));
    {
        let sup = suppress.clone();
        let ph = placeholder.clone();
        buffer.connect_changed(move |b| {
            ffi_guard::contain((), || {
                let text = buffer_text(b);
                ph.set_visible(text.is_empty());
                if sup.get() {
                    return;
                }
                day_gtk::emit(id, Event::TextChanged(text));
            });
        });
    }
    {
        // "mark-set" fires for both ends of a selection and for the caret; anything else (a named
        // mark an app placed) is not a selection change and is ignored.
        let sup = suppress.clone();
        buffer.connect_mark_set(move |b, _iter, mark| {
            ffi_guard::contain((), || {
                let name = mark.name().unwrap_or_default();
                if sup.get() || (name != "insert" && name != "selection_bound") {
                    return;
                }
                let text = buffer_text(b);
                let (s, e) = match b.selection_bounds() {
                    Some((a, z)) => (a.offset(), z.offset()),
                    None => {
                        let c = b.cursor_position();
                        (c, c)
                    }
                };
                let start = byte_of_char(&text, s.max(0) as usize);
                let end = byte_of_char(&text, e.max(0) as usize);
                day_gtk::emit(id, Event::custom("texteditor:sel", selection_payload(start, end)));
            });
        });
    }

    let w: gtk4::Widget = overlay.upcast();
    let st = EdState {
        textview,
        buffer,
        placeholder,
        suppress,
        tags: Rc::new(RefCell::new(HashMap::new())),
        base_points: day_gtk::gtk_style(p.base).0,
        line_h,
        min_lines: p.min_lines,
        max_lines: p.max_lines,
    };
    if !p.doc.is_empty() {
        set_document(&st, &p.doc);
    }
    STATE.with(|m| m.insert(key(&w), st));
    w
}

fn update(_backend: &mut Gtk, h: &gtk4::Widget, patch: &EditorPatch) {
    STATE.with(|m| {
        m.with(key(h), |st| match patch {
            EditorPatch::SetDocument(doc) => set_document(st, doc),
            EditorPatch::SetAttributes(attrs) => {
                // Same characters: re-tag over the text the BUFFER holds, never a stale copy.
                let text = buffer_text(&st.buffer);
                st.suppress.set(true);
                apply_attributes(st, &text, &attrs.runs, &attrs.paragraphs);
                st.suppress.set(false);
            }
            EditorPatch::SetSelection(r) => {
                let text = buffer_text(&st.buffer);
                let Some((cs, cl)) = char_range(&text, r) else {
                    return;
                };
                let (a, z) = (
                    st.buffer.iter_at_offset(cs as i32),
                    st.buffer.iter_at_offset((cs + cl) as i32),
                );
                // Suppressed: this IS the app's own write, and echoing it back would fight the
                // signal that produced it.
                st.suppress.set(true);
                st.buffer.select_range(&a, &z);
                st.suppress.set(false);
            }
            // GTK has no typing attributes (see the header) — the piece styles the inserted
            // characters in its model instead, and the next `SetAttributes` paints them.
            EditorPatch::SetTypingStyle(_) => {}
            EditorPatch::SetEditable(v) => st.textview.set_editable(*v),
        });
    });
}

fn measure(_backend: &mut Gtk, h: &gtk4::Widget, p: Proposal) -> Size {
    STATE
        .with(|m| {
            m.with(key(h), |st| {
                let (_, nat_w, _, _) = st.textview.measure(gtk4::Orientation::Horizontal, -1);
                let avail_w = p.width.unwrap_or(nat_w as f64).max(120.0);
                let (_, nat_h, _, _) = st
                    .textview
                    .measure(gtk4::Orientation::Vertical, avail_w as i32);
                let min_h = (st.min_lines as f64) * st.line_h + PAD;
                let max_h = if st.max_lines > 0 {
                    (st.max_lines as f64) * st.line_h + PAD
                } else {
                    f64::MAX
                };
                Size::new(avail_w, (nat_h as f64).clamp(min_h, max_h))
            })
        })
        .unwrap_or_else(|| {
            let (_, nat_w, _, _) = h.measure(gtk4::Orientation::Horizontal, -1);
            Size::new(p.width.unwrap_or(nat_w as f64).max(120.0), 88.0)
        })
}

fn release(_backend: &mut Gtk, h: &gtk4::Widget) {
    STATE.with(|m| {
        m.remove(key(h));
    });
}

day_pieces::renderer!(day_gtk::RENDERERS, Gtk,
    kind: KIND, props: EditorProps, patch: EditorPatch,
    make: make, update: update, measure: measure, release: release);
