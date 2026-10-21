//! Generates the typed `res::` constants for `resource/`: https://daybrite.dev/docs/resources
fn main() {
    day_build::prebuild_project().expect("day-build: prebuild");
}
