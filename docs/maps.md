---
navTitle: "Maps"
title: "Maps"
description: "Share camera, annotations, and overlays across Day’s canvas renderer, Apple Maps, and Google Maps."
---

<!-- Copyright © The Daybrite Project; SPDX-License-Identifier: CC-BY-SA-4.0 -->

# Maps

Maps place locations and geographic features in context. A shared camera and annotation model
lets an app offer different map providers while preserving the position and places people selected.

The external [Day maps repository](https://github.com/daybrite/day-piece-map) supplies
`day-piece-map` for shared state and separate canvas, Apple, and Google implementations.

| Provider | Platforms | Rendering |
| --- | --- | --- |
| Day canvas | All Day targets | Day's canvas renderer, supporting MapLibre styles and vector tiles |
| Apple Maps | [iOS](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit")/[UIKit](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit"), [macOS](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit")/[AppKit](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit") | Swift [MKMapView](https://developer.apple.com/documentation/mapkit/mkmapview) |
| Google Maps | [Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material")/Material | [Maps SDK for Android](https://developers.google.com/maps/documentation/android-sdk/overview) |

## Dependencies

For an application offering every provider supported on its target:

```toml
[dependencies]
day-piece-map = "0.*"
day-piece-map-canvas = "0.*"
day-piece-map-apple = "0.*"
day-piece-map-google = "0.*"

[features]
appkit = ["day/appkit"]
uikit = ["day/uikit"]
material = ["day/material"]
```

Keep the application's other toolkit features. The native providers declare their backends in
`package.metadata.day.piece`; the CLI enables the matching renderer feature automatically.
The canvas renderer needs no toolkit feature of its own. For direct Cargo checks, enable the provider's
`appkit`, `uikit`, or `material` feature explicitly.
Build native apps with `day build`: Cargo alone does not compile the contributed Swift/Java sources.

## Shared state

```rust
use day::prelude::*;
use day_piece_map::{LngLat, MapView, Marker, Overlay};

let view = MapView::new(LngLat::new(-0.1276, 51.5072), 12.0);
let marker = view.add_marker(
    Marker::new(LngLat::new(-0.0877, 51.5055))
        .title(res::str::destination().format())
        .subtitle(res::str::destination_detail().format()),
);
view.overlays.set(vec![Overlay::Polyline {
    points: vec![view.center.get_untracked(), LngLat::new(-0.0877, 51.5055)],
    color: Color::rgb(0.0, 0.4, 0.9),
    width: 3.0,
}]);
let piece = day_piece_map_canvas::map(
    "https://tiles.openfreemap.org/styles/liberty",
    day_piece_map_canvas::CanvasMapView::with_view(view),
);
```

The resource accessors above stand for messages in the application's Fluent catalog. Marker
text may also come directly from user data or a place-data service. Providers do not translate it.

- `center`, `zoom`, and `bearing` are reactive and bidirectional. Write them to move the camera;
  native camera callbacks and canvas gestures update them after movement.
- `jump_to`, `jump_to_with`, `zoom_by`, `fly_to`, `fly_to_with`, `fit_bounds`, `rotate_to`, and
  `reset_north` control the mounted map. Animation commands issued before mounting are not replayed;
  initialize the camera with [`MapView::new`](https://docs.rs/day-piece-map/latest/day_piece_map/struct.MapView.html#method.new) or `jump_to` instead.
- Coordinates use WGS84 degrees in **longitude, latitude** order. Zoom uses a 512-logical-point
  Mercator world at level zero. The Google adapter converts to its SDK's 256-point convention;
  MapKit converts between visible map extent and zoom. Native limits and pitch can affect the result.
- Marker IDs survive updates. Use `update_marker`, `remove_marker`, `clear_markers`, and `marker`;
  `tap` reports the geographic position, local logical point, and optional marker ID.
- `Overlay` supports polylines, polygons, and circles. Widths are logical points and radii are
  meters. Polygons have one exterior ring. Overlay hit testing and arbitrary native annotation
  views are not part of this shared API.
- `interaction` enables or disables pan, zoom, and rotation gestures independently. Programmatic
  commands remain available. Provider-specific pitch is configured through native options.
- `status` reports loading, readiness, or a diagnostic error. Map error codes to localized app
  messages; do not display diagnostic strings as UI. Readiness does not guarantee every tile has
  loaded. Google authorization failures can leave the SDK loading without a useful error callback.
- Native SDKs retain their attribution controls. The canvas renderer shows compact, clickable source credits at the lower-left. Its OpenStreetMap notice
  links to [the copyright page](https://www.openstreetmap.org/copyright); additional required
  source credits remain visible.
  `attribution` exposes source credit text where available; an empty native value is not permission
  to hide the SDK's attribution.

Keep one `MapView` attached to **one mounted provider at a time**. Create it outside a `switch`
arm; switching disposes the old map and preserves shared camera, markers, and overlays. Cameras
are shared at rest; providers choose their animation path. MapKit treats a positive requested
animation duration as an animation request and chooses its duration. Google and the canvas renderer use the
requested duration. `fit_bounds` resets heading; a southwest longitude larger than northeast
means the bounds cross the antimeridian.

## Canvas renderer extensions

```rust
let canvas_view = day_piece_map_canvas::CanvasMapView::with_view(view);
canvas_view.set_language(Some("fr"));
let piece = day_piece_map_canvas::map(style_url, canvas_view);
```

`CanvasMapView` owns `common: MapView` plus label-language selection, tile counts (`pending`),
renderer statistics (`frame`), and `feature_hits` for the last basemap tap. Access camera and
marker methods through `common`, and read all taps from `common.tap`. There is no second tap
signal. Keep one wrapper and pass it explicitly to `map` or `map_with_client`; constructing a
new wrapper creates fresh language and diagnostic state.

The provider retains style JSON/URL inputs, reactive style changes, custom HTTP clients,
expression evaluation, vector tile decoding, sprites, sample data, and PMTiles sources.
See the [canvas renderer reference](https://github.com/daybrite/day-piece-map/blob/main/crates/canvas/README.md) for its rendering limits.
Day's canvas renderer supports a subset of the MapLibre style specification and renders vector tiles directly.
It can display OpenStreetMap-based maps or custom geographic data. It does not embed a MapLibre SDK.

## Apple extensions and Swift bridge

```rust
use day_piece_map_apple::{AppleMapBuilder, AppleOptions, MapType};
let options = Signal::new(AppleOptions {
    map_type: MapType::Hybrid,
    traffic: true,
    buildings: true,
    ..AppleOptions::default()
});
let piece = day_piece_map_apple::map(view).options(options);
```

Options include standard/satellite/hybrid basemaps, traffic, buildings, points of interest,
compass visibility, and camera pitch. Bind `AppleOptions` to a signal to change them live.
MapKit selects its label language from the system; it does not consume the canvas renderer's language setting.
MapKit annotation callouts show the shared title and subtitle.

The adapter uses **Swift MapKit APIs on `MKMapView`**, on both AppKit and UIKit. Its metadata
contributes `platform/swift` through `[package.metadata.day.macos]` and
`[package.metadata.day.ios]`, and declares `frameworks = ["MapKit"]`. This follows Lottie's Swift
source and C-ABI bridge technique. MapKit is supplied by Apple, so there is no remote SwiftPM
dependency. Day compiles the source into its generated Swift package and links the framework.

The Swift view owns its delegate state. Rust owns the retained native view. Renderer release
clears callbacks and delegate references before Day releases the view. These APIs fit Day's
existing deployment floors; newer MapKit APIs must be guarded in Swift when added.

## Google extensions and [Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material") setup

```rust
use day_piece_map_google::{GoogleMapBuilder, GoogleOptions, MapType};
let options = Signal::new(GoogleOptions {
    map_type: MapType::Terrain,
    traffic: true,
    indoor: true,
    zoom_controls: true,
    ..GoogleOptions::default()
});
let piece = day_piece_map_google::map(view).options(options);
```

Options include normal/satellite/terrain/hybrid/empty basemaps, traffic, indoor maps, buildings,
compass, zoom controls, pitch, and local Google style JSON (`style_json`). MapLibre style JSON
cannot be used as Google style JSON. Google marker color uses its standard marker hue and alpha;
it does not reproduce arbitrary saturation and brightness. Labels use the SDK's locale.

Like Lottie's [Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material") adapter, the crate contributes Java source and its Maven dependency through
`[package.metadata.day.android]`. Day adds `com.google.android.gms:play-services-maps:20.0.0` to
the generated Gradle project. That SDK requires Android API 23 or later and Google Play services.
The provider also contributes an optional `org.apache.http.legacy` manifest entry for devices
whose Play services Maps module uses the fallback renderer; applications need no manual entry.
The adapter forwards attachment, activity resume/pause/start/stop, low-memory, state-saving, and
final destruction to the SDK's `MapView`. Switching providers unregisters activity callbacks and
removes native map listeners. Day's shared camera is the source of state when a view is remounted.

Enable **Maps SDK for [Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material")** in a Google Cloud project with billing, create a key restricted
to the application's Android package and signing certificate, and supply it through the app's
`platform/android/AndroidManifest.xml` overlay:

```xml
<manifest xmlns:android="http://schemas.android.com/apk/res/android">
    <application>
        <meta-data android:name="com.google.android.geo.API_KEY"
                   android:value="YOUR_ANDROID_RESTRICTED_KEY" />
    </application>
</manifest>
```

Use your build's secret/configuration injection for the value. The demo uses Skip Showcase's
development sample key; the Day Showcase overlay remains empty until configured. Use your own
Android-restricted key for your application. Missing keys report `missing-api-key`; unavailable Google
Play services report `play-services-unavailable`. No account or key is needed for the bundled OpenStreetMap sources or
native Apple Maps. The map adapters do not request device location permission: add a user-location
marker using the separate Day location and permissions APIs if your application needs one.

See Google's [setup guide](https://developers.google.com/maps/documentation/android-sdk/config)
and [MapView lifecycle reference](https://developers.google.com/android/reference/com/google/android/gms/maps/MapView).

## Provider selection and extension

The [demo](https://github.com/daybrite/day-piece-map/blob/main/demo/src/lib.rs) and [Showcase Maps page](https://github.com/daybrite/Day-Showcase/blob/main/src/pages/map.rs)
use `switch` with a shared `MapView`: OpenStreetMap/Apple Maps on iOS and macOS, OpenStreetMap/Google Maps on Android,
and OpenStreetMap elsewhere. The OpenStreetMap selection uses Day's canvas renderer with
OpenStreetMap-based sources; the same renderer also displays the demo's fictional sample island.
Provider-specific controls are shown only for their provider.
[`support()`](https://docs.rs/day-piece-map-apple/latest/day_piece_map_apple/fn.support.html) on the native crates reports whether their renderer was compiled. It does not verify
credentials or network access. Optional features are described by each provider's options.

A new provider depends on the base crate, accepts `MapView`, and exposes its own options and
operations. There is no registry or closed enum to extend in the base crate. Implement the
shared signals directly; keep service-specific search, routing, offline management, and platform
annotation customization in provider crates or separate service parts.

The Apple and Google adapters enable an internal `native-bridge` feature. Its hidden module is
not an application extension API. It binds separate typed camera, marker, overlay, interaction,
option, and camera-command patches; only the native FFI boundary encodes each patch as JSON.
Native camera reports update the shared signals without becoming returning camera commands.
Annotation and option changes do not reapply an older camera.

Google forwards the mounted view's Activity lifecycle and memory-pressure events. Remounts use
shared `MapView` state; SDK bundles are not persisted across Activity recreation or process death.
Apps needing process-death restoration must persist and restore their own map state.

A future HarmonyOS adapter can wrap `@kit.MapKit`'s `MapComponent` and
`MapComponentController`. That Huawei service must be distinguished from the OpenHarmony
platform and its location APIs; support cannot be inferred from `target_env = "ohos"` alone.
See [HarmonyOS provider research](https://github.com/daybrite/day-piece-map/blob/main/docs/harmonyos.md) for dependencies, capabilities, coordinate
systems, and proposed integration boundaries. The canvas renderer remains the available implementation there.


For device location, use [Location](/docs/reference/location) and
[Permissions](/docs/reference/permissions). For reactive provider selection, use the
[Piece composition API](/docs/explanation/pieces).
