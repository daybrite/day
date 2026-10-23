// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

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
/// sync from the bound signal). An items swap must keep the typed text, because the text is the
/// value.
#[derive(Clone, Debug, PartialEq)]
pub enum ComboPatch {
    Items(Vec<String>),
    SetText(String),
}
