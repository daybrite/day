// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Generates the daybridge glue for the HarmonyOS notification arm in src/ohos.rs
//! (docs/bridge.md). Runs on any host with no foreign toolchain installed.
fn main() {
    day_build::bridge::generate().expect("day-build: bridge codegen");
}
