// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Generates this crate's daybridge glue (docs/bridge.md).
//!
//! `day-build` reads the `day_bridge::bridge!` block in `src/` and writes
//! `$OUT_DIR/day-bridge/mod.rs` (the Rust side, which `bridge!` includes) plus the foreign adapter
//! `day build` stages. Runs on any host with no foreign toolchain installed.
fn main() {
    day_build::bridge::generate().expect("day-build: bridge codegen");
}
