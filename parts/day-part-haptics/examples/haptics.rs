//! `cargo run -p day-part-haptics --example haptics` — fire one haptic of each style.
//! Demonstrates that any Rust code can depend on this crate and use the API with no Day framework at
//! all. On a platform/host without a haptic engine this prints the (lack of) support and the calls
//! are silent no-ops.

use day_part_haptics::Haptic;

fn main() {
    println!("haptics supported: {}", day_part_haptics::is_supported());
    for h in [
        Haptic::Light,
        Haptic::Medium,
        Haptic::Heavy,
        Haptic::Success,
        Haptic::Warning,
        Haptic::Error,
        Haptic::Selection,
    ] {
        println!("play {h:?}");
        day_part_haptics::play(h);
    }
}
