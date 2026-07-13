use day::prelude::*;

use crate::widgets::{battery_line, page};

/// Device & sensors (docs/battery.md, docs/sensors.md, docs/network.md): every headless
/// device-state part in one grouped form — the battery visualization with preview controls,
/// the connectivity snapshot, the motion sensors, and the device identity. Each group is a
/// form `section`; the readout rows are `labeled`, so their labels align form-wide.
pub(crate) fn system_page() -> AnyPiece {
    page(
        tr("nav-system"),
        "system-title",
        Some(tr("system-caption")),
        form((
            battery_section(),
            network_section(),
            sensors_section(),
            device_section(),
        ))
        .any(),
    )
}

fn battery_section() -> impl Piece {
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
    section((
        battery_view(level, charging),
        labeled(
            tr("battery-level"),
            row((
                slider(level).range(0.0..=100.0).id("battery-level"),
                label(move || format!("{:.0}%", level.get())).id("battery-level-value"),
            ))
            .spacing(8.0),
        ),
        labeled(
            tr("battery-charging"),
            toggle(charging).id("battery-charging"),
        ),
        row((
            button(tr("battery-refresh"))
                .bordered()
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
    ))
    .title(tr("nav-battery"))
}

fn network_section() -> impl Piece {
    let reading = Signal::new(network_line().format());
    section((row((
        button(tr("network-refresh"))
            .bordered()
            .action(move || reading.set(network_line().format()))
            .id("network-refresh"),
        label(move || reading.get()).id("network-reading"),
    ))
    .spacing(8.0),))
    .title(tr("nav-network"))
}

fn sensors_section() -> impl Piece {
    use day_part_sensors::SensorKind;
    fn sensor_line(kind: SensorKind, unit: &str) -> String {
        match day_part_sensors::read(kind) {
            Some(r) => tr("sensor-reading")
                .arg("x", format!("{:+.2}", r.x))
                .arg("y", format!("{:+.2}", r.y))
                .arg("z", format!("{:+.2}", r.z))
                .arg("unit", unit)
                .format(),
            None if day_part_sensors::is_available(kind) => tr("sensor-waiting").format(),
            None => tr("sensor-unavailable").format(),
        }
    }
    let accel = Signal::new(sensor_line(SensorKind::Accelerometer, "m/s²"));
    let gyro = Signal::new(sensor_line(SensorKind::Gyroscope, "rad/s"));
    let magnet = Signal::new(sensor_line(SensorKind::Magnetometer, "µT"));
    section((
        labeled(
            tr("sensor-accelerometer"),
            label(move || accel.get()).id("sensor-accel"),
        ),
        labeled(
            tr("sensor-gyroscope"),
            label(move || gyro.get()).id("sensor-gyro"),
        ),
        labeled(
            tr("sensor-magnetometer"),
            label(move || magnet.get()).id("sensor-magnet"),
        ),
        button(tr("sensors-refresh"))
            .bordered()
            .action(move || {
                accel.set(sensor_line(SensorKind::Accelerometer, "m/s²"));
                gyro.set(sensor_line(SensorKind::Gyroscope, "rad/s"));
                magnet.set(sensor_line(SensorKind::Magnetometer, "µT"));
            })
            .id("sensors-refresh"),
    ))
    .title(tr("nav-sensors"))
}

fn device_section() -> impl Piece {
    // Read the device identity once now (headless day-part-deviceinfo); Refresh re-polls it.
    let (m, s, sim) = deviceinfo_lines();
    let model = Signal::new(m);
    let system = Signal::new(s);
    let simulator = Signal::new(sim);
    section((
        label(move || model.get()).id("deviceinfo-model"),
        label(move || system.get()).id("deviceinfo-system"),
        label(move || simulator.get()).id("deviceinfo-simulator"),
        button(tr("deviceinfo-refresh"))
            .bordered()
            .action(move || {
                let (m, s, sim) = deviceinfo_lines();
                model.set(m);
                system.set(s);
                simulator.set(sim);
            })
            .id("deviceinfo-refresh"),
    ))
    .title(tr("nav-deviceinfo"))
}

/// Draw a battery on a canvas: rounded body + terminal nub, a level fill colored by band
/// (red < 20% ≤ amber < 50% ≤ green), a lightning bolt when charging, and a percent caption.
fn battery_view(level: Signal<f64>, charging: Signal<bool>) -> AnyPiece {
    canvas(move |d, size| {
        if size.width <= 0.0 || size.height <= 0.0 {
            return;
        }
        // RTL (docs/localization): the layout engine mirrors widget *placement*, but a canvas draws
        // in its own coordinate space, so this custom drawing mirrors itself. Under a right-to-left
        // locale (e.g. `ar`) the battery flips horizontally — terminal nub on the left, charge
        // draining from the right. `mx` mirrors an x, `mrect` a rect; both are the identity in LTR.
        let rtl = is_rtl();
        let mx = |x: f64| if rtl { size.width - x } else { x };
        let mrect = |r: Rect| {
            if rtl {
                Rect::new(
                    size.width - r.max_x(),
                    r.min_y(),
                    r.size.width,
                    r.size.height,
                )
            } else {
                r
            }
        };
        let frac = (level.get() / 100.0).clamp(0.0, 1.0);
        let band = if frac < 0.2 {
            Color::hex(0xFF3B30) // red
        } else if frac < 0.5 {
            Color::hex(0xFF9F0A) // amber
        } else {
            Color::hex(0x34C759) // green
        };
        let outline = Color::rgba(0.55, 0.55, 0.6, 0.9);

        // Geometry (defined LTR; mirrored at draw time via `mrect`/`mx`). The body fills the canvas
        // minus the terminal nub past its trailing edge and a caption strip below.
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
        d.stroke(Shape::RoundedRect(mrect(body), 12.0), outline, 3.0);
        d.fill(Shape::RoundedRect(mrect(nub), 3.0), outline);

        // The charge fill, inset within the body and clipped to the level fraction — it grows from
        // the leading edge, so under RTL `mrect` makes it drain from the right.
        let well = body.inset(6.0);
        let fill_w = well.size.width * frac;
        if fill_w > 0.5 {
            let fill_rect = Rect::new(well.min_x(), well.min_y(), fill_w, well.size.height);
            d.fill(
                Shape::RoundedRect(mrect(fill_rect), 7.0_f64.min(fill_w / 2.0)),
                band,
            );
        }

        // Charging: a lightning bolt centered in the body (white with a dark edge, so it reads on
        // both the colored fill and the empty well).
        if charging.get() {
            let c = body.center();
            let (bw, bh) = (body.size.height * 0.42, body.size.height * 0.72);
            let p = |rx: f64, ry: f64| {
                Point::new(mx(c.x - bw / 2.0 + rx * bw), c.y - bh / 2.0 + ry * bh)
            };
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

/// The current connectivity snapshot as a localized line (Fluent; kind stays the API's enum
/// debug form — it is a value, not prose).
fn network_line() -> LocalizedText {
    match day_part_network::status() {
        Some(n) => {
            let line = if n.online {
                tr("network-reading-online")
            } else {
                tr("network-reading-offline")
            };
            line.arg("kind", format!("{:?}", n.kind)).arg(
                "expensive",
                match n.expensive {
                    Some(true) => "yes",
                    Some(false) => "no",
                    None => "?",
                },
            )
        }
        None => tr("network-reading-none"),
    }
}

/// Read the native device identity and format each field as a localized line:
/// `(model, "name version", simulator)`. Values vary by host, so nothing is asserted exactly.
fn deviceinfo_lines() -> (String, String, String) {
    let d = day_part_deviceinfo::get();
    let model = tr("deviceinfo-model").arg("value", d.model).format();
    let system = tr("deviceinfo-system")
        .arg("name", d.system_name)
        .arg("version", d.system_version)
        .format();
    // Each branch is a full literal tr(...) call so `day lint` sees both keys (never tr(if ...)).
    let sim_value = if d.is_simulator {
        tr("deviceinfo-yes").format()
    } else {
        tr("deviceinfo-no").format()
    };
    let simulator = tr("deviceinfo-simulator").arg("value", sim_value).format();
    (model, system, simulator)
}
