// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! day_pieces::pullrefresh: pull-to-refresh for any Day scrollable, modeled on SwiftUI's
//! `refreshable(action:)` (DESIGN.md §15; docs/extending.md).
//!
//! ```ignore
//! let refreshing = Signal::new(false);
//! pull_to_refresh(refreshing, scroll(rows))        // or list(items, …)
//!     .on_refresh(move || {
//!         let done = refreshing.setter();
//!         std::thread::spawn(move || { reload(); done.set(false); });
//!     })
//! ```
//!
//! Semantics (the same contract as `UIRefreshControl` / `SwipeRefreshLayout.setRefreshing` /
//! ArkUI `Refresh.refreshing`): the bound `refreshing: Signal<bool>` is TWO-WAY. A user pull sets
//! it `true` and runs `on_refresh`; the app sets it `false` when its reload completes (from a
//! thread via [`day_reactive::Signal::setter`], or inside `day::task`); the app may also set it
//! `true` to begin a refresh programmatically. The piece's node additionally accepts
//! `Event::ToggleChanged` as a synthetic begin/end, so dayscript's existing `toggle:` step drives
//! it on every backend.
//!
//! Tiers ([`support`]):
//! - **Native**: iOS (`UIRefreshControl` attached to the descendant `UIScrollView`), Android
//!   (the Android toolkit's `DayPullRefresh extends SwipeRefreshLayout`), HarmonyOS (`ARKUI_NODE_REFRESH`).
//!   The piece is a container: its realized native view hosts the scrollable as a Day child, using
//!   the `cx.native` + fill-layout + `cx.under` recipe.
//! - **Emulated**, everywhere else: a pure-composition spinner overlay (`when(refreshing, …)`),
//!   with the pull gesture detected on AppKit (elastic-scroll overscroll) and GTK
//!   (`edge-overshot`); Qt and XAML are spinner + programmatic in v1.

use std::rc::Rc;

use crate::Decorated;
use day_core::{BuildCx, Flex, Piece, RNode};
use day_reactive::Signal;
use day_spec::Event;

pub use day_spec::controls::pullrefresh::*;

/// How pull-to-refresh is realized on the compiled backend: `Native` (a real platform refresh
/// control drives the gesture and indicator) or `Emulated` (composition overlay; gesture where the
/// toolkit exposes overscroll). Never `Unsupported`: the emulated tier always works.
pub fn support() -> day_spec::Support {
    let support = day_core::capability(day_spec::Cap::PullRefresh);
    if support == day_spec::Support::Unsupported {
        day_spec::Support::Emulated
    } else {
        support
    }
}

/// A scrollable wrapped with pull-to-refresh. Build with [`pull_to_refresh`].
pub struct PullRefresh<P: Piece> {
    refreshing: Signal<bool>,
    child: P,
    on_refresh: Option<Rc<dyn Fn()>>,
}

/// Wrap `child` (a `scroll(...)` or `list(...)`) with pull-to-refresh bound to `refreshing`.
pub fn pull_to_refresh<P: Piece>(refreshing: Signal<bool>, child: P) -> PullRefresh<P> {
    PullRefresh {
        refreshing,
        child,
        on_refresh: None,
    }
}

#[crate::builder(PullRefreshBuilder)]
impl<P: Piece> PullRefresh<P> {
    /// Run `f` when a refresh BEGINS (user pull, dayscript toggle, or programmatic
    /// `refreshing.set(true)` routed through a pull). Start the reload here and set the bound
    /// signal back to `false` when it completes.
    pub fn on_refresh(mut self, f: impl Fn() + 'static) -> Self {
        self.on_refresh = Some(Rc::new(f));
        self
    }
}

/// The begin path shared by every entry point: flip the signal (if not already refreshing) and run
/// the app's `on_refresh` exactly once per begin.
fn begin(refreshing: Signal<bool>, on_refresh: &Option<Rc<dyn Fn()>>) {
    if !refreshing.get_untracked() {
        refreshing.set(true);
        if let Some(f) = on_refresh {
            f();
        }
    }
}

/// Wire the piece's node: native/emulated pull begins arrive as `Event::Custom`; dayscript's
/// `toggle:` step arrives as `Event::ToggleChanged` (synthetic begin/end on every backend).
fn wire(cx: &mut BuildCx, node: RNode, refreshing: Signal<bool>, on_refresh: Option<Rc<dyn Fn()>>) {
    cx.on(node, move |ev| match ev {
        Event::Custom { .. } => begin(refreshing, &on_refresh),
        Event::ToggleChanged(true) => begin(refreshing, &on_refresh),
        Event::ToggleChanged(false) => refreshing.set(false),
        _ => {}
    });
}

impl<P: Piece> Piece for PullRefresh<P> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        if support() == day_spec::Support::Native {
            build_native(self, cx)
        } else {
            build_emulated(self, cx)
        }
    }
}

// ---------------------------------------------------------------------------
// Native tier: the piece is a container; its realized native view (passthrough host on iOS,
// SwipeRefreshLayout on Android, Refresh node on ArkUI) hosts the scrollable as a Day child.
// FrameLayout places that single child at the container's full bounds; the native side owns the
// refresh indicator. App-driven signal changes patch through (watch skips the initial value).
// ---------------------------------------------------------------------------

fn build_native<P: Piece>(piece: PullRefresh<P>, cx: &mut BuildCx) -> RNode {
    use day_core::{FrameLayout, with_tree};
    use day_reactive::watch;

    let PullRefresh {
        refreshing,
        child,
        on_refresh,
    } = piece;
    let node = cx.native(
        KIND,
        &RefreshProps {
            refreshing: refreshing.get_untracked(),
        },
        Rc::new(FrameLayout {
            width: None,
            height: None,
        }),
        Flex {
            grow_w: true,
            grow_h: true,
            ..Default::default()
        },
        day_core::Boundary::Yes,
    );
    cx.under(node, |cx| {
        let _ = child.build(cx);
    });
    wire(cx, node, refreshing, on_refresh.clone());
    // Two-way: app writes → native indicator. A native-initiated begin sets the signal, which
    // patches back a redundant-but-idempotent SetRefreshing(true) (begin/setRefreshing while
    // already refreshing is a no-op on every platform).
    watch(
        move || refreshing.get(),
        move |now, _| {
            let set = *now;
            with_tree(|t| t.patch(node, Box::new(RefreshPatch::SetRefreshing(set)), false));
        },
    );
    node
}

// ---------------------------------------------------------------------------
// Emulated tier: pure composition, an overlay container whose first child is the scrollable
// (optionally tweaked with per-backend overscroll observation) and whose second is a
// `when(refreshing, spinner-chip)` overlay pinned top-center. No custom kind, no renderer: the
// container is the same native panel as column/row, so this path works on every backend
// (including mock) with zero native code.
// ---------------------------------------------------------------------------

fn build_emulated<P: Piece>(piece: PullRefresh<P>, cx: &mut BuildCx) -> RNode {
    use crate::prelude::{Color, Decorate, spinner, when};
    use day_core::{Alignment, Boundary, OverlayLayout};
    use day_spec::props::ContainerProps;

    let PullRefresh {
        refreshing,
        child,
        on_refresh,
    } = piece;
    let node = cx.native(
        day_spec::kinds::CONTAINER,
        &ContainerProps::default(),
        Rc::new(OverlayLayout {
            align: Alignment::Top,
            size_to_first: false,
        }),
        Flex {
            grow_w: true,
            grow_h: true,
            ..Default::default()
        },
        Boundary::No,
    );
    cx.under(node, |cx| {
        // The scrollable (fills: it grows in both axes), with the backend's overscroll
        // observation attached where the toolkit exposes one (AppKit elastic scroll,
        // GTK edge-overshot). The tweak runs at mount with the realized scroll node. The glue
        // does not call into the reactive runtime from inside native dispatch: it emits a
        // `pullrefresh:begin` Custom event on the host node through the backend's sink (queued,
        // pumped at a safe point, and panic-contained), which `wire` below turns into the begin.
        let host = day_core::rnode_to_id(node);
        child
            .tweak(move |child| {
                day_core::with_tree(|tree| tree.attach_refresh(child, host));
            })
            .build(cx);
        // The refresh indicator: a floating chip with the native indeterminate spinner, shown
        // while `refreshing`, the emulated stand-in for the platform refresh header.
        let _ = when(
            move || refreshing.get(),
            || {
                spinner()
                    .padding(10.0)
                    .background(Color::rgba(0.5, 0.5, 0.55, 0.18))
                    .corner_radius(21.0)
                    .padding(10.0)
            },
        )
        .build(cx);
    });
    wire(cx, node, refreshing, on_refresh);
    node
}

// ---------------------------------------------------------------------------
// Per-toolkit native renderers + emulated gesture glue: one file per backend, gated to its
// feature + target (the webview convention).
// ---------------------------------------------------------------------------

// --- Typed builders, forwarded through `Decorated` (docs/api-style.md) ---

/// Pull-to-refresh's conformance cases (docs/testing.md). The `toggle` step is the synthetic
/// pull, on every backend.
#[cfg(feature = "conformance")]
pub mod conformance {
    use crate::*;
    use day_core::conformance::{Case, Drive};
    use day_reactive::Signal;

    use super::{KIND, pull_to_refresh};

    /// A pull begins a refresh: `refreshing` goes true and `on_refresh` runs; the app ending it
    /// sets it false.
    #[day_macros::test(day_core)]
    fn pullrefresh_begins_and_ends() -> Case {
        Case::new()
            .proves(KIND)
            .page(|| {
                let refreshing = Signal::new(false);
                let runs = Signal::new(0i64);
                column((
                    label(move || format!("refreshing {} runs {}", refreshing.get(), runs.get()))
                        .id(day_core::id!(pullrefresh::state)),
                    button("Done")
                        .action(move || refreshing.set(false))
                        .id(day_core::id!(pullrefresh::done)),
                    pull_to_refresh(refreshing, scroll(column((label("Row 1"), label("Row 2")))))
                        .on_refresh(move || runs.update(|n| *n += 1))
                        .id(day_core::id!(pullrefresh::pull))
                        .height(200.0),
                ))
            })
            .drive(|d: Drive| async move {
                d.toggle("pullrefresh::pull", true).await?;
                d.assert_text("pullrefresh::state", "refreshing true runs 1")
                    .await?;
                d.tap("pullrefresh::done").await?;
                d.assert_text("pullrefresh::state", "refreshing false runs 1")
                    .await
            })
    }
}

/// Whether a user pull gesture can begin a refresh on this toolkit.
pub fn gesture_support() -> day_spec::Support {
    day_core::capability(day_spec::Cap::PullRefreshGesture)
}
