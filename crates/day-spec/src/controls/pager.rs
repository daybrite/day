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
    pub fn accepts(&self, epoch: u64, slot: usize) -> bool {
        self.epoch == epoch && self.order.contains(&slot)
    }
}
#[derive(Clone, Debug)]
pub struct PagerProps {
    pub slots: usize,
}
#[derive(Clone, Debug)]
pub struct PagerPatch(pub PagerState);
