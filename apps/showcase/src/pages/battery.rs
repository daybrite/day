use day::prelude::*;

use crate::widgets::battery_line;

/// Battery playground (docs/battery.md): the headless `day-part-battery` part feeds a canvas-drawn
/// battery visualization — level fill colored by charge band, a bolt when charging. The preview
/// slider + toggle drive arbitrary states; "Read Device Battery" snaps back to the real reading.
pub(crate) fn battery_page() -> AnyPiece {
    // Seed the preview signals from the device's real reading (a demo value when there's none).
    let status = day_part_battery::status();
    let level = Signal::new(
        status
            .and_then(|b| b.percent())
            .map(f64::from)
            .unwrap_or(80.0),
    );
    let charging = Signal::new(status.map(|b| b.is_charging()).unwrap_or(false));
    let reading = Signal::new(battery_line().format());
    column((
        label(tr("nav-battery"))
            .font(Font::Title)
            .id("battery-title"),
        label(tr("battery-caption")),
        battery_view(level, charging),
        row((
            button(tr("battery-refresh"))
                .action(move || {
                    reading.set(battery_line().format());
                    if let Some(b) = day_part_battery::status() {
                        if let Some(p) = b.percent() {
                            level.set(f64::from(p));
                        }
                        charging.set(b.is_charging());
                    }
                })
                .id("battery-refresh"),
            label(move || reading.get()).id("battery-reading"),
        ))
        .spacing(8.0),
        divider(),
        // Preview controls: explore the visualization at any level / charge state.
        label(tr("battery-preview")).font(Font::Headline),
        row((
            label(tr("battery-level")),
            slider(level).range(0.0..=100.0).id("battery-level"),
            label(move || format!("{:.0}%", level.get())).id("battery-level-value"),
        ))
        .spacing(8.0),
        row((
            label(tr("battery-charging")),
            toggle(charging).id("battery-charging"),
        ))
        .spacing(8.0),
    ))
    .spacing(10.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .any()
}

/// Draw a battery on a canvas: rounded body + terminal nub, a level fill colored by band
/// (red < 20% ≤ amber < 50% ≤ green), a lightning bolt when charging, and a percent caption.
fn battery_view(level: Signal<f64>, charging: Signal<bool>) -> AnyPiece {
    canvas(move |d, size| {
        if size.width <= 0.0 || size.height <= 0.0 {
            return;
        }
        let frac = (level.get() / 100.0).clamp(0.0, 1.0);
        let band = if frac < 0.2 {
            Color::hex(0xFF3B30) // red
        } else if frac < 0.5 {
            Color::hex(0xFF9F0A) // amber
        } else {
            Color::hex(0x34C759) // green
        };
        let outline = Color::rgba(0.55, 0.55, 0.6, 0.9);

        // Geometry: the body fills the canvas minus the nub on the right and a caption strip below.
        let caption_h = 26.0;
        let nub_w = (size.width * 0.05).clamp(6.0, 14.0);
        let body = Rect::new(
            2.0,
            2.0,
            size.width - nub_w - 6.0,
            size.height - caption_h - 4.0,
        );
        let nub_h = body.size.height * 0.4;
        let nub = Rect::new(
            body.max_x() + 2.0,
            body.center().y - nub_h / 2.0,
            nub_w,
            nub_h,
        );
        d.stroke(Shape::RoundedRect(body, 12.0), outline, 3.0);
        d.fill(Shape::RoundedRect(nub, 3.0), outline);

        // The charge fill, inset within the body and clipped to the level fraction.
        let well = body.inset(6.0);
        let fill_w = well.size.width * frac;
        if fill_w > 0.5 {
            let fill_rect = Rect::new(well.min_x(), well.min_y(), fill_w, well.size.height);
            d.fill(
                Shape::RoundedRect(fill_rect, 7.0_f64.min(fill_w / 2.0)),
                band,
            );
        }

        // Charging: a lightning bolt centered in the body (white with a dark edge, so it reads on
        // both the colored fill and the empty well).
        if charging.get() {
            let c = body.center();
            let (bw, bh) = (body.size.height * 0.42, body.size.height * 0.72);
            let p =
                |rx: f64, ry: f64| Point::new(c.x - bw / 2.0 + rx * bw, c.y - bh / 2.0 + ry * bh);
            let bolt = vec![
                p(0.62, 0.0),
                p(0.0, 0.58),
                p(0.42, 0.58),
                p(0.38, 1.0),
                p(1.0, 0.42),
                p(0.58, 0.42),
            ];
            d.fill(
                Shape::Polygon(bolt.clone()),
                Color::rgba(1.0, 1.0, 1.0, 0.95),
            );
            d.stroke(Shape::Polygon(bolt), Color::rgba(0.0, 0.0, 0.0, 0.35), 1.5);
        }

        // Percent caption below the battery, in the band color.
        d.text(
            &format!("{:.0}%", level.get()),
            Point::new(size.width / 2.0, size.height - caption_h / 2.0),
            TextStyle {
                size: 16.0,
                color: band,
                anchor: TextAnchor::Centered,
            },
        );
    })
    // Accessibility (§13): like the gauge, the canvas gets an explicit Meter role + spoken
    // label/value (value is a build-time snapshot; reactive a11y is a follow-up).
    .a11y(move |a| {
        a.role(Role::Meter)
            .label(tr("nav-battery").format())
            .value(format!("{:.0}%", level.get_untracked()))
    })
    .id("battery")
    .frame(260.0, 120.0)
}
