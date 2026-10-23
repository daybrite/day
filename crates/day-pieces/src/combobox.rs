// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Core editable combo box. Protocol types live in `day-spec`; each supported toolkit owns
//! its native renderer and any required C++ or Java shim.
//!
//! A combo box: free-form text entry plus a dropdown of suggestions, as the platform's
//! native combo control: `NSComboBox` (AppKit), `GtkComboBoxText` with an entry (GTK), an
//! editable `QComboBox` (Qt), `AutoCompleteTextView` (Android), an editable `ComboBox` (XAML).
//! Because a typed value need not be in the list, the value is the text: a `Signal<String>`
//! bound two-way, exactly like `search_field` (typing sets the signal; setting the signal
//! patches the control, echo-guarded). Picking a dropdown item is just another way to set the
//! text; every backend reports it through the same `Event::TextChanged`. The suggestion list
//! is reactive: `items` is a `Signal<Vec<String>>` and changes patch the native list live.
//!
//! iOS has no native combo-box control, so this piece carries **no uikit renderer**; day
//! renders its placeholder leaf there (docs/combobox.md).
//!
//! ```ignore
//! let flavor = Signal::new(String::new());
//! combo_box(flavors, flavor).placeholder(res::str::combo_placeholder())
//! ```

use crate::Decorated;
use crate::{IntoText, TextSource};
use day_core::{BuildCx, Flex, Piece, RNode, with_tree};
use day_reactive::{Signal, bind_seeded};
use day_spec::Event;
use std::cell::RefCell;
use std::rc::Rc;

pub use day_spec::controls::combobox::*;

/// A native combo box bound two-way to `text`, with a reactive dropdown of `items`.
pub struct ComboBox {
    items: Signal<Vec<String>>,
    text: Signal<String>,
    placeholder: Option<TextSource>,
}

/// `combo_box(items, text)`: free-form text entry plus a native dropdown of suggestions.
/// Typing (or picking an item) writes `text`; setting either signal patches the control.
pub fn combo_box(items: Signal<Vec<String>>, text: Signal<String>) -> ComboBox {
    ComboBox {
        items,
        text,
        placeholder: None,
    }
}

#[crate::builder(ComboBoxBuilder)]
impl ComboBox {
    /// The empty-state prompt shown while the entry has no text (a constant, `Signal<String>`, or
    /// closure, evaluated once for the initial value; not reactive after build).
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
        // that arrived from the native control so bind_seeded does not patch it straight back.
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
            // pick sets the entry text), so this arm serves the synthetic path (dayscript's
            // `select` step), mapping the index through the current items. No guard poke: the
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
// Per-toolkit native renderers, one file per backend, in the house `#[cfg]`/`#[path]` gates.
// No uikit arm: iOS has no native combo-box control (day renders its placeholder leaf there).
// ---------------------------------------------------------------------------

// --- Typed builders, forwarded through `Decorated` (docs/api-style.md) ---

/// The combo box's conformance cases (docs/testing.md).
#[cfg(feature = "conformance")]
pub mod conformance {
    use crate::*;
    use day_core::conformance::{Case, Drive};
    use day_reactive::Signal;

    use super::{KIND, combo_box};

    fn page() -> impl day_core::Piece {
        let items = Signal::new(vec![
            "Vanilla".to_string(),
            "Chocolate".into(),
            "Pistachio".into(),
        ]);
        let text = Signal::new(String::new());
        column((
            label(move || format!("chose {}", text.get())).id(day_core::id!(combobox::chose)),
            combo_box(items, text)
                .placeholder("Flavor")
                .id(day_core::id!(combobox::combo)),
        ))
    }

    /// Choosing from the menu writes the item's text.
    #[day_macros::test(day_core)]
    fn combobox_menu_selects() -> Case {
        Case::new()
            .proves(KIND)
            .page(page)
            .drive(|d: Drive| async move {
                d.select("combobox::combo", 2).await?;
                d.assert_text("combobox::chose", "chose Pistachio").await
            })
    }

    /// Typing writes free-form text that is in no menu item.
    #[day_macros::test(day_core)]
    fn combobox_free_text() -> Case {
        Case::new()
            .proves(KIND)
            .page(page)
            .drive(|d: Drive| async move {
                d.input("combobox::combo", "Stracciatella").await?;
                d.assert_text("combobox::chose", "chose Stracciatella")
                    .await
            })
    }
}

/// Whether the selected toolkit implements this control.
pub fn support() -> day_spec::Support {
    day_core::capability(day_spec::Cap::ComboBox)
}
