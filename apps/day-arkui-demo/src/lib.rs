//! A minimal Day app for the HarmonyOS Next ArkUI backend: a reactive counter that exercises the
//! container / label / button pieces, native events, and build-once/bind-forever reactivity — all
//! rendered as real ArkUI nodes (Stack / Text / Button) mounted into an ArkTS `NodeContent`.
//!
//! Built as `libentry.so` and loaded by the ArkTS host (`harmony/`), which calls
//! `native.start(nodeContent, width, height, density)`.

use day::prelude::*;

fn root() -> day::AnyPiece {
    let count = Signal::new(0i64);
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
    ))
    .spacing(20.0)
    .padding(24.0)
    .any()
}

// Exports `day_arkui_start` (called by the ArkUI shim's `start` NAPI wrapper). No-op off HarmonyOS.
day::arkui_main!(root);
