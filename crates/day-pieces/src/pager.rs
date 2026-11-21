// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Native interactive horizontal paging. Slots have stable native identities; `order`
//! chooses which slots participate and their reading order. Changing order never rebuilds
//! a page. Apps must defer changing slot contents while `busy` is true. Native callbacks
//! carry the presentation epoch, so delayed completions cannot select a recycled slot.
use crate::Decorated;
use day_core::{Boundary, BuildCx, Flex, Layout, LayoutOps, Piece, RNode};
use day_reactive::{AnyBinding, Binding, Signal};
use day_spec::{Event, Proposal, Rect, Size};
use std::rc::Rc;

pub use day_spec::controls::pager::*;

/// A native interactive pager (docs/pager.md). Built by [`pager`].
#[must_use]
pub struct Pager {
    state: Signal<PagerState>,
    busy: Signal<bool>,
    pages: Vec<day_core::AnyPiece>,
    settled: Rc<dyn Fn(usize)>,
    progress: Option<AnyBinding<f64>>,
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
        progress: None,
    }
}

#[day_macros::builder(PagerBuilder)]
impl Pager {
    /// Follow the pager's position (docs/pager.md "Progress"): `progress` is the place in
    /// reading order, in pages, so `0.0` is the first page and `1.5` is halfway from the
    /// second to the third. A native pager writes it continuously while a page is dragged
    /// (probe `Cap::PagerProgress`), which is what lets a page indicator track the finger;
    /// everywhere else it steps to the selected page's position when the selection changes.
    ///
    /// ```ignore
    /// let progress = Signal::new(0.0);
    /// column((pager(state, busy, pages, settle).progress(progress), dots(progress)))
    /// ```
    pub fn progress(mut self, progress: impl Binding<f64>) -> Self {
        self.progress = Some(AnyBinding::new(progress));
        self
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
        use crate::prelude::*;
        let Self {
            state,
            busy,
            pages,
            settled,
            progress,
        } = self;
        let native = support() == day_spec::Support::Native;
        let node = if native {
            cx.native(
                KIND,
                &PagerProps {
                    slots: pages.len(),
                    track_progress: progress.is_some(),
                },
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
                let page = column((page,)).grow(Dimension::Both);
                if native {
                    page.build(cx);
                } else {
                    let child = page.build(cx);
                    let show =
                        move || state.get().selected == slot && state.get().order.contains(&slot);
                    day_core::with_tree(|t| t.set_presented(child, show()));
                    day_reactive::watch(show, move |visible, _| {
                        day_core::with_tree(|t| t.set_presented(child, *visible));
                    });
                }
            }
        });
        // The selected page's place in reading order is the progress at rest: where a toolkit
        // reports nothing continuous, this is all of it.
        if let Some(progress) = progress {
            day_reactive::bind(
                move || {
                    let state = state.get();
                    state.order.iter().position(|slot| *slot == state.selected)
                },
                move |at: &Option<usize>| {
                    if let Some(at) = at {
                        progress.set_if_changed(*at as f64);
                    }
                },
            );
        }
        let track = progress;
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
                    "progress" => {
                        if let (Some(progress), Some(at)) =
                            (&track, fields.get(2).and_then(|s| s.parse::<f64>().ok()))
                            && at.is_finite()
                        {
                            progress.set_if_changed(at);
                        }
                    }
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
                    tree.patch(node, Box::new(PagerPatch(value.clone())), false);
                });
            });
        }
        node
    }
}

/// The pager's conformance cases (docs/testing.md). `select` is the synthetic settle on every
/// backend: it reports a reading-order position, which the pager maps through `order` to a slot.
#[cfg(feature = "conformance")]
pub mod conformance {
    use crate::*;
    use day_core::conformance::{Case, Drive};
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
                            });
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

    /// The progress binding rests on the selected page's place in reading order
    /// (docs/pager.md "Progress").
    #[day_macros::test(day_core)]
    fn pager_progress_rests_on_the_selected_page() -> Case {
        Case::new()
            .proves(KIND)
            .page(|| {
                let state = Signal::new(PagerState {
                    order: vec![0, 1, 2],
                    selected: 0,
                    epoch: 1,
                });
                let busy = Signal::new(false);
                let progress = Signal::new(0.0_f64);
                column((
                    label(move || format!("at {:.1}", progress.get()))
                        .id(day_core::id!(pager::progress_label)),
                    pager(
                        state,
                        busy,
                        vec![
                            label("Page A").any(),
                            label("Page B").any(),
                            label("Page C").any(),
                        ],
                        move |slot| state.update(|s| s.selected = slot),
                    )
                    .progress(progress)
                    .id(day_core::id!(pager::progress_pager))
                    .height(200.0),
                ))
            })
            .drive(|d: Drive| async move {
                d.assert_text("pager::progress_label", "at 0.0").await?;
                d.select("pager::progress_pager", 2).await?;
                d.assert_text("pager::progress_label", "at 2.0").await?;
                d.select("pager::progress_pager", 1).await?;
                d.assert_text("pager::progress_label", "at 1.0").await
            })
    }
}

/// Whether the selected toolkit implements this control.
#[must_use]
pub fn support() -> day_spec::Support {
    day_core::capability(day_spec::Cap::Pager)
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
