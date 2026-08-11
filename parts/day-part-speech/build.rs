// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Generates this crate's daybridge glue (docs/bridge.md).
//!
//! `day-build` reads the `day_bridge::bridge!` block in `src/lib.rs` and writes
//! `$OUT_DIR/day-bridge/mod.rs` (the Rust side, which `bridge!` includes) plus a manifest
//! `day build` reads to emit each foreign arm's adapter. Runs on any host, with no Swift, Kotlin,
//! ArkTS, or C toolchain required — those are needed only by the targets that use them.
fn main() {
    day_build::bridge::generate().expect("day-build: bridge codegen");
}
