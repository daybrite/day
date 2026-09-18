// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

#[cfg(not(target_os = "macos"))]
fn main() {}

/// Exercise the real field editor, without Accessibility permissions or synthetic mouse events.
#[cfg(target_os = "macos")]
fn main() {
    use std::{cell::RefCell, rc::Rc};

    use day_appkit::AppKit;
    use day_spec::props::{LabelPatch, LabelProps, TextAlign};
    use day_spec::{Cap, Event, Font, NodeId, Support, Toolkit, kinds, markdown};
    use objc2::runtime::NSObjectProtocol;
    use objc2::{MainThreadMarker, MainThreadOnly, sel};
    use objc2_app_kit::{
        NSApplication, NSBackingStoreType, NSTextField, NSTextView, NSWindow, NSWindowStyleMask,
    };
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

    let mtm = MainThreadMarker::new().expect("native test runs on main thread");
    let _app = NSApplication::sharedApplication(mtm);
    let mut toolkit = AppKit::new();
    assert_eq!(toolkit.capability(Cap::TextLinks), Support::Native);
    let events = Rc::new(RefCell::new(Vec::new()));
    let captured = events.clone();
    toolkit.set_event_sink(Box::new(move |node, event| {
        if let Event::LinkActivated(target) = event {
            captured.borrow_mut().push((node.0, target));
        }
    }));

    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(440.0, 240.0)),
            NSWindowStyleMask::Titled,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    unsafe { window.setReleasedWhenClosed(false) };

    // Both an initial link and a link added reactively must use the label as the field
    // editor's delegate. An NSTextField's separate control delegate does not receive this
    // callback; that used to send #navigate to NSWorkspace and produce system error -50.
    for reactive in [false, true] {
        let (text, runs) = markdown::parse("Hello 🌅 [**Navigate**](#navigate)", Font::Body);
        let props = LabelProps {
            text: if reactive {
                "Loading".into()
            } else {
                text.clone()
            },
            runs: if reactive { Vec::new() } else { runs.clone() },
            align: TextAlign::Center,
            ..Default::default()
        };
        let handle = toolkit.realize(kinds::LABEL, &props, NodeId(41));
        handle.setFrame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(440.0, 100.0),
        ));
        window.contentView().unwrap().addSubview(&handle);
        if reactive {
            toolkit.update(&handle, kinds::LABEL, &LabelPatch::Runs(text, runs), None);
        }
        let field = handle.downcast_ref::<NSTextField>().unwrap();
        assert!(field.isSelectable(), "links need native hit testing");
        assert_eq!(field.alignment(), objc2_app_kit::NSTextAlignment::Center);
        unsafe { field.selectText(None) };
        let editor = field.currentEditor().expect("label has a field editor");
        let editor = editor.downcast::<NSTextView>().unwrap();
        let delegate = editor.delegate().expect("field editor has a delegate");
        assert!(delegate.respondsToSelector(sel!(textView:clickedOnLink:atIndex:)));
        for target in ["#navigate", "#settings", "https://daybrite.dev/#docs"] {
            events.borrow_mut().clear();
            unsafe { editor.clickedOnLink_atIndex(&NSString::from_str(target), 9) };
            assert_eq!(*events.borrow(), [(41, target.to_string())]);
        }
        unsafe {
            window.endEditingFor(None);
            handle.removeFromSuperview();
        }
        toolkit.update(
            &handle,
            kinds::LABEL,
            &LabelPatch::Runs("Plain".into(), vec![]),
            None,
        );
        assert!(
            !field.isSelectable(),
            "removing links restores plain-label behavior"
        );
        toolkit.set_selectable(&handle, true);
        toolkit.update(
            &handle,
            kinds::LABEL,
            &LabelPatch::Text("Still selectable".into()),
            None,
        );
        assert!(
            field.isSelectable(),
            "explicit selection survives text updates"
        );
        toolkit.release(handle);
    }
    window.close();
    println!("native link activation: initial and reactive labels passed");
}
