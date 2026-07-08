# Map view (external piece)

> **Status: implemented** as `day-piece-map`, an external Day Piece (like `day-piece-media`),
> registered link-time into each backend's renderer slice without touching day. It wraps the
> platform's native map view (`MKMapView`) and is the reference for a piece that deliberately does
> not support every backend: only AppKit and UIKit render a real map. It fills the space it's
> offered (constrain it with `.frame(w, h)`).

## Authoring

```rust
use day_piece_map::map;

let where_to = Signal::new((37.7749, -122.4194)); // San Francisco

button("San Francisco").action(move || where_to.set((37.7749, -122.4194)));
button("New York").action(move || where_to.set((40.7128, -74.0060)));

map()
    .center_signal(where_to)      // preset buttons recenter the map live
    .span(0.05)                   // ~5 km across (smaller = more zoomed in)
    .marker(37.7749, -122.4194)   // drop a pin
    .frame(320.0, 240.0)          // a map is a growing leaf, so constrain it
    .id("map")
```

`map()` starts at `(0, 0)` with the default span (a few city blocks). Set a fixed region at build
with `.center(lat, lon)` + `.span(degrees)` and drop a pin with `.marker(lat, lon)`. The center can
also be bound reactively with `.center_signal(signal)`, which takes a `Signal<(f64, f64)>` or a
`Fn() -> (f64, f64)`, so a preset row or a location feed recenters the map live (a `Center` patch to
the native view; a `Const` center from `.center` seeds once and never patches). `.center` and
`.center_signal` are last-writer-wins. `Map` implements `Piece`, so `.id()` / `.a11y()` / `.frame()`
chain via `Decorate`. It's a growing leaf (`Flex { grow_w, grow_h }` + `day_pieces::fill_measure`),
so put it in a `.frame(w, h)` (or last in a `column`) and it fills the space it's offered.

The span is a coarse zoom knob (a `MKCoordinateSpan` latitude/longitude delta). v1 keeps a single
marker and no delegate callbacks (tap/region-change readback); the `Event::custom` channel is the
seam if those are wanted later.

## Per-backend native realization

| | AppKit | UIKit | GTK / Qt / Android / WinUI |
|---|---|---|---|
| control | `MKMapView` (NSView) | `MKMapView` (UIView) | — (placeholder leaf) |
| native code | `objc2-map-kit` typed binding | hand-rolled `extern_class!` + `msg_send!` | none |
| region | `setRegion(MKCoordinateRegion)` | `setRegion:animated:` | — |
| marker | `MKPointAnnotation` + `addAnnotation` | `MKPointAnnotation` + `addAnnotation:` | — |
| recenter | `setCenterCoordinate:animated:` | `setCenterCoordinate:animated:` | — |

**Backend notes:**

- **AppKit**: `objc2-map-kit` binds `MKMapView` as an `NSView` subclass (macOS only). The region is
  an `MKCoordinateRegion { center: CLLocationCoordinate2D, span: MKCoordinateSpan }` built from the
  props; a marker is an `MKPointAnnotation` added via the typed `addAnnotation`. MapKit renders
  without a key: no developer token or entitlement is needed for a plain map. The recenter patch uses
  `setCenterCoordinate:animated:` (keeps the current zoom).
- **UIKit**: `objc2-map-kit` generates the `MKMapView` struct for macOS only (the WKWebView /
  AVPlayerViewController situation again), so the piece hand-rolls the class via `extern_class!` +
  `msg_send!` and reuses the crate's cross-platform `MKCoordinateRegion` / `MKPointAnnotation`. MapKit
  + CoreLocation must be linked for the ObjC classes to register; they're declared via
  `[package.metadata.day.ios] frameworks = ["MapKit", "CoreLocation"]` and linked by the generated
  DayPieces SwiftPM package (the framework-contribution seam).
- **GTK / Qt / Android / WinUI / mock**: the features exist (so an app can enable
  `day-piece-map/<feature>` uniformly per backend) but register no renderer; the map kind falls
  back to day's placeholder leaf. There is no de-facto native slippy-map widget in these toolkits
  without a heavy external dependency (a WebView + tile provider, `osm-gps-map`, `QtLocation`, Google
  Maps SDK, `MapControl`), each with its own API-key and licensing story, which is out of scope for a
  small reference piece. This is the intentional "not every piece supports every platform" example:
  the gap is explicit rather than hidden behind a broken stub.

## What it shows about the extension system

`day-piece-map` is the counterpart to `day-piece-lottie`: a piece whose demo page is `#[cfg]`-gated
to the platforms it supports (`macos` + `ios`). The front-end compiles everywhere (it depends only
on core day crates), but the `map` page and the crate's backend features are enabled only for the
Apple backends; every other backend renders the placeholder leaf for the `day.piece.map` kind.
It also exercises the iOS framework-contribution seam (from the webview/media work,
docs/extending.md): `[package.metadata.day.ios] frameworks = ["MapKit", "CoreLocation"]` links the
system frameworks the hand-rolled `MKMapView` needs, with no changes to any core Day crate.

## Testing

The crate's smoke test boots the piece on the mock toolkit (which realizes unknown kinds as plain
widgets and ignores unknown patches, just like a backend built without the feature), moves the
bound center (a `Center` patch), and must never panic: `cargo test -p day-piece-map`.

For a live check, wire the showcase map page and use the media/webview walkthrough recipe on
appkit/macOS: navigate to the route, `pause` so the map tiles arrive, then screenshot.
