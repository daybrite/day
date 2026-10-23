// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

pub const KIND: &str = "day.piece.pullrefresh";

/// Full props (realize) for the native-tier container.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RefreshProps {
    pub refreshing: bool,
}

/// Sparse command sent to the native wrapper after creation.
#[derive(Clone, Debug, PartialEq)]
pub enum RefreshPatch {
    /// Show (`true`) or dismiss (`false`) the native refresh indicator.
    SetRefreshing(bool),
}
