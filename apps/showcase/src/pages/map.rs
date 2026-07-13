use day::prelude::*;
use day_piece_map::map;

#[cfg(any(target_os = "macos", target_os = "ios"))]
/// A native map view (day-piece-map, an EXTERNAL standalone piece) — Apple platforms only. Preset
/// buttons recenter the map live via a bound coordinate `Signal` (a `Center` patch to the native
/// `MKMapView`). The map fills its `.frame`, and a marker pins the initial San Francisco center.
pub(crate) fn map_page() -> AnyPiece {
    const SF: (f64, f64) = (37.7749, -122.4194);
    const NYC: (f64, f64) = (40.7128, -74.0060);
    let center = Signal::new(SF);
    column((
        label(tr("nav-map")).font(Font::Title).id("map-title"),
        label(tr("map-caption")).id("map-caption"),
        row((
            button(tr("map-sf"))
                .bordered()
                .action(move || center.set(SF))
                .id("map-sf"),
            button(tr("map-nyc"))
                .bordered()
                .action(move || center.set(NYC))
                .id("map-nyc"),
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
            .marker(SF.0, SF.1)
            .id("map")
            .grow(),
    ))
    .spacing(12.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .grow()
}
