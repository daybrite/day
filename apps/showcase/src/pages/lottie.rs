use day::prelude::*;
use day_piece_lottie::lottie;

/// A native Lottie animation (day-piece-lottie): a LottieAnimationView driven by airbnb's lottie-ios
/// (SwiftPM) / lottie-android (Gradle). Renders the bundled `hello.json`, looping. iOS + Android only.
#[cfg(any(target_os = "ios", target_os = "android"))]
pub(crate) fn lottie_page() -> AnyPiece {
    // Playback rate, bound two ways: the slider drives it and `.speed(speed)` pushes it to the
    // native LottieAnimationView live (a `Speed` patch per change).
    let speed = Signal::new(1.0);
    column((
        label(tr("nav-lottie")).font(Font::Title).id("lottie-title"),
        label(tr("lottie-caption")).id("lottie-caption"),
        lottie("hello")
            .speed(speed)
            .frame(220.0, 220.0)
            .id("lottie-view"),
        // — speed slider with live readout (0.25×–3×) —
        row((
            label(tr("lottie-speed")),
            slider(speed)
                .range(0.25..=3.0)
                .step(0.25)
                .id("lottie-speed-slider"),
            label(move || format!("{:.2}×", speed.get())).id("lottie-speed-value"),
        ))
        .spacing(8.0),
    ))
    .spacing(12.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .any()
}
