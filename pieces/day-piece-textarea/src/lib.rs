//! day-piece-textarea — an EXTERNAL Day Piece (DESIGN.md §15): a NATIVE MULTI-LINE text editor for a
//! message composer, realized as a scrolling native editor per toolkit (NSTextView-in-NSScrollView /
//! UITextView / GtkTextView-in-GtkScrolledWindow / a QPlainTextEdit shim / a multiline EditText / a
//! wrapping WinUI TextBox), registered link-time into each backend's renderer slice with **zero edits**
//! to day. It is the multi-line complement to day-core's single-line `text_field`.
//!
//! It is bound **two-way** to a `Signal<String>` — the same pattern as day-piece-searchfield: a native
//! edit dispatches an `Event::TextChanged` back to Rust which `set`s the signal, and an external signal
//! change patches the control with `TextPatch::SetText`. A per-build echo guard remembers the last value
//! that arrived FROM the native control so its own change is not written straight back (which some
//! toolkits would re-emit → a feedback loop).
//!
//! Unlike the single-line field it is a **growing** editor: it fills the available width (`grow_w =
//! true`) and its height grows with the content between `min_lines` and `max_lines`, then the control
//! scrolls internally. `max_lines(0)` (the default) means unbounded — the editor keeps growing.
//!
//! ```ignore
//! let draft = Signal::new(String::new());
//! text_area(draft).placeholder("Message…").min_lines(1).max_lines(6)
//! ```

use day_core::{BuildCx, Flex, Piece, RNode, with_tree};
use day_pieces::{IntoText, TextSource};
use day_reactive::{Signal, bind_seeded};
use day_spec::Event;
use std::cell::RefCell;
use std::rc::Rc;

pub const KIND: &str = "day.piece.textarea";

/// Full props (realize). `text` seeds the editor; `placeholder` is the empty-state prompt; `min_lines`
/// / `max_lines` bound the auto-growing height (in text lines; `max_lines == 0` = unbounded). Only
/// `text` changes after build (via [`TextPatch::SetText`]); the rest are fixed at build.
#[derive(Clone, Debug, PartialEq)]
pub struct TextProps {
    pub text: String,
    pub placeholder: String,
    pub min_lines: u32,
    pub max_lines: u32,
}

impl Default for TextProps {
    fn default() -> Self {
        TextProps {
            text: String::new(),
            placeholder: String::new(),
            min_lines: 1,
            max_lines: 0,
        }
    }
}

/// The single imperative update: replace the editor's text (programmatic sync from the signal).
#[derive(Clone, Debug, PartialEq)]
pub enum TextPatch {
    SetText(String),
}

/// A native multi-line text editor bound two-way to `text`. Configure a prompt with `.placeholder(_)`
/// and the auto-growing height band with `.min_lines(_)` / `.max_lines(_)`.
pub struct TextArea {
    text: Signal<String>,
    placeholder: Option<TextSource>,
    min_lines: u32,
    max_lines: u32,
}

/// `text_area(text)` — a native multi-line editor whose contents mirror `text` in both directions.
pub fn text_area(text: Signal<String>) -> TextArea {
    TextArea {
        text,
        placeholder: None,
        min_lines: 1,
        max_lines: 0,
    }
}

impl TextArea {
    /// The empty-state prompt shown when the editor is empty (a constant, `Signal<String>`, or closure
    /// — evaluated once for the initial value; the placeholder is not reactive after build).
    pub fn placeholder<M>(mut self, t: impl IntoText<M>) -> Self {
        self.placeholder = Some(t.into_text());
        self
    }

    /// The minimum height, in text lines (default 1): the editor never shrinks below this.
    pub fn min_lines(mut self, lines: u32) -> Self {
        self.min_lines = lines.max(1);
        self
    }

    /// The maximum height, in text lines, before the editor scrolls internally. `0` (the default) means
    /// unbounded — the editor keeps growing with its content and never scrolls.
    pub fn max_lines(mut self, lines: u32) -> Self {
        self.max_lines = lines;
        self
    }
}

impl Piece for TextArea {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let TextArea {
            text,
            placeholder,
            min_lines,
            max_lines,
        } = self;
        let initial = text.get_untracked();
        let ph = placeholder.map(|p| p.initial()).unwrap_or_default();
        let node = cx.leaf(
            KIND,
            &TextProps {
                text: initial.clone(),
                placeholder: ph,
                min_lines,
                // A 0 max is normalized to "unbounded"; a non-zero max is floored to min so the band is
                // never inverted (min > max would make the clamp meaningless per backend).
                max_lines: if max_lines == 0 {
                    0
                } else {
                    max_lines.max(min_lines)
                },
            },
            // A composer editor fills the available width; its height is content-driven (the backend's
            // `measure` grows it between min/max lines), so it is NOT a height-growing leaf.
            Flex {
                grow_w: true,
                ..Default::default()
            },
        );
        // Controlled input with origin tracking (§4.4): the echo guard remembers the last value that
        // arrived FROM the native widget so bind_seeded does not patch that same value straight back.
        let guard: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
        let g = guard.clone();
        bind_seeded(
            initial,
            move || text.get(),
            move |t: &String| {
                let from_native = g.borrow_mut().take().as_deref() == Some(t.as_str());
                if !from_native {
                    with_tree(|tr| tr.patch(node, Box::new(TextPatch::SetText(t.clone())), true));
                }
            },
        );
        cx.on(node, move |ev| {
            if let Event::TextChanged(t) = ev {
                *guard.borrow_mut() = Some(t.clone());
                text.set(t.clone());
            }
        });
        node
    }
}

// ---------------------------------------------------------------------------
// Per-toolkit native renderers — one file per backend. Every module registers a `Renderer` link-time
// into its backend's `RENDERERS` slice; the `#[cfg]` gates each to its feature + target, and `#[path]`
// keeps the files grouped next to lib.rs (the day-piece-searchfield layout).
// ---------------------------------------------------------------------------

#[cfg(all(feature = "appkit", target_os = "macos"))]
#[path = "lib-appkit.rs"]
mod appkit_impl;

#[cfg(feature = "gtk")]
#[path = "lib-gtk.rs"]
mod gtk_impl;

#[cfg(feature = "qt")]
#[path = "lib-qt.rs"]
mod qt_impl;

#[cfg(all(feature = "uikit", target_os = "ios"))]
#[path = "lib-uikit.rs"]
mod uikit_impl;

#[cfg(all(feature = "widget", target_os = "android"))]
#[path = "lib-android.rs"]
mod android_impl;

#[cfg(all(feature = "winui", windows))]
#[path = "lib-winui.rs"]
mod winui_impl;
