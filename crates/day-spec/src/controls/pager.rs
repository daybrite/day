// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

pub const KIND: &str = "day.piece.pager";
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct PagerState {
    pub order: Vec<usize>,
    pub selected: usize,
    pub epoch: u64,
}
impl PagerState {
    #[must_use]
    pub fn accepts(&self, epoch: u64, slot: usize) -> bool {
        self.epoch == epoch && self.order.contains(&slot)
    }
}
#[derive(Clone, Debug)]
pub struct PagerProps {
    pub slots: usize,
    /// Report the pager's continuous position while a page is dragged (docs/pager.md
    /// "Progress"): an `Event::Custom` whose text is `progress <epoch> <position>`, the
    /// position in reading-order pages (`1.5` is halfway from the second page to the third),
    /// at most once per display frame.
    pub track_progress: bool,
}
#[derive(Clone, Debug)]
pub struct PagerPatch(pub PagerState);
