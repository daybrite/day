// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

use day_script_proto::{MemorySize, Reply};

pub(crate) fn sample(limit: Option<MemorySize>) -> Reply {
    let reply = report(day_core::lifecycle::resident_memory(), limit);
    if let Some(message) = reply
        .data
        .as_ref()
        .and_then(|data| data["message"].as_str())
    {
        log::info!("{message}");
    }
    reply
}

pub(crate) fn reading() -> day_script_proto::MemorySample {
    day_script_proto::MemorySample {
        bytes: day_core::lifecycle::resident_memory(),
        metric: metric().into(),
    }
}

fn metric() -> &'static str {
    if cfg!(any(target_os = "macos", target_os = "ios")) {
        "physical_footprint"
    } else if cfg!(any(
        target_os = "linux",
        target_os = "android",
        target_env = "ohos"
    )) {
        "resident_set"
    } else if cfg!(windows) {
        "working_set"
    } else if cfg!(target_arch = "wasm32") {
        "wasm_linear_memory"
    } else {
        "resident_memory"
    }
}

fn report(bytes: Option<u64>, limit: Option<MemorySize>) -> Reply {
    let limit = limit.map(|size| size.0);
    let usage = bytes.map_or_else(
        || "unavailable".into(),
        |bytes| format!("{:.2} MB ({bytes} bytes)", bytes as f64 / 1e6),
    );
    let mut message = format!("Memory usage: {usage} [{}]", metric());
    if let Some(limit) = limit {
        message.push_str(&format!(
            "; limit {:.2} MB ({limit} bytes)",
            limit as f64 / 1e6
        ));
    }
    let error = match bytes {
        None => Some(format!("{message}; cannot measure current app memory")),
        Some(bytes) if limit.is_some_and(|limit| bytes > limit) => {
            Some(format!("{message}; exceeds limit"))
        }
        Some(_) => None,
    };
    Reply {
        ok: error.is_none(),
        error,
        data: Some(serde_json::json!({
            "bytes": bytes,
            "limit_bytes": limit,
            "metric": metric(),
            "message": message,
        })),
        ..Reply::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn threshold_is_strict_and_failures_keep_measurement_without_retrying() {
        assert!(report(Some(500), Some(MemorySize(500))).ok);
        assert!(report(Some(499), Some(MemorySize(500))).ok);
        let exceeded = report(Some(501), Some(MemorySize(500)));
        assert!(!exceeded.ok);
        assert!(!exceeded.retryable);
        assert!(exceeded.error.unwrap().contains("exceeds limit"));
        let data = exceeded.data.unwrap();
        assert_eq!(data["bytes"], 501);
        assert_eq!(data["limit_bytes"], 500);
        assert!(report(Some(u64::MAX), None).ok);
        assert!(report(Some(0), Some(MemorySize(0))).ok);
    }

    #[test]
    fn unavailable_measurements_never_pass_a_budget() {
        for limit in [None, Some(MemorySize(500))] {
            let reply = report(None, limit);
            assert!(!reply.ok);
            assert!(!reply.retryable);
            assert!(reply.data.unwrap()["bytes"].is_null());
            assert!(reply.error.unwrap().contains("cannot measure"));
        }
    }

    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    fn executor_samples_the_app_process_and_rejects_an_impossible_budget() {
        let reply = crate::exec(
            crate::Step::MemoryUsage {
                fail_if_above: None,
            },
            0,
        );
        assert!(reply.ok, "{reply:?}");
        assert!(reply.data.unwrap()["bytes"].as_u64().unwrap() > 0);
        let reply = crate::exec(
            crate::Step::MemoryUsage {
                fail_if_above: Some(MemorySize(0)),
            },
            0,
        );
        assert!(!reply.ok);
        assert!(!reply.retryable);
    }
}
