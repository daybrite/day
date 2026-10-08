// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

use day_script_proto::{MemorySize, Script, Step};
use serde_json::json;

#[test]
fn memory_sizes_accept_explicit_decimal_and_binary_units_without_rounding() {
    for (size, bytes) in [
        ("500MB", 500_000_000),
        ("1G", 1_000_000_000),
        ("1.5 GB", 1_500_000_000),
        ("512MiB", 536_870_912),
        (" 1gib ", 1_073_741_824),
        ("1KB", 1000),
        ("1KiB", 1024),
        ("1T", 1_000_000_000_000),
        ("1TiB", 1_099_511_627_776),
        ("0", 0),
        ("0.000001MB", 1),
        ("18446744073709551615B", u64::MAX),
    ] {
        let limit: MemorySize = serde_json::from_value(json!(size)).unwrap();
        assert_eq!(limit.0, bytes, "{size}");
        assert_eq!(serde_json::to_value(limit).unwrap(), json!(bytes));
    }
    assert_eq!(
        serde_json::from_value::<MemorySize>(json!(42)).unwrap().0,
        42
    );
}

#[test]
fn malformed_or_overflowing_sizes_are_rejected_before_execution() {
    for size in [
        "",
        "MB",
        "-1MB",
        "+1MB",
        "NaN",
        "inf",
        "1PB",
        "1e3MB",
        "1.2.3MB",
        "1.MB",
        "0.1B",
        "18446744073709551616B",
        "18446744073709551615GB",
        "1.000000000000000000000000000000000000000001B",
    ] {
        assert!(
            serde_json::from_value::<MemorySize>(json!(size)).is_err(),
            "{size}"
        );
    }
    for size in [json!(-1), json!(1.5), json!(true), json!({"macos": "1G"})] {
        assert!(serde_json::from_value::<MemorySize>(size).is_err());
    }
    assert!(Script::from_yaml("flow:\n- memory_usage: {fail_if_above: wrong}").is_err());
}

#[test]
fn report_only_and_target_specific_thresholds_round_trip() {
    let script = Script::from_yaml("flow:\n- memory_usage:\n- memory_usage: {fail_if_above: 250MB, only_on: [ios, android]}\n- memory_usage: {fail_if_above: 1G, only_on: [macos-appkit]}\n").unwrap();
    assert_eq!(
        script.steps[0].step,
        Step::MemoryUsage {
            fail_if_above: None
        }
    );
    assert_eq!(
        script.steps[1].step,
        Step::MemoryUsage {
            fail_if_above: Some(MemorySize(250_000_000))
        }
    );
    assert_eq!(
        script.steps[1].annotations["only_on"],
        json!(["ios", "android"])
    );
    assert_eq!(Script::from_yaml(&script.to_yaml()).unwrap(), script);
}
