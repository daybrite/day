//! Generate typed resource constants from `resource/` (§18.5) — the same one-liner `day new`
//! scaffolds into every app. `day-build` writes `$OUT_DIR/day_resources.rs`, surfaced as the `res`
//! module in lib.rs, so the showcase references its bundled icons/data/fonts by checked symbol.
fn main() {
    day_build::generate_resources().expect("day-build: resource codegen");
}
