// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The day-dom host forwards DeviceMotionEvent values directly. Safari motion permission must
//! be requested from a user gesture through day-part-permissions before samples can arrive.
use super::{Calibration, Sensor, SensorReading, push};
#[link(wasm_import_module = "env")]
unsafe extern "C" {
    fn day_dom_sensor_start(kind: u32, generation: u32);
    fn day_dom_sensor_stop(kind: u32, generation: u32);
    fn day_dom_sensor_available(kind: u32) -> u32;
}
pub fn start(kind: Sensor, generation: u32) {
    if !is_available(kind) {
        return;
    }
    unsafe { day_dom_sensor_start(kind.index() as u32, generation) };
}
pub fn stop(kind: Sensor, generation: u32) {
    if kind.calibration == Calibration::Uncalibrated {
        return;
    }
    unsafe { day_dom_sensor_stop(kind.index() as u32, generation) };
}
pub fn is_available(kind: Sensor) -> bool {
    kind.calibration == Calibration::Calibrated
        && unsafe { day_dom_sensor_available(kind.index() as u32) != 0 }
}
/// The host calls this only for a new measurement. Generations reject callbacks from old watches.
#[unsafe(no_mangle)]
pub extern "C" fn day_sensors_sample(kind: u32, generation: u32, x: f64, y: f64, z: f64) {
    if let Some(kind) = Sensor::from_code(kind) {
        push(kind, generation, SensorReading { x, y, z });
    }
}
