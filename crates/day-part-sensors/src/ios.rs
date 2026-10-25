// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

use super::{Calibration, Sensor, SensorKind, SensorReading, lock, push};
use block2::RcBlock;
use objc2::rc::Retained;
use objc2_core_motion::{
    CMAccelerometerData, CMAttitudeReferenceFrame, CMDeviceMotion, CMGyroData,
    CMMagneticFieldCalibrationAccuracy, CMMagnetometerData, CMMotionManager,
};
use objc2_foundation::{NSError, NSOperationQueue};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

// One CoreMotion manager per app. Lifecycle calls are serialized; callbacks never lock it.
// Calibrated gyro and magnetic field share device motion, independently of the raw streams.
struct Manager {
    native: Retained<CMMotionManager>,
    queue: Retained<NSOperationQueue>,
    motion_generations: [Option<u32>; 2],
    motion_active: Option<Arc<AtomicBool>>,
}
unsafe impl Send for Manager {}
fn manager() -> &'static Mutex<Manager> {
    static MANAGER: OnceLock<Mutex<Manager>> = OnceLock::new();
    MANAGER.get_or_init(|| {
        let queue = NSOperationQueue::new();
        queue.setMaxConcurrentOperationCount(1);
        Mutex::new(Manager {
            native: unsafe { CMMotionManager::new() },
            queue,
            motion_generations: [None; 2],
            motion_active: None,
        })
    })
}
pub fn is_available(sensor: Sensor) -> bool {
    let m = lock(manager());
    unsafe {
        match (sensor.kind, sensor.calibration) {
            (SensorKind::Accelerometer, Calibration::Calibrated) => {
                m.native.isAccelerometerAvailable()
            }
            // CoreMotion exposes no selectable accelerometer bias-correction mode.
            (SensorKind::Accelerometer, Calibration::Uncalibrated) => false,
            (SensorKind::Gyroscope, Calibration::Uncalibrated) => m.native.isGyroAvailable(),
            (SensorKind::Gyroscope, Calibration::Calibrated) => m.native.isDeviceMotionAvailable(),
            (SensorKind::Magnetometer, Calibration::Uncalibrated) => {
                m.native.isMagnetometerAvailable()
            }
            (SensorKind::Magnetometer, Calibration::Calibrated) => {
                m.native.isMagnetometerAvailable()
                    && m.native.isDeviceMotionAvailable()
                    && CMMotionManager::availableAttitudeReferenceFrames()
                        .contains(CMAttitudeReferenceFrame::XArbitraryCorrectedZVertical)
            }
        }
    }
}

fn motion_slot(sensor: Sensor) -> Option<usize> {
    match (sensor.kind, sensor.calibration) {
        (SensorKind::Gyroscope, Calibration::Calibrated) => Some(0),
        (SensorKind::Magnetometer, Calibration::Calibrated) => Some(1),
        _ => None,
    }
}

// Restart with an immutable snapshot of subscribers. Old callbacks retain their original
// generations, and their cancellation flag is cleared before stopping the native stream.
// Starting/stopping the compass must not stop a subscribed calibrated gyroscope.
fn refresh_motion(m: &mut Manager) {
    unsafe {
        if let Some(active) = m.motion_active.take() {
            active.store(false, Ordering::Release);
            m.native.stopDeviceMotionUpdates();
        }
        let [gyro, magnet] = m.motion_generations;
        if gyro.is_none() && magnet.is_none() {
            return;
        }
        let active = Arc::new(AtomicBool::new(true));
        m.motion_active = Some(active.clone());
        m.native.setDeviceMotionUpdateInterval(0.05);
        let block = RcBlock::new(move |data: *mut CMDeviceMotion, _: *mut NSError| {
            if !active.load(Ordering::Acquire) {
                return;
            }
            if let Some(data) = data.as_ref() {
                if let Some(generation) = gyro {
                    let r = data.rotationRate();
                    push(
                        SensorKind::Gyroscope.into(),
                        generation,
                        SensorReading {
                            x: r.x,
                            y: r.y,
                            z: r.z,
                        },
                    );
                }
                if let Some(generation) = magnet {
                    let r = data.magneticField();
                    if r.accuracy != CMMagneticFieldCalibrationAccuracy::Uncalibrated {
                        push(
                            SensorKind::Magnetometer.into(),
                            generation,
                            SensorReading {
                                x: r.field.x,
                                y: r.field.y,
                                z: r.field.z,
                            },
                        );
                    }
                }
            }
        });
        // Gyro bias correction does not require a magnetometer. Enable magnetic calibration
        // only when requested, retaining device-relative axes and µT for the resulting field.
        let frame = if magnet.is_some() {
            CMAttitudeReferenceFrame::XArbitraryCorrectedZVertical
        } else {
            CMAttitudeReferenceFrame::XArbitraryZVertical
        };
        m.native
            .startDeviceMotionUpdatesUsingReferenceFrame_toQueue_withHandler(
                frame,
                &m.queue,
                RcBlock::as_ptr(&block),
            );
    }
}

pub struct Subscription(Sensor);
impl Drop for Subscription {
    fn drop(&mut self) {
        let mut m = lock(manager());
        if let Some(slot) = motion_slot(self.0) {
            m.motion_generations[slot] = None;
            refresh_motion(&mut m);
        } else {
            unsafe {
                match self.0.kind {
                    SensorKind::Accelerometer => m.native.stopAccelerometerUpdates(),
                    SensorKind::Gyroscope => m.native.stopGyroUpdates(),
                    SensorKind::Magnetometer => m.native.stopMagnetometerUpdates(),
                }
            }
        }
    }
}
pub fn start(sensor: Sensor, generation: u32) -> Option<Subscription> {
    if !is_available(sensor) {
        return None;
    }
    let mut m = lock(manager());
    if let Some(slot) = motion_slot(sensor) {
        m.motion_generations[slot] = Some(generation);
        refresh_motion(&mut m);
        return Some(Subscription(sensor));
    }
    // Raw handlers copy their sample; user code runs on our dispatcher, not the FFI stack.
    unsafe {
        match sensor.kind {
            SensorKind::Accelerometer => {
                m.native.setAccelerometerUpdateInterval(0.05);
                let block = RcBlock::new(move |data: *mut CMAccelerometerData, _: *mut NSError| {
                    if let Some(data) = data.as_ref() {
                        let a = data.acceleration();
                        push(
                            sensor,
                            generation,
                            SensorReading {
                                x: a.x * 9.80665,
                                y: a.y * 9.80665,
                                z: a.z * 9.80665,
                            },
                        );
                    }
                });
                m.native.startAccelerometerUpdatesToQueue_withHandler(
                    &m.queue,
                    RcBlock::as_ptr(&block),
                );
            }
            SensorKind::Gyroscope => {
                m.native.setGyroUpdateInterval(0.05);
                let block = RcBlock::new(move |data: *mut CMGyroData, _: *mut NSError| {
                    if let Some(data) = data.as_ref() {
                        let a = data.rotationRate();
                        push(
                            sensor,
                            generation,
                            SensorReading {
                                x: a.x,
                                y: a.y,
                                z: a.z,
                            },
                        );
                    }
                });
                m.native
                    .startGyroUpdatesToQueue_withHandler(&m.queue, RcBlock::as_ptr(&block));
            }
            SensorKind::Magnetometer => {
                m.native.setMagnetometerUpdateInterval(0.05);
                let block = RcBlock::new(move |data: *mut CMMagnetometerData, _: *mut NSError| {
                    if let Some(data) = data.as_ref() {
                        let a = data.magneticField();
                        push(
                            sensor,
                            generation,
                            SensorReading {
                                x: a.x,
                                y: a.y,
                                z: a.z,
                            },
                        );
                    }
                });
                m.native
                    .startMagnetometerUpdatesToQueue_withHandler(&m.queue, RcBlock::as_ptr(&block));
            }
        }
    }
    Some(Subscription(sensor))
}
