// Windows: stub for now — most PCs have no motion sensors anyway. The real implementation would be
// Windows.Devices.Sensors (Accelerometer/Gyrometer/Magnetometer::GetDefault() + GetCurrentReading(),
// genuinely poll-based, no XAML needed) via the `windows` crate's Devices_Sensors feature — a new
// dependency nothing in the workspace uses today, deferred like the winui backend itself.

use super::{SensorKind, SensorReading};

pub fn is_available(_kind: SensorKind) -> bool {
    false
}

pub fn read(_kind: SensorKind) -> Option<SensorReading> {
    None
}
