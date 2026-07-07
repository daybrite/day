//! `cargo run -p day-part-sensors --example sensors` — print the current motion-sensor readings.
//! Demonstrates that any Rust code can depend on this crate and use the API with no Day framework
//! at all. (On the mac host every kind is unavailable; try a Linux laptop with an iio accelerometer.)

use day_part_sensors::SensorKind;

fn main() {
    for (kind, unit) in [
        (SensorKind::Accelerometer, "m/s²"),
        (SensorKind::Gyroscope, "rad/s"),
        (SensorKind::Magnetometer, "µT"),
    ] {
        match day_part_sensors::read(kind) {
            Some(r) => println!("{kind:?}: x {:+.3} y {:+.3} z {:+.3} {unit}", r.x, r.y, r.z),
            None => println!(
                "{kind:?}: {}",
                if day_part_sensors::is_available(kind) {
                    "no sample yet"
                } else {
                    "unavailable"
                }
            ),
        }
    }
}
