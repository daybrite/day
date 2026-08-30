//! Generates typed `res::` constants from `resource/` (https://daybrite.dev/docs/resources).
fn main() {
    day_build::generate_resources().expect("day-build: resource codegen");
}
