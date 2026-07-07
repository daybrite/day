//! `cargo run -p day-part-clipboard --example clipboard [text]` — read the clipboard, or place
//! `text` on it first when given. Demonstrates that any Rust code can depend on this crate and use
//! the API with no Day framework at all. (Verify against the OS: `pbpaste` on macOS.)

fn main() {
    if let Some(text) = std::env::args().nth(1) {
        let ok = day_part_clipboard::set_text(&text);
        println!("set_text({text:?}) -> {ok}");
    }
    println!("has_text: {}", day_part_clipboard::has_text());
    match day_part_clipboard::get_text() {
        Some(text) => println!("clipboard: {text:?}"),
        None => println!("clipboard: empty (or no clipboard API on this platform)"),
    }
}
