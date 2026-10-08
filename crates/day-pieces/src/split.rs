// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The split (docs/split.md): two panes with a divider the user drags, side by side or
//! stacked, the first pane's share bound to an app-owned `Signal<f64>`. Where `Cap::Split` is
//! `Native` the panes are the toolkit's own splitter (an `NSSplitView`, a `GtkPaned`, a
//! `QSplitter`); everywhere else the piece lays the panes out itself and draws the divider,
//! which it also drags.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use day_core::*;
use day_geometry::{Proposal, Rect};
use day_reactive::{Signal, bind_seeded};
use day_spec::props::{SplitAxis, SplitPaneProps, SplitPatch, SplitProps};
use day_spec::{Cap, Color, Cursor, DragPhase, Event, Shape, Size, Support, kinds};

use crate::*;

/// The least length either pane may be dragged to, in points (default).
pub const SPLIT_MIN_PANE: f64 = 80.0;
/// The composed divider's grab width, in points: a hairline to look at, this much to hit.
const HANDLE: f64 = 9.0;

/// Two panes and the divider between them, bound to the app (docs/split.md).
///
/// ```ignore
/// let share = Signal::new(0.5);
/// let stacked = Signal::new(false);
/// split(request_form(), response_view())
///     .axis(move || if stacked.get() { SplitAxis::Vertical } else { SplitAxis::Horizontal })
///     .fraction(share)
/// ```
pub struct Split {
    first: AnyPiece,
    second: AnyPiece,
    axis: Reactive<SplitAxis>,
    fraction: Option<Signal<f64>>,
    min_pane: f64,
}

/// Build a [`Split`]: `first` beside (or above) `second`, divided at half until dragged.
pub fn split<A: Piece, B: Piece>(first: A, second: B) -> Split {
    Split {
        first: AnyPiece::new(first),
        second: AnyPiece::new(second),
        axis: Reactive::Const(SplitAxis::Horizontal),
        fraction: None,
        min_pane: SPLIT_MIN_PANE,
    }
}

impl Split {
    /// Side by side ([`SplitAxis::Horizontal`], the default) or stacked; a signal or a closure
    /// turns the split live, keeping the share.
    pub fn axis<M>(mut self, axis: impl IntoReactive<SplitAxis, M>) -> Self {
        self.axis = axis.into_reactive();
        self
    }
    /// The first pane's share of the length, `0.0..=1.0`, two-way: a dragged divider writes
    /// it, writing it moves the divider. Unbound, the split keeps a share of its own from 0.5.
    pub fn fraction(mut self, fraction: Signal<f64>) -> Self {
        self.fraction = Some(fraction);
        self
    }
    /// The least length either pane may be dragged to (default [`SPLIT_MIN_PANE`]).
    pub fn min_pane(mut self, points: f64) -> Self {
        self.min_pane = points.max(0.0);
        self
    }
}

/// [`Split`]'s builders, forwarded through `Decorated` (docs/api-style.md).
pub trait SplitBuilder: Sized {
    fn axis<M>(self, axis: impl IntoReactive<SplitAxis, M>) -> Self;
    fn fraction(self, fraction: Signal<f64>) -> Self;
    fn min_pane(self, points: f64) -> Self;
}

impl SplitBuilder for Split {
    fn axis<M>(self, axis: impl IntoReactive<SplitAxis, M>) -> Self {
        Split::axis(self, axis)
    }
    fn fraction(self, fraction: Signal<f64>) -> Self {
        Split::fraction(self, fraction)
    }
    fn min_pane(self, points: f64) -> Self {
        Split::min_pane(self, points)
    }
}

impl<Inner: SplitBuilder + Piece> SplitBuilder for Decorated<Inner> {
    fn axis<M>(self, axis: impl IntoReactive<SplitAxis, M>) -> Self {
        self.map_inner(|inner| inner.axis(axis))
    }
    fn fraction(self, fraction: Signal<f64>) -> Self {
        self.map_inner(|inner| inner.fraction(fraction))
    }
    fn min_pane(self, points: f64) -> Self {
        self.map_inner(|inner| inner.min_pane(points))
    }
}

impl Piece for Split {
    fn build(self, cx: &mut BuildCx) -> RNode {
        if day_core::capability(Cap::Split) == Support::Native {
            build_native(self, cx)
        } else {
            build_composed(self, cx)
        }
    }
}

/// A share kept inside `0..1` with both panes at least `min` of a `total`.
fn clamp_share(share: f64, total: f64, min: f64) -> f64 {
    if total <= 0.0 || 2.0 * min >= total {
        return share.clamp(0.0, 1.0);
    }
    let lo = min / total;
    share.clamp(lo, 1.0 - lo)
}

// ---------------------------------------------------------------------------
// Native: the toolkit's own splitter (`Cap::Split == Native`)
// ---------------------------------------------------------------------------

fn build_native(split: Split, cx: &mut BuildCx) -> RNode {
    let Split {
        first,
        second,
        axis,
        fraction,
        min_pane,
    } = split;
    let fraction = fraction.unwrap_or_else(|| Signal::new(0.5));
    let initial_axis = axis.get_untracked();
    let initial = fraction.get_untracked().clamp(0.0, 1.0);
    let sizes: Rc<RefCell<std::collections::HashMap<RNode, Size>>> = Rc::default();
    let stacked = Rc::new(Cell::new(initial_axis == SplitAxis::Vertical));
    let share = Rc::new(Cell::new(initial));
    let node = cx.native(
        kinds::SPLIT,
        &SplitProps {
            axis: initial_axis,
            fraction: initial,
            min_pane,
        },
        Rc::new(SplitLayout {
            sizes: sizes.clone(),
            stacked: stacked.clone(),
            fraction: share.clone(),
        }),
        Flex {
            grow_w: true,
            grow_h: true,
            ..Default::default()
        },
        Boundary::Yes,
    );

    // The two panes, first then second (`SplitPaneProps::second`); frames native-owned,
    // reported like nav pages and inspector panes.
    let first_pane = pane(node, false, &sizes);
    let mut fcx = BuildCx::new(first_pane);
    let _ = first.build(&mut fcx);
    let second_pane = pane(node, true, &sizes);
    let mut scx = BuildCx::new(second_pane);
    let _ = second.build(&mut scx);

    // Axis → native, as a targeted patch; the layout cell keeps the fallback split in step.
    {
        let stacked = stacked.clone();
        bind_seeded(
            initial_axis,
            move || axis.get(),
            move |a: &SplitAxis| {
                stacked.set(*a == SplitAxis::Vertical);
                with_tree(|t| {
                    t.patch(node, Box::new(SplitPatch::Axis(*a)), false);
                    t.mark_needs_measure(node);
                    t.mark_layout_dirty();
                    t.layout_if_needed();
                });
            },
        );
    }
    // Share → native. `native_share` is what the splitter last reported, so a report written
    // into the signal is not patched straight back (the echo guard every bound control has).
    let native_share: Rc<Cell<Option<f64>>> = Rc::new(Cell::new(None));
    {
        let share = share.clone();
        let native_share = native_share.clone();
        bind_seeded(
            initial,
            move || fraction.get(),
            move |f: &f64| {
                let f = f.clamp(0.0, 1.0);
                share.set(f);
                if native_share.get().is_some_and(|n| (n - f).abs() < 1e-6) {
                    return;
                }
                with_tree(|t| {
                    t.patch(node, Box::new(SplitPatch::Fraction(f)), false);
                    t.mark_needs_measure(node);
                    t.mark_layout_dirty();
                    t.layout_if_needed();
                });
            },
        );
    }
    // Native divider → signal.
    cx.on(node, move |ev| {
        if let Event::ValueChanged(f) = ev {
            let f = f.clamp(0.0, 1.0);
            native_share.set(Some(f));
            if (fraction.get_untracked() - f).abs() > 1e-6 {
                fraction.set(f);
            }
        }
    });
    node
}

/// One native-owned pane container under the split host, with the same `FrameChanged`
/// wiring as an inspector pane: the backend reports each pane's real frame, and Day lays the
/// pane's content out inside it.
fn pane(
    host: RNode,
    second: bool,
    sizes: &Rc<RefCell<std::collections::HashMap<RNode, Size>>>,
) -> RNode {
    let mut cx = BuildCx::new(host);
    let pane = cx.native(
        kinds::SPLIT_PANE,
        &SplitPaneProps { second },
        Rc::new(PassThrough),
        Flex::default(),
        Boundary::Yes,
    );
    let sizes = sizes.clone();
    cx.on(pane, move |ev| {
        if let Event::FrameChanged(sz) = ev {
            let changed = sizes.borrow().get(&pane) != Some(sz);
            if changed {
                sizes.borrow_mut().insert(pane, *sz);
                with_tree(|t| {
                    t.mark_needs_measure(pane);
                    t.mark_layout_dirty();
                    t.layout_if_needed();
                });
            }
        }
    });
    pane
}

// ---------------------------------------------------------------------------
// Composed: Day's own layout, a drawn divider, a drag (every other backend)
// ---------------------------------------------------------------------------

/// The composed split's state, shared by its layout, its divider and its drag.
struct ComposedState {
    stacked: Cell<bool>,
    share: Cell<f64>,
    min_pane: f64,
    /// The length along the axis at the last placement: what a drag's translation is a share of.
    length: Cell<f64>,
}

struct ComposedLayout {
    state: Rc<ComposedState>,
}

impl Layout for ComposedLayout {
    fn measure(&self, _cx: &mut dyn LayoutOps, _children: &[RNode], p: Proposal) -> Size {
        // Greedy, like the native split: the panes take whatever the parent proposes.
        Size::new(p.width.unwrap_or(480.0), p.height.unwrap_or(640.0))
    }
    fn place(&self, cx: &mut dyn LayoutOps, children: &[RNode], bounds: Rect) {
        // Children: 0 first, 1 the divider, 2 second.
        let (w, h) = (bounds.size.width, bounds.size.height);
        let stacked = self.state.stacked.get();
        let length = if stacked { h } else { w };
        self.state.length.set(length);
        let usable = (length - HANDLE).max(0.0);
        let share = clamp_share(self.state.share.get(), usable, self.state.min_pane);
        let a = (usable * share).round();
        let b = (usable - a).max(0.0);
        let rects = if stacked {
            [
                Rect::new(0.0, 0.0, w, a),
                Rect::new(0.0, a, w, HANDLE),
                Rect::new(0.0, a + HANDLE, w, b),
            ]
        } else {
            [
                Rect::new(0.0, 0.0, a, h),
                Rect::new(a, 0.0, HANDLE, h),
                Rect::new(a + HANDLE, 0.0, b, h),
            ]
        };
        for (child, rect) in children.iter().zip(rects) {
            cx.place_child(*child, rect);
        }
    }
}

fn build_composed(split: Split, cx: &mut BuildCx) -> RNode {
    let Split {
        first,
        second,
        axis,
        fraction,
        min_pane,
    } = split;
    let fraction = fraction.unwrap_or_else(|| Signal::new(0.5));
    let initial_axis = axis.get_untracked();
    let state = Rc::new(ComposedState {
        stacked: Cell::new(initial_axis == SplitAxis::Vertical),
        share: Cell::new(fraction.get_untracked().clamp(0.0, 1.0)),
        min_pane,
        length: Cell::new(0.0),
    });
    let node = cx.layout_only(
        Rc::new(ComposedLayout {
            state: state.clone(),
        }),
        Flex {
            grow_w: true,
            grow_h: true,
            ..Default::default()
        },
        Boundary::No,
    );
    let relayout = move || {
        with_tree(|t| {
            t.mark_needs_measure(node);
            t.mark_layout_dirty();
        });
    };
    // The divider: a hairline in the middle of its grab width, drawn by Day, dragged by Day.
    // The drag moves the share by the pointer's travel as a fraction of the split's length.
    let drag_axis = axis.clone();
    let cursor_axis = axis.clone();
    let draw_axis = axis.clone();
    let drag_state = state.clone();
    let drag_start = Rc::new(Cell::new(0.0f64));
    let divider = canvas(move |d, size| {
        let stacked = draw_axis.get() == SplitAxis::Vertical;
        let line = if stacked {
            Rect::new(0.0, (size.height / 2.0).floor(), size.width, 1.0)
        } else {
            Rect::new((size.width / 2.0).floor(), 0.0, 1.0, size.height)
        };
        d.fill(Shape::Rect(line), Color::rgba(0.5, 0.5, 0.5, 0.45));
    })
    .cursor(move || {
        if cursor_axis.get() == SplitAxis::Vertical {
            Cursor::RowResize
        } else {
            Cursor::ColResize
        }
    })
    .on_drag(move |drag| {
        let stacked = drag_axis.get_untracked() == SplitAxis::Vertical;
        match drag.phase {
            DragPhase::Began => drag_start.set(drag_state.share.get()),
            DragPhase::Changed | DragPhase::Ended => {
                let usable = (drag_state.length.get() - HANDLE).max(1.0);
                let travel = if stacked {
                    drag.translation.y
                } else {
                    drag.translation.x
                };
                let next = clamp_share(
                    drag_start.get() + travel / usable,
                    usable,
                    drag_state.min_pane,
                );
                if (fraction.get_untracked() - next).abs() > 1e-6 {
                    fraction.set(next);
                }
            }
        }
    })
    .id("day-split-divider");
    cx.under(node, |cx| {
        let _ = first.build(cx);
        let _ = divider.build(cx);
        let _ = second.build(cx);
    });
    // Axis and share → the layout, which re-places the three children.
    {
        let state = state.clone();
        bind_seeded(
            initial_axis,
            move || axis.get(),
            move |a: &SplitAxis| {
                state.stacked.set(*a == SplitAxis::Vertical);
                relayout();
            },
        );
    }
    bind_seeded(
        state.share.get(),
        move || fraction.get(),
        move |f: &f64| {
            state.share.set(f.clamp(0.0, 1.0));
            relayout();
        },
    );
    node
}

#[cfg(feature = "conformance")]
pub(crate) mod conformance {
    use day_core::conformance::{Case, Drive, FrameExpect};
    use day_reactive::Signal;
    use day_spec::kinds;
    use day_spec::props::SplitAxis;

    use crate::*;

    /// Both panes show, and the bound share moves the divider: writing the signal re-lays
    /// the panes, and the first pane's width follows it.
    #[day_macros::test(day_core)]
    fn split_panes_follow_the_share() -> Case {
        Case::new()
            .proves(kinds::SPLIT)
            .proves(kinds::SPLIT_PANE)
            .page(|| {
                let share = Signal::new(0.5);
                column((
                    label(move || format!("share {:.2}", share.get())).id("share"),
                    button("Quarter")
                        .action(move || share.set(0.25))
                        .id("quarter"),
                    split(
                        column((label("First"),)).grow().id("first"),
                        column((label("Second"),)).grow().id("second"),
                    )
                    .fraction(share)
                    .width(400.0)
                    .height(200.0)
                    .id("split"),
                ))
            })
            .drive(|d: Drive| async move {
                d.assert_visible("first").await?;
                d.assert_visible("second").await?;
                d.tap("quarter").await?;
                d.wait_idle().await?;
                d.assert_text("share", "share 0.25").await?;
                // A quarter of the length, less the divider's own width, which differs per
                // toolkit (a hairline on AppKit, a handle on GTK and the composed tier); the
                // pane keeps the split's full height whatever its share.
                d.assert_frame(
                    "first",
                    FrameExpect {
                        width: Some(100.0),
                        height: Some(200.0),
                        tolerance: Some(10.0),
                        ..Default::default()
                    },
                )
                .await
            })
    }

    /// A share that would leave a pane narrower than the minimum is held at the minimum: the
    /// first pane keeps `min_pane` of width whatever the signal says, on the toolkit's own
    /// splitter and on the composed one alike.
    #[day_macros::test(day_core)]
    fn split_min_pane_holds_the_share() -> Case {
        Case::new()
            .proves(kinds::SPLIT)
            .page(|| {
                let share = Signal::new(0.5);
                column((
                    button("Sliver")
                        .action(move || share.set(0.02))
                        .id("sliver"),
                    split(
                        column((label("First"),)).grow().id("first"),
                        column((label("Second"),)).grow().id("second"),
                    )
                    .fraction(share)
                    .min_pane(80.0)
                    .width(400.0)
                    .height(200.0)
                    .id("split"),
                ))
            })
            .drive(|d: Drive| async move {
                d.tap("sliver").await?;
                d.wait_idle().await?;
                d.assert_frame(
                    "first",
                    FrameExpect {
                        width: Some(80.0),
                        tolerance: Some(10.0),
                        ..Default::default()
                    },
                )
                .await
            })
    }

    /// Turning the split the other way stacks the panes: the second pane moves below the
    /// first instead of beside it.
    #[day_macros::test(day_core)]
    fn split_axis_stacks_the_panes() -> Case {
        Case::new()
            .proves(kinds::SPLIT)
            .page(|| {
                let stacked = Signal::new(false);
                column((
                    button("Stack")
                        .action(move || stacked.set(true))
                        .id("stack"),
                    // Each pane's content fills its pane, so the pane's extent is what the
                    // frame assertions read; a native pane's Day frame carries its toolkit
                    // size but not its toolkit position.
                    split(
                        column((label("First"),)).grow().id("first"),
                        column((label("Second"),)).grow().id("second"),
                    )
                    .axis(move || {
                        if stacked.get() {
                            SplitAxis::Vertical
                        } else {
                            SplitAxis::Horizontal
                        }
                    })
                    .width(400.0)
                    .height(200.0)
                    .id("split"),
                ))
            })
            .drive(|d: Drive| async move {
                // Side by side: the second pane stands the split's full height.
                d.assert_frame(
                    "second",
                    FrameExpect {
                        height: Some(200.0),
                        ..Default::default()
                    },
                )
                .await?;
                d.tap("stack").await?;
                d.wait_idle().await?;
                // Stacked: it spans the split's full width instead.
                d.assert_frame(
                    "second",
                    FrameExpect {
                        width: Some(400.0),
                        ..Default::default()
                    },
                )
                .await
            })
    }

    day_core::tests! {
        split_panes_follow_the_share,
        split_axis_stacks_the_panes,
        split_min_pane_holds_the_share,
    }
}

#[cfg(test)]
mod tests {
    use super::clamp_share;

    #[test]
    fn a_share_keeps_both_panes_at_the_minimum() {
        assert_eq!(clamp_share(0.5, 400.0, 80.0), 0.5);
        assert_eq!(clamp_share(0.05, 400.0, 80.0), 0.2);
        assert_eq!(clamp_share(0.99, 400.0, 80.0), 0.8);
        // Too short for two minimums: only the 0..1 clamp is left.
        assert_eq!(clamp_share(1.5, 100.0, 80.0), 1.0);
    }
}
