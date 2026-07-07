//! `cargo run -p day-part-battery --example battery` — print the current battery status. Demonstrates that
//! any Rust code can depend on this crate and use the API with no Day framework at all.

fn main() {
    match day_part_battery::status() {
        Some(b) => println!(
            "battery: {:?}, level {:?} ({}), charging: {}",
            b.state,
            b.level,
            b.percent().map(|p| format!("{p}%")).unwrap_or("?".into()),
            b.is_charging()
        ),
        None => println!("no battery API (or no battery) on this platform"),
    }
}
