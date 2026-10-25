// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Generates this crate's daybridge glue (docs/bridge.md): the system tier's Android and HarmonyOS
//! arms in `src/bridge.rs`.
fn main() {
    day_build::bridge::generate().expect("day-build: bridge codegen");
}
