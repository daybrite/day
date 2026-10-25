// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The HarmonyOS arm's wire format (src/ohos.rs): one line per fix or failure. Kept apart from
//! the arm, which compiles only for HarmonyOS, so the host test suite covers the parsing.

use crate::{Fix, LocationError};

/// One line from the arm: `fix,<lat>,<lon>,<alt>,<acc>,<vacc>,<speed>,<course>,<ms>` (a field the
/// platform did not report is `NaN`) or `error,<BusinessError code>`.
pub(crate) fn parse(line: &str) -> Result<Fix, LocationError> {
    let mut parts = line.split(',');
    match parts.next() {
        Some("fix") => {
            let v: Vec<f64> = parts.map(|p| p.parse().unwrap_or(f64::NAN)).collect();
            if v.len() < 8 || !v[0].is_finite() || !v[1].is_finite() {
                return Err(LocationError::Io(format!("malformed fix: {line}")));
            }
            let some = |x: f64| x.is_finite().then_some(x);
            Ok(Fix {
                latitude: v[0],
                longitude: v[1],
                altitude: some(v[2]),
                accuracy_m: some(v[3]),
                vertical_accuracy_m: some(v[4]),
                speed_mps: some(v[5]),
                course_deg: some(v[6]),
                timestamp_ms: some(v[7]).map(|t| t as i64),
            })
        }
        Some("error") => Err(match parts.next().and_then(|c| c.parse::<i64>().ok()) {
            Some(201) => LocationError::PermissionDenied,
            // The location switch is off.
            Some(3301100) => LocationError::Disabled,
            // Positioning failed, or timed out, before a fix.
            Some(3301200) => LocationError::Timeout,
            Some(code) => LocationError::Io(format!("location error {code}")),
            None => LocationError::Io(line.to_string()),
        }),
        _ => Err(LocationError::Io(line.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fix_line_keeps_unreported_fields_empty() {
        let fix = parse("fix,48.85,2.35,NaN,12.5,NaN,0,NaN,1700000000000").unwrap();
        assert_eq!((fix.latitude, fix.longitude), (48.85, 2.35));
        assert_eq!(fix.altitude, None);
        assert_eq!(fix.accuracy_m, Some(12.5));
        assert_eq!(fix.speed_mps, Some(0.0));
        assert_eq!(fix.course_deg, None);
        assert_eq!(fix.timestamp_ms, Some(1_700_000_000_000));
    }

    #[test]
    fn a_fix_without_a_position_is_malformed() {
        assert!(matches!(
            parse("fix,NaN,2.35,0,0,0,0,0,0"),
            Err(LocationError::Io(_))
        ));
        assert!(matches!(parse("fix,48.85,2.35"), Err(LocationError::Io(_))));
    }

    #[test]
    fn error_codes_map_onto_the_crate_errors() {
        assert_eq!(parse("error,201"), Err(LocationError::PermissionDenied));
        assert_eq!(parse("error,3301100"), Err(LocationError::Disabled));
        assert_eq!(parse("error,3301200"), Err(LocationError::Timeout));
        assert!(matches!(parse("error,9"), Err(LocationError::Io(_))));
        assert!(matches!(parse("garbage"), Err(LocationError::Io(_))));
    }
}
