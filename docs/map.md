---
navTitle: "Maps"
title: "Map view"
description: "Embed a MapKit map on macOS or iOS, set its region, and add a marker."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Map view

Show a place on an embedded map, set the visible region, and add a marker. `day-piece-map`
uses MapKit on macOS and iOS; other targets currently show a placeholder.

## Add to your app

Defined in [`day-piece-map`](https://github.com/daybrite/day/tree/main/pieces/day-piece-map). Add this to your app’s `Cargo.toml`
`[dependencies]` table, using the same Day Git source as the rest of your app:

```toml
[dependencies]
day-piece-map = { git = "https://github.com/daybrite/day.git" }
```

`day build -t <target>` enables this direct dependency’s matching toolkit feature
automatically. No dependency entry is needed in `Day.toml`.

| Platforms / toolkits | Availability |
| --- | --- |
| AppKit, UIKit | MapKit map and marker; no API key required. |
| GTK, Qt, Android, WinUI, ArkUI, Web | No map renderer; shows a placeholder. |

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](snippet-catalog.md) and [catalog setup](/docs/how-to/localization).

## Authoring

```rust
use day_piece_map::map;

let where_to = Signal::new((37.7749, -122.4194)); // San Francisco

button(res::str::san_francisco()).action(move || where_to.set((37.7749, -122.4194)));
button(res::str::new_york()).action(move || where_to.set((40.7128, -74.0060)));

map()
    .center(where_to) // preset buttons recenter the map live
    .span(0.05) // ~5 km across (smaller = more zoomed in)
    .marker(37.7749, -122.4194) // drop a pin
    .frame(320.0, 240.0) // a map is a growing leaf, so constrain it
    .id(id!(map))
```

[`map()`](https://daybrite.dev/api/day_piece_map/fn.map.html) starts at `(0, 0)` with the default span (a few city blocks). Set a fixed region at build
with [`.center((lat, lon))`](https://daybrite.dev/api/day_piece_map/trait.MapBuilder.html#tymethod.center) + [`.span(degrees)`](https://daybrite.dev/api/day_piece_map/trait.MapBuilder.html#tymethod.span) and drop a pin with [`.marker(lat, lon)`](https://daybrite.dev/api/day_piece_map/trait.MapBuilder.html#tymethod.marker). [`.center`](https://daybrite.dev/api/day_piece_map/trait.MapBuilder.html#tymethod.center)
takes a constant or a reactive source: a `Signal<(f64, f64)>` or a `Fn() -> (f64, f64)` recenters
the map live (a `Center` patch to the native view) whenever it changes, so a preset row or a
location feed drives it; a constant seeds once and never patches. A later [`.center`](https://daybrite.dev/api/day_piece_map/trait.MapBuilder.html#tymethod.center) replaces an
earlier one. `Map` implements `Piece`, so [`.id()`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.id) / [`.a11y()`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.a11y) / [`.frame()`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.frame)
chain via `Decorate`. It's a growing leaf (`Flex { grow_w, grow_h }` + [`day_pieces::fill_measure`](https://daybrite.dev/api/day_pieces/render/fn.fill_measure.html)),
so put it in a [`.frame(w, h)`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.frame) (or last in a [`column`](https://daybrite.dev/api/day_pieces/fn.column.html)) and it fills the space it's offered.

The span is a coarse zoom knob (a [`MKCoordinateSpan`](https://developer.apple.com/documentation/mapkit/mkcoordinatespan) latitude/longitude delta). v1 keeps a single
marker and no delegate callbacks (tap/region-change readback); the [`Event::custom`](https://daybrite.dev/api/day_spec/enum.Event.html#method.custom) channel is
where those would be added later.

## Per-toolkit native realization

Platform guidance: [Apple: maps](https://developer.apple.com/design/human-interface-guidelines/maps); [Apple: MapKit](https://developer.apple.com/documentation/mapkit).

| | AppKit | UIKit | GTK / Qt / Android / XAML |
|---|---|---|---|
| control | [`MKMapView`](https://developer.apple.com/documentation/mapkit/mkmapview) ([NSView](https://developer.apple.com/documentation/appkit/nsview)) | [`MKMapView`](https://developer.apple.com/documentation/mapkit/mkmapview) ([UIView](https://developer.apple.com/documentation/uikit/uiview)) | — (placeholder leaf) |
| native code | `objc2-map-kit` typed binding | hand-rolled `extern_class!` + `msg_send!` | none |
| region | `setRegion(MKCoordinateRegion)` | `setRegion:animated:` | — |
| marker | [`MKPointAnnotation`](https://developer.apple.com/documentation/mapkit/mkpointannotation) + `addAnnotation` | [`MKPointAnnotation`](https://developer.apple.com/documentation/mapkit/mkpointannotation) + `addAnnotation:` | — |
| recenter | `setCenterCoordinate:animated:` | `setCenterCoordinate:animated:` | — |

**Toolkit notes:**

- **AppKit**: `objc2-map-kit` binds [`MKMapView`](https://developer.apple.com/documentation/mapkit/mkmapview) as an [`NSView`](https://developer.apple.com/documentation/appkit/nsview) subclass (macOS only). The region is
  an `MKCoordinateRegion { center: CLLocationCoordinate2D, span: MKCoordinateSpan }` built from the
  props; a marker is an [`MKPointAnnotation`](https://developer.apple.com/documentation/mapkit/mkpointannotation) added via the typed `addAnnotation`. MapKit renders
  without a key: no developer token or entitlement is needed for a plain map. The recenter patch uses
  `setCenterCoordinate:animated:` (keeps the current zoom).
- **UIKit**: `objc2-map-kit` generates the [`MKMapView`](https://developer.apple.com/documentation/mapkit/mkmapview) struct for macOS only (the [WKWebView](https://developer.apple.com/documentation/webkit/wkwebview) /
  [AVPlayerViewController](https://developer.apple.com/documentation/avkit/avplayerviewcontroller) situation again), so the piece hand-rolls the class via `extern_class!` +
  `msg_send!` and reuses the crate's cross-platform [`MKCoordinateRegion`](https://developer.apple.com/documentation/mapkit/mkcoordinateregion) / [`MKPointAnnotation`](https://developer.apple.com/documentation/mapkit/mkpointannotation). MapKit
  + CoreLocation must be linked for the ObjC classes to register; they're declared via
  `[package.metadata.day.ios] frameworks = ["MapKit", "CoreLocation"]` and linked by the generated
  DayPieces SwiftPM package (the framework-contribution mechanism).
- **GTK / Qt / Android / XAML / mock**: the features exist (so an app can enable
  `day-piece-map/<feature>` uniformly per toolkit) but register no renderer; the map kind falls
  back to day's placeholder leaf. There is no de-facto native slippy-map widget in these toolkits
  without a heavy external dependency (a WebView + tile provider, `osm-gps-map`, `QtLocation`, Google
  Maps SDK, `MapControl`), each with different API-key and licensing requirements, which is out of scope for a
  small reference piece. This piece is the reference example of one that supports a subset of
  platforms; the placeholder leaf keeps the gap visible.

## What it shows about the extension system

`day-piece-map` is the counterpart to [day-piece-lottie](https://github.com/daybrite/day-piece-lottie): a piece whose demo page is `#[cfg]`-gated
to the platforms it supports (`macos` + `ios`). The front-end compiles everywhere (it depends only
on core day crates), but the [`map`](https://daybrite.dev/api/day_piece_map/fn.map.html) page and the crate's toolkit features are enabled only for the
Apple toolkits; every other toolkit renders the placeholder leaf for the `day.piece.map` kind.
It also exercises the iOS framework-contribution mechanism (added for the webview/media work,
[docs/extending](extending.md)): `[package.metadata.day.ios] frameworks = ["MapKit", "CoreLocation"]` links the
system frameworks the hand-rolled [`MKMapView`](https://developer.apple.com/documentation/mapkit/mkmapview) needs, with no changes to any core Day crate.
