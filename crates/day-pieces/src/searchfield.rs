// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Core search input with a toolkit-owned native renderer: NSSearchField, UISearchTextField,
//! GtkSearchEntry, a Qt search field, Android search input, XAML AutoSuggestBox, or DOM search input.
//!
//! It is bound **two-way** to a `Signal<String>`, the same pattern as the core picker: a native
//! edit dispatches an `Event::TextChanged` back to Rust which `set`s the signal, and an external
//! signal change patches the control with `SearchPatch::SetText`. A per-build echo guard remembers
//! the last value that arrived from the native control so its own change is not written straight
//! back (which some toolkits would re-emit → a feedback loop). Like the core `text_field` it is a
//! width-growing leaf (`grow_w = true`, natural height): a search field fills its row.
//!
//! ```ignore
//! let query = Signal::new(String::new());
//! search_field(query).placeholder(res::str::search_placeholder())
//! ```

use crate::Decorated;
use crate::{IntoText, TextSource};
use day_core::{BuildCx, Flex, Piece, RNode, with_tree};
use day_reactive::{Binding, bind_seeded};
use day_spec::Event;
use std::cell::RefCell;
use std::rc::Rc;

pub use day_spec::controls::searchfield::*;

/// A native search field bound two-way to `query`. Set a prompt with `.placeholder(_)`.
pub struct SearchField<S: Binding<String>> {
    query: S,
    placeholder: Option<TextSource>,
}

/// `search_field(query)`: a native search input whose text mirrors `query` in both directions;
/// `query` is a `Signal<String>` or any other two-way binding (a day-model `Field`).
pub fn search_field<S: Binding<String>>(query: S) -> SearchField<S> {
    SearchField {
        query,
        placeholder: None,
    }
}

#[crate::builder(SearchFieldBuilder)]
impl<S: Binding<String>> SearchField<S> {
    /// The empty-state prompt shown when the field has no text (a constant, `Signal<String>`, or
    /// closure, evaluated once for the initial value; the placeholder is not reactive after build).
    pub fn placeholder<M>(mut self, t: impl IntoText<M>) -> Self {
        self.placeholder = Some(t.into_text());
        self
    }
}

impl<S: Binding<String>> Piece for SearchField<S> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let SearchField { query, placeholder } = self;
        let initial = query.peek();
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
        // arrived from the native widget so bind_seeded does not patch that same value straight back.
        let guard: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
        let g = guard.clone();
        let q = query.clone();
        bind_seeded(
            initial,
            move || q.read(),
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
                query.write(t.clone());
            }
        });
        node
    }
}

// ---------------------------------------------------------------------------
// Per-toolkit native renderers, one file per backend. Every module registers a `Renderer`
// link-time into its backend's `RENDERERS` slice; the `#[cfg]` gates each to its feature + target,
// and `#[path]` keeps the files grouped next to lib.rs (the day-piece-picker layout).
// ---------------------------------------------------------------------------

// --- Typed builders, forwarded through `Decorated` (docs/api-style.md) ---

/// The search field's conformance cases (docs/testing.md).
#[cfg(feature = "conformance")]
pub mod conformance {
    use crate::*;
    use day_core::conformance::{Case, Drive};
    use day_reactive::Signal;

    use super::{KIND, search_field};

    /// Typing reaches the query, and a cleared query empties the field.
    #[day_macros::test(day_core)]
    fn searchfield_query_two_way() -> Case {
        Case::new()
            .proves(KIND)
            .page(|| {
                let query = Signal::new(String::new());
                column((
                    label(move || format!("query {}", query.get()))
                        .id(day_core::id!(searchfield::query)),
                    search_field(query)
                        .placeholder("Search")
                        .id(day_core::id!(searchfield::search)),
                    button("Clear")
                        .action(move || query.set(String::new()))
                        .id(day_core::id!(searchfield::clear)),
                ))
            })
            .drive(|d: Drive| async move {
                d.input("searchfield::search", "choc").await?;
                d.assert_text("searchfield::query", "query choc").await?;
                d.tap("searchfield::clear").await?;
                d.assert_text("searchfield::query", "query ").await?;
                d.assert_text("searchfield::search", "").await
            })
    }
}

/// Whether the selected toolkit implements this control.
pub fn support() -> day_spec::Support {
    day_core::capability(day_spec::Cap::SearchField)
}
