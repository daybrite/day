use std::time::Duration;

use day::prelude::*;

use crate::widgets::page;

/// Crash Reporting — demonstrates day-break (docs/break.md). The buttons intentionally crash (or
/// trip day-core's panic containment); on the NEXT launch the saved report shows in the scrollable
/// viewer, and "Send report" opens a prefilled email to the developer. Nothing leaves the device
/// without the user's action.
///
/// The three crash flavors cover day-break's capture paths: a native `abort` (SIGABRT) and a
/// `segfault` (SIGSEGV) both die and are recorded by the signal handler; the "contained panic"
/// stays alive (day-core catches panics at its trampoline boundaries — see docs/break.md) and is
/// recorded as a NON-fatal report on the next launch.
pub(crate) fn crash_page() -> AnyPiece {
    // The report viewer text, refreshed whenever the pending list changes (send/discard/relaunch).
    let report = Signal::new(String::new());
    let pending = day_break::pending();
    Effect::new(move || {
        pending.get(); // track
        report.set(day_break::latest_report_text().unwrap_or_default());
    });

    let crash_controls = section((
        // Each crash is scheduled ~150 ms out so the dayscript tap gets its reply before we die.
        labeled(
            crate::res::str::crash_abort_label(),
            button(crate::res::str::crash_abort())
                .action(|| schedule(|| std::process::abort()))
                .id("crash-abort"),
        ),
        labeled(
            crate::res::str::crash_segv_label(),
            button(crate::res::str::crash_segv())
                .action(|| schedule(segfault))
                .id("crash-segv"),
        ),
        labeled(
            crate::res::str::crash_contained_label(),
            // Panics in a button handler run inside day-core's event pump, which CONTAINS the
            // panic (the app survives); it becomes a non-fatal report on the next launch.
            button(crate::res::str::crash_contained())
                .action(|| panic!("intentional contained panic from the showcase crash page"))
                .id("crash-contained"),
        ),
    ))
    .title(crate::res::str::crash_trigger_section());

    // What "Send report" will do, disclosed to the user (from the configured reporter).
    let disclosure = day_break::reporter_description().unwrap_or_default();
    let has_disclosure = !disclosure.is_empty();

    let report_view = section((
        // The report shown ONCE, in a scrollable text view; an empty-state line when there is none.
        when(
            move || !report.get().is_empty(),
            move || {
                text_area(report)
                    .min_lines(8)
                    .max_lines(20)
                    .id("crash-report")
                    .any()
            },
        ),
        when(
            move || report.get().is_empty(),
            move || {
                label(crate::res::str::crash_empty())
                    .id("crash-empty")
                    .any()
            },
        ),
        // Actions appear only when there is a report to act on: send it (opens an email), or clear.
        when(
            move || !report.get().is_empty(),
            move || {
                row((
                    button(crate::res::str::crash_send())
                        .action(send_newest)
                        .id("crash-send"),
                    button(crate::res::str::crash_clear())
                        .action(clear_reports)
                        .id("crash-clear"),
                ))
                .spacing(8.0)
                .any()
            },
        ),
        when(
            move || has_disclosure && !report.get().is_empty(),
            move || {
                label(disclosure.clone())
                    .font(Font::Caption2)
                    .id("crash-disclosure")
                    .any()
            },
        ),
    ))
    .title(crate::res::str::crash_report_section());

    page(
        crate::res::str::nav_crash(),
        "crash-title",
        Some(crate::res::str::crash_caption()),
        form((crash_controls, report_view)).any(),
    )
}

/// Run `crash` shortly after returning, so the caller (a button handler inside the event pump) can
/// finish and reply to the driving dayscript step before the process dies. The crash is
/// process-wide (abort / fault), so a worker thread is a fine place to fire it.
fn schedule(crash: fn()) {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(150));
        crash();
    });
}

fn segfault() {
    // A null write the optimizer can't elide.
    unsafe {
        let p = std::hint::black_box(std::ptr::null_mut::<u8>());
        std::ptr::write_volatile(p, 1u8);
    }
}

/// Send the newest pending report through the configured reporter (here, an email compose). The
/// email app opening is the feedback; the report clears from the pending list once handed off.
fn send_newest() {
    if let Some(meta) = day_break::pending().get_untracked().into_iter().next() {
        day_break::send(&meta, |_result| {});
    }
}

fn clear_reports() {
    for meta in day_break::pending().get_untracked() {
        day_break::discard(&meta);
    }
}
