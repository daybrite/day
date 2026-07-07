//! `cargo run -p day-part-prefs --example prefs [key] [value]` — a tiny persistent store from the
//! command line. `prefs greeting` reads the value stored under `greeting`; `prefs greeting hello`
//! stores `hello` under `greeting`, then reads it back.
//!
//! Run it twice to see that the value PERSISTS across processes (macOS: `~/Library/Preferences`;
//! Linux: `~/.config/day/day-part-prefs.store`). Demonstrates that any Rust code can depend on this
//! crate and use the API with no Day framework at all.

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(key) = args.next() else {
        eprintln!("usage: prefs <key> [value]");
        return;
    };

    if let Some(value) = args.next() {
        let ok = day_part_prefs::set(&key, &value);
        println!("set({key:?}, {value:?}) -> {ok}");
    }

    println!("contains({key:?}): {}", day_part_prefs::contains(&key));
    match day_part_prefs::get(&key) {
        Some(value) => println!("get({key:?}): {value:?}"),
        None => println!("get({key:?}): <absent> (or no store on this platform)"),
    }
}
