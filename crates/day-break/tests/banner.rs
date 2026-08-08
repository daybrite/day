// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Mock-backend e2e for the consent banner (feature `ui`, on by default). Seeds a finalized report
//! on disk, arms day-break, mounts `consent_banner` under the mock toolkit, and asserts it discloses
//! the report and that Discard removes it. The full cross-platform drive is the showcase page's
//! dayscript (M4); this pins the piece wiring in-process.

use day_break::Report;
use day_mock::{MockProbe, MockToolkit};
use day_reactive::flush_sync;
use day_spec::{Event, NodeId, Size, WindowOptions};

fn boot(root: impl FnOnce() -> day_core::AnyPiece + 'static) -> MockProbe {
    day_core::uninstall_tree();
    let (mock, probe) = MockToolkit::new();
    let options = WindowOptions {
        title: "test".into(),
        size: Size::new(400.0, 640.0),
        ..Default::default()
    };
    day_core::launch_with(mock, options, root);
    probe
}

fn labels(probe: &MockProbe) -> Vec<String> {
    probe
        .find_by_kind("day.label")
        .into_iter()
        .map(|(_, w)| w.text)
        .collect()
}

fn tap_button(probe: &MockProbe, text: &str) {
    let btn = probe
        .find_by_kind("day.button")
        .into_iter()
        .find(|(_, w)| w.text == text)
        .unwrap_or_else(|| {
            panic!(
                "button {text:?} not found; have {:?}",
                probe
                    .find_by_kind("day.button")
                    .iter()
                    .map(|(_, w)| w.text.clone())
                    .collect::<Vec<_>>()
            )
        });
    probe.emit(NodeId(btn.1.node), Event::Pressed);
    flush_sync();
}

#[test]
fn banner_discloses_a_pending_report_and_discard_clears_it() {
    // A scratch dir with one seeded finalized report.
    let dir = std::env::temp_dir().join(format!("day-break-banner-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("reports")).unwrap();
    let report = Report {
        kind_str: "panic".into(),
        fatal: true,
        message: "kaboom in the widget".into(),
        location: "src/w.rs:3:1".into(),
        ..Default::default()
    };
    std::fs::write(dir.join("reports/report-1000.json"), report.to_json()).unwrap();

    day_break::Config::new()
        .dir(&dir)
        .app_id("dev.test.banner")
        .init()
        .expect("init");

    let probe = boot(day_break::consent_banner);
    flush_sync();

    // The banner discloses the crash (en title) and the report body once expanded.
    let shown = labels(&probe);
    assert!(
        shown.iter().any(|t| t.contains("A previous run crashed")),
        "title missing: {shown:?}"
    );

    // Expand the report and confirm the message text is on screen.
    tap_button(&probe, "View report");
    let shown = labels(&probe);
    assert!(
        shown.iter().any(|t| t.contains("kaboom in the widget")),
        "report body missing after View: {shown:?}"
    );

    // Discard removes it — the whole banner (title included) unmounts.
    tap_button(&probe, "Discard");
    let shown = labels(&probe);
    assert!(
        !shown.iter().any(|t| t.contains("A previous run crashed")),
        "banner still present after Discard: {shown:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
