//! `cargo run -p day-part-network --example network` — print the current connectivity snapshot.
//! Demonstrates that any Rust code can depend on this crate and use the API with no Day framework at all.

fn main() {
    match day_part_network::status() {
        Some(n) => println!(
            "network: online: {}, kind: {:?}, expensive: {}",
            n.online,
            n.kind,
            n.expensive.map(|e| e.to_string()).unwrap_or("?".into())
        ),
        None => println!("no connectivity API on this platform"),
    }
}
