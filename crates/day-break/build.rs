// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Bake the app identity (Day.toml `[app]`, exported by `day build`/`day launch` as `DAY_APP_*`,
//! see `crates/day-cli/src/ops.rs::apply_app_identity`) into the crate so a crash report can carry
//! id/version/build without reading a platform manifest at runtime.
//!
//! `option_env!` alone would not rebuild when the value changes (env is not a source input), so we
//! re-export each var through `cargo:rustc-env` and pair it with `cargo:rerun-if-env-changed` — a
//! rustc-env change correctly invalidates the lib compile even in a shared target dir. A bare
//! `cargo build` (no `day` CLI) sets nothing; the lib then falls back to a runtime `DAY_APP_*`
//! lookup and finally to `"unknown"`.

fn main() {
    for var in ["DAY_APP_ID", "DAY_APP_VERSION", "DAY_APP_BUILD"] {
        println!("cargo:rerun-if-env-changed={var}");
        if let Ok(val) = std::env::var(var) {
            // Baked copy under a distinct name so `option_env!("DAY_BREAK_APP_ID")` in lib.rs reads
            // the value frozen at compile time, independent of the process's runtime environment.
            println!("cargo:rustc-env=DAY_BREAK_{var}={val}");
        }
    }
}
