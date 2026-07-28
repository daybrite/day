use day::prelude::*;

use crate::widgets::{battery_line, page};

/// Device & sensors (docs/battery.md, docs/sensors.md, docs/network.md): every headless
/// device-state part in one grouped form — the battery visualization with preview controls,
/// the connectivity snapshot, the motion sensors, and the device identity. Each group is a
/// form `section`; the readout rows are `labeled`, so their labels align form-wide.
pub(crate) fn system_page() -> AnyPiece {
    page(
        crate::res::str::nav_system(),
        "system-title",
        Some(crate::res::str::system_caption()),
        form((
            battery_section(),
            network_section(),
            sensors_section(),
            location_section(),
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
            crate::res::str::battery_level(),
            row((
                slider(level).range(0.0..=100.0).id("battery-level"),
                label(move || format!("{:.0}%", level.get())).id("battery-level-value"),
            ))
            .spacing(8.0),
        ),
        labeled(
            crate::res::str::battery_charging(),
            toggle(charging).id("battery-charging"),
        ),
        row((
            button(crate::res::str::battery_refresh())
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
    .title(crate::res::str::nav_battery())
}

fn network_section() -> impl Piece {
    let reading = Signal::new(network_line().format());
    section((row((
        button(crate::res::str::network_refresh())
            .bordered()
            .action(move || reading.set(network_line().format()))
            .id("network-refresh"),
        label(move || reading.get()).id("network-reading"),
    ))
    .spacing(8.0),))
    .title(crate::res::str::nav_network())
}

/// Live sensor readouts (docs/sensors.md). Each row subscribes with `day_part_sensors::watch`, whose
/// samples arrive on a background thread — so the value crosses to the UI through a `Setter`, the
/// standard idiom (DESIGN §4.5). The subscriptions are tied to this page's scope: leaving the page
/// drops the `Watch` handles and the platform stops sampling.
fn sensors_section() -> impl Piece {
    use day_part_sensors::SensorKind;

    /// One row's text: a reading, "waiting" while the sensor exists but has not reported yet, or
    /// "unavailable" (each branch a full `tr(...)` so `day lint` sees the key).
    fn line(
        reading: Option<day_part_sensors::SensorReading>,
        kind: SensorKind,
        unit: &str,
    ) -> String {
        match reading {
            Some(r) => crate::res::str::sensor_reading(
                unit,
                format!("{:+.2}", r.x),
                format!("{:+.2}", r.y),
                format!("{:+.2}", r.z),
            )
            .format(),
            None if day_part_sensors::is_available(kind) => {
                crate::res::str::sensor_waiting().format()
            }
            None => crate::res::str::sensor_unavailable().format(),
        }
    }

    // A signal per sensor, fed by its own subscription.
    let mut watches = Vec::new();
    let mut row = |kind: SensorKind, unit: &'static str| {
        let text = Signal::new(line(None, kind, unit));
        if day_part_sensors::is_available(kind) {
            let set = text.setter();
            watches.push(day_part_sensors::watch(kind, move |r| {
                set.set(line(Some(r), kind, unit))
            }));
        }
        text
    };
    let accel = row(SensorKind::Accelerometer, "m/s²");
    let gyro = row(SensorKind::Gyroscope, "rad/s");
    let magnet = row(SensorKind::Magnetometer, "µT");

    // A rolling history per sensor, for the strip charts. The same subscription feeds both the
    // readout and the chart — one stream, two views.
    let mut history = |kind: SensorKind| {
        let series = Signal::new(Vec::<day_part_sensors::SensorReading>::new());
        if day_part_sensors::is_available(kind) {
            let set = series.setter();
            let buf = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            watches.push(day_part_sensors::watch(kind, move |r| {
                let snapshot = {
                    let Ok(mut b) = buf.lock() else { return };
                    b.push(r);
                    if b.len() > CHART_SAMPLES {
                        b.remove(0);
                    }
                    b.clone()
                };
                set.set(snapshot);
            }));
        }
        series
    };
    let accel_series = history(SensorKind::Accelerometer);
    let gyro_series = history(SensorKind::Gyroscope);
    let magnet_series = history(SensorKind::Magnetometer);

    // Holding the handles until the page goes away is what keeps the streams alive — and dropping
    // them here is what stops the platform sampling when it doesn't.
    Scope::current().on_cleanup(move || drop(watches));

    section((
        permission_row(
            crate::res::str::sensor_permission(),
            day_part_permissions::Permission::Motion,
            "sensors-perm",
        ),
        labeled(
            crate::res::str::sensor_accelerometer(),
            label(move || accel.get()).id("sensor-accel"),
        ),
        when(
            move || day_part_sensors::is_available(SensorKind::Accelerometer),
            move || strip_chart(accel_series).id("sensor-accel-chart"),
        ),
        labeled(
            crate::res::str::sensor_gyroscope(),
            label(move || gyro.get()).id("sensor-gyro"),
        ),
        when(
            move || day_part_sensors::is_available(SensorKind::Gyroscope),
            move || strip_chart(gyro_series).id("sensor-gyro-chart"),
        ),
        labeled(
            crate::res::str::sensor_magnetometer(),
            label(move || magnet.get()).id("sensor-magnet"),
        ),
        when(
            move || day_part_sensors::is_available(SensorKind::Magnetometer),
            move || strip_chart(magnet_series).id("sensor-magnet-chart"),
        ),
        // The legend only means something next to a chart.
        when(
            move || {
                [
                    SensorKind::Accelerometer,
                    SensorKind::Gyroscope,
                    SensorKind::Magnetometer,
                ]
                .into_iter()
                .any(day_part_sensors::is_available)
            },
            move || {
                label(crate::res::str::chart_axes())
                    .font(Font::Footnote)
                    .id("sensor-chart-legend")
            },
        ),
    ))
    .title(crate::res::str::nav_sensors())
}

/// How many samples a strip chart keeps — at day-part-sensors' ~20 Hz, about six seconds.
const CHART_SAMPLES: usize = 120;

/// A live x/y/z strip chart: three polylines over a rolling window, newest on the right.
///
/// Unlike `battery_view` below, this deliberately does NOT mirror under a right-to-left locale.
/// A battery is a picture of an object and mirrors with the layout; a time series is a chart whose
/// x axis IS time, and time reads left-to-right in a chart regardless of the reading direction of
/// the surrounding text.
fn strip_chart(series: Signal<Vec<day_part_sensors::SensorReading>>) -> AnyPiece {
    canvas(move |d, size| {
        if size.width <= 2.0 || size.height <= 2.0 {
            return;
        }
        let samples = series.get();
        // A baseline is drawn even when empty, so the chart reads as "nothing yet" rather than as a
        // rendering failure.
        let mid = size.height / 2.0;
        d.stroke(
            Shape::Line(Point::new(0.0, mid), Point::new(size.width, mid)),
            Color::rgba(0.5, 0.5, 0.55, 0.35),
            1.0,
        );
        if samples.len() < 2 {
            return;
        }
        // Scale to the window's own peak so a gentle signal is still legible; the floor keeps a
        // near-still device from amplifying its noise to full height.
        let peak = samples
            .iter()
            .flat_map(|r| [r.x.abs(), r.y.abs(), r.z.abs()])
            .fold(1.0f64, f64::max);
        let dx = size.width / (CHART_SAMPLES.max(2) - 1) as f64;
        // Right-align the window so new samples enter at the right edge and scroll left.
        let x0 = size.width - dx * (samples.len() - 1) as f64;
        for (axis, color) in [
            (0usize, crate::palette::CORAL),
            (1, crate::palette::TEAL),
            (2, crate::palette::VIOLET),
        ] {
            for i in 1..samples.len() {
                let v = |r: &day_part_sensors::SensorReading| match axis {
                    0 => r.x,
                    1 => r.y,
                    _ => r.z,
                };
                let y = |r: &day_part_sensors::SensorReading| mid - (v(r) / peak) * (mid - 2.0);
                d.stroke(
                    Shape::Line(
                        Point::new(x0 + dx * (i - 1) as f64, y(&samples[i - 1])),
                        Point::new(x0 + dx * i as f64, y(&samples[i])),
                    ),
                    color,
                    1.5,
                );
            }
        }
    })
    .height(56.0)
    .any()
}

/// A permission row: its live status, and a button that either asks or opens Settings.
///
/// Worth being honest about what this demonstrates. Raw motion sensors need NO permission on iOS or
/// Android (docs/sensors.md), so on most targets this reads `ungated / granted` and the button is
/// hidden. The rows that really prompt are the web on iOS Safari, and HarmonyOS with its declared
/// `ohos.permission.ACCELEROMETER`.
fn permission_row(
    title: day::LocalizedText,
    perm: day_part_permissions::Permission,
    id: &'static str,
) -> impl Piece {
    use day_part_permissions as perms;
    let status = Signal::new(format!(
        "{} / {}",
        perms::gate(perm).label(),
        perms::status(perm).label()
    ));
    let refresh = move || {
        status.set(format!(
            "{} / {}",
            perms::gate(perm).label(),
            perms::status(perm).label()
        ))
    };
    // `can_prompt` decides which affordance to show: asking when the answer is already final would
    // do nothing, and Settings is the only remedy then.
    let can_prompt = perms::can_prompt(perm);
    // Where the platform has no such gate at all, neither asking nor Settings can change anything —
    // so offer no affordance rather than a button that does nothing.
    let gated = perms::gate(perm) != perms::Gate::Absent;
    let action_label = if can_prompt {
        crate::res::str::perm_request()
    } else {
        crate::res::str::perm_open_settings()
    };
    labeled(
        title,
        row((
            label(move || status.get()).id(id),
            when(
                move || gated,
                move || {
                    button(action_label.clone())
                        .bordered()
                        .action(move || {
                            if can_prompt {
                                // The completion runs on an unspecified thread — a Setter crosses back.
                                let set = status.setter();
                                perms::request(perm, move |s| {
                                    set.set(format!(
                                        "{} / {}",
                                        perms::gate(perm).label(),
                                        s.label()
                                    ))
                                });
                            } else {
                                perms::open_settings(perm);
                                refresh();
                            }
                        })
                        .id(format!("{id}-action"))
                },
            ),
        ))
        .spacing(8.0),
    )
}

/// Live location (docs/location.md): a permission row, a Start/Stop toggle, and the fix itself.
///
/// The subscription is held in the page's scope, so leaving the page stops the GPS rather than
/// leaving it warm behind a closed screen.
fn location_section() -> impl Piece {
    use day_part_location::{Accuracy, LocationError};

    let coords = Signal::new(if day_part_location::is_available() {
        crate::res::str::location_waiting().format()
    } else {
        crate::res::str::location_unavailable().format()
    });
    let altitude = Signal::new(crate::res::str::location_unknown().format());
    let accuracy = Signal::new(crate::res::str::location_unknown().format());
    let running = Signal::new(false);
    // `Rc<RefCell<…>>` because the handle is created in a button action and dropped in another —
    // both on the UI thread, so no lock is needed.
    let watch: std::rc::Rc<std::cell::RefCell<Option<day_part_location::Watch>>> =
        std::rc::Rc::new(std::cell::RefCell::new(None));
    let held = watch.clone();
    Scope::current().on_cleanup(move || {
        held.borrow_mut().take();
    });

    section((
        permission_row(
            crate::res::str::location_permission(),
            day_part_permissions::Permission::Location,
            "location-perm",
        ),
        labeled(
            crate::res::str::nav_location(),
            label(move || coords.get()).id("location-coords"),
        ),
        labeled(
            crate::res::str::location_altitude(),
            label(move || altitude.get()).id("location-altitude"),
        ),
        labeled(
            crate::res::str::location_accuracy(),
            label(move || accuracy.get()).id("location-accuracy"),
        ),
        button(move || {
            if running.get() {
                crate::res::str::location_stop().format()
            } else {
                crate::res::str::location_start().format()
            }
        })
        .bordered()
        .action(move || {
            if running.get() {
                watch.borrow_mut().take();
                running.set(false);
                return;
            }
            let (set_coords, set_alt, set_acc) =
                (coords.setter(), altitude.setter(), accuracy.setter());
            let handle = day_part_location::watch(Accuracy::Balanced, move |fix| match fix {
                Ok(f) => {
                    set_coords.set(
                        crate::res::str::location_coords(
                            format!("{:.5}", f.latitude),
                            format!("{:.5}", f.longitude),
                        )
                        .format(),
                    );
                    set_alt.set(f.altitude.map_or_else(
                        || crate::res::str::location_unknown().format(),
                        |a| format!("{a:.0} m"),
                    ));
                    set_acc.set(f.accuracy_m.map_or_else(
                        || crate::res::str::location_unknown().format(),
                        |a| format!("±{a:.0} m"),
                    ));
                }
                // A denial is the interesting error: it is what the permission row above is for.
                Err(LocationError::PermissionDenied) => {
                    set_coords.set(crate::res::str::location_permission().format())
                }
                Err(e) => set_coords.set(e.to_string()),
            });
            *watch.borrow_mut() = Some(handle);
            running.set(true);
        })
        .id("location-toggle"),
    ))
    .title(crate::res::str::nav_location())
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
        button(crate::res::str::deviceinfo_refresh())
            .bordered()
            .action(move || {
                let (m, s, sim) = deviceinfo_lines();
                model.set(m);
                system.set(s);
                simulator.set(sim);
            })
            .id("deviceinfo-refresh"),
    ))
    .title(crate::res::str::nav_deviceinfo())
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
            .label(crate::res::str::nav_battery().format())
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
            if n.online {
                crate::res::str::network_reading_online(
                    match n.expensive {
                        Some(true) => "yes",
                        Some(false) => "no",
                        None => "?",
                    },
                    format!("{:?}", n.kind),
                )
            } else {
                crate::res::str::network_reading_offline()
            }
        }
        None => crate::res::str::network_reading_none(),
    }
}

/// Read the native device identity and format each field as a localized line:
/// `(model, "name version", simulator)`. Values vary by host, so nothing is asserted exactly.
fn deviceinfo_lines() -> (String, String, String) {
    let d = day_part_deviceinfo::get();
    let model = crate::res::str::deviceinfo_model(d.model).format();
    let system = crate::res::str::deviceinfo_system(d.system_name, d.system_version).format();
    // Each branch is a full literal tr(...) call so `day lint` sees both keys (never tr(if ...)).
    let sim_value = if d.is_simulator {
        crate::res::str::deviceinfo_yes().format()
    } else {
        crate::res::str::deviceinfo_no().format()
    };
    let simulator = crate::res::str::deviceinfo_simulator(sim_value).format();
    (model, system, simulator)
}
