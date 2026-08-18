// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// Web (web-dom): a `contenteditable` element — NOT a `<textarea>`, which is plain text by
// definition. Contenteditable is the browser's own rich text editing, and it is what every web
// editor is built on: IME composition, the undo stack, spell-check, drag-and-drop, dictation and
// a full accessibility tree all come with it.
//
// What it does not come with is a document model. Enter inserts a `<div>` in one browser and a
// `<p>` in another; a paste arrives as whatever markup it was copied from. So this arm never reads
// the DOM's shape as meaning: the shim flattens it to text under one set of rules
// (`dayEditorText`), and Day writes it back in ONE canonical form — the same
// `day_spec::styled_to_html` an export produces, so what the editor holds and what "Export HTML"
// writes are the same markup.
//
// `document.execCommand` is not used anywhere here. It is deprecated, differs per browser, and
// inserts `<b>`/`<font>` markup Day would then have to normalize away — pushing serialized HTML is
// both simpler and exactly what the other seven arms do with their attributed strings.
// ---------------------------------------------------------------------------

use super::*;
use day_dom::{Dom, DomHandle, listen};
use day_spec::sidetable::SideTable;
use day_spec::{DocStyle, NodeId, Proposal, Size, styled_to_html};

/// Per-element state: the realize props later patches do not carry, plus the line count the
/// height band is measured from.
struct EdState {
    base: Font,
    min_lines: u32,
    max_lines: u32,
    lines: usize,
}

thread_local! {
    static STATE: SideTable<EdState> = SideTable::new();
}

/// The base point size the web editor draws at, matching day-dom's own body text.
const FONT_SIZE: f64 = 16.0;
const PAD: f64 = 8.0;

/// The document as the markup the element holds.
///
/// One transform on top of the shared serializer: the inter-block newlines come out, because
/// inside a contenteditable they would be text nodes the flattener counts as characters the
/// document does not have.
fn editor_html(doc: &StyledText, base: Font) -> String {
    let style = DocStyle {
        base,
        base_points: FONT_SIZE,
    };
    let html = styled_to_html(doc, style).replace(">\n<", "><");
    let html = html.trim_end().to_string();
    if html.is_empty() {
        // An empty document still needs a block to put the caret in, and an empty block needs a
        // filler <br> to have any height — the browser's own convention, which the shim's
        // flattener knows to skip.
        return "<p><br></p>".into();
    }
    // A document ENDING in a newline has a final empty paragraph, which the serializer drops
    // (there is no text in it to write). Put it back, or the flattened text would come up one
    // character short and the piece would read it as the user deleting the newline.
    if doc.text.ends_with('\n') {
        return html + "<p><br></p>";
    }
    html
}

fn make(backend: &mut Dom, p: &EditorProps, _id: NodeId) -> DomHandle {
    let h = backend.element("div");
    backend.set_attr(&h, "contenteditable", if p.editable { "true" } else { "false" });
    backend.set_attr(&h, "spellcheck", if p.spellcheck { "true" } else { "false" });
    // `role="textbox"` + `aria-multiline` is what makes a contenteditable div an editor to a
    // screen reader; without it the browser announces a group of paragraphs.
    backend.set_attr(&h, "role", "textbox");
    backend.set_attr(&h, "aria-multiline", "true");
    // Marks the element for day.css's editor block rules (paragraph margins, list markers).
    backend.set_attr(&h, "data-day-editor", "-");
    if !p.placeholder.is_empty() {
        // The empty-state prompt is a CSS `::before` on the empty element — the web's own idiom
        // for a contenteditable placeholder, since the element has no `placeholder` attribute.
        backend.set_attr(&h, "data-day-placeholder", &p.placeholder);
    }
    // NO `min-height`: Day sets this element's frame itself, and a CSS minimum would win over
    // that frame and push the element out from under the siblings laid out below it.
    backend.set_attr(
        &h,
        "style",
        &format!(
            "white-space:pre-wrap;overflow-wrap:break-word;overflow-y:auto;\
             font-size:{FONT_SIZE}px;padding:{PAD}px;box-sizing:border-box;outline:none"
        ),
    );
    set_document(backend, &h, &p.doc, p.base);
    backend.listen(&h, listen::EDITABLE);
    STATE.with(|t| {
        t.insert(
            h.0 as usize,
            EdState {
                base: p.base,
                min_lines: p.min_lines,
                max_lines: p.max_lines,
                lines: line_count(&p.doc.text),
            },
        )
    });
    h
}

/// The document's line count, as the height band's input. A wrapped line counts once — the
/// browser is the only thing that knows where it wrapped, and asking it per layout pass would
/// cost a synchronous reflow on every keystroke.
fn line_count(text: &str) -> usize {
    text.lines().count().max(1)
}

/// Write the markup, and mark the empty state the placeholder rule keys off.
fn set_document(backend: &mut Dom, h: &DomHandle, doc: &StyledText, base: Font) {
    backend.set_html(h, &editor_html(doc, base));
    backend.set_attr(h, "data-day-empty", if doc.is_empty() { "-" } else { "" });
}

fn update(backend: &mut Dom, h: &DomHandle, patch: &EditorPatch) {
    let base = STATE
        .with(|t| t.with(h.0 as usize, |st| st.base))
        .unwrap_or(Font::Body);
    if let EditorPatch::SetDocument(doc) = patch {
        let lines = line_count(&doc.text);
        STATE.with(|t| t.with(h.0 as usize, |st| st.lines = lines));
    }
    match patch {
        // Both patches write the same markup: on the web there is no separate "attributes only"
        // call, and the shim's caret preservation is what makes rewriting cheap enough to do on
        // every keystroke.
        EditorPatch::SetDocument(doc) => set_document(backend, h, doc, base),
        EditorPatch::SetAttributes(doc) => set_document(backend, h, doc, base),
        EditorPatch::SetSelection(r) => backend.set_editor_selection(h, r.start, r.end),
        // The browser has no typing-attributes concept either (`execCommand` is the only thing
        // that ever did). The piece styles the inserted characters in its model instead, and the
        // markup that follows carries them — see the GTK arm, which resolves this the same way.
        EditorPatch::SetTypingStyle(_) => {}
        EditorPatch::SetEditable(v) => {
            backend.set_attr(h, "contenteditable", if *v { "true" } else { "false" })
        }
    }
}

/// A growing leaf: take the proposed width, and a height from the content, clamped to the line
/// band. Past `max_lines` the element scrolls, which is what the band promises.
fn measure(_backend: &mut Dom, h: &DomHandle, p: Proposal) -> Size {
    let avail_w = p.width.unwrap_or(320.0).max(120.0);
    let line_h = FONT_SIZE * 1.4;
    STATE
        .with(|t| {
            t.with(h.0 as usize, |st| {
                let pad = 2.0 * PAD;
                let min_h = f64::from(st.min_lines.max(1)) * line_h + pad;
                let max_h = if st.max_lines > 0 {
                    f64::from(st.max_lines) * line_h + pad
                } else {
                    f64::MAX
                };
                let want = st.lines as f64 * line_h + pad;
                Size::new(avail_w, want.clamp(min_h, max_h).ceil())
            })
        })
        .unwrap_or_else(|| Size::new(avail_w, 3.0 * line_h + 2.0 * PAD))
}

fn release(_backend: &mut Dom, h: &DomHandle) {
    STATE.with(|t| {
        t.remove(h.0 as usize);
    });
}

// Defines `register()`, which `text_editor()` calls — web-dom's registry is populated at runtime,
// unlike the link-time `renderer!` the other seven arms use.
day_pieces::dom_renderer!(day_dom::register_renderer, Dom,
    kind: KIND, props: EditorProps, patch: EditorPatch,
    make: make, update: update, measure: measure, release: release);
