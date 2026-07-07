// Windows: Windows.Devices.Sensors (WinRT) via the `windows` crate. `GetDefault()` +
// `GetCurrentReading()` is a synchronous poll — no XAML, no event subscription — so it fits the
// crate's poll API directly. Most desktops / CI runners have no motion hardware, so `GetDefault`
// returns null and reads yield `None`; real coverage is tablets / 2-in-1 laptops with an
// accelerometer. Units are normalized to SI: the accelerometer reports g's (→ m/s²), the gyrometer
// degrees/second (→ rad/s), the magnetometer µT (already SI).

use windows::Devices::Sensors::{Accelerometer, Gyrometer, Magnetometer};

use super::{SensorKind, SensorReading};

const G: f64 = 9.806_65; // standard gravity — AccelerometerReading is in g
const DEG_TO_RAD: f64 = std::f64::consts::PI / 180.0; // GyrometerReading is in degrees/second

pub fn is_available(kind: SensorKind) -> bool {
    read(kind).is_some()
}

pub fn read(kind: SensorKind) -> Option<SensorReading> {
    // A missing sensor makes `GetDefault` yield a null object whose `GetCurrentReading` errors, so
    // every `.ok()?` naturally collapses to `None` — no explicit null check needed.
    match kind {
        SensorKind::Accelerometer => {
            let r = Accelerometer::GetDefault().ok()?.GetCurrentReading().ok()?;
            Some(SensorReading {
                x: r.AccelerationX().ok()? * G,
                y: r.AccelerationY().ok()? * G,
                z: r.AccelerationZ().ok()? * G,
            })
        }
        SensorKind::Gyroscope => {
            let r = Gyrometer::GetDefault().ok()?.GetCurrentReading().ok()?;
            Some(SensorReading {
                x: r.AngularVelocityX().ok()? * DEG_TO_RAD,
                y: r.AngularVelocityY().ok()? * DEG_TO_RAD,
                z: r.AngularVelocityZ().ok()? * DEG_TO_RAD,
            })
        }
        SensorKind::Magnetometer => {
            let r = Magnetometer::GetDefault().ok()?.GetCurrentReading().ok()?;
            Some(SensorReading {
                x: r.MagneticFieldX().ok()? as f64,
                y: r.MagneticFieldY().ok()? as f64,
                z: r.MagneticFieldZ().ok()? as f64,
            })
        }
    }
}
