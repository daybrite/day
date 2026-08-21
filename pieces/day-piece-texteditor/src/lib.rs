// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! day-piece-texteditor — a styled-text editor bound two-way to a `Signal<StyledText>`
//! (docs/texteditor.md).
//!
//! ```ignore
//! let doc = Signal::new(StyledText::markdown("**Hello**, _world_.", Font::Body));
//! let sel = Signal::new(0..0);
//!
//! column((
//!     // The toolbar is ordinary Day pieces; the piece ships no chrome of its own.
//!     button("B").action(move || doc.update(|d| {
//!         let on = !d.style_of(sel.get_untracked(), Font::Body).bold();
//!         d.apply(sel.get_untracked(), Font::Body, move |s| s.set_bold(on));
//!     })),
//!     text_editor(doc).selection(sel).min_lines(8),
//! ))
//! ```
//!
//! It edits the same [`StyledText`] a label renders and the Markdown / HTML / RTF codecs read and
//! write, in each platform's own rich-text view: `NSTextView`, `UITextView`, `GtkTextView`,
//! `QTextEdit`, an `EditText` over a `SpannableStringBuilder`, `RichEditBox`, the ArkTS
//! `RichEditor`, and a `contenteditable` element. There is **no composed tier** and there must not
//! be one — a hand-rolled editor loses IME composition, bidirectional cursor movement, the
//! platform's undo stack, dictation, and the accessibility tree, and loses them invisibly.
//!
//! ## Who owns the attributes
//!
//! Day does. The native view owns the *characters* — typing, deletion, IME, undo, autocorrect —
//! and reports them as [`Event::TextChanged`]; the piece diffs that against the text it last knew,
//! reflows its runs over the edit ([`StyledText::reflow`]), and writes the signal. Attributes only
//! ever travel Day → native.
//!
//! That is why every arm turns the platform's own formatting UI OFF (iOS's
//! `allowsEditingTextAttributes`, the macOS font panel) rather than reading it back: an editor
//! whose attributes can change from two directions has to reconcile them, and reconciling an
//! attributed string across eight toolkits is a much larger promise than this piece makes.
//! Everything a toolbar does goes through the bound signal instead, in Rust, where it behaves
//! identically on all nine targets and is testable on the headless one. docs/texteditor.md records
//! the per-toolkit hook a future read-back would use.

use day_core::{BuildCx, Flex, Piece, RNode, with_tree};
use day_pieces::{IntoText, TextSource};
use day_reactive::{Signal, bind_seeded};
use day_spec::{Event, Font, RunStyle, StyledText, Support};
// Re-exported for the per-toolkit arms below, which build native attributes out of them.
#[allow(unused_imports)]
use day_spec::{ParagraphRun, TextRun};
use std::cell::RefCell;
use std::rc::Rc;

pub const KIND: &str = "day.piece.texteditor";

/// The prefix a selection report carries in its payload. The tag cannot cross a JNI / C-ABI / JS
/// boundary (§8.2), so the payload has to be self-describing: `"sel <start> <end>"`, in BYTE
/// offsets, which every arm converts into from its own indexing.
pub const SEL_PREFIX: &str = "sel ";

/// The prefix a TEXT report carries on the same channel, for a backend whose piece bridge can
/// only send `Event::Custom` — HarmonyOS's ArkTS side is the one that has to: `"txt <the text>"`,
/// the whole document, exactly as [`Event::TextChanged`] carries it.
pub const TEXT_PREFIX: &str = "txt ";

/// Full props (realize). Only `doc`, `selection` and `editable` change after build.
#[derive(Clone, Debug, PartialEq)]
pub struct EditorProps {
    pub doc: StyledText,
    /// The style unstyled text is drawn in, and the size relative runs scale against.
    pub base: Font,
    pub editable: bool,
    pub placeholder: String,
    pub min_lines: u32,
    pub max_lines: u32,
    /// Show the platform's spell-check squiggles. Prose wants them; a code editor does not.
    pub spellcheck: bool,
}

impl Default for EditorProps {
    fn default() -> Self {
        EditorProps {
            doc: StyledText::default(),
            base: Font::Body,
            editable: true,
            placeholder: String::new(),
            min_lines: 3,
            max_lines: 0,
            spellcheck: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum EditorPatch {
    /// Replace the text AND its attributes. Moves the caret, so it is sent only when the text
    /// itself changed under the app's hand.
    SetDocument(StyledText),
    /// Restyle text the native view ALREADY holds, preserving the selection and the undo stack.
    ///
    /// The patch a live syntax highlighter sends on every keystroke: re-tokenizing produces fresh
    /// runs for the same characters, and pushing them as a document would move the caret back to
    /// wherever the app last set it.
    ///
    /// It carries the whole document even though only the attributes are new. The text is what an
    /// arm converts byte ranges against, and the four whose toolkit cannot hand back its own
    /// string cheaply (Qt, Android, HarmonyOS, XAML) would otherwise each need a cache that a
    /// keystroke could leave one edit stale.
    SetAttributes(StyledText),
    SetSelection(std::ops::Range<usize>),
    /// What the next typed character takes. Native where the toolkit has the concept, emulated on
    /// GTK and the web, which do not.
    SetTypingStyle(RunStyle),
    SetEditable(bool),
}

/// Whether the compiled backend edits styled text natively.
///
/// [`Support::Native`] on all eight toolkits — every one ships a rich-text view
/// (docs/texteditor.md). [`Support::Emulated`] only on the headless mock and on an external
/// toolkit with no arm, where the piece degrades to plain text: the characters round-trip and the
/// styling does not.
pub fn support() -> Support {
    if cfg!(any(
        all(feature = "appkit", target_os = "macos"),
        all(feature = "uikit", target_os = "ios"),
        all(feature = "mdc", target_os = "android"),
        all(feature = "xaml", windows),
        all(feature = "arkui", target_env = "ohos"),
        all(feature = "dom", target_arch = "wasm32"),
        feature = "gtk",
        feature = "qt",
    )) {
        Support::Native
    } else {
        Support::Emulated
    }
}

/// A styled-text editor bound two-way to a [`StyledText`] signal. Build with [`text_editor`].
pub struct TextEditor {
    doc: Signal<StyledText>,
    selection: Option<Signal<std::ops::Range<usize>>>,
    typing: Option<Signal<RunStyle>>,
    base: Font,
    editable: day_pieces::Reactive<bool>,
    placeholder: Option<TextSource>,
    min_lines: u32,
    max_lines: u32,
    spellcheck: bool,
}

/// `text_editor(doc)` — the platform's rich-text view over a [`StyledText`] signal.
pub fn text_editor(doc: Signal<StyledText>) -> TextEditor {
    // web-dom's registry is populated at RUNTIME (no `linkme` on wasm), and a constructor always
    // runs before the node it returns is realized.
    #[cfg(all(feature = "dom", target_arch = "wasm32"))]
    dom_impl::register();
    TextEditor {
        doc,
        selection: None,
        typing: None,
        base: Font::Body,
        editable: day_pieces::IntoReactive::into_reactive(true),
        placeholder: None,
        min_lines: 3,
        max_lines: 0,
        spellcheck: true,
    }
}

impl TextEditor {
    /// Bind the selection, in BYTE offsets into `doc.text`, two-way: the user moving the caret
    /// writes it, and writing it moves the caret. Collapsed when `start == end`.
    pub fn selection(mut self, sel: Signal<std::ops::Range<usize>>) -> Self {
        self.selection = Some(sel);
        self
    }
    /// Bind what the NEXT typed character will be styled with.
    ///
    /// The one piece of editor state an app cannot derive from the document and the selection:
    /// with a collapsed caret there is no text to read a style off, and "what happens if I type
    /// now" is the platform's own pending state. Writing it is how a toolbar makes the next word
    /// bold rather than the last one.
    pub fn typing_style(mut self, style: Signal<RunStyle>) -> Self {
        self.typing = Some(style);
        self
    }
    /// The style unstyled text draws in, and what relative runs scale against (default `Body`).
    pub fn base(mut self, base: Font) -> Self {
        self.base = base;
        self
    }
    pub fn editable<M>(mut self, v: impl day_pieces::IntoReactive<bool, M>) -> Self {
        self.editable = v.into_reactive();
        self
    }
    /// Show the platform's spell-check squiggles (default on). Turn it off for code.
    pub fn spellcheck(mut self, on: bool) -> Self {
        self.spellcheck = on;
        self
    }
    pub fn placeholder<M>(mut self, t: impl IntoText<M>) -> Self {
        self.placeholder = Some(t.into_text());
        self
    }
    /// The height band, in lines: never shorter than `min_lines`, grows to `max_lines` and then
    /// scrolls (`0` = unbounded).
    pub fn min_lines(mut self, n: u32) -> Self {
        self.min_lines = n;
        self
    }
    pub fn max_lines(mut self, n: u32) -> Self {
        self.max_lines = n;
        self
    }
}

impl Piece for TextEditor {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let TextEditor {
            doc,
            selection,
            typing,
            base,
            editable,
            placeholder,
            min_lines,
            max_lines,
            spellcheck,
        } = self;
        let initial = doc.get_untracked();
        let node = cx.leaf(
            KIND,
            &EditorProps {
                doc: initial.clone(),
                base,
                editable: editable.get_untracked(),
                placeholder: placeholder.map(|p| p.initial()).unwrap_or_default(),
                min_lines,
                max_lines,
                spellcheck,
            },
            Flex {
                grow_w: true,
                ..Default::default()
            },
        );

        // The text the NATIVE view is known to hold. Two jobs: it is what an incoming
        // `TextChanged` is diffed against to recover the edit, and it is what an outgoing patch
        // is compared against to decide whether the text changed at all (which decides between
        // replacing the document and repainting its attributes).
        let native_text = Rc::new(RefCell::new(initial.text.clone()));

        // App writes → the native view.
        {
            let native_text = native_text.clone();
            bind_seeded(
                initial.clone(),
                move || doc.get(),
                move |d: &StyledText| {
                    let text_changed = *native_text.borrow() != d.text;
                    let patch = if text_changed {
                        *native_text.borrow_mut() = d.text.clone();
                        EditorPatch::SetDocument(d.clone())
                    } else {
                        // Same characters, different attributes: the syntax-highlighting path.
                        // Keeps the caret and the platform's undo stack, which replacing the
                        // document would not.
                        EditorPatch::SetAttributes(d.clone())
                    };
                    with_tree(|t| t.patch(node, Box::new(patch), text_changed));
                },
            );
        }

        // The selection the NATIVE view last reported, and the same echo guard `native_text` is:
        // a caret move the user made must not be written back to the view it came from.
        //
        // Writing it back is not merely redundant. A `selectionchange` fires on every mouse-move
        // of a drag, and re-setting the selection mid-drag re-anchors it — on the web
        // (`removeAllRanges` + `addRange`) that visibly collapses the selection the user is in
        // the middle of making, so a drag cannot select anything at all.
        let native_sel: Rc<RefCell<Option<std::ops::Range<usize>>>> = Rc::new(RefCell::new(None));
        if let Some(sel) = selection {
            let native_sel = native_sel.clone();
            bind_seeded(
                0..0,
                move || sel.get(),
                move |r: &std::ops::Range<usize>| {
                    if native_sel.borrow().as_ref() == Some(r) {
                        return; // the echo of the view's own report
                    }
                    with_tree(|t| {
                        t.patch(node, Box::new(EditorPatch::SetSelection(r.clone())), false)
                    });
                },
            );
        }
        // What the next typed character takes, as Day last knew it. `None` when the app bound no
        // typing style, which is what leaves plain inheritance in charge.
        let typing_now: Rc<RefCell<Option<RunStyle>>> = Rc::new(RefCell::new(None));
        if let Some(style) = typing {
            let typing_now = typing_now.clone();
            bind_seeded(
                RunStyle::plain(base),
                move || style.get(),
                move |s: &RunStyle| {
                    // Recorded as well as patched. A native `typingAttributes` styles the
                    // keystroke, but Day's own attribute patch lands a moment later and would
                    // repaint it from the model — so the model has to learn the style too.
                    *typing_now.borrow_mut() = Some(s.clone());
                    with_tree(|t| {
                        t.patch(
                            node,
                            Box::new(EditorPatch::SetTypingStyle(s.clone())),
                            false,
                        )
                    });
                },
            );
        }
        {
            let editable = editable.clone();
            bind_seeded(
                editable.get_untracked(),
                move || editable.get(),
                move |v: &bool| {
                    with_tree(|t| t.patch(node, Box::new(EditorPatch::SetEditable(*v)), false));
                },
            );
        }

        // Native edits → the signal.
        let edited = move |new_text: &str| {
            let old = native_text.borrow().clone();
            if old == new_text {
                // The echo of a patch Day just sent. Nothing changed, so nothing to write — which
                // is what keeps `SetDocument` from looping.
                return;
            }
            let (offset, removed, inserted) = diff_edit(&old, new_text);
            *native_text.borrow_mut() = new_text.to_string();
            let pending = typing_now.borrow().clone();
            doc.update(|d| {
                d.text = new_text.to_string();
                // Inserted text inherits the run it landed in — which is what an editor does when
                // nothing is pending, and what typing at the end of a bold word should do. A
                // pending typing style then overrides exactly the new characters.
                d.reflow(offset, removed, inserted);
                if inserted > 0
                    && let Some(style) = &pending
                {
                    d.apply(offset..offset + inserted, base, |s| *s = style.clone());
                }
            });
        };
        cx.on(node, move |ev| match ev {
            Event::TextChanged(new_text) => edited(new_text),
            Event::Custom { text, .. } => {
                // A backend whose piece channel carries only `Event::Custom` reports its text
                // here instead (HarmonyOS's ArkTS bridge is the one that must).
                if let Some(new_text) = text.strip_prefix(TEXT_PREFIX) {
                    edited(new_text);
                    return;
                }
                let Some(range) = parse_selection(text) else {
                    return;
                };
                // The caret's own style becomes the pending one, so a toolbar bound to it reads
                // the text the caret sits in and typing continues that text's style. An app write
                // lands the same way and simply wins until the caret moves again.
                //
                // `set_if_changed`, because this runs on every mouse-move of a drag: an unchanged
                // style would patch the native typing attributes hundreds of times for nothing.
                if let Some(style) = typing {
                    style.set_if_changed(doc.with_untracked(|d| d.style_of(range.clone(), base)));
                }
                if let Some(sel) = selection {
                    // Recorded BEFORE the write, so the bind above sees it as an echo and sends
                    // no patch back to the view the report came from.
                    *native_sel.borrow_mut() = Some(range.clone());
                    sel.set_if_changed(range);
                }
            }
            _ => {}
        });
        node
    }
}

/// Recover a single contiguous edit from the old and new text: `(offset, removed, inserted)` in
/// bytes.
///
/// The whole reason a keystroke does not need the backend's cooperation. A common prefix and a
/// common suffix bracket exactly what changed, which for one keystroke, one deletion, one paste or
/// one autocorrect IS the edit. An edit that touched two separate places (a multi-cursor change)
/// comes back as one span covering both — coarser, never wrong, and it costs only the styling
/// between them.
///
/// Both ends are pulled back to character boundaries, so a multi-byte character that changed
/// mid-sequence never produces a range that would split one.
pub fn diff_edit(old: &str, new: &str) -> (usize, usize, usize) {
    let (ob, nb) = (old.as_bytes(), new.as_bytes());
    let max_prefix = ob.len().min(nb.len());
    let mut prefix = 0usize;
    while prefix < max_prefix && ob[prefix] == nb[prefix] {
        prefix += 1;
    }
    while prefix > 0 && (!old.is_char_boundary(prefix) || !new.is_char_boundary(prefix)) {
        prefix -= 1;
    }
    let mut suffix = 0usize;
    while suffix < max_prefix - prefix && ob[ob.len() - 1 - suffix] == nb[nb.len() - 1 - suffix] {
        suffix += 1;
    }
    while suffix > 0
        && (!old.is_char_boundary(ob.len() - suffix) || !new.is_char_boundary(nb.len() - suffix))
    {
        suffix -= 1;
    }
    (
        prefix,
        ob.len() - suffix - prefix,
        nb.len() - suffix - prefix,
    )
}

/// Decode a `"sel <start> <end>"` payload into a byte range.
fn parse_selection(payload: &str) -> Option<std::ops::Range<usize>> {
    let rest = payload.strip_prefix(SEL_PREFIX)?;
    let (a, b) = rest.split_once(' ')?;
    let (a, b) = (a.parse::<usize>().ok()?, b.parse::<usize>().ok()?);
    Some(a.min(b)..a.max(b))
}

// ---------------------------------------------------------------------------
// Offset conversion. Every arm speaks its toolkit's index unit and converts here, so the
// arithmetic is written and tested once rather than eight times — an off-by-N here styles the
// wrong words, and only in text with an emoji or a CJK character.
// ---------------------------------------------------------------------------

/// A byte range as a UTF-16 range (Apple, Android, XAML).
pub fn utf16_range(text: &str, r: &std::ops::Range<usize>) -> Option<(usize, usize)> {
    let start = text.get(..r.start)?.encode_utf16().count();
    let len = text.get(r.clone())?.encode_utf16().count();
    Some((start, len))
}

/// A UTF-16 offset back to a byte offset, clamped into the string.
pub fn byte_of_utf16(text: &str, units: usize) -> usize {
    let mut seen = 0usize;
    for (i, c) in text.char_indices() {
        if seen >= units {
            return i;
        }
        seen += c.len_utf16();
    }
    text.len()
}

/// A byte range as a CHARACTER range (GTK's `TextIter`, ArkUI's spans).
pub fn char_range(text: &str, r: &std::ops::Range<usize>) -> Option<(usize, usize)> {
    let start = text.get(..r.start)?.chars().count();
    let len = text.get(r.clone())?.chars().count();
    Some((start, len))
}

/// A character offset back to a byte offset, clamped into the string.
pub fn byte_of_char(text: &str, chars: usize) -> usize {
    text.char_indices()
        .nth(chars)
        .map(|(i, _)| i)
        .unwrap_or(text.len())
}

/// The selection payload every arm emits.
pub fn selection_payload(start: usize, end: usize) -> String {
    format!("{SEL_PREFIX}{start} {end}")
}

/// The text payload for an arm that has only the custom channel — see [`TEXT_PREFIX`].
pub fn text_payload(text: &str) -> String {
    format!("{TEXT_PREFIX}{text}")
}

// ---------------------------------------------------------------------------
// Per-toolkit native renderers — one file per backend.
// ---------------------------------------------------------------------------

day_pieces::glue_modules!(appkit, gtk, qt, uikit, mdc, xaml, arkui, dom);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_recovers_a_keystroke() {
        assert_eq!(diff_edit("hello", "hellXo"), (4, 0, 1));
        assert_eq!(diff_edit("hello", "hello!"), (5, 0, 1));
        assert_eq!(diff_edit("hello", "Xhello"), (0, 0, 1));
    }

    #[test]
    fn diff_recovers_a_deletion_and_a_replacement() {
        assert_eq!(diff_edit("hello", "helo"), (3, 1, 0));
        assert_eq!(diff_edit("hello", "heLLo"), (2, 2, 2));
        assert_eq!(diff_edit("hello", ""), (0, 5, 0));
        assert_eq!(diff_edit("", "hi"), (0, 0, 2));
    }

    #[test]
    fn diff_never_splits_a_character() {
        // "é" is two bytes; replacing it must not produce a range inside it.
        let (o, r, i) = diff_edit("café", "cafe");
        assert!("café".is_char_boundary(o) && "café".is_char_boundary(o + r));
        assert!("cafe".is_char_boundary(o + i));
        // An emoji swapped for another, both four bytes.
        let (o, r, i) = diff_edit("a😀b", "a😃b");
        assert!("a😀b".is_char_boundary(o) && "a😀b".is_char_boundary(o + r));
        assert!("a😃b".is_char_boundary(o + i));
    }

    #[test]
    fn an_unchanged_string_is_an_empty_edit() {
        assert_eq!(diff_edit("hello", "hello"), (5, 0, 0));
    }

    #[test]
    fn selection_payload_round_trips() {
        assert_eq!(parse_selection(&selection_payload(3, 9)), Some(3..9));
        // Backwards drags arrive with end < start; the range comes back ordered.
        assert_eq!(parse_selection(&selection_payload(9, 3)), Some(3..9));
        assert_eq!(parse_selection("something else"), None);
        assert_eq!(parse_selection("sel 3"), None);
    }

    #[test]
    fn offset_conversions_agree_on_astral_text() {
        let s = "a😀é b";
        let r = 1..(1 + '😀'.len_utf8());
        let (u16_start, u16_len) = utf16_range(s, &r).unwrap();
        assert_eq!(
            (u16_start, u16_len),
            (1, 2),
            "a surrogate pair is two units"
        );
        assert_eq!(byte_of_utf16(s, u16_start), 1);
        assert_eq!(byte_of_utf16(s, u16_start + u16_len), r.end);
        let (c_start, c_len) = char_range(s, &r).unwrap();
        assert_eq!((c_start, c_len), (1, 1), "and one character");
        assert_eq!(byte_of_char(s, c_start + c_len), r.end);
    }
}

// --- Typed builders, forwarded through `Decorated` (docs/api-style.md) ---

/// [`TextEditor`]'s own builders, reachable THROUGH a decoration (§5.2): `day_pieces::Decorated` forwards them
/// to the piece it wraps, so generic modifiers and typed ones chain in any order.
pub trait TextEditorBuilder: Sized {
    fn selection(self, sel: Signal<std::ops::Range<usize>>) -> Self;
    fn typing_style(self, style: Signal<RunStyle>) -> Self;
    fn base(self, base: Font) -> Self;
    fn editable<M>(self, v: impl day_pieces::IntoReactive<bool, M>) -> Self;
    fn spellcheck(self, on: bool) -> Self;
    fn placeholder<M>(self, t: impl IntoText<M>) -> Self;
    fn min_lines(self, n: u32) -> Self;
    fn max_lines(self, n: u32) -> Self;
}

impl TextEditorBuilder for TextEditor {
    fn selection(self, sel: Signal<std::ops::Range<usize>>) -> Self {
        TextEditor::selection(self, sel)
    }
    fn typing_style(self, style: Signal<RunStyle>) -> Self {
        TextEditor::typing_style(self, style)
    }
    fn base(self, base: Font) -> Self {
        TextEditor::base(self, base)
    }
    fn editable<M>(self, v: impl day_pieces::IntoReactive<bool, M>) -> Self {
        TextEditor::editable(self, v)
    }
    fn spellcheck(self, on: bool) -> Self {
        TextEditor::spellcheck(self, on)
    }
    fn placeholder<M>(self, t: impl IntoText<M>) -> Self {
        TextEditor::placeholder(self, t)
    }
    fn min_lines(self, n: u32) -> Self {
        TextEditor::min_lines(self, n)
    }
    fn max_lines(self, n: u32) -> Self {
        TextEditor::max_lines(self, n)
    }
}

impl<Inner: TextEditorBuilder + day_pieces::prelude::Piece> TextEditorBuilder
    for day_pieces::Decorated<Inner>
{
    fn selection(self, sel: Signal<std::ops::Range<usize>>) -> Self {
        self.map_inner(|inner_piece| inner_piece.selection(sel))
    }
    fn typing_style(self, style: Signal<RunStyle>) -> Self {
        self.map_inner(|inner_piece| inner_piece.typing_style(style))
    }
    fn base(self, base: Font) -> Self {
        self.map_inner(|inner_piece| inner_piece.base(base))
    }
    fn editable<M>(self, v: impl day_pieces::IntoReactive<bool, M>) -> Self {
        self.map_inner(|inner_piece| inner_piece.editable(v))
    }
    fn spellcheck(self, on: bool) -> Self {
        self.map_inner(|inner_piece| inner_piece.spellcheck(on))
    }
    fn placeholder<M>(self, t: impl IntoText<M>) -> Self {
        self.map_inner(|inner_piece| inner_piece.placeholder(t))
    }
    fn min_lines(self, n: u32) -> Self {
        self.map_inner(|inner_piece| inner_piece.min_lines(n))
    }
    fn max_lines(self, n: u32) -> Self {
        self.map_inner(|inner_piece| inner_piece.max_lines(n))
    }
}
