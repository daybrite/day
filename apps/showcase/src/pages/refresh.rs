use std::time::Duration;

use day::prelude::*;
use day_piece_pullrefresh::pull_to_refresh;

use crate::widgets::heading;

/// Pull-to-refresh (day-piece-pullrefresh, an EXTERNAL standalone piece — and the reference
/// CONTAINER piece): a feed in a plain `scroll()` wrapped with `pull_to_refresh`. The bound
/// `refreshing: Signal<bool>` is two-way — a pull (native on iOS/Android/HarmonyOS, elastic/
/// overshoot on AppKit/GTK) or the button sets it true; a `watch` starts the fake reload off the
/// UI thread and its `Setter`s append rows + end the refresh. The List page wraps its recycling
/// `list()` the same way.
pub(crate) fn refresh_page() -> AnyPiece {
    let refreshing = Signal::new(false);
    let items = Signal::new(8i64);

    // ONE reload path for every begin — pull gesture, dayscript `toggle:`, or the button: the
    // watch fires whenever `refreshing` turns true (the SwiftUI `refreshable` action, expressed
    // as Day's signal-watch idiom). The work leaves the UI thread; `Setter`s hop back.
    watch(
        move || refreshing.get(),
        move |now, _| {
            if *now {
                let next = items.get_untracked() + 4;
                let grow = items.setter();
                let done = refreshing.setter();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(900)); // the "network"
                    grow.set(next);
                    done.set(false);
                });
            }
        },
    );

    // The tier this backend realizes (docs: native vs emulated) — CI-visible documentation.
    let tier = match day_piece_pullrefresh::support() {
        day::prelude::Support::Native => crate::res::str::refresh_tier_native(),
        _ => crate::res::str::refresh_tier_emulated(),
    };

    column((
        row((
            heading(
                crate::res::str::nav_refresh(),
                "refresh-title",
                Some(crate::res::str::refresh_caption()),
            ),
            spacer(),
            button(crate::res::str::refresh_now())
                .prominent()
                .action(move || refreshing.set(true))
                .id("refresh-now"),
        )),
        label(move || {
            if refreshing.get() {
                crate::res::str::refresh_status_refreshing().format()
            } else {
                crate::res::str::refresh_status_idle().format()
            }
        })
        .font(Font::Footnote)
        .id("refresh-status"),
        label(tier).font(Font::Footnote).id("refresh-tier"),
        pull_to_refresh(
            refreshing,
            scroll(
                column((each(
                    move || (1..=items.get()).rev().collect::<Vec<i64>>(),
                    |i| *i,
                    |slot: ItemSlot<i64, i64>| {
                        label(move || crate::res::str::refresh_row(slot.get()).format())
                            .padding(Insets::symmetric(12.0, 8.0))
                            .id_keyed("refresh-row", slot.key())
                    },
                ),))
                .align(HAlign::Leading),
            ),
        )
        .id("feed-refresh"),
    ))
    .spacing(10.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .any()
}
