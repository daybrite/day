// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Generates this crate's daybridge glue (docs/bridge.md).
//!
//! `day-build` reads the `day_bridge::bridge!` block in `src/android.rs` and writes
//! `$OUT_DIR/day-bridge/mod.rs` (the Rust side, which `bridge!` includes); `day build` stages the
//! Java arm into the app's Gradle build. Runs on any host with no Android toolchain installed.
fn main() {
    day_build::bridge::generate().expect("day-build: bridge codegen");
}
