// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

#[cfg(not(target_os = "macos"))]
fn main() {}

/// The toolbar search field's real field editor: typed text reports `Text` as it changes, and
/// Return reports `Submit` once, after it, without ending the edit. Driven through the field
/// editor's own entry points (`insertText:`, `doCommandBySelector:`), so it needs neither
/// Accessibility permissions nor synthetic key events.
#[cfg(target_os = "macos")]
fn main() {
    use std::{cell::RefCell, rc::Rc};

    use day_appkit::AppKit;
    use day_spec::{
        Event, ToolbarItem, ToolbarItemKind, ToolbarOp, ToolbarPatch, ToolbarValue, Toolkit,
    };
    use objc2::{MainThreadMarker, MainThreadOnly, msg_send, sel};
    use objc2_app_kit::{
        NSApplication, NSBackingStoreType, NSButton, NSButtonType, NSControlStateValueOff, NSEvent,
        NSEventModifierFlags, NSEventType, NSSearchToolbarItem, NSTextView, NSWindow,
        NSWindowStyleMask,
    };
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

    const ACTION: u64 = 7;

    /// One short turn of the run loop, for what the field defers to it.
    fn settle() {
        let until = objc2_foundation::NSDate::dateWithTimeIntervalSinceNow(0.6);
        objc2_foundation::NSRunLoop::currentRunLoop().runUntilDate(&until);
    }

    let mtm = MainThreadMarker::new().expect("native test runs on main thread");
    let _app = NSApplication::sharedApplication(mtm);
    let mut toolkit = AppKit::new();
    let values = Rc::new(RefCell::new(Vec::new()));
    let captured = values.clone();
    toolkit.set_event_sink(Box::new(move |_, event| {
        if let Event::ToolbarChanged { action, value } = event {
            assert_eq!(action, ACTION);
            captured.borrow_mut().push(value);
        }
    }));

    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(640.0, 400.0)),
            NSWindowStyleMask::Titled,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    unsafe { window.setReleasedWhenClosed(false) };
    let host = window.contentView().expect("window has a content view");

    let item = ToolbarItem {
        id: day_spec::SEARCH_ITEM_ID.into(),
        kind: ToolbarItemKind::Search {
            text: String::new(),
            placeholder: "Search".into(),
            suggestions: Vec::new(),
        },
        label: "Search".into(),
        tooltip: None,
        icon: None,
        enabled: true,
        action: ACTION,
        placement: Default::default(),
        label_style: Default::default(),
        prominent: false,
        column: Default::default(),
    };
    assert!(toolkit.edit_toolbar(&host, &[ToolbarOp::Insert { index: 0, item }]));
    let bar = window
        .toolbar()
        .expect("the edit installs a window toolbar");
    let search = bar
        .items()
        .iter()
        .find_map(|i| i.downcast::<NSSearchToolbarItem>().ok())
        .expect("a search item draws as NSSearchToolbarItem");
    let field = search.searchField();

    // Focus the field and type into its field editor, as the input system does.
    assert!(window.makeFirstResponder(Some(&field)));
    let editor = || {
        field
            .currentEditor()
            .expect("a focused field has a field editor")
            .downcast::<NSTextView>()
            .expect("the field editor is a text view")
    };
    let type_text = |text: &str| {
        let range = objc2_foundation::NSRange::new(usize::MAX, 0);
        let _: () = unsafe {
            msg_send![&*editor(), insertText: &*NSString::from_str(text), replacementRange: range]
        };
    };
    let press_return = || {
        let _: () = unsafe { msg_send![&*editor(), doCommandBySelector: sel!(insertNewline:)] };
    };

    // NSSearchField defers its action to the run loop, longer for a short string. Return
    // pressed before it fires reports the text first, then the submission, and the deferred
    // action that follows has nothing new to say.
    for ch in ["a", "l", "i", "c", "e"] {
        type_text(ch);
    }
    press_return();
    assert_eq!(
        *values.borrow(),
        [
            ToolbarValue::Text("alice".into()),
            ToolbarValue::Submit("alice".into())
        ],
        "Return reports the unsent text, then submits"
    );
    assert!(
        field.currentEditor().is_some(),
        "Return keeps the edit going in the field"
    );
    settle();
    assert_eq!(values.borrow().len(), 2, "no echo of the submitted text");
    values.borrow_mut().clear();

    // Each edit reports the field's full text once the action fires; Return after it is the
    // submission alone.
    type_text(" in");
    settle();
    assert_eq!(*values.borrow(), [ToolbarValue::Text("alice in".into())]);
    press_return();
    assert_eq!(
        *values.borrow(),
        [
            ToolbarValue::Text("alice in".into()),
            ToolbarValue::Submit("alice in".into())
        ]
    );
    values.borrow_mut().clear();

    // Day sets the text (the model re-seeding the field): a Return on it reports no edit.
    toolkit.update_toolbar(
        &host,
        &ToolbarPatch::Text {
            item: day_spec::SEARCH_ITEM_ID.into(),
            text: "carroll".into(),
        },
    );
    assert_eq!(field.stringValue().to_string(), "carroll");

    // A Return key-down through the window's own event routing, with a default button in the
    // window: the field claims the key, so the button never sees it.
    let button = unsafe {
        NSButton::buttonWithTitle_target_action(&NSString::from_str("OK"), None, None, mtm)
    };
    button.setButtonType(NSButtonType::PushOnPushOff);
    button.setKeyEquivalent(&NSString::from_str("\r"));
    host.addSubview(&button);
    window.setDefaultButtonCell(button.cell().and_then(|c| c.downcast().ok()).as_deref());
    let key = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
        NSEventType::KeyDown,
        NSPoint::new(0.0, 0.0),
        NSEventModifierFlags::empty(),
        0.0,
        window.windowNumber(),
        None,
        &NSString::from_str("\r"),
        &NSString::from_str("\r"),
        false,
        36,
    )
    .expect("a Return key-down event");
    window.sendEvent(&key);
    assert_eq!(*values.borrow(), [ToolbarValue::Submit("carroll".into())]);
    assert_eq!(
        button.state(),
        NSControlStateValueOff,
        "Return in the field leaves the default button alone"
    );
    assert!(field.currentEditor().is_some());

    unsafe { window.endEditingFor(None) };
    assert!(toolkit.edit_toolbar(
        &host,
        &[ToolbarOp::Remove {
            id: day_spec::SEARCH_ITEM_ID.into()
        }]
    ));
    window.close();
    println!("native toolbar search: typed text and Return submission passed");
}
