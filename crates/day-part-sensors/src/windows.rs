// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

use super::{Calibration, Sensor, SensorKind, SensorReading, push};
use windows::Devices::Sensors::*;
use windows::Foundation::TypedEventHandler;
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize};

struct Apartment(bool);
impl Apartment {
    fn new() -> Self {
        Self(unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.is_ok())
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        if self.0 {
            unsafe { RoUninitialize() };
        }
    }
}
pub fn is_available(kind: Sensor) -> bool {
    if kind.calibration == Calibration::Uncalibrated {
        return false;
    }
    let _apartment = Apartment::new();
    match kind.kind {
        SensorKind::Accelerometer => Accelerometer::GetDefault().is_ok(),
        SensorKind::Gyroscope => Gyrometer::GetDefault().is_ok(),
        SensorKind::Magnetometer => Magnetometer::GetDefault().is_ok(),
    }
}
pub struct Subscription {
    stop: Option<Box<dyn FnOnce()>>,
    _apartment: Apartment,
}
impl Drop for Subscription {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            stop();
        }
    }
}
pub fn start(kind: Sensor, generation: u32) -> Option<Subscription> {
    if kind.calibration == Calibration::Uncalibrated {
        return None;
    }
    let apartment = Apartment::new();
    macro_rules! subscribe {
        ($sensor:ty, $args:ty, $x:ident, $y:ident, $z:ident, $scale:expr) => {{
            let sensor = <$sensor>::GetDefault().ok()?;
            sensor
                .SetReportInterval(sensor.MinimumReportInterval().ok()?.max(50))
                .ok()?;
            let handler = TypedEventHandler::<$sensor, $args>::new(move |_, args| {
                if let Some(args) = args.as_ref() {
                    let r = args.Reading()?;
                    push(
                        kind,
                        generation,
                        SensorReading {
                            x: f64::from(r.$x()?) * $scale,
                            y: f64::from(r.$y()?) * $scale,
                            z: f64::from(r.$z()?) * $scale,
                        },
                    );
                }
                Ok(())
            });
            let token = match sensor.ReadingChanged(&handler) {
                Ok(token) => token,
                Err(_) => {
                    let _ = sensor.SetReportInterval(0);
                    return None;
                }
            };
            Some(Subscription {
                stop: Some(Box::new(move || {
                    let _ = sensor.RemoveReadingChanged(token);
                    let _ = sensor.SetReportInterval(0);
                })),
                _apartment: apartment,
            })
        }};
    }
    match kind.kind {
        SensorKind::Accelerometer => subscribe!(
            Accelerometer,
            AccelerometerReadingChangedEventArgs,
            AccelerationX,
            AccelerationY,
            AccelerationZ,
            9.80665
        ),
        SensorKind::Gyroscope => subscribe!(
            Gyrometer,
            GyrometerReadingChangedEventArgs,
            AngularVelocityX,
            AngularVelocityY,
            AngularVelocityZ,
            std::f64::consts::PI / 180.0
        ),
        SensorKind::Magnetometer => subscribe!(
            Magnetometer,
            MagnetometerReadingChangedEventArgs,
            MagneticFieldX,
            MagneticFieldY,
            MagneticFieldZ,
            1.0
        ),
    }
}
