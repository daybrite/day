use day::prelude::*;
use day_piece_rating::Card;
use day_tweak_button_bezel::{Bezel, ButtonBezelTweak};
use day_tweak_label_selectable::LabelSelectableTweak;
use day_tweak_slider_tickmarks::{SliderTickmarksTweak, TickPosition, Tickmarks};

use crate::widgets::heading;

// Tweaks (docs/tweaks.md): packaged per-toolkit configuration of BUILT-IN pieces. Each card shows
// a stock piece beside its Tweaked Piece; the captions name the toolkits the tweak affects — on
// every other toolkit the tweak is a documented no-op and the two sides look identical.
pub(crate) fn tweaks_page() -> AnyPiece {
    let free = Signal::new(50.0f64);
    let snapped = Signal::new(50.0f64);
    let ref_mounted = Signal::new(true);
    let slider_ref = NativeRef::new();

    // day-tweak-button-bezel: the trivial single-toolkit tweak (AppKit bezel constants).
    let bezel_card = column((
        label(crate::res::str::tweaks_bezel_title()).font(Font::Headline),
        label(crate::res::str::tweaks_bezel_caption()).font(Font::Footnote),
        row((
            button(crate::res::str::tweaks_stock()).id("tweak-bezel-stock"),
            button(crate::res::str::tweaks_tweaked())
                .bezel(Bezel::Toolbar)
                .id("tweak-bezel-toolbar"),
            button(crate::res::str::tweaks_tweaked())
                .bezel(Bezel::Badge)
                .id("tweak-bezel-badge"),
        ))
        .spacing(10.0),
    ))
    .spacing(8.0)
    .align(HAlign::Leading)
    .modifier(Card);

    // day-tweak-label-selectable: three toolkits, three access tiers (objc2 / gtk4-rs / JNI).
    let selectable_card = column((
        label(crate::res::str::tweaks_selectable_title()).font(Font::Headline),
        label(crate::res::str::tweaks_selectable_caption()).font(Font::Footnote),
        label(crate::res::str::tweaks_selectable_text())
            .selectable()
            .id("tweak-selectable-label"),
    ))
    .spacing(8.0)
    .align(HAlign::Leading)
    .modifier(Card);

    // day-tweak-slider-tickmarks: the full-range tweak — six toolkits, incl. its own Qt/WinUI/
    // ArkUI native code. The tweaked slider snaps to its marks where the platform supports it.
    let ticks_card = column((
        label(crate::res::str::tweaks_ticks_title()).font(Font::Headline),
        label(crate::res::str::tweaks_ticks_caption()).font(Font::Footnote),
        row((
            label(crate::res::str::tweaks_stock()).font(Font::Caption),
            slider(free).range(0.0..=100.0).id("tweak-ticks-stock"),
            label(move || format!("{:.0}", free.get())).id("tweak-ticks-stock-value"),
        ))
        .spacing(8.0),
        row((
            label(crate::res::str::tweaks_tweaked()).font(Font::Caption),
            slider(snapped)
                .range(0.0..=100.0)
                .tickmarks(
                    Tickmarks::count(11)
                        .snap(true)
                        .position(TickPosition::Below),
                )
                .id("tweak-ticks-slider"),
            label(move || format!("{:.0}", snapped.get())).id("tweak-ticks-value"),
        ))
        .spacing(8.0),
    ))
    .spacing(8.0)
    .align(HAlign::Leading)
    .modifier(Card);

    // NativeRef: imperative access with liveness — unmount the Tweaked Piece and the ref clears.
    let ref_card = column((
        label(crate::res::str::tweaks_ref_title()).font(Font::Headline),
        label(crate::res::str::tweaks_ref_caption()).font(Font::Footnote),
        toggle(ref_mounted)
            .id("tweak-ref-toggle")
            .a11y(|a| a.label("Mount the tweaked piece")),
        when(move || ref_mounted.get(), {
            let r = slider_ref.clone();
            // On the tick grid (count 5 over 0..=100 → step 25): a stepped Material slider
            // requires values on the grid, and programmatic snapping doesn't write back.
            let v = Signal::new(25.0f64);
            move || {
                slider(v)
                    .range(0.0..=100.0)
                    .tickmarks(Tickmarks::count(5))
                    .native_ref(&r)
                    .id("tweak-ref-slider")
            }
        }),
        label({
            let r = slider_ref.clone();
            // NativeRef reads are tracked: this re-runs on the ref's mount/clear transitions.
            move || {
                if r.node().is_some() {
                    crate::res::str::tweaks_ref_live().format()
                } else {
                    crate::res::str::tweaks_ref_cleared().format()
                }
            }
        })
        .id("tweak-ref-status"),
    ))
    .spacing(8.0)
    .align(HAlign::Leading)
    .modifier(Card);

    scroll(
        column((
            heading(crate::res::str::nav_tweaks(), "tweaks-title", Some(crate::res::str::tweaks_intro())),
            bezel_card,
            selectable_card,
            ticks_card,
            ref_card,
        ))
        .spacing(14.0)
        .align(HAlign::Leading)
        .padding(16.0),
    )
    .any()
}
