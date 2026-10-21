// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Native interactive horizontal paging. Slots have stable native identities; `order`
//! chooses which slots participate and their reading order. Changing order never rebuilds
//! a page. Apps must defer changing slot contents while `busy` is true. Native callbacks
//! carry the presentation epoch, so delayed completions cannot select a recycled slot.
use day_core::{Boundary, BuildCx, Flex, Layout, LayoutOps, Piece, RNode};
use day_reactive::Signal;
use day_spec::{Event, Proposal, Rect, Size};
use std::rc::Rc;

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

pub struct Pager {
    state: Signal<PagerState>,
    busy: Signal<bool>,
    pages: Vec<day_core::AnyPiece>,
    settled: Rc<dyn Fn(usize)>,
}
pub fn pager(
    state: Signal<PagerState>,
    busy: Signal<bool>,
    pages: Vec<day_core::AnyPiece>,
    settled: impl Fn(usize) + 'static,
) -> Pager {
    Pager {
        state,
        busy,
        pages,
        settled: Rc::new(settled),
    }
}
struct PageLayout;
impl Layout for PageLayout {
    fn measure(&self, _cx: &mut dyn LayoutOps, _children: &[RNode], p: Proposal) -> Size {
        Size::new(p.width.unwrap_or(0.0), p.height.unwrap_or(0.0))
    }
    fn place(&self, cx: &mut dyn LayoutOps, children: &[RNode], bounds: Rect) {
        for &child in children {
            cx.place_child_native(
                child,
                Rect::new(0.0, 0.0, bounds.size.width, bounds.size.height),
            );
        }
    }
}
impl Piece for Pager {
    fn build(self, cx: &mut BuildCx) -> RNode {
        use day_pieces::prelude::*;
        let Self {
            state,
            busy,
            pages,
            settled,
        } = self;
        let native = cfg!(any(
            all(feature = "uikit", target_os = "ios"),
            all(feature = "mdc", target_os = "android"),
            all(feature = "arkui", target_env = "ohos")
        ));
        let node = if native {
            cx.native(
                KIND,
                &PagerProps { slots: pages.len() },
                Rc::new(PageLayout),
                Flex {
                    grow_w: true,
                    grow_h: true,
                    ..Default::default()
                },
                Boundary::Yes,
            )
        } else {
            cx.native(
                day_spec::kinds::CONTAINER,
                &day_spec::props::ContainerProps::default(),
                Rc::new(PageLayout),
                Flex {
                    grow_w: true,
                    grow_h: true,
                    ..Default::default()
                },
                Boundary::Yes,
            )
        };
        cx.under(node, |cx| {
            for (slot, page) in pages.into_iter().enumerate() {
                // A stable native wrapper per slot, even when its content is a conditional.
                let page = column((page,)).grow();
                if native {
                    page.build(cx);
                } else {
                    page.opacity(move || {
                        if state.get().selected == slot {
                            1.0
                        } else {
                            0.0
                        }
                    })
                    .build(cx);
                }
            }
        });
        cx.on(node, move |event| {
            // Standard accessibility/automation selection uses logical positions. Native
            // gestures report only settled slot identities on the epoch-tagged channel.
            if let Event::SelectionChanged(index) = event {
                if !busy.get_untracked()
                    && *index >= 0
                    && let Some(slot) = state.get_untracked().order.get(*index as usize).copied()
                {
                    settled(slot);
                }
                return;
            }
            if let Event::Custom { text, .. } = event {
                let fields: Vec<_> = text.split_whitespace().collect();
                if fields.len() < 2 {
                    return;
                }
                let Ok(epoch) = fields[1].parse::<u64>() else {
                    return;
                };
                if epoch != state.get_untracked().epoch {
                    return;
                }
                match fields[0] {
                    "begin" => busy.set(true),
                    "settle" => {
                        if let Some(slot) = fields.get(2).and_then(|s| s.parse::<usize>().ok())
                            && state.get_untracked().accepts(epoch, slot)
                        {
                            // Keep the ordering frozen through the commit callback.
                            settled(slot);
                        }
                        busy.set(false);
                    }
                    _ => {}
                }
            }
        });
        if native {
            day_reactive::Effect::new(move || {
                let value = state.get();
                day_core::with_tree(|tree| {
                    tree.patch(node, Box::new(PagerPatch(value.clone())), false)
                });
            });
        }
        node
    }
}
day_pieces::glue_modules!(uikit, mdc, arkui);

/// The pager's conformance cases (docs/testing.md). `select` is the synthetic settle on every
/// backend: it reports a reading-order position, which the pager maps through `order` to a slot.
#[cfg(feature = "conformance")]
pub mod conformance {
    use day_core::conformance::{Case, Drive};
    use day_pieces::*;
    use day_reactive::Signal;

    use super::{KIND, PagerState, pager};

    /// Selecting a position commits that position's slot, and reversing the order maps the same
    /// position to a different slot without rebuilding a page.
    #[day_macros::test(day_core)]
    fn pager_selection_commits_the_slot_in_reading_order() -> Case {
        Case::new()
            .proves(KIND)
            .page(|| {
                let state = Signal::new(PagerState {
                    order: vec![0, 1, 2],
                    selected: 0,
                    epoch: 1,
                });
                let busy = Signal::new(false);
                let committed = Signal::new(String::from("none"));
                column((
                    label(move || format!("slot {}", committed.get()))
                        .id(day_core::id!(pager::pager_state)),
                    button("Reverse")
                        .action(move || {
                            state.update(|s| {
                                s.order.reverse();
                                s.epoch += 1;
                            })
                        })
                        .id(day_core::id!(pager::pager_reverse)),
                    pager(
                        state,
                        busy,
                        vec![
                            label("Page A").any(),
                            label("Page B").any(),
                            label("Page C").any(),
                        ],
                        move |slot| {
                            committed.set(slot.to_string());
                            state.update(|s| s.selected = slot);
                        },
                    )
                    .id(day_core::id!(pager::pager))
                    .height(200.0),
                ))
            })
            .drive(|d: Drive| async move {
                d.select("pager::pager", 1).await?;
                d.assert_text("pager::pager_state", "slot 1").await?;
                d.tap("pager::pager_reverse").await?;
                d.select("pager::pager", 0).await?;
                d.assert_text("pager::pager_state", "slot 2").await
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recycled_slots_reject_stale_completions_and_absent_neighbors() {
        let state = PagerState {
            order: vec![2, 0],
            selected: 2,
            epoch: 4,
        };
        assert!(state.accepts(4, 0));
        assert!(!state.accepts(3, 0));
        assert!(!state.accepts(4, 1));
    }
}
