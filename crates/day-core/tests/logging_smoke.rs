// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The default logger's contract (docs/logging.md): installed with no ceremony, formatted as
//! `LEVEL target: message`, routed through the sink when a backend installs one, and filtered by
//! `set_log_level`.
//!
//! One test rather than several: `log::set_logger` and the sink are both process-global
//! one-shots, so separate `#[test]`s would race for them under the default threaded harness.

use std::sync::{Mutex, OnceLock};

static LINES: OnceLock<Mutex<Vec<(log::Level, String)>>> = OnceLock::new();

fn captured() -> &'static Mutex<Vec<(log::Level, String)>> {
    LINES.get_or_init(|| Mutex::new(Vec::new()))
}

fn sink(level: log::Level, line: &str) {
    captured().lock().unwrap().push((level, line.to_string()));
}

#[test]
fn default_logger_formats_routes_and_filters() {
    day_core::set_log_sink(sink);
    day_core::init_logging();
    day_core::set_log_level(log::LevelFilter::Info);

    log::warn!("bundled font {} is unreadable", "Inter.ttf");
    log::info!("hello");
    log::debug!("filtered out at Info");

    let lines = captured().lock().unwrap().clone();
    let rendered: Vec<&str> = lines.iter().map(|(_, l)| l.as_str()).collect();

    // Level first, then the emitting crate, then the message.
    assert!(
        rendered.contains(&"WARN  logging_smoke: bundled font Inter.ttf is unreadable"),
        "unexpected lines: {rendered:?}"
    );
    assert!(rendered.contains(&"INFO  logging_smoke: hello"));
    // `set_log_level` is honored: Debug is below Info and never reaches the sink.
    assert!(
        !rendered.iter().any(|l| l.contains("filtered out")),
        "{rendered:?}"
    );
    // The level rides alongside the line so a sink can choose console.warn vs console.error.
    assert!(lines.iter().any(|(lv, _)| *lv == log::Level::Warn));
}
