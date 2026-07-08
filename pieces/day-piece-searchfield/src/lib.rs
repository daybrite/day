//! day-piece-searchfield — an EXTERNAL Day Piece (DESIGN.md §15): a NATIVE search input realized as a
//! distinct search control per toolkit (NSSearchField / UISearchTextField / GtkSearchEntry / a
//! QLineEdit search shim / an EditText styled for search / a WinUI AutoSuggestBox), registered
//! link-time into each backend's renderer slice without touching day.
//!
//! It is bound **two-way** to a `Signal<String>` — the same pattern as day-piece-picker: a native
//! edit dispatches an `Event::TextChanged` back to Rust which `set`s the signal, and an external
//! signal change patches the control with `SearchPatch::SetText`. A per-build echo guard remembers
//! the last value that arrived FROM the native control so its own change is not written straight
//! back (which some toolkits would re-emit → a feedback loop). Like day-core's `text_field` it is a
//! width-growing leaf (`grow_w = true`, natural height): a search field fills its row.
//!
//! ```ignore
//! let query = Signal::new(String::new());
//! search_field(query).placeholder("Search fruit…")
//! ```

use day_core::{BuildCx, Flex, Piece, RNode, with_tree};
use day_pieces::{IntoText, TextSource};
use day_reactive::{Signal, bind_seeded};
use day_spec::Event;
use std::cell::RefCell;
use std::rc::Rc;

pub const KIND: &str = "day.piece.searchfield";

/// Full props (realize). `text` seeds the control; `placeholder` is the empty-state prompt. Only
/// `text` changes after build (via [`SearchPatch::SetText`]); the placeholder is fixed at build.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SearchProps {
    pub text: String,
    pub placeholder: String,
}

/// The single imperative update: replace the control's text (programmatic sync from the signal).
#[derive(Clone, Debug, PartialEq)]
pub enum SearchPatch {
    SetText(String),
}

/// A native search field bound two-way to `query`. Set a prompt with `.placeholder(_)`.
pub struct SearchField {
    query: Signal<String>,
    placeholder: Option<TextSource>,
}

/// `search_field(query)` — a native search input whose text mirrors `query` in both directions.
pub fn search_field(query: Signal<String>) -> SearchField {
    SearchField {
        query,
        placeholder: None,
    }
}

impl SearchField {
    /// The empty-state prompt shown when the field has no text (a constant, `Signal<String>`, or
    /// closure — evaluated once for the initial value; the placeholder is not reactive after build).
    pub fn placeholder<M>(mut self, t: impl IntoText<M>) -> Self {
        self.placeholder = Some(t.into_text());
        self
    }
}

impl Piece for SearchField {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let SearchField { query, placeholder } = self;
        let initial = query.get_untracked();
        let ph = placeholder.map(|p| p.initial()).unwrap_or_default();
        let node = cx.leaf(
            KIND,
            &SearchProps {
                text: initial.clone(),
                placeholder: ph,
            },
            // A search field fills the available width and keeps its natural (single-line) height.
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
            move || query.get(),
            move |t: &String| {
                let from_native = g.borrow_mut().take().as_deref() == Some(t.as_str());
                if !from_native {
                    with_tree(|tr| {
                        tr.patch(node, Box::new(SearchPatch::SetText(t.clone())), false)
                    });
                }
            },
        );
        cx.on(node, move |ev| {
            if let Event::TextChanged(t) = ev {
                *guard.borrow_mut() = Some(t.clone());
                query.set(t.clone());
            }
        });
        node
    }
}

// ---------------------------------------------------------------------------
// Per-toolkit native renderers — one file per backend. Every module registers a `Renderer`
// link-time into its backend's `RENDERERS` slice; the `#[cfg]` gates each to its feature + target,
// and `#[path]` keeps the files grouped next to lib.rs (the day-piece-picker layout).
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
