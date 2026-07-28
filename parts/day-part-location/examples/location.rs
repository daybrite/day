//! `cargo run -p day-part-location --example location` — print the device's position for a few
//! seconds. Demonstrates that any Rust code can depend on this crate and use the API with no Day
//! framework at all.
//!
//! On a Mac this prints nothing even when CoreLocation exists: a plain binary has no run loop for
//! CoreLocation to deliver on, and an unbundled process has no Info.plist for TCC to read. Both are
//! documented in docs/location.md; run the showcase to see it work.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use day_part_location::Accuracy;

fn main() {
    if !day_part_location::is_available() {
        println!("no location API on this platform");
        return;
    }
    let seen = Arc::new(AtomicUsize::new(0));
    let counter = seen.clone();
    // The handle stops updates when dropped, so hold it for as long as you want them.
    let _watch = day_part_location::watch(Accuracy::Balanced, move |fix| {
        counter.fetch_add(1, Ordering::Relaxed);
        match fix {
            Ok(f) => println!(
                "{:.5}, {:.5}  altitude {}  ±{}",
                f.latitude,
                f.longitude,
                f.altitude.map_or("—".to_string(), |a| format!("{a:.0} m")),
                f.accuracy_m
                    .map_or("—".to_string(), |a| format!("{a:.0} m")),
            ),
            Err(e) => println!("error: {e}"),
        }
    });
    std::thread::sleep(std::time::Duration::from_secs(5));
    if seen.load(Ordering::Relaxed) == 0 {
        println!("no updates in 5s (see the run-loop note at the top of this file)");
    }
}
