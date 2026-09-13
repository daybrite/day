// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Generates this crate's daybridge glue (docs/bridge.md): the Android arm in `src/bridge.rs`
//! runs OkHttp asynchronously and completes a `Done<Vec<u8>>` with the response envelope.
fn main() {
    day_build::bridge::generate().expect("day-build: bridge codegen");
}
