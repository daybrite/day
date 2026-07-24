//! day-piece-activity — an EXTERNAL Day Piece (DESIGN.md §15) wrapping each toolkit's NATIVE
//! indeterminate activity/loading spinner: `NSProgressIndicator` (Spinning style) on AppKit,
//! `UIActivityIndicatorView` on UIKit, `GtkSpinner` on GTK, a busy `QProgressBar` (range 0..0) on
//! Qt, `android.widget.ProgressBar` on Android, and `ProgressRing` on WinUI. One Rust API registered
//! link-time into each backend's renderer slice without touching day, carrying both a
//! front-end AND its own native backends (including an Android Java shim), see docs/extending.md.
//!
//! Unlike a media player, a spinner has an **intrinsic size** — the piece is a natural-size leaf
//! (no `fill_measure`; each backend's default `measure` returns the native indicator's fitting
//! size), so it does not need a `.frame(w, h)` to be visible (though one gives it a stable region).
//! `.animating(_)` accepts a `bool`, a `Signal<bool>`, or a closure (default true): a reactive
//! source starts/stops the animation through an `ActivityPatch::Animating` write. `.large(true)`
//! selects the platform's large control size.

use day_core::{BuildCx, Flex, Piece, RNode, with_tree};
use day_pieces::{IntoReactive, Reactive};
use day_reactive::bind_seeded;

pub const KIND: &str = "day.piece.activity";

/// Full props (realize). Both are set at build; only `animating` patches thereafter.
#[derive(Clone, Debug, PartialEq)]
pub struct ActivityProps {
    /// Whether the indicator is spinning (default true).
    pub animating: bool,
    /// Use the platform's large control size (default false).
    pub large: bool,
}

impl Default for ActivityProps {
    fn default() -> Self {
        ActivityProps {
            animating: true,
            large: false,
        }
    }
}

/// The one sparse update the spinner takes after creation.
#[derive(Clone, Debug, PartialEq)]
pub enum ActivityPatch {
    /// Start (`true`) or stop (`false`) the animation.
    Animating(bool),
}

/// A native activity/loading spinner. Bind its running state with `.animating(_)` and pick the
/// large control size with `.large(true)`.
pub struct Activity {
    animating: Reactive<bool>,
    large: bool,
}

/// `activity()` — a native indeterminate spinner, animating by default. Configure with
/// `.animating(_)` (a `bool`, `Signal<bool>`, or closure) and `.large(bool)`.
pub fn activity() -> Activity {
    Activity {
        animating: Reactive::Const(true),
        large: false,
    }
}

impl Activity {
    /// Start/stop the animation from a `bool`, a `Signal<bool>`, or a closure (default true). A
    /// reactive source is watched and drives an `ActivityPatch::Animating` on every change.
    pub fn animating<M>(mut self, source: impl IntoReactive<bool, M>) -> Self {
        self.animating = source.into_reactive();
        self
    }
    /// Use the platform's large control size (default false).
    pub fn large(mut self, large: bool) -> Self {
        self.large = large;
        self
    }
}

impl Piece for Activity {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let Activity { animating, large } = self;
        let initial = ActivityProps {
            animating: animating.get_untracked(),
            large,
        };
        // A spinner keeps its intrinsic size — no grow flags; the backend's default `measure`
        // returns the native indicator's natural size.
        let node = cx.leaf(KIND, &initial, Flex::default());

        // Only a dynamic source needs a binding; a constant is carried by the initial props (and
        // `watch`/`bind_seeded` never fire for the seed, so wiring this issues no spurious patch).
        if let Reactive::Dyn(f) = animating {
            bind_seeded(
                initial.animating,
                move || f(),
                move |on: &bool| {
                    with_tree(|t| t.patch(node, Box::new(ActivityPatch::Animating(*on)), false));
                },
            );
        }
        node
    }
}

// ---------------------------------------------------------------------------
// Per-toolkit native renderers — one file per backend. Each module registers a `Renderer`
// link-time into its backend's `RENDERERS` slice; `#[cfg]` gates each to its feature + target, and
// `#[path]` keeps the files grouped next to lib.rs. mock registers nothing (the activity kind falls
// back to day's placeholder leaf there).
// ---------------------------------------------------------------------------

day_pieces::glue_modules!(appkit, gtk, qt, uikit, mdc, winui);

#[cfg(test)]
mod tests {
    use super::*;
    use day_mock::MockToolkit;
    use day_reactive::{Signal, flush_sync};
    use day_spec::{Size, WindowOptions};

    // Building + driving the piece must never panic — even with no native renderer registered (the
    // mock toolkit realizes unknown kinds as plain widgets and ignores unknown patches, exactly
    // like a backend built without this piece's feature).
    #[test]
    fn build_and_toggle_do_not_panic() {
        let spinning = Signal::new(true);

        day_core::uninstall_tree();
        let (mock, probe) = MockToolkit::new();
        let options = WindowOptions {
            title: "test".into(),
            size: Size::new(200.0, 200.0),
            ..Default::default()
        };
        day_core::launch_with(mock, options, move || {
            day_core::AnyPiece::new(activity().animating(spinning).large(true))
        });

        let found = probe.find_by_kind(KIND);
        assert_eq!(found.len(), 1, "one activity leaf realized");

        // Flip the bound signal both ways; each becomes an ActivityPatch the mock ignores.
        spinning.set(false);
        flush_sync();
        spinning.set(true);
        flush_sync();
    }

    // A constant `.animating(false)` needs no binding and still realizes cleanly.
    #[test]
    fn constant_source_builds() {
        day_core::uninstall_tree();
        let (mock, probe) = MockToolkit::new();
        day_core::launch_with(mock, WindowOptions::default(), move || {
            day_core::AnyPiece::new(activity().animating(false))
        });
        assert_eq!(probe.find_by_kind(KIND).len(), 1);
    }
}
