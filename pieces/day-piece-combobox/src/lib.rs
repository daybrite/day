//! day-piece-combobox — an EXTERNAL Day Piece (DESIGN.md §15 tier 1, Appendix B.1): one Rust
//! API, per-toolkit native renderers registered link-time into each backend's slice, with no
//! edits to Day or its toolkit crates. The Qt and WinUI renderers carry their own C++ shims;
//! the Android renderer its own Java factory.
//!
//! A REAL combo box: free-form text entry PLUS a dropdown of suggestions, as the platform's
//! genuine combo control — `NSComboBox` (AppKit), `GtkComboBoxText` with an entry (GTK), an
//! editable `QComboBox` (Qt), `AutoCompleteTextView` (Android), an editable `ComboBox` (WinUI).
//! Because a typed value need not be in the list, the VALUE is the text: a `Signal<String>`
//! bound two-way, exactly like `search_field` (typing sets the signal; setting the signal
//! patches the control, echo-guarded). Picking a dropdown item is just another way to set the
//! text — every backend reports it through the same `Event::TextChanged`. The suggestion list
//! is reactive: `items` is a `Signal<Vec<String>>` and changes patch the native list live.
//!
//! iOS has no native combo-box control, so this piece deliberately carries **no uikit
//! renderer** — day renders its placeholder leaf there (docs/combobox.md).
//!
//! ```ignore
//! let flavor = Signal::new(String::new());
//! combo_box(flavors, flavor).placeholder("Type or choose…")
//! ```

use day_core::{BuildCx, Flex, Piece, RNode, with_tree};
use day_pieces::{IntoText, TextSource};
use day_reactive::{Signal, bind_seeded};
use day_spec::Event;
use std::cell::RefCell;
use std::rc::Rc;

pub const KIND: &str = "day.piece.combobox";

/// Full props (realize). `items` seeds the dropdown, `text` the entry; `placeholder` is the
/// empty-state prompt (fixed at build). After build, `items` and `text` change via [`ComboPatch`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ComboProps {
    pub items: Vec<String>,
    pub text: String,
    pub placeholder: String,
}

/// Sparse imperative updates: replace the dropdown's items, or the entry's text (programmatic
/// sync from the bound signal). An items swap must keep the typed text — the text is the value.
#[derive(Clone, Debug, PartialEq)]
pub enum ComboPatch {
    Items(Vec<String>),
    SetText(String),
}

/// A native combo box bound two-way to `text`, with a reactive dropdown of `items`.
pub struct ComboBox {
    items: Signal<Vec<String>>,
    text: Signal<String>,
    placeholder: Option<TextSource>,
}

/// `combo_box(items, text)` — free-form text entry plus a native dropdown of suggestions.
/// Typing (or picking an item) writes `text`; setting either signal patches the control.
pub fn combo_box(items: Signal<Vec<String>>, text: Signal<String>) -> ComboBox {
    ComboBox {
        items,
        text,
        placeholder: None,
    }
}

impl ComboBox {
    /// The empty-state prompt shown while the entry has no text (a constant, `Signal<String>`, or
    /// closure — evaluated once for the initial value; not reactive after build).
    pub fn placeholder<M>(mut self, t: impl IntoText<M>) -> Self {
        self.placeholder = Some(t.into_text());
        self
    }
}

impl Piece for ComboBox {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let ComboBox {
            items,
            text,
            placeholder,
        } = self;
        let initial_items = items.get_untracked();
        let initial_text = text.get_untracked();
        let ph = placeholder.map(|p| p.initial()).unwrap_or_default();
        let node = cx.leaf(
            KIND,
            &ComboProps {
                items: initial_items.clone(),
                text: initial_text.clone(),
                placeholder: ph,
            },
            // A text entry fills the available width and keeps its natural (single-line) height.
            Flex {
                grow_w: true,
                ..Default::default()
            },
        );
        bind_seeded(
            initial_items,
            move || items.get(),
            move |v: &Vec<String>| {
                with_tree(|t| t.patch(node, Box::new(ComboPatch::Items(v.clone())), true));
            },
        );
        // Controlled input with origin tracking (§4.4): the echo guard remembers the last value
        // that arrived FROM the native control so bind_seeded does not patch it straight back.
        let guard: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
        let g = guard.clone();
        bind_seeded(
            initial_text,
            move || text.get(),
            move |t: &String| {
                let from_native = g.borrow_mut().take().as_deref() == Some(t.as_str());
                if !from_native {
                    with_tree(|tr| tr.patch(node, Box::new(ComboPatch::SetText(t.clone())), false));
                }
            },
        );
        cx.on(node, move |ev| match ev {
            Event::TextChanged(t) => {
                *guard.borrow_mut() = Some(t.clone());
                text.set(t.clone());
            }
            // Menu-driven selection by index. Native backends report picks as TextChanged (the
            // pick sets the entry text), so this arm serves the SYNTHETIC path — dayscript's
            // `select` step — mapping the index through the current items. No guard poke: the
            // native control did not originate this, so the SetText patch must reach it.
            Event::SelectionChanged(i) => {
                if *i >= 0
                    && let Some(t) = items.get_untracked().get(*i as usize)
                {
                    text.set(t.clone());
                }
            }
            _ => {}
        });
        node
    }
}

// ---------------------------------------------------------------------------
// Per-toolkit native renderers — one file per backend, in the house `#[cfg]`/`#[path]` gates.
// No uikit arm: iOS has no native combo-box control (day renders its placeholder leaf there).
// ---------------------------------------------------------------------------

day_pieces::glue_modules!(appkit, gtk, qt, widget, winui);
