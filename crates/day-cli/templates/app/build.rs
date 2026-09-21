//! Everything day-build does for this project before it compiles: today that is the typed `res::`
//! constants generated from `resource/` (https://daybrite.dev/docs/resources). Keep the one call —
//! a step day-build adds later arrives through it.
fn main() {
    day_build::prebuild_project().expect("day-build: prebuild");
}
