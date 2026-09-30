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
    println!("Native application discovery and list focus passed");
}
