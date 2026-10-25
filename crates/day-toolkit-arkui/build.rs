// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Generates the daybridge glue for src/host.rs (docs/bridge.md): the Rust side of the ArkTS
//! arms this toolkit reaches its ArkTS-only duties through. Runs on any host.
//!
//! Also names the NAPI module. `napi-derive-ohos` registers the module its first `#[napi]`
//! export expands in under `NAPI_BUILD_TARGET_NAME` (else the crate's own name), and the ArkTS
//! runtime matches that name against the library the host imports: `import native from
//! 'libentry.so'` (docs/harmonyos.md). Day's app cdylib is always staged as `libentry.so`, so the
//! module is `entry` whatever crate the exports live in.
fn main() {
    println!("cargo:rustc-env=NAPI_BUILD_TARGET_NAME=entry");
    day_build::bridge::generate().expect("day-build: bridge codegen");
}
