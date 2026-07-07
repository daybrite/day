// Linux: the kernel's Industrial I/O subsystem exposes sensors under /sys/bus/iio/devices/ with
// per-channel `in_<chan>_{x,y,z}_raw` files plus `_scale`/`_offset` — value = (raw + offset) × scale
// (accelerometer m/s², gyroscope rad/s, magnetometer Gauss → ×100 for µT). Pure std, truly
// poll-based (sysfs reads are cheap), no caching or subscription needed. Most desktops/CI runners
// have no motion sensors → None; real coverage is laptops/tablets with rotation accelerometers.

use std::fs;
use std::path::{Path, PathBuf};

use super::{SensorKind, SensorReading};

/// The iio channel prefix for the kind, and the factor from post-scale units to our SI units.
fn channel(kind: SensorKind) -> (&'static str, f64) {
    match kind {
        SensorKind::Accelerometer => ("in_accel", 1.0), // m/s²
        SensorKind::Gyroscope => ("in_anglvel", 1.0),   // rad/s
        SensorKind::Magnetometer => ("in_magn", 100.0), // Gauss → µT
    }
}

/// The first iio device directory exposing the kind's x-channel, if any.
fn device_dir(prefix: &str) -> Option<PathBuf> {
    for entry in fs::read_dir("/sys/bus/iio/devices").ok()?.flatten() {
        let dir = entry.path();
        if dir.join(format!("{prefix}_x_raw")).exists() {
            return Some(dir);
        }
    }
    None
}

fn read_f64(path: &Path) -> Option<f64> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// A channel parameter (`scale`/`offset`), preferring the per-axis file over the shared one.
fn channel_param(dir: &Path, prefix: &str, axis: &str, param: &str) -> Option<f64> {
    read_f64(&dir.join(format!("{prefix}_{axis}_{param}")))
        .or_else(|| read_f64(&dir.join(format!("{prefix}_{param}"))))
}

fn read_axis(dir: &Path, prefix: &str, axis: &str, unit: f64) -> Option<f64> {
    let raw = read_f64(&dir.join(format!("{prefix}_{axis}_raw")))?;
    let offset = channel_param(dir, prefix, axis, "offset").unwrap_or(0.0);
    let scale = channel_param(dir, prefix, axis, "scale").unwrap_or(1.0);
    Some((raw + offset) * scale * unit)
}

pub fn is_available(kind: SensorKind) -> bool {
    device_dir(channel(kind).0).is_some()
}

pub fn read(kind: SensorKind) -> Option<SensorReading> {
    let (prefix, unit) = channel(kind);
    let dir = device_dir(prefix)?;
    Some(SensorReading {
        x: read_axis(&dir, prefix, "x", unit)?,
        y: read_axis(&dir, prefix, "y", unit)?,
        z: read_axis(&dir, prefix, "z", unit)?,
    })
}
