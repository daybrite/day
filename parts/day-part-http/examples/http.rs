//! Manual smoke: one real HTTPS GET through the platform stack (the examples/network.rs
//! pattern — run per platform to eyeball TLS, proxies, and the tier; not run in CI).
//!
//! ```sh
//! cargo run -p day-part-http --example http [URL]
//! ```

fn main() {
    let url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "https://crates.io/api/v1/crates/day-cli".to_string());
    println!("tier: {}", day_part_http::tier().label());
    match day_part_http::fetch(&day_part_http::Request::get(&url)) {
        Ok(resp) => {
            println!("status: {}", resp.status);
            for (k, v) in resp.headers.iter().take(8) {
                println!("  {k}: {v}");
            }
            println!("body: {} bytes", resp.body.len());
        }
        Err(e) => println!("error: {e}"),
    }
}
