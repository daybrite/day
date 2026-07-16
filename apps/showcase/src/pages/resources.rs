use day::prelude::*;

use crate::widgets::page;

/// Bundled resources (§18.3): an image loaded *by name* from the `images/` resource (the native
/// image pipeline) shown in each content mode, plus efficient random-access reads of arbitrary
/// embedded data via `resource()`.
pub(crate) fn resources_page() -> AnyPiece {
    page(
        crate::res::str::nav_resources(),
        "resources-title",
        Some(crate::res::str::resources_caption()),
        form((image_section(), data_section())).any(),
    )
}

/// `image(res::images::…)` resolves by name through the backend's native image path (bundle
/// file / Assets.car / R.drawable / …). One asset rendered under each content mode shows what
/// Fit (default), Fill, and Stretch each do to a non-square frame.
fn image_section() -> impl Piece {
    fn mode(label_text: LocalizedText, img: AnyPiece) -> AnyPiece {
        column((img, label(label_text).font(Font::Caption)))
            .spacing(6.0)
            .align(HAlign::Center)
            .any()
    }
    section((
        image(crate::res::images::day_logo).frame(96.0, 96.0),
        label(crate::res::str::resources_modes_note()).font(Font::Footnote),
        row((
            mode(
                crate::res::str::image_mode_fit(),
                image(crate::res::images::day_logo).frame(120.0, 72.0).any(),
            ),
            mode(
                crate::res::str::image_mode_fill(),
                image(crate::res::images::day_logo)
                    .fill()
                    .frame(120.0, 72.0)
                    .any(),
            ),
            mode(
                crate::res::str::image_mode_stretch(),
                image(crate::res::images::day_logo)
                    .stretch()
                    .frame(120.0, 72.0)
                    .any(),
            ),
        ))
        .spacing(16.0),
    ))
    .title(crate::res::str::resources_image_section())
}

/// Random-access reads of two bundled data resources, via the zero-copy `resource()` view.
fn data_section() -> impl Piece {
    let (numbers_line, greeting_line) = resource_lines();
    section((
        label(move || numbers_line.clone()).id("resources-numbers"),
        label(move || greeting_line.clone()).id("resources-greeting"),
    ))
    .title(crate::res::str::resources_data_section())
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
        Some(r) => {
            crate::res::str::resources_greeting(String::from_utf8_lossy(r.as_slice()).into_owned())
                .format()
        }
        None => "greeting.txt: (not bundled)".to_string(),
    };
    (numbers, greeting)
}
