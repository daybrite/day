//! A minimal Day app for the HarmonyOS Next ArkUI backend: a reactive counter that exercises the
//! container / label / button pieces, native events, and build-once/bind-forever reactivity — all
//! rendered as real ArkUI nodes (Stack / Text / Button) mounted into an ArkTS `NodeContent`.
//!
//! Built as `libentry.so` and loaded by the ArkTS host (`platform/ohos/`), which calls
//! `native.start(nodeContent, width, height, density)`.

use day::prelude::*;

/// Format the current device battery (via the headless `day-part-battery` crate → HarmonyOS's native
/// `libohbattery_info.so`). Returns a display string for the demo readout.
fn battery_text() -> String {
    match day_part_battery::status() {
        Some(b) => format!(
            "Battery: {} ({:?})",
            b.percent().map(|p| format!("{p}%")).unwrap_or("?".into()),
            b.state
        ),
        None => "Battery: unavailable".into(),
    }
}

pub fn root() -> day::AnyPiece {
    let count = Signal::new(0i64);
    // Native file open/save via the ArkTS DocumentViewPicker (docs/files.md).
    let file_text = Signal::new(String::from("Hello from Day on HarmonyOS!"));
    let status = Signal::new(String::new());
    // Headless day-part-battery capability crate → native libohbattery_info.so (docs/battery.md).
    let battery = Signal::new(battery_text());
    column((
        label("Day on HarmonyOS ArkUI").font(Font::Title),
        label(move || format!("Count: {}", count.get()))
            .font(Font::LargeTitle)
            .id("count"),
        row((
            button("–")
                .action(move || count.update(|c| *c -= 1))
                .id("dec"),
            button("+")
                .action(move || count.update(|c| *c += 1))
                .id("inc"),
        ))
        .spacing(16.0),
        // Native file pickers: Open reads a file into the editor; Save writes it back out.
        text_field(file_text).id("file-text"),
        row((
            button("Open File")
                .action(move || {
                    day::task(async move {
                        match open_file().filter("Text", &["txt", "md"]).await {
                            Some(f) => match f.read_to_string() {
                                Ok(t) => {
                                    file_text.set(t);
                                    status.set("opened".into());
                                }
                                Err(_) => status.set("open-error".into()),
                            },
                            None => status.set("open-cancel".into()),
                        }
                    });
                })
                .id("open-file"),
            button("Save File")
                .action(move || {
                    day::task(async move {
                        let data = file_text.get_untracked().into_bytes();
                        match save_file(data).suggested_name("day-notes.txt").await {
                            Some(d) => {
                                status.set(format!("saved:{}", d.file_name().unwrap_or_default()))
                            }
                            None => status.set("save-cancel".into()),
                        }
                    });
                })
                .id("save-file"),
        ))
        .spacing(16.0),
        label(move || status.get()).id("file-status"),
        // Headless capability crate: device battery via HarmonyOS's native libohbattery_info.so.
        row((
            label(move || battery.get()).id("battery"),
            button("Refresh")
                .action(move || battery.set(battery_text()))
                .id("battery-refresh"),
        ))
        .spacing(16.0),
    ))
    .spacing(20.0)
    .padding(24.0)
    .any()
}

// Exports `day_arkui_start` (called by the ArkUI shim's `start` NAPI wrapper). No-op off HarmonyOS.
day::arkui_main!(root);
