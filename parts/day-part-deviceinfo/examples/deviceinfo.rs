//! `cargo run -p day-part-deviceinfo --example deviceinfo` — print the current device identity.
//! Demonstrates that any Rust code can depend on this crate and use the API with no Day framework at all.

fn main() {
    let d = day_part_deviceinfo::get();
    println!("model:          {}", d.model);
    println!("system_name:    {}", d.system_name);
    println!("system_version: {}", d.system_version);
    println!("is_simulator:   {}", d.is_simulator);
}
