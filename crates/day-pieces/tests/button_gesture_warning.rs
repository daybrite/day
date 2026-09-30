// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Synthetic controls: capture the process-global logger in a separate test binary.

use day_mock::MockToolkit;
use day_pieces::prelude::*;
use day_spec::{Event, NodeId, Point, WindowOptions};
use std::{cell::Cell, rc::Rc, sync::Mutex};

static WARNINGS: Mutex<Vec<String>> = Mutex::new(Vec::new());

#[test]
fn button_tap_warns_through_wrappers_without_changing_event_routing() {
    day_core::set_log_sink(|level, line| {
        if level == log::Level::Warn {
            WARNINGS.lock().unwrap().push(line.to_owned());
        }
    });
    day_core::init_logging();
    day_core::set_log_level(log::LevelFilter::Warn);
    let taps = Rc::new(Cell::new(0));
    let actions = Rc::new(Cell::new(0));
    let tap_count = taps.clone();
    let action_count = actions.clone();
    let (mock, probe) = MockToolkit::new();
    day_core::launch_with(mock, WindowOptions::default(), move || {
        column((
            button("Fixture tap")
                .id("direct-button")
                .on_tap(move || tap_count.set(tap_count.get() + 1)),
            button("Fixture wrapped tap")
                .id("wrapped-button")
                .padding(8.0)
                .frame(160.0, 40.0)
                .on_tap(|| {}),
            button("Fixture positioned tap")
                .id("positioned-button")
                .on_tap_at(|_| {}),
            button("Fixture action")
                .padding(8.0)
                .action(move || action_count.set(action_count.get() + 1)),
            label("Fixture label").on_tap(|| {}),
        ))
        .any()
    });

    let warnings = WARNINGS.lock().unwrap().clone();
    assert_eq!(warnings.len(), 3, "{warnings:?}");
    for id in ["direct-button", "wrapped-button", "positioned-button"] {
        assert!(warnings.iter().any(|line| {
            line.contains(id)
                && line.contains("tap gesture attached to native button")
                && line.contains("use .action(...)")
        }));
    }
    let buttons = probe.find_by_kind("day.button");
    probe.emit(NodeId(buttons[0].1.node), Event::Tap(Point::ZERO));
    assert_eq!(taps.get(), 1, "the warning must not discard existing taps");
    probe.emit(NodeId(buttons[3].1.node), Event::Pressed);
    assert_eq!(actions.get(), 1, "decorated native action must still fire");
    assert_eq!(
        probe
            .log()
            .iter()
            .filter(|line| line.contains("enable_gesture") && line.contains("Tap"))
            .count(),
        4,
        "only explicit tap handlers attach recognizers"
    );
    day_core::uninstall_tree();
}
