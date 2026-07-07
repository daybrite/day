// macOS: no public motion-sensor API — the HDD-era Sudden Motion Sensor is gone and CoreMotion's
// CMMotionManager reports no sensors on desktop hardware — so everything is unavailable. A dedicated
// stub file keeps the per-OS symmetry (and the smoke test green on the mac host).

use super::{SensorKind, SensorReading};

pub fn is_available(_kind: SensorKind) -> bool {
    false
}

pub fn read(_kind: SensorKind) -> Option<SensorReading> {
    None
}
