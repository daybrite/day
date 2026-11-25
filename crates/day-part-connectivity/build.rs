// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Generates this crate's daybridge glue (docs/bridge.md).
//!
//! `day-build` reads the `day_bridge::bridge!` block in `src/platform_bridge.rs` and writes
//! `$OUT_DIR/day-bridge/mod.rs` (the Rust side, which `bridge!` includes); `day build` stages the
//! Java arm into the app's Gradle build. Runs on any host with no Android toolchain installed.
fn main() {
    if matches!(
        std::env::var("CARGO_CFG_TARGET_OS").as_deref(),
        Ok("ios" | "macos")
    ) {
        println!("cargo:rerun-if-changed=src/path_monitor.c");
        cc::Build::new()
            .file("src/path_monitor.c")
            .flag("-fblocks")
            .compile("day_path_monitor");
        println!("cargo:rustc-link-lib=framework=Network");
    }
    day_build::bridge::generate().expect("day-build: bridge codegen");
}
