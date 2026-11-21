// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
fn main() {
    native::run();
}

/// `follow_end` and the reader's own scrolls on a real AppKit list (docs/list.md "Following
/// the end", `Cap::ListReaderScroll`). Rows arrive while a followed scroll is on its way, and
/// the reader takes the list with a mouse wheel, a trackpad gesture, a key or the scroller:
/// the list must stay where the reader put it, and follow again once they are back at the end.
/// The events are synthetic `NSEvent`s handed to the list's own views inside this process,
/// not input posted to the screen.
#[cfg(target_os = "macos")]
mod native {
    use day_pieces::prelude::*;
    use day_spec::{Size, WindowOptions};
    use day_toolkit_appkit::AppKit;
    use objc2::{MainThreadMarker, Message, rc::Retained};
    use objc2_app_kit::{
        NSApplication, NSEvent, NSEventModifierFlags, NSEventType, NSScrollView,
        NSScrollViewDidEndLiveScrollNotification, NSScrollerPart, NSTableView, NSView, NSWindow,
    };
    use objc2_core_graphics::{
        CGEvent, CGEventField, CGEventSource, CGEventSourceStateID, CGScrollEventUnit,
    };
    use objc2_foundation::{NSNotificationCenter, NSPoint, NSString};

    /// Every row is this tall, and the list shows a little under seven of them.
    const ROW: f64 = 30.0;

    fn find_table(view: &NSView) -> Option<Retained<NSTableView>> {
        if let Some(table) = view.downcast_ref::<NSTableView>() {
            return Some(table.retain());
        }
        view.subviews().iter().find_map(|v| find_table(&v))
    }

    fn window() -> Retained<NSWindow> {
        let app = NSApplication::sharedApplication(MainThreadMarker::new().unwrap());
        app.windows().objectAtIndex(0)
    }

    fn check(ok: bool, what: &str) {
        if !ok {
            eprintln!("native list reader scroll: FAILED {what}");
            std::process::exit(1);
        }
        println!("native list reader scroll: {what}");
    }

    /// How far the end of the last row is below the viewport's end.
    fn distance(table: &NSTableView) -> f64 {
        let rows = table.numberOfRows();
        let end = table.rectOfRow(rows - 1);
        let visible = table.visibleRect();
        end.origin.y + end.size.height - (visible.origin.y + visible.size.height)
    }

    fn origin_y(scroll: &NSScrollView) -> f64 {
        scroll.contentView().bounds().origin.y
    }

    /// A wheel event: `lines` notches of a mouse wheel (positive scrolls toward the top), or,
    /// with a `phase`, one event of a trackpad gesture moving `lines` points.
    fn wheel(scroll: &NSScrollView, lines: i32, phase: Option<i64>) {
        let unit = if phase.is_some() {
            CGScrollEventUnit::Pixel
        } else {
            CGScrollEventUnit::Line
        };
        // Made from a private event source: one made from the session's combined state now and
        // then comes back with no delta at all (every delta field 0, for a while), and moves
        // nothing.
        let source = CGEventSource::new(CGEventSourceStateID::Private);
        let cg = CGEvent::new_scroll_wheel_event2(source.as_deref(), unit, 1, lines, 0, 0).unwrap();
        if let Some(phase) = phase {
            CGEvent::set_integer_value_field(
                Some(&cg),
                CGEventField::ScrollWheelEventIsContinuous,
                1,
            );
            CGEvent::set_integer_value_field(
                Some(&cg),
                CGEventField::ScrollWheelEventScrollPhase,
                phase,
            );
        }
        let event = NSEvent::eventWithCGEvent(&cg).unwrap();
        if lines != 0 && event.scrollingDeltaY() == 0. {
            eprintln!("native list reader scroll: FAILED the synthetic wheel event has no delta");
            std::process::exit(1);
        }
        scroll.scrollWheel(&event);
    }

    /// Keep a trackpad gesture's fingers on the pad until the returned flag is cleared: a
    /// gesture still moving goes on sending events, and AppKit ends the live scroll of one
    /// that falls silent for most of a second, fingers down or not.
    fn hold_gesture(scroll: Retained<NSScrollView>) -> std::rc::Rc<std::cell::Cell<bool>> {
        let on = std::rc::Rc::new(std::cell::Cell::new(true));
        let held = on.clone();
        day_core::task(async move {
            while held.get() {
                day_core::sleep(100).await;
                if held.get() {
                    wheel(&scroll, 0, Some(2)); // changed, the fingers at rest
                }
            }
        });
        on
    }

    /// A key press the table handles as the focused view does: Home, End.
    fn key(table: &NSTableView, code: u16, character: u32) {
        let text = NSString::from_str(&char::from_u32(character).unwrap().to_string());
        let event = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
            NSEventType::KeyDown,
            NSPoint::new(0., 0.),
            NSEventModifierFlags::Function,
            0.,
            window().windowNumber(),
            None,
            &text,
            &text,
            false,
            code,
        )
        .unwrap();
        table.keyDown(&event);
    }

    fn mouse(kind: NSEventType, at: NSPoint) -> Retained<NSEvent> {
        NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
            kind,
            at,
            NSEventModifierFlags::empty(),
            0.,
            window().windowNumber(),
            None,
            0,
            1,
            1.,
        )
        .unwrap()
    }

    /// Drag the vertical scroller's knob `by` points toward the top. The drag and the release
    /// are queued first, so the scroller's tracking loop finds them.
    fn drag_knob(scroll: &NSScrollView, by: f64) {
        let scroller = scroll.verticalScroller().unwrap();
        let knob = scroller.rectForPart(NSScrollerPart::Knob);
        let center = NSPoint::new(
            knob.origin.x + knob.size.width / 2.,
            knob.origin.y + knob.size.height / 2.,
        );
        let from = scroller.convertPoint_toView(center, None);
        // The window's coordinates grow upward.
        let to = NSPoint::new(from.x, from.y + by);
        let app = NSApplication::sharedApplication(MainThreadMarker::new().unwrap());
        app.postEvent_atStart(&mouse(NSEventType::LeftMouseDragged, to), false);
        app.postEvent_atStart(&mouse(NSEventType::LeftMouseUp, to), false);
        scroller.mouseDown(&mouse(NSEventType::LeftMouseDown, from));
    }

    /// Append `count` rows, one every 40 ms: each lands while the list's scroll to the one
    /// before is still on its way.
    async fn rows_arrive(rows: Signal<Vec<u64>>, count: usize) {
        for _ in 0..count {
            rows.update(|v| v.push(v.len() as u64));
            day_core::sleep(40).await;
        }
    }

    /// Ten rows at once, then a moment: the list's followed scroll to them is on its way,
    /// and still further from the end than `follow_end`'s threshold.
    async fn follow_in_flight(rows: Signal<Vec<u64>>) {
        rows.update(|v| v.extend(v.len() as u64..v.len() as u64 + 10));
        day_core::sleep(50).await;
    }

    /// After the reader took the list: more rows arrive, and the list stays at `left`, far
    /// from its end.
    async fn stays(table: &NSTableView, rows: Signal<Vec<u64>>, left: f64, what: &str) {
        let scroll = table.enclosingScrollView().unwrap();
        rows_arrive(rows, 6).await;
        day_core::sleep(700).await;
        let (top, end) = (origin_y(&scroll), distance(table));
        check(
            (top - left).abs() < 1. && end > 5. * ROW,
            &format!(
                "after {what}, the list stays where the reader put it (at {top:.0}, left at {left:.0}, {end:.0} from the end)"
            ),
        );
    }

    /// The list is at its end again: the next rows are followed.
    async fn follows(table: &NSTableView, rows: Signal<Vec<u64>>, what: &str) {
        check(
            distance(table) < 1.,
            &format!("{what} takes the list back to its end"),
        );
        rows_arrive(rows, 3).await;
        day_core::sleep(700).await;
        let end = distance(table);
        check(
            end < 1.,
            &format!("after {what}, the list follows again ({end:.0} from the end)"),
        );
    }

    pub fn run() {
        std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::from_secs(40));
            eprintln!("native list reader scroll test timed out");
            std::process::exit(2);
        });
        let rows = Signal::new((0..300_u64).collect::<Vec<_>>());
        let end = day_reactive::Trigger::new();
        day_core::launch_with(
            AppKit::new(),
            WindowOptions {
                title: "Fixture reader scroll".into(),
                size: Size::new(400., 300.),
                ..Default::default()
            },
            move || {
                day_core::task(async move {
                    day_core::sleep(300).await;
                    let table = find_table(&window().contentView().unwrap()).unwrap();
                    let scroll = table.enclosingScrollView().unwrap();
                    // AppKit may observe the view through a key-value-observing subclass
                    // named after it.
                    check(
                        scroll
                            .class()
                            .name()
                            .to_str()
                            .is_ok_and(|n| n.ends_with("DayListScrollView")),
                        "the list's scroll view watches the reader's wheel events",
                    );
                    end.notify();
                    day_core::sleep(700).await;
                    check(distance(&table) < 1., "the list jumps to its end");
                    rows_arrive(rows, 12).await;
                    day_core::sleep(700).await;
                    check(
                        distance(&table) < 1.,
                        "rows arriving during the list's own scrolls are all followed",
                    );

                    // A mouse wheel.
                    follow_in_flight(rows).await;
                    wheel(&scroll, 10, None);
                    stays(&table, rows, origin_y(&scroll), "the wheel").await;
                    wheel(&scroll, -1000, None);
                    follows(&table, rows, "the wheel").await;
                    // A wheel pushing at the end leaves the list following.
                    wheel(&scroll, -5, None);
                    follows(&table, rows, "a wheel pushing at the end").await;

                    // Keys, with the list focused.
                    window().makeFirstResponder(Some(&table));
                    follow_in_flight(rows).await;
                    key(&table, 115, 0xF729); // Home
                    check(origin_y(&scroll) < 1., "Home takes the list to its top");
                    stays(&table, rows, 0., "Home").await;
                    key(&table, 119, 0xF72B); // End
                    follows(&table, rows, "End").await;

                    // The scroller's knob.
                    follow_in_flight(rows).await;
                    drag_knob(&scroll, 60.);
                    check(distance(&table) > 20. * ROW, "the knob moves the list up");
                    stays(&table, rows, origin_y(&scroll), "the knob").await;
                    end.notify();
                    day_core::sleep(700).await;
                    follows(&table, rows, "the app").await;

                    // A trackpad gesture, its fingers still down. AppKit moves the list for it on
                    // a later turn. Last: AppKit's concurrent scrolling waits on for the rest of
                    // a gesture these few events never finish.
                    follow_in_flight(rows).await;
                    // Measured from where the gesture starts, not from the end: on a slow
                    // runner the followed scroll is over before the fingers land, and the list
                    // moves by the gesture alone. AppKit moves it by the delta of the event that
                    // begins the gesture (it takes the rest of a gesture from the event queue,
                    // not from these direct calls), so that one carries more than five rows.
                    let start = origin_y(&scroll);
                    wheel(&scroll, 0, Some(128)); // may begin: fingers down
                    wheel(&scroll, 240, Some(1)); // began
                    wheel(&scroll, 120, Some(2)); // changed
                    let fingers = hold_gesture(scroll.clone());
                    // AppKit applies the changes on a later turn; a loaded runner takes longer.
                    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
                    while start - origin_y(&scroll) <= 5. * ROW
                        && std::time::Instant::now() < deadline
                    {
                        day_core::sleep(50).await;
                    }
                    let moved = start - origin_y(&scroll);
                    check(
                        moved > 5. * ROW,
                        &format!("the gesture moves the list up ({moved:.0} points)"),
                    );
                    let left = origin_y(&scroll);
                    stays(&table, rows, left, "a gesture").await;
                    end.notify();
                    day_core::sleep(700).await;
                    check(
                        (origin_y(&scroll) - left).abs() < 1.,
                        "the app's scroll waits while the reader's gesture holds the list",
                    );
                    // The fingers lift. With no momentum to come, AppKit may end the live
                    // scroll on its own, a moment later or never (it waits on for the rest of
                    // the gesture): not before this point, which is why the gesture stays open
                    // until the check above.
                    fingers.set(false);
                    wheel(&scroll, 0, Some(4)); // ended
                    // What AppKit posts when a real gesture's momentum ends.
                    // SAFETY: AppKit's own notification name, with the scroll view it is about
                    // as the object, as AppKit posts it.
                    unsafe {
                        NSNotificationCenter::defaultCenter().postNotificationName_object(
                            NSScrollViewDidEndLiveScrollNotification,
                            Some(&scroll),
                        );
                    }
                    end.notify();
                    day_core::sleep(700).await;
                    follows(&table, rows, "the app once the gesture is over").await;
                    std::process::exit(0);
                });
                list(items(move || rows.get(), |r: &u64| *r), move |slot| {
                    label(move || format!("Row {}", slot.get())).grow(Dimension::Width)
                })
                .row_height(RowHeight::Uniform(ROW))
                .follow_end(40.)
                .scroll_to_end(end)
                .grow(Dimension::Both)
                .id(day_core::id!(reader_list))
                .any()
            },
        );
    }
}
