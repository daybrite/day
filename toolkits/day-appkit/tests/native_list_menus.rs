// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
fn main() {
    use day_appkit::AppKit;
    use day_spec::{
        ListSource, MenuItem, NodeId, Toolkit, kinds,
        props::{ContainerProps, LabelProps, ListProps, RowHeight},
    };
    use objc2::{MainThreadMarker, MainThreadOnly, rc::Retained};
    use objc2_app_kit::{
        NSApplication, NSBackingStoreType, NSEvent, NSEventModifierFlags, NSEventType, NSMenu,
        NSScrollView, NSTableView, NSView, NSWindow, NSWindowStyleMask,
    };
    use objc2_foundation::{NSIndexSet, NSPoint, NSRect, NSSize};
    use std::{cell::Cell, rc::Rc};

    let mtm = MainThreadMarker::new().unwrap();
    let _app = NSApplication::sharedApplication(mtm);
    let mut toolkit = AppKit::new();
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            NSRect::new(NSPoint::new(0., 0.), NSSize::new(400., 300.)),
            NSWindowStyleMask::Titled,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    unsafe { window.setReleasedWhenClosed(false) };
    let host = toolkit.realize(
        kinds::LIST,
        &ListProps {
            row_height: RowHeight::Uniform(60.),
            selectable: true,
            ..Default::default()
        },
        NodeId(100),
    );
    host.setFrame(NSRect::new(NSPoint::new(0., 0.), NSSize::new(400., 300.)));
    window.contentView().unwrap().addSubview(&host);
    let revision = Rc::new(Cell::new(0));
    let calls = Rc::new(Cell::new(0));
    let (rev, count) = (revision.clone(), calls.clone());
    toolkit.attach_list(
        &host,
        ListSource {
            len: Rc::new(|| 2),
            token_at: Rc::new(|i| i as u64),
            bind_row: Rc::new(move |index, raw| {
                let cell = unsafe { Retained::retain(raw as *mut NSView) }.unwrap();
                // Nested content exercises discovery through the native cell's layout wrapper.
                let mut row_toolkit = AppKit::new();
                let content = row_toolkit.realize(
                    kinds::CONTAINER,
                    &ContainerProps::default(),
                    NodeId(300 + index as u64),
                );
                cell.addSubview(&content);
                let label = row_toolkit.realize(
                    kinds::LABEL,
                    &LabelProps {
                        text: "Fixture article title".into(),
                        ..Default::default()
                    },
                    NodeId(400 + index as u64),
                );
                content.addSubview(&label);
                let (rev, count) = (rev.clone(), count.clone());
                AppKit::new().set_context_menu_fn(
                    &content,
                    NodeId(200 + index as u64),
                    Rc::new(move |_| {
                        count.set(count.get() + 1);
                        if rev.get() == 2 {
                            return vec![];
                        }
                        vec![MenuItem::Action {
                            id: None,
                            action: 1,
                            label: format!("Fixture row {index} revision {}", rev.get()),
                            shortcut: None,
                            enabled: true,
                            checked: None,
                            role: None,
                            icon: None,
                        }]
                    }),
                );
            }),
            recycle: Rc::new(|_| {}),
            layout_cell: Rc::new(|_, _| {}),
            reorder: None,
            delete: None,
            swipe: None,
        },
    );
    let table = host
        .downcast_ref::<NSScrollView>()
        .unwrap()
        .documentView()
        .unwrap()
        .downcast::<NSTableView>()
        .unwrap();
    table.layoutSubtreeIfNeeded();
    table.viewAtColumn_row_makeIfNecessary(0, 0, true).unwrap();
    table.viewAtColumn_row_makeIfNecessary(0, 1, true).unwrap();
    table.selectRowIndexes_byExtendingSelection(&NSIndexSet::indexSetWithIndex(0), false);
    let event_for = |row: isize| {
        let y = if row < 0 {
            260.
        } else {
            table.rectOfRow(row).origin.y + 20.
        };
        let point = table.convertPoint_toView(NSPoint::new(20., y), None);
        NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
            NSEventType::RightMouseDown, point, NSEventModifierFlags::empty(), 0., window.windowNumber(), None, 0, 1, 1.,
        ).unwrap()
    };
    let summon = |row| -> Option<Retained<NSMenu>> { table.menuForEvent(&event_for(row)) };
    assert_eq!(
        calls.get(),
        0,
        "providers must not run during list creation"
    );
    let first = summon(1).expect("right-click on unselected row opens its menu");
    assert_eq!(
        first.itemAtIndex(0).unwrap().title().to_string(),
        "Fixture row 1 revision 0"
    );
    assert_eq!(
        table.selectedRow(),
        0,
        "opening a row menu preserves reader selection"
    );
    revision.set(1);
    assert_eq!(
        summon(1)
            .unwrap()
            .itemAtIndex(0)
            .unwrap()
            .title()
            .to_string(),
        "Fixture row 1 revision 1"
    );
    assert_eq!(
        summon(0)
            .unwrap()
            .itemAtIndex(0)
            .unwrap()
            .title()
            .to_string(),
        "Fixture row 0 revision 1"
    );
    assert_eq!(
        first.itemAtIndex(0).unwrap().title().to_string(),
        "Fixture row 1 revision 0"
    );
    let cell = table.viewAtColumn_row_makeIfNecessary(0, 1, false).unwrap();
    let content = cell.subviews().objectAtIndex(0);
    let label = content.subviews().objectAtIndex(0);
    for target in [&content, &label] {
        let menu = target
            .menuForEvent(&event_for(1))
            .expect("container and plain-label right-clicks reach the row owner");
        assert_eq!(
            menu.itemAtIndex(0).unwrap().title().to_string(),
            "Fixture row 1 revision 1"
        );
    }
    revision.set(2);
    assert!(label.menuForEvent(&event_for(1)).is_none());
    assert!(summon(1).is_none(), "an empty provider suppresses the menu");
    let before = calls.get();
    assert!(summon(-1).is_none());
    assert_eq!(
        calls.get(),
        before,
        "blank list space must not invoke a row provider"
    );
    toolkit.release(host);
    window.close();
    println!("native list menus: clicked row, live state, empty result, blank space passed");
}
