// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! `cargo run -p day-part-sensors --example sensors` — stream the device's motion sensors for a few
//! seconds. Demonstrates that any Rust code can depend on this crate and use the API with no Day
//! framework at all. (On the mac host every kind is unavailable; try a Linux laptop with an iio
//! accelerometer, or a phone.)

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use day_part_sensors::SensorKind;

fn main() {
    let mut watches = Vec::new();
    for (kind, unit) in [
        (SensorKind::Accelerometer, "m/s²"),
        (SensorKind::Gyroscope, "rad/s"),
        (SensorKind::Magnetometer, "µT"),
    ] {
        if !day_part_sensors::is_available(kind) {
            println!("{kind:?}: unavailable on this device");
            continue;
        }
        let seen = Arc::new(AtomicUsize::new(0));
        let counter = seen.clone();
        // The handle stops delivery when dropped, so keep it for as long as you want samples.
        let watch = day_part_sensors::watch(kind, move |r| {
            // Print the first few of each kind, then just count — 20 Hz fills a terminal fast.
            if counter.fetch_add(1, Ordering::Relaxed) < 3 {
                println!("{kind:?}: x {:+.3} y {:+.3} z {:+.3} {unit}", r.x, r.y, r.z);
            }
        });
        watches.push((kind, seen, watch));
    }
    if watches.is_empty() {
        return;
    }
    std::thread::sleep(std::time::Duration::from_secs(3));
    for (kind, seen, _watch) in &watches {
        match seen.load(Ordering::Relaxed) {
            0 => println!("{kind:?}: available, but no sample arrived in 3s"),
            n => println!("{kind:?}: {n} samples in 3s"),
        }
    }
    // Dropping the watches here ends every subscription.
}
