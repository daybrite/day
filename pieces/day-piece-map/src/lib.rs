//! day-piece-map — an EXTERNAL Day Piece (DESIGN.md §15) wrapping the platform's NATIVE map view,
//! **APPLE PLATFORMS ONLY**. It is the reference for a piece that deliberately does NOT support every
//! backend: AppKit and UIKit render a real `MKMapView`; on GTK/Qt/Android/WinUI the `map` kind falls
//! back to day's placeholder leaf (those features exist but register no renderer). One Rust API,
//! registered link-time into each Apple backend's renderer slice without touching day.
//!
//! `map()` shows a slippy map centered on a coordinate. Configure the region at build with
//! `.center(lat, lon)` + `.span(degrees)` (a smaller span zooms in) and drop a pin with
//! `.marker(lat, lon)`. The center can also be bound reactively with `.center_signal(signal)` so a
//! preset button row or a location feed recenters the map live (a `Center` patch to the native view;
//! a `Const` center from `.center` seeds once and never patches). A map fills the space it's offered
//! (a growing leaf), so constrain it with `.frame(w, h)`. See docs/map.md for the per-backend caveats.
//!
//! ```ignore
//! let where_to = Signal::new((37.7749, -122.4194)); // San Francisco
//! map()
//!     .center_signal(where_to)  // preset buttons `where_to.set(..)` recenter live
//!     .span(0.05)               // ~5 km across
//!     .marker(37.7749, -122.4194)
//!     .frame(320.0, 240.0)
//!     .id("map");
//! ```

use day_core::{BuildCx, Flex, Piece, RNode, with_tree};
use day_pieces::{IntoReactive, Reactive};
use day_reactive::bind_seeded;

pub const KIND: &str = "day.piece.map";

/// A (latitude, longitude) coordinate in degrees.
pub type Coord = (f64, f64);

/// The default zoom span (latitude/longitude delta, in degrees) — a few city blocks across.
pub const DEFAULT_SPAN: f64 = 0.02;

/// Full props (realize). `lat`/`lon` seed the center (thereafter patched when the center is a
/// reactive source), while `span` and `marker` are fixed at build time.
#[derive(Clone, Debug, PartialEq)]
pub struct MapProps {
    /// Center latitude in degrees.
    pub lat: f64,
    /// Center longitude in degrees.
    pub lon: f64,
    /// Latitude/longitude delta covered by the viewport, in degrees (smaller = more zoomed in).
    pub span: f64,
    /// An optional pin dropped at `(lat, lon)`.
    pub marker: Option<Coord>,
}

impl Default for MapProps {
    fn default() -> Self {
        MapProps {
            lat: 0.0,
            lon: 0.0,
            span: DEFAULT_SPAN,
            marker: None,
        }
    }
}

/// Sparse reconcile patch — only the center moves after build (span/marker are fixed).
#[derive(Clone, Debug, PartialEq)]
pub enum MapPatch {
    /// Recenter the map on a new coordinate — pushed whenever a bound center source changes.
    Center { lat: f64, lon: f64 },
}

/// A native map view centered on a coordinate. Set the region with `.center()`/`.span()`, drop a pin
/// with `.marker()`, and optionally bind the center reactively with `.center_signal()`.
pub struct Map {
    center: Reactive<Coord>,
    span: f64,
    marker: Option<Coord>,
}

/// `map()` — a native map view (defaults to `(0, 0)` at the default span, no marker). Point it at a
/// place with `.center(lat, lon)` (or `.center_signal(..)` to follow a coordinate signal).
pub fn map() -> Map {
    Map {
        center: Reactive::Const((0.0, 0.0)),
        span: DEFAULT_SPAN,
        marker: None,
    }
}

impl Map {
    /// Center the map on a fixed coordinate (degrees). Build-time; use `.center_signal` for a
    /// reactive center.
    pub fn center(mut self, lat: f64, lon: f64) -> Self {
        self.center = Reactive::Const((lat, lon));
        self
    }

    /// Bind the center to a reactive coordinate — a `Signal<(f64, f64)>` or a `Fn() -> (f64, f64)`.
    /// When it changes, the map recenters live (a `Center` patch). Last of `.center`/`.center_signal`
    /// wins.
    pub fn center_signal<M>(mut self, coord: impl IntoReactive<Coord, M>) -> Self {
        self.center = coord.into_reactive();
        self
    }

    /// The latitude/longitude delta covered by the viewport, in degrees — the zoom level. A smaller
    /// span is more zoomed in (default [`DEFAULT_SPAN`], a few city blocks).
    pub fn span(mut self, degrees: f64) -> Self {
        self.span = degrees;
        self
    }

    /// Drop a pin at `(lat, lon)` (degrees). Build-time; a single marker in v1.
    pub fn marker(mut self, lat: f64, lon: f64) -> Self {
        self.marker = Some((lat, lon));
        self
    }
}

impl Piece for Map {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let Map {
            center,
            span,
            marker,
        } = self;
        let seed = center.get_untracked();
        let props = MapProps {
            lat: seed.0,
            lon: seed.1,
            span,
            marker,
        };
        // A map has no intrinsic size — it fills whatever space its container offers.
        let node = cx.leaf(
            KIND,
            &props,
            Flex {
                grow_w: true,
                grow_h: true,
                ..Default::default()
            },
        );
        // A `Const` center reads the same value forever, so this seeds once and never patches; a
        // `Signal`/`Fn` center re-runs and pushes a `Center` patch on every change.
        bind_seeded(
            seed,
            move || center.get(),
            move |c: &Coord| {
                with_tree(|t| {
                    t.patch(
                        node,
                        Box::new(MapPatch::Center { lat: c.0, lon: c.1 }),
                        false,
                    )
                });
            },
        );
        node
    }
}

// ---------------------------------------------------------------------------
// Per-toolkit native renderers — Apple only. Each registers a `Renderer` link-time into its backend's
// `RENDERERS` slice; `#[cfg]` gates each to its feature + target. gtk/qt/widget/winui/mock register
// nothing (the map kind falls back to day's placeholder leaf there).
// ---------------------------------------------------------------------------

day_pieces::glue_modules!(appkit, uikit);

#[cfg(test)]
mod tests {
    use super::*;
    use day_mock::MockToolkit;
    use day_reactive::{Signal, flush_sync};
    use day_spec::{Size, WindowOptions};

    // Building + recentering the piece must never panic — even with no native renderer registered
    // (the mock toolkit realizes unknown kinds as plain widgets and ignores unknown patches, exactly
    // like a backend built without this piece's feature, e.g. GTK/Qt/Android/WinUI).
    #[test]
    fn build_and_recenter_do_not_panic() {
        let center = Signal::new((37.7749, -122.4194)); // San Francisco

        day_core::uninstall_tree();
        let (mock, probe) = MockToolkit::new();
        let options = WindowOptions {
            title: "test".into(),
            size: Size::new(400.0, 300.0),
            ..Default::default()
        };
        day_core::launch_with(mock, options, move || {
            day_core::AnyPiece::new(
                map()
                    .center_signal(center)
                    .span(0.05)
                    .marker(37.8199, -122.4783),
            )
        });

        let found = probe.find_by_kind(KIND);
        assert_eq!(found.len(), 1, "one map leaf realized");

        // Moving the bound center becomes a `Center` patch the mock ignores gracefully.
        center.set((40.7128, -74.0060)); // New York
        flush_sync();
    }
}
