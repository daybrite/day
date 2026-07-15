use day::prelude::*;

use crate::widgets::heading;

/// Bundled resources (§18.3): an image loaded *by name* from the `images/` resource (the native
/// image pipeline), plus efficient random-access reads of arbitrary embedded data via `resource()`.
pub(crate) fn resources_page() -> AnyPiece {
    let (numbers_line, greeting_line) = resource_lines();
    column((
        heading(
            crate::res::str::nav_resources(),
            "resources-title",
            Some(crate::res::str::resources_caption()),
        ),
        // `image(crate::res::images::day_logo)` resolves `images/day_logo.png` by name through the backend's native
        // image path (bundle file / Assets.car / R.drawable / …). `.frame` gives it a fixed box;
        // it scales to Fit (default content mode) — preserving aspect, never stretching.
        image(crate::res::images::day_logo).frame(96.0, 96.0),
        label(move || numbers_line.clone()).id("resources-numbers"),
        label(move || greeting_line.clone()).id("resources-greeting"),
    ))
    .spacing(10.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .any()
}

/// Open two bundled data resources and format one random-access read from each. `numbers.bin` holds
/// the bytes `0..=255`, so `byte[100]` must be `100`; `greeting.txt` is a short UTF-8 string.
fn resource_lines() -> (String, String) {
    let numbers = match resource(crate::res::assets::numbers_bin) {
        Some(r) => {
            let mut b = [0u8; 1];
            r.read_at(100, &mut b);
            crate::res::str::resources_numbers(b[0] as f64, r.len() as f64).format()
        }
        None => "numbers.bin: (not bundled)".to_string(),
    };
    let greeting = match resource(crate::res::assets::greeting_txt) {
        Some(r) => crate::res::str::resources_greeting(
            String::from_utf8_lossy(r.as_slice()).into_owned(),
        )
        .format(),
        None => "greeting.txt: (not bundled)".to_string(),
    };
    (numbers, greeting)
}
