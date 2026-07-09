use day::prelude::*;

/// Sensors playground (docs/sensors.md): the headless `day-part-sensors` part polls the device's
/// motion sensors natively. Sensors are push-model on Android/OHOS, so the first read arms the
/// listener — Refresh twice on a fresh launch; desktops/simulators report "unavailable".
pub(crate) fn sensors_page() -> AnyPiece {
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
    column((
        label(tr("nav-sensors"))
            .font(Font::Title)
            .id("sensors-title"),
        label(tr("sensors-caption")),
        row((
            label(tr("sensor-accelerometer")),
            label(move || accel.get()).id("sensor-accel"),
        ))
        .spacing(8.0),
        row((
            label(tr("sensor-gyroscope")),
            label(move || gyro.get()).id("sensor-gyro"),
        ))
        .spacing(8.0),
        row((
            label(tr("sensor-magnetometer")),
            label(move || magnet.get()).id("sensor-magnet"),
        ))
        .spacing(8.0),
        button(tr("sensors-refresh"))
            .action(move || {
                accel.set(sensor_line(SensorKind::Accelerometer, "m/s²"));
                gyro.set(sensor_line(SensorKind::Gyroscope, "rad/s"));
                magnet.set(sensor_line(SensorKind::Magnetometer, "µT"));
            })
            .id("sensors-refresh"),
    ))
    .spacing(10.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .any()
}
