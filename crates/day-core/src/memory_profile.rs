// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Launch-wide, constant-space memory statistics. No TLS, UI work or app resources are
//! touched from the sampling thread, including when exit runs after TLS destruction.
use std::sync::Mutex;

#[derive(Default)]
struct Stats {
    count: u64,
    total: u128,
    min: Option<u64>,
    max: Option<u64>,
    end: Option<u64>,
    stopped: bool,
}
impl Stats {
    fn sample(&mut self, bytes: Option<u64>) {
        self.end = bytes;
        if let Some(bytes) = bytes {
            self.count += 1;
            self.total += u128::from(bytes);
            self.min = Some(self.min.unwrap_or(bytes).min(bytes));
            self.max = Some(self.max.unwrap_or(bytes).max(bytes));
        }
    }
    fn summary(&self) -> String {
        fn mb(bytes: Option<f64>) -> String {
            bytes
                .map(|b| format!("{:.2}", b / 1e6))
                .unwrap_or_else(|| "unknown".into())
        }
        format!(
            "Memory profile: min {} MB, max {} MB, avg {} MB, end {} MB ({} samples; 1s interval)",
            mb(self.min.map(|b| b as f64)),
            mb(self.max.map(|b| b as f64)),
            mb((self.count > 0).then(|| self.total as f64 / self.count as f64)),
            mb(self.end.map(|b| b as f64)),
            self.count
        )
    }
}
static PROFILE: Mutex<Option<Stats>> = Mutex::new(None);

pub(crate) fn start() {
    // Browser lifetimes have no reliable process-exit notification; DayScript reports
    // separately stream Wasm samples while their page remains alive.
    #[cfg(not(target_arch = "wasm32"))]
    if std::env::var("DAY_MEMORY_PROFILE").as_deref() == Ok("1") {
        let mut profile = PROFILE.lock().unwrap();
        if profile.is_some() {
            return;
        }
        let mut stats = Stats::default();
        stats.sample(crate::lifecycle::resident_memory());
        *profile = Some(stats);
        std::thread::spawn(|| {
            loop {
                std::thread::sleep(std::time::Duration::from_secs(1));
                let mut profile = PROFILE.lock().unwrap();
                let Some(stats) = profile.as_mut().filter(|s| !s.stopped) else {
                    break;
                };
                stats.sample(crate::lifecycle::resident_memory());
            }
        });
    }
}

pub(crate) fn finish(end: Option<u64>) -> Option<String> {
    let mut profile = PROFILE.lock().ok()?;
    let stats = profile.as_mut()?;
    if !stats.stopped {
        stats.sample(end);
        stats.stopped = true;
    }
    Some(stats.summary())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profile_uses_all_samples_and_keeps_unavailable_end_explicit() {
        let mut stats = Stats::default();
        for value in [Some(100_000_000), Some(300_000_000), Some(200_000_000)] {
            stats.sample(value);
        }
        assert_eq!(
            stats.summary(),
            "Memory profile: min 100.00 MB, max 300.00 MB, avg 200.00 MB, end 200.00 MB (3 samples; 1s interval)"
        );
        stats.sample(None);
        assert!(stats.summary().contains("avg 200.00 MB, end unknown MB"));
    }
}
