//! Generate typed resource constants from `resource/` (§18.5) — the same one-liner `day new`
//! scaffolds into every app. `day-build` writes `$OUT_DIR/day_resources.rs`, surfaced as the `res`
//! module in lib.rs, so the showcase references its bundled icons/data/fonts by checked symbol.
fn main() {
    day_build::generate_resources().expect("day-build: resource codegen");
    // Bake the app identity (Day.toml `[app].id`, exported by `day build`/`day launch` as
    // `DAY_APP_ID` — crates/day-cli/src/ops.rs::apply_app_identity) so the About page can show
    // the bundle id without a runtime manifest read. Same pattern as day-break's build.rs:
    // re-exporting through `cargo:rustc-env` makes a value change invalidate the compile.
    println!("cargo:rerun-if-env-changed=DAY_APP_ID");
    if let Ok(id) = std::env::var("DAY_APP_ID") {
        println!("cargo:rustc-env=DAY_SHOWCASE_APP_ID={id}");
    }
}
