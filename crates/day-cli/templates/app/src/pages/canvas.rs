use day::prelude::*;

/// A reactive canvas dial (https://daybrite.dev/docs/canvas): the draw closure reads the
/// slider's signal, so dragging the slider redraws the dial — no invalidation calls. The
/// display list replays natively on every backend.
pub(crate) fn canvas_page() -> AnyPiece {
    let level = Signal::new(40.0f64);
    column((
        label(crate::res::str::canvas_title())
            .font(Font::Title)
            .id("canvas-title"),
        label(crate::res::str::canvas_blurb()),
        slider(level).range(0.0..=100.0).id("canvas-slider"),
        canvas(move |d, size| {
            if size.width <= 0.0 {
                return;
            }
            let r = Rect::from_size(size).inset(8.0);
            let track = Color::rgba(0.5, 0.5, 0.55, 0.35);
            let accent = Color::hex(0x2F6FDE);
            d.stroke(
                Shape::Arc {
                    rect: r,
                    start_deg: 135.0,
                    sweep_deg: 270.0,
                },
                track,
                6.0,
            );
            let frac = (level.get() / 100.0).clamp(0.0, 1.0);
            if frac > 0.0 {
                d.stroke(
                    Shape::Arc {
                        rect: r,
                        start_deg: 135.0,
                        sweep_deg: 270.0 * frac,
                    },
                    accent,
                    6.0,
                );
            }
            d.text(
                &format!("{:.0}", level.get()),
                Point::new(size.width / 2.0, size.height / 2.0),
                TextStyle {
                    size: 22.0,
                    color: accent,
                    anchor: TextAnchor::Centered,
                },
            );
        })
        .id("canvas-dial")
        .frame(140.0, 140.0),
    ))
    .spacing(12.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .any()
}
