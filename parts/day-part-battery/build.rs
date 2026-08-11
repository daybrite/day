//! Generates this crate's daybridge glue (docs/bridge.md): the Rust side of the Kotlin arm
//! declared in `src/lib.rs`. Runs on any host with no Android toolchain — `day build` stages and
//! compiles the Kotlin itself.
fn main() {
    day_build::bridge::generate().expect("day-build: bridge codegen");
}
