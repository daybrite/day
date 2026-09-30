// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
fn main() {
    use day_appkit::AppKit;
    use day_spec::{Event, NodeId, Toolkit, applications::*, kinds, props::ListProps};
    use objc2::rc::Retained;
    use objc2::{MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{
        NSApplication, NSBackingStoreType, NSScrollView, NSTableView, NSWindow, NSWindowStyleMask,
    };
    use objc2_foundation::{NSPoint, NSRect, NSSize};
    use std::{
        cell::RefCell,
        rc::Rc,
        sync::{Arc, Mutex},
    };

    let mtm = MainThreadMarker::new().unwrap();
    let _app = NSApplication::sharedApplication(mtm);
    let mut toolkit = AppKit::new();
    for query in [
        HandlerQuery::Scheme("https".into()),
        HandlerQuery::Scheme("mailto".into()),
        HandlerQuery::MimeType("application/pdf".into()),
        HandlerQuery::Extension("pdf".into()),
        HandlerQuery::Url("https://example.invalid/".into()),
    ] {
        let handlers = toolkit.application_handlers(&query).unwrap();
        if let Some(default) = handlers.default {
            assert_eq!(
                handlers
                    .applications
                    .iter()
                    .filter(|a| a.id == default.id)
                    .count(),
                1
            );
        }
        let unique: std::collections::HashSet<_> =
            handlers.applications.iter().map(|a| &a.id).collect();
        assert_eq!(unique.len(), handlers.applications.len());
        assert!(
            handlers
                .applications
                .iter()
                .all(|a| !a.name.is_empty() && a.id.starts_with("file:"))
        );
    }
    for query in [
        HandlerQuery::Scheme("https:".into()),
        HandlerQuery::Extension(".pdf".into()),
        HandlerQuery::Url("relative/path".into()),
        HandlerQuery::MimeType("bad type".into()),
        HandlerQuery::MimeType("/pdf".into()),
        HandlerQuery::MimeType("application/".into()),
        HandlerQuery::MimeType("application/pdf/extra".into()),
    ] {
        assert_eq!(
            toolkit.application_handlers(&query),
            Err(ApplicationError::InvalidInput)
        );
    }
    let result = Arc::new(Mutex::new(None));
    let captured = result.clone();
    toolkit.open_url_with_application(
        "https://example.invalid/",
        "file:///nonexistent/test-browser.app",
        Box::new(move |value| *captured.lock().unwrap() = Some(value)),
    );
    assert_eq!(
        *result.lock().unwrap(),
        Some(Err(ApplicationError::NotFound))
    );

    let events = Rc::new(RefCell::new(Vec::new()));
    let captured = events.clone();
    toolkit.set_event_sink(Box::new(move |node, event| {
        captured.borrow_mut().push((node, event))
    }));
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            NSRect::new(NSPoint::ZERO, NSSize::new(400.0, 300.0)),
            NSWindowStyleMask::Titled,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    unsafe { window.setReleasedWhenClosed(false) };
    let node = NodeId(901);
    let list = toolkit.realize(
        kinds::LIST,
        &ListProps {
            selectable: true,
            ..Default::default()
        },
        node,
    );
    window.contentView().unwrap().addSubview(&list);
    let table = list
        .downcast_ref::<NSScrollView>()
        .unwrap()
        .documentView()
        .unwrap()
        .downcast::<NSTableView>()
        .unwrap();
    // Programmatic selection must target the real table, survive a content reload, and
    // tolerate indexes invalidated by a shrinking source without raising NSException.
    let rows = Rc::new(RefCell::new(vec![10u64, 20, 30]));
    let rows_for_count = rows.clone();
    let rows_for_token = rows.clone();
    toolkit.attach_list(
        &list,
        day_spec::ListSource {
            len: Rc::new(move || rows_for_count.borrow().len()),
            token_at: Rc::new(move |i| rows_for_token.borrow()[i]),
            bind_row: Rc::new(|_, _| {}),
            recycle: Rc::new(|_| {}),
            layout_cell: Rc::new(|_, _| {}),
            reorder: None,
            delete: None,
            swipe: None,
        },
    );
    toolkit.update(
        &list,
        kinds::LIST,
        &day_spec::props::ListPatch::Reload,
        None,
    );
    toolkit.update(
        &list,
        kinds::LIST,
        &day_spec::props::ListPatch::Selected(vec![2]),
        None,
    );
    assert_eq!(table.selectedRow(), 2);
    toolkit.update(
        &list,
        kinds::LIST,
        &day_spec::props::ListPatch::Reload,
        None,
    );
    assert_eq!(table.selectedRow(), 2);
    *rows.borrow_mut() = vec![30, 10, 20];
    toolkit.update(
        &list,
        kinds::LIST,
        &day_spec::props::ListPatch::Reload,
        None,
    );
    assert_eq!(
        table.selectedRow(),
        0,
        "selection follows the token after a reorder"
    );
    *rows.borrow_mut() = vec![10, 20];
    toolkit.update(
        &list,
        kinds::LIST,
        &day_spec::props::ListPatch::Reload,
        None,
    );
    assert_eq!(
        table.selectedRow(),
        -1,
        "removal must not select the successor"
    );
    rows.borrow_mut().clear();
    toolkit.update(
        &list,
        kinds::LIST,
        &day_spec::props::ListPatch::Reload,
        None,
    );
    toolkit.update(
        &list,
        kinds::LIST,
        &day_spec::props::ListPatch::Selected(vec![2]),
        None,
    );
    assert_eq!(table.selectedRow(), -1);
    toolkit.focus(&list, node, true);
    assert_eq!(
        Retained::as_ptr(&window.firstResponder().unwrap()) as usize,
        Retained::as_ptr(&table) as usize
    );
    assert!(events.borrow().contains(&(node, Event::FocusChanged(true))));
    toolkit.focus(&list, node, false);
    assert!(
        events
            .borrow()
            .contains(&(node, Event::FocusChanged(false)))
    );
    // An occluded, freshly attached list must recover a cell whose first bind was
    // skipped (as when cacheDisplay/layout enters while day-core holds its tree borrow).
    let cold_list = toolkit.realize(kinds::LIST, &ListProps::default(), NodeId(903));
    window.contentView().unwrap().addSubview(&cold_list);
    cold_list.setFrame(NSRect::new(NSPoint::ZERO, NSSize::new(300.0, 200.0)));
    let cold_table = cold_list
        .downcast_ref::<NSScrollView>()
        .unwrap()
        .documentView()
        .unwrap()
        .downcast::<NSTableView>()
        .unwrap();
    let allow_bind = Rc::new(std::cell::Cell::new(false));
    let allow = allow_bind.clone();
    toolkit.attach_list(
        &cold_list,
        day_spec::ListSource {
            len: Rc::new(|| 1),
            token_at: Rc::new(|_| 1),
            bind_row: Rc::new(move |_, raw| {
                if allow.get() {
                    let cell = unsafe { &*(raw as *const objc2_app_kit::NSView) };
                    let child = objc2_app_kit::NSView::new(mtm);
                    cell.addSubview(&child);
                }
            }),
            recycle: Rc::new(|_| {}),
            layout_cell: Rc::new(|_, _| {}),
            reorder: None,
            delete: None,
            swipe: None,
        },
    );
    let cold_cell = cold_table
        .viewAtColumn_row_makeIfNecessary(0, 0, true)
        .unwrap();
    assert!(cold_cell.subviews().is_empty());
    allow_bind.set(true);
    // Drain the posted initial-realization pass without ordering the window onscreen.
    objc2_foundation::NSRunLoop::currentRunLoop()
        .runUntilDate(&objc2_foundation::NSDate::dateWithTimeIntervalSinceNow(0.1));
    assert!(
        !cold_cell.subviews().is_empty(),
        "cached empty cell must be bound offscreen"
    );

    // Synthetic labels: duplicate titles must remain independent options across a native
    // separator, and selected indexes must refer to options rather than native menu slots.
    use day_spec::props::{PickerPatch, PickerProps};
    let picker_node = NodeId(902);
    let picker = toolkit.realize(
        kinds::PICKER,
        &PickerProps {
            options: vec!["Fixture A".into(), "Fixture B".into(), "Fixture A".into()],
            separators_before: vec![2],
            selected: 2,
            ..Default::default()
        },
        picker_node,
    );
    let popup = picker
        .downcast_ref::<objc2_app_kit::NSPopUpButton>()
        .unwrap();
    assert_eq!(popup.numberOfItems(), 4);
    assert!(popup.itemAtIndex(2).unwrap().isSeparatorItem());
    assert_eq!(popup.selectedItem().unwrap().tag(), 2);
    assert_eq!(popup.indexOfSelectedItem(), 3);
    toolkit.update(&picker, kinds::PICKER, &PickerPatch::Selected(1), None);
    assert_eq!(popup.selectedItem().unwrap().tag(), 1);
    toolkit.update(
        &picker,
        kinds::PICKER,
        &PickerPatch::Options(vec![
            "Changed A".into(),
            "Changed B".into(),
            "Changed A".into(),
        ]),
        None,
    );
    assert_eq!(popup.numberOfItems(), 4);
    assert!(popup.itemAtIndex(2).unwrap().isSeparatorItem());
    assert_eq!(popup.selectedItem().unwrap().tag(), 1);
    popup.selectItemWithTag(2);
    unsafe {
        popup.sendAction_to(popup.action(), popup.target().as_deref());
    }
    assert!(
        events
            .borrow()
            .contains(&(picker_node, Event::SelectionChanged(2)))
    );
    // Synthetic multiline text: the native line cap must affect measurement too, or a
    // compact list row still lays its footer underneath an unlimited-height label.
    let measure_label = |toolkit: &mut AppKit, max_lines, wraps| {
        let label = toolkit.realize(kinds::LABEL, &day_spec::props::LabelProps {
            text: "Fixture first line\nFixture second line\nFixture third line\nFixture fourth line".into(),
            max_lines,
            wraps,
            ..Default::default()
        }, NodeId(903));
        let field = label.downcast_ref::<objc2_app_kit::NSTextField>().unwrap();
        assert_eq!(
            field.maximumNumberOfLines(),
            if wraps { max_lines as isize } else { 1 }
        );
        toolkit.measure(
            &label,
            kinds::LABEL,
            day_spec::Proposal {
                width: Some(180.0),
                height: None,
            },
        )
    };
    let unlimited = measure_label(&mut toolkit, 0, true);
    let two = measure_label(&mut toolkit, 2, true);
    let one = measure_label(&mut toolkit, 5, false);
    assert!(
        one.height < two.height,
        "single-line precedence: {one:?} vs {two:?}"
    );
    assert!(
        two.height < unlimited.height,
        "line limit must constrain measurement: {two:?} vs {unlimited:?}"
    );
    println!("Native discovery, list focus, and grouped picker selection passed");
}
