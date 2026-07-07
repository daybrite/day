// iOS: CoreMotion's CMMotionManager is natively poll-friendly — start updates once, then read the
// `accelerometerData`/`gyroData`/`magnetometerData` properties (None until the first sample). Apple
// recommends a single CMMotionManager per app, so one is kept in a static for the process lifetime.
// Unlike UIDevice (battery), CMMotionManager is NOT MainThreadOnly, so no main-thread gate is needed;
// a Mutex serializes access instead. Raw CMMotionManager needs no Info.plist usage key (that gates
// the Motion & Fitness APIs, not these). The Simulator has no sensors → unavailable → None.
// Accelerometer values arrive in g and are normalized to m/s² to match the other platforms.

use std::sync::{Mutex, OnceLock};

use objc2::rc::Retained;
use objc2_core_motion::CMMotionManager;

use super::{SensorKind, SensorReading};

/// Standard gravity, for the g → m/s² normalization.
const STANDARD_GRAVITY: f64 = 9.80665;

// Retained<CMMotionManager> is not Send only because objc2 can't prove it; CMMotionManager itself is
// documented safe to use off the main thread, and the Mutex serializes all access.
struct Manager(Retained<CMMotionManager>);
unsafe impl Send for Manager {}

fn manager() -> &'static Mutex<Manager> {
    static MANAGER: OnceLock<Mutex<Manager>> = OnceLock::new();
    MANAGER.get_or_init(|| Mutex::new(Manager(unsafe { CMMotionManager::new() })))
}

pub fn is_available(kind: SensorKind) -> bool {
    let Ok(m) = manager().lock() else {
        return false;
    };
    unsafe {
        match kind {
            SensorKind::Accelerometer => m.0.isAccelerometerAvailable(),
            SensorKind::Gyroscope => m.0.isGyroAvailable(),
            SensorKind::Magnetometer => m.0.isMagnetometerAvailable(),
        }
    }
}

pub fn read(kind: SensorKind) -> Option<SensorReading> {
    let m = manager().lock().ok()?;
    unsafe {
        match kind {
            SensorKind::Accelerometer => {
                if !m.0.isAccelerometerAvailable() {
                    return None;
                }
                if !m.0.isAccelerometerActive() {
                    m.0.startAccelerometerUpdates();
                }
                let a = m.0.accelerometerData()?.acceleration();
                Some(SensorReading {
                    x: a.x * STANDARD_GRAVITY,
                    y: a.y * STANDARD_GRAVITY,
                    z: a.z * STANDARD_GRAVITY,
                })
            }
            SensorKind::Gyroscope => {
                if !m.0.isGyroAvailable() {
                    return None;
                }
                if !m.0.isGyroActive() {
                    m.0.startGyroUpdates();
                }
                let r = m.0.gyroData()?.rotationRate();
                Some(SensorReading {
                    x: r.x,
                    y: r.y,
                    z: r.z,
                })
            }
            SensorKind::Magnetometer => {
                if !m.0.isMagnetometerAvailable() {
                    return None;
                }
                if !m.0.isMagnetometerActive() {
                    m.0.startMagnetometerUpdates();
                }
                let f = m.0.magnetometerData()?.magneticField();
                Some(SensorReading {
                    x: f.x,
                    y: f.y,
                    z: f.z,
                })
            }
        }
    }
}
