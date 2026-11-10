// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

#[cfg(not(target_os = "macos"))]
fn main() {}

/// A text field's selection requests and claimed keys against the real field editor
/// (docs/textfield.md "Selection" and "Keys"): a selection lands in UTF-16 units on the editor,
/// and with a key handler declared the field editor's `moveUp:`, `moveDown:` and
/// `cancelOperation:` commands become `Event::Key` and leave the caret where it was. Driven
/// through the field editor's own entry points, so it needs neither Accessibility permissions
/// nor synthetic key events.
#[cfg(target_os = "macos")]
fn main() {
    use std::{cell::RefCell, rc::Rc};

    use day_spec::props::{TextFieldPatch, TextFieldProps, TextSelection};
    use day_spec::{Event, NodeId, Toolkit, kinds};
    use day_toolkit_appkit::AppKit;
    use objc2::{MainThreadMarker, MainThreadOnly, sel};
    use objc2_app_kit::{
        NSApplication, NSBackingStoreType, NSStandardKeyBindingResponding as _, NSTextField,
        NSTextView, NSWindow, NSWindowStyleMask,
    };
    use objc2_foundation::{NSPoint, NSRange, NSRect, NSSize};

    const BAR: NodeId = NodeId(51);
    const PLAIN: NodeId = NodeId(52);

    let mtm = MainThreadMarker::new().expect("native test runs on main thread");
    let _app = NSApplication::sharedApplication(mtm);
    let mut toolkit = AppKit::new();
    let keys = Rc::new(RefCell::new(Vec::new()));
    let captured = keys.clone();
    toolkit.set_event_sink(Box::new(move |node, event| {
        if let Event::Key(k) = event {
            captured.borrow_mut().push((node, k.key));
        }
    }));

    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(440.0, 120.0)),
            NSWindowStyleMask::Titled,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    unsafe { window.setReleasedWhenClosed(false) };
    let content = window.contentView().expect("window has a content view");

    // The address bar has a key handler; the plain field does not.
    day_spec::keys::mark(BAR);
    for (node, y) in [(BAR, 0.0), (PLAIN, 40.0)] {
        let props = TextFieldProps {
            text: String::new(),
            placeholder: String::new(),
            enabled: true,
        };
        let handle = toolkit.realize(kinds::TEXT_FIELD, &props, node);
        handle.setFrame(NSRect::new(NSPoint::new(0.0, y), NSSize::new(400.0, 24.0)));
        content.addSubview(&handle);
        let field = handle.downcast_ref::<NSTextField>().unwrap();

        // An unfocused field has no field editor and so no selection: the request is a no-op.
        let set_text = |toolkit: &mut AppKit, text: &str| {
            toolkit.update(
                &handle,
                kinds::TEXT_FIELD,
                &TextFieldPatch::Text {
                    text: text.into(),
                    from_native: false,
                },
                None,
            )
        };
        let select = |toolkit: &mut AppKit, start: usize, end: usize| {
            toolkit.update(
                &handle,
                kinds::TEXT_FIELD,
                &TextFieldPatch::Select(TextSelection::new(start, end)),
                None,
            )
        };
        set_text(&mut toolkit, "github.com");
        select(&mut toolkit, 3, 10);
        assert!(field.currentEditor().is_none());

        assert!(window.makeFirstResponder(Some(field)));
        let editor = field
            .currentEditor()
            .expect("a focused field has a field editor")
            .downcast::<NSTextView>()
            .unwrap();

        // Inline completion: the text the user did not type is selected, caret after "git".
        select(&mut toolkit, 3, usize::MAX);
        assert_eq!(editor.selectedRange(), NSRange::new(3, 7));
        // Again, unchanged: a request applies every time.
        editor.setSelectedRange(NSRange::new(0, 0));
        select(&mut toolkit, 3, usize::MAX);
        assert_eq!(editor.selectedRange(), NSRange::new(3, 7));

        // Characters, converted to UTF-16 units: the emoji is one character and two units.
        set_text(&mut toolkit, "a😀b.example");
        select(&mut toolkit, 2, 3);
        assert_eq!(editor.selectedRange(), NSRange::new(3, 1));
        select(&mut toolkit, 0, usize::MAX);
        assert_eq!(editor.selectedRange(), NSRange::new(0, 12));

        // The arrows and Escape: claimed by the bar, the field's own on the plain field. The
        // caret goes after the fourth character, the fifth UTF-16 unit.
        select(&mut toolkit, 4, 4);
        assert_eq!(editor.selectedRange(), NSRange::new(5, 0));
        keys.borrow_mut().clear();
        for command in [sel!(moveDown:), sel!(moveUp:), sel!(cancelOperation:)] {
            unsafe { editor.doCommandBySelector(command) };
        }
        if node == BAR {
            assert_eq!(
                *keys.borrow(),
                [
                    (BAR, "ArrowDown".to_owned()),
                    (BAR, "ArrowUp".to_owned()),
                    (BAR, "Escape".to_owned()),
                ]
            );
            assert_eq!(
                editor.selectedRange(),
                NSRange::new(5, 0),
                "a claimed arrow leaves the caret alone"
            );
        } else {
            assert!(keys.borrow().is_empty(), "a plain field reports no keys");
            assert_ne!(
                editor.selectedRange(),
                NSRange::new(5, 0),
                "the field editor moved the caret itself"
            );
        }
        // Other commands stay the field editor's either way.
        select(&mut toolkit, 4, 4);
        unsafe { editor.doCommandBySelector(sel!(moveToBeginningOfLine:)) };
        assert_eq!(editor.selectedRange(), NSRange::new(0, 0));
        assert!(keys.borrow().len() <= 3);

        unsafe { window.endEditingFor(None) };
    }
    window.close();
    println!("native text field: selection and claimed keys passed");
}
