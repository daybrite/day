use day::prelude::*;
use day_piece_map::map;

#[cfg(any(target_os = "macos", target_os = "ios"))]
/// A native map view (day-piece-map, an EXTERNAL standalone piece) — Apple platforms only. Preset
/// buttons recenter the map live via a bound coordinate `Signal` (a `Center` patch to the native
/// `MKMapView`). The map fills its `.frame`, and a marker pins the initial Boston center.
pub(crate) fn map_page() -> AnyPiece {
    const BOSTON: (f64, f64) = (42.3601, -71.0589);
    const PARIS: (f64, f64) = (48.8566, 2.3522);
    let center = Signal::new(BOSTON);
    column((
        label(crate::res::str::nav_map())
            .font(Font::Title)
            .id("map-title"),
        label(crate::res::str::map_caption()).id("map-caption"),
        row((
            button(crate::res::str::map_boston())
                .bordered()
                .action(move || center.set(BOSTON))
                .id("map-boston"),
            button(crate::res::str::map_paris())
                .bordered()
                .action(move || center.set(PARIS))
                .id("map-paris"),
        ))
        .spacing(8.0),
        label(move || {
            let (lat, lon) = center.get();
            format!("{lat:.4}, {lon:.4}")
        })
        .id("map-coords"),
        map()
            .center_signal(center)
            .span(0.05)
            .marker(BOSTON.0, BOSTON.1)
            .id("map")
            .grow(),
    ))
    .spacing(12.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .grow()
}
