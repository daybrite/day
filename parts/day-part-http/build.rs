// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Generates this crate's daybridge glue (docs/bridge.md): the Android, HarmonyOS and web arms in
//! `src/bridge.rs`, which stream each exchange's frames to `src/bridged.rs`.
fn main() {
    day_build::bridge::generate().expect("day-build: bridge codegen");
}
