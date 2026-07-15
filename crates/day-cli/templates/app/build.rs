//! Generates typed resource constants from this app's `resource/` directory (§18.5).
//!
//! `day-build` scans `resource/{images,assets,fonts}` and writes `$OUT_DIR/day_resources.rs`, which
//! `src/lib.rs` surfaces as the `res` module. App code then references bundled resources by a
//! compiler-checked symbol — `image(res::images::app_logo)` — instead of a bare string: a typo is a
//! build error, the resource is guaranteed bundled, and the available names autocomplete. Adding or
//! removing a file under `resource/` regenerates on the next build.
fn main() {
    day_build::generate_resources().expect("day-build: resource codegen");
}
