// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

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
