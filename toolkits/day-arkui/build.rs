// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Generates the daybridge glue for src/host.rs (docs/bridge.md): the Rust side of the ArkTS
//! arms this toolkit reaches its ArkTS-only duties through. Runs on any host.
fn main() {
    day_build::bridge::generate().expect("day-build: bridge codegen");
}
