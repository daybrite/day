// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

#[cfg(not(target_os = "macos"))]
fn main() {}

/// Pass-through, hit slop and subtree enablement (docs/gestures.md, docs/forms.md), asked of
/// AppKit's own `hitTest:` the way `NSWindow` asks it for a click. No event is posted.
#[cfg(target_os = "macos")]
fn main() {
    use day_spec::props::{ButtonProps, ContainerProps, ImageProps, LabelProps};
    use day_spec::{Cap, GestureKind, Insets, NodeId, Support, Toolkit, kinds};
    use day_toolkit_appkit::AppKit;
    use objc2::rc::Retained;
    use objc2::{MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{
        NSApplication, NSBackingStoreType, NSButton, NSView, NSWindow, NSWindowStyleMask,
    };
    use objc2_foundation::{NSPoint, NSRect, NSSize};

    fn ptr(v: &NSView) -> usize {
        std::ptr::from_ref(v) as usize
    }
    fn frame(v: &NSView, x: f64, y: f64, w: f64, h: f64) {
        v.setFrame(NSRect::new(NSPoint::new(x, y), NSSize::new(w, h)));
    }

    let mtm = MainThreadMarker::new().expect("native test runs on main thread");
    let _app = NSApplication::sharedApplication(mtm);
    let mut toolkit = AppKit::new();
    toolkit.set_event_sink(Box::new(|_, _| {}));
    for (cap, want) in [
        (Cap::HitSlop, Support::Native),
        (Cap::Focusable, Support::Native),
        (Cap::KeyPhases, Support::Native),
    ] {
        assert_eq!(toolkit.capability(cap), want, "{cap:?}");
    }
    for (kind, want) in [
        (GestureKind::Press, Support::Native),
        (GestureKind::LongPress, Support::Native),
        (GestureKind::DoubleTap, Support::Native),
        (GestureKind::Touches, Support::Unsupported),
    ] {
        assert_eq!(toolkit.gesture_support(kind), want, "{kind:?}");
    }

    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(300.0, 200.0)),
            NSWindowStyleMask::Titled,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    unsafe { window.setReleasedWhenClosed(false) };
    let content = window.contentView().expect("content view");

    // A stack (flipped, 300×200 at the window's origin) holding a button with three passive
    // pieces over it: a canvas, a label and an image, each on the button's frame.
    let stack = toolkit.realize(kinds::CONTAINER, &ContainerProps::default(), NodeId(1));
    frame(&stack, 0.0, 0.0, 300.0, 200.0);
    content.addSubview(&stack);
    let button = toolkit.realize(
        kinds::BUTTON,
        &ButtonProps {
            title: "Under".into(),
            enabled: true,
            ..Default::default()
        },
        NodeId(2),
    );
    let canvas = toolkit.realize(kinds::CANVAS, &(), NodeId(3));
    let label = toolkit.realize(
        kinds::LABEL,
        &LabelProps {
            text: "decor".into(),
            ..Default::default()
        },
        NodeId(4),
    );
    let image = toolkit.realize(kinds::IMAGE, &ImageProps::default(), NodeId(5));
    for (i, v) in [&button, &canvas, &label, &image].into_iter().enumerate() {
        toolkit.insert(&stack, v, i);
        frame(v, 10.0, 10.0, 100.0, 40.0);
    }
    // The content view is not flipped: a point `y` down from the stack's top is
    // `200 - y` up from the window's bottom, which is where `hitTest:` measures it.
    let hit = |x: f64, y: f64| -> Option<Retained<NSView>> {
        content.hitTest(NSPoint::new(x, 200.0 - y))
    };
    let on_button = hit(50.0, 30.0).expect("something is under the point");
    assert_eq!(
        ptr(&on_button),
        ptr(&button),
        "a canvas, label and image with no handler let the press through to the button"
    );
    assert!(on_button.downcast_ref::<NSButton>().is_some());
    // An empty stack takes no press of its own.
    let empty = hit(200.0, 150.0).expect("the content view is under the point");
    assert_ne!(
        ptr(&empty),
        ptr(&stack),
        "a plain stack lets the press through"
    );

    // A tap on the canvas: the canvas takes the press (it is the front-most piece).
    toolkit.enable_gesture(&canvas, NodeId(3), GestureKind::Tap);
    assert_eq!(hit(50.0, 30.0).map(|v| ptr(&v)), Some(ptr(&canvas)));
    // A press observer alone also makes a view take the press.
    toolkit.enable_gesture(&image, NodeId(5), GestureKind::Press);
    assert_eq!(hit(50.0, 30.0).map(|v| ptr(&v)), Some(ptr(&image)));

    // Hit slop: a 12-point dot at (200, 100) with 30 points of slop takes a press 20 points
    // outside it, not one 40 points outside; its frame does not change.
    let dot = toolkit.realize(kinds::CANVAS, &(), NodeId(6));
    toolkit.insert(&stack, &dot, 4);
    frame(&dot, 200.0, 100.0, 12.0, 12.0);
    toolkit.enable_gesture(&dot, NodeId(6), GestureKind::Tap);
    assert_ne!(hit(186.0, 106.0).map(|v| ptr(&v)), Some(ptr(&dot)));
    toolkit.set_hit_slop(&dot, NodeId(6), Insets::all(30.0));
    assert_eq!(hit(186.0, 106.0).map(|v| ptr(&v)), Some(ptr(&dot)));
    assert_eq!(hit(206.0, 80.0).map(|v| ptr(&v)), Some(ptr(&dot)));
    assert_ne!(hit(160.0, 106.0).map(|v| ptr(&v)), Some(ptr(&dot)));
    assert_eq!(dot.frame().size.width, 12.0);
    toolkit.set_hit_slop(&dot, NodeId(6), Insets::ZERO);
    assert_ne!(hit(186.0, 106.0).map(|v| ptr(&v)), Some(ptr(&dot)));

    // A focusable stack takes focus and a press; disabled, it takes neither.
    let card = toolkit.realize(kinds::CONTAINER, &ContainerProps::default(), NodeId(7));
    toolkit.insert(&stack, &card, 5);
    frame(&card, 150.0, 150.0, 100.0, 40.0);
    assert_ne!(hit(200.0, 170.0).map(|v| ptr(&v)), Some(ptr(&card)));
    toolkit.set_focusable(&card, NodeId(7), true);
    assert!(card.acceptsFirstResponder());
    assert_eq!(hit(200.0, 170.0).map(|v| ptr(&v)), Some(ptr(&card)));

    // Disabling the whole stack: the button is off, nothing inside takes a press, the card
    // cannot take focus. Nested inside it, the card disabled too: enabling the stack leaves
    // the card's own group off until that is enabled as well.
    let control = button.downcast_ref::<NSButton>().expect("a button");
    toolkit.set_subtree_enabled(&stack, NodeId(1), false);
    toolkit.set_subtree_enabled(&card, NodeId(7), false);
    assert!(!control.isEnabled());
    assert!(hit(50.0, 30.0).is_none_or(|v| ptr(&v) == ptr(&content)));
    assert!(!card.acceptsFirstResponder());
    toolkit.set_subtree_enabled(&stack, NodeId(1), true);
    assert!(control.isEnabled(), "the button is back to its own state");
    assert_eq!(hit(50.0, 30.0).map(|v| ptr(&v)), Some(ptr(&image)));
    assert!(
        !card.acceptsFirstResponder(),
        "still inside a disabled group"
    );
    toolkit.set_subtree_enabled(&card, NodeId(7), true);
    assert!(card.acceptsFirstResponder());

    // A button that was disabled on its own stays disabled through a group's round trip.
    control.setEnabled(false);
    toolkit.set_subtree_enabled(&stack, NodeId(1), false);
    toolkit.set_subtree_enabled(&stack, NodeId(1), true);
    assert!(!control.isEnabled());

    window.close();
    println!("native hit testing: pass-through, slop, focus and disabled groups passed");
}
