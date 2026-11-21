// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The derived recognizers end to end (docs/gestures.md "Recognizers"): the mock answers
//! `gesture_support` the way a backend without press, long-press, double-tap or touch
//! recognizers does, so everything here comes out of day-core's own derivation from the tap
//! and drag events, through the modifiers an app writes.

use day_mock::MockToolkit;
use day_pieces::prelude::*;
use day_spec::{Event, NodeId, Point, WindowOptions};
use std::{cell::RefCell, rc::Rc};

fn drag(phase: DragPhase, x: f64, y: f64, from: (f64, f64)) -> Event {
    Event::Drag {
        phase,
        location: Point::new(x, y),
        translation: Point::new(x - from.0, y - from.1),
    }
}

#[test]
fn press_touches_and_axis_come_from_taps_and_drags() {
    let log = Rc::new(RefCell::new(Vec::<String>::new()));
    let (mock, probe) = MockToolkit::new();
    let page_log = log.clone();
    // The nodes the modifiers registered on: where a backend addresses the events.
    let nodes = Rc::new(RefCell::new(Vec::<NodeId>::new()));
    let page_nodes = nodes.clone();
    day_core::launch_with(mock, WindowOptions::default(), move || {
        let note = move |line: String| page_log.borrow_mut().push(line);
        let (first, second) = (page_nodes.clone(), page_nodes.clone());
        let (a, b, c, d, e) = (
            note.clone(),
            note.clone(),
            note.clone(),
            note.clone(),
            note.clone(),
        );
        column((
            rectangle()
                .frame(100.0, 60.0)
                .tweak(move |n| first.borrow_mut().push(day_core::rnode_to_id(n)))
                .hit_slop(12.0)
                .on_pressed(move |down| a(format!("pressed {down}")))
                .on_touches(move |t: &[Touch]| b(format!("touch {:?}", t[0].phase)))
                .on_tap(move || c("tap".into()))
                .on_double_tap(move || d("double".into()))
                .id(day_core::id!(pad)),
            rectangle()
                .frame(100.0, 60.0)
                .tweak(move |n| second.borrow_mut().push(day_core::rnode_to_id(n)))
                .on_drag_along(DragAxis::Horizontal, move |drag: Drag| {
                    e(format!("drag {:?} {:.0}", drag.phase, drag.translation.x));
                })
                .id(day_core::id!(slider)),
        ))
        .any()
    });

    // The mock has no recognizer for any of the four: day-core asked for taps and drags.
    let calls = probe.log();
    let asked = |kind: &str| {
        calls
            .iter()
            .any(|l| l.starts_with("enable_gesture") && l.ends_with(kind))
    };
    assert!(asked("Tap") && asked("Drag"), "{calls:?}");
    assert!(
        !asked("Press") && !asked("Touches") && !asked("DoubleTap"),
        "{calls:?}"
    );
    assert_eq!(
        day_core::gesture_support(GestureKind::Press),
        day_spec::Support::Emulated
    );
    assert_eq!(
        day_core::gesture_support(GestureKind::Drag),
        day_spec::Support::Native
    );

    let canvases = probe.find_by_kind("day.canvas");
    let (pad, slider) = (nodes.borrow()[0], nodes.borrow()[1]);
    assert_eq!(canvases[0].1.hit_slop, Insets::all(12.0));
    assert_eq!(canvases[1].1.drag_axis, Some(DragAxis::Horizontal));

    // A press-drag is a press, one contact, and (for this piece, with no drag handler) nothing
    // else.
    probe.emit(pad, drag(DragPhase::Began, 10.0, 10.0, (10.0, 10.0)));
    probe.emit(pad, drag(DragPhase::Changed, 30.0, 10.0, (10.0, 10.0)));
    probe.emit(pad, drag(DragPhase::Ended, 30.0, 10.0, (10.0, 10.0)));
    assert_eq!(
        *log.borrow(),
        [
            "pressed true",
            "touch Began",
            "touch Moved",
            "pressed false",
            "touch Ended"
        ]
    );
    log.borrow_mut().clear();

    // A tap after a press that just ended belongs to that press: no second pair. (The
    // double-tap interval itself is timed, and a headless run has no clock to wait on; the
    // recognizer's own tests in day-core cover it.)
    probe.emit(pad, Event::Tap(Point::new(5.0, 5.0)));
    assert!(
        !log.borrow().iter().any(|l| l.starts_with("pressed")),
        "{:?}",
        log.borrow()
    );
    log.borrow_mut().clear();

    // A vertical drag is not the horizontal slider's; a horizontal one is, from `Began`.
    probe.emit(slider, drag(DragPhase::Began, 50.0, 10.0, (50.0, 10.0)));
    probe.emit(slider, drag(DragPhase::Changed, 51.0, 40.0, (50.0, 10.0)));
    probe.emit(slider, drag(DragPhase::Ended, 51.0, 40.0, (50.0, 10.0)));
    assert!(log.borrow().is_empty(), "{:?}", log.borrow());
    probe.emit(slider, drag(DragPhase::Began, 10.0, 30.0, (10.0, 30.0)));
    probe.emit(slider, drag(DragPhase::Changed, 40.0, 31.0, (10.0, 30.0)));
    probe.emit(slider, drag(DragPhase::Ended, 40.0, 31.0, (10.0, 30.0)));
    assert_eq!(
        *log.borrow(),
        ["drag Began 0", "drag Changed 30", "drag Ended 30"]
    );
}

/// A piece that only follows its press hands its taps and drags to the piece around it, and
/// they arrive as if they had landed there (docs/gestures.md "Press"): in that piece's
/// coordinates, through its drag axis and its double tap, with the release velocity intact.
#[test]
fn a_watched_gesture_reaches_the_piece_around_it_as_its_own() {
    let log = Rc::new(RefCell::new(Vec::<String>::new()));
    let (mock, probe) = MockToolkit::new();
    let child = Rc::new(RefCell::new(None::<NodeId>));
    let (page_log, page_child) = (log.clone(), child.clone());
    day_core::launch_with(mock, WindowOptions::default(), move || {
        let (a, b, c) = (page_log.clone(), page_log.clone(), page_log.clone());
        column((
            spacer().frame(100.0, 30.0),
            rectangle()
                .frame(80.0, 40.0)
                .tweak(move |n| *page_child.borrow_mut() = Some(day_core::rnode_to_id(n)))
                .on_pressed(move |down| a.borrow_mut().push(format!("pressed {down}"))),
        ))
        .on_drag_along(DragAxis::Horizontal, move |d: Drag| {
            b.borrow_mut().push(format!(
                "drag {:?} at {:.0},{:.0} v {:.0}",
                d.phase, d.location.x, d.location.y, d.velocity.x
            ));
        })
        .on_double_tap(move || c.borrow_mut().push("double".into()))
        .any()
    });
    let child = child.borrow().expect("the child registered its press");
    let below = |line: &str| log.borrow().iter().any(|l| l == line);

    // Two taps on the child are the parent's double tap. (First: a headless run has no clock,
    // and the release of a press-drag swallows the tap that follows it.)
    probe.emit(child, Event::Tap(Point::new(5.0, 5.0)));
    probe.emit(child, Event::Tap(Point::new(6.0, 5.0)));
    assert!(below("double"), "{:?}", log.borrow());
    log.borrow_mut().clear();

    // Vertical: the horizontal parent never sees it, though the child's press still does.
    probe.emit(child, drag(DragPhase::Began, 5.0, 5.0, (5.0, 5.0)));
    probe.emit(child, drag(DragPhase::Changed, 6.0, 35.0, (5.0, 5.0)));
    probe.emit(child, drag(DragPhase::Ended, 6.0, 35.0, (5.0, 5.0)));
    assert!(
        !log.borrow().iter().any(|l| l.starts_with("drag")),
        "{:?}",
        log.borrow()
    );
    assert!(below("pressed true") && below("pressed false"));
    log.borrow_mut().clear();

    // Horizontal, with the backend's own release velocity: the parent's, in its coordinates
    // (the column centers the 80-point child under the 100-point spacer, 30 points down), at
    // the velocity the native recognizer measured.
    probe.emit(child, drag(DragPhase::Began, 5.0, 5.0, (5.0, 5.0)));
    probe.emit(child, drag(DragPhase::Changed, 35.0, 6.0, (5.0, 5.0)));
    probe.emit(
        child,
        Event::GestureVelocity {
            kind: GestureKind::Drag,
            velocity: Point::new(500.0, 0.0),
        },
    );
    probe.emit(child, drag(DragPhase::Ended, 35.0, 6.0, (5.0, 5.0)));
    let drags: Vec<String> = log
        .borrow()
        .iter()
        .filter(|l| l.starts_with("drag"))
        .cloned()
        .collect();
    assert_eq!(drags.len(), 3, "{drags:?}");
    assert_eq!(drags[2], "drag Ended at 45,36 v 500", "{drags:?}");
    log.borrow_mut().clear();

    day_core::uninstall_tree();
}
