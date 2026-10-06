// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
//! Native GTK selection, including sync before attachment and after a model reload.
use day_gtk::Gtk;
use day_spec::{
    Event, ListSource, NodeId, Toolkit, kinds,
    props::{ListPatch, ListProps},
};
use gtk4::prelude::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

fn drain() {
    let context = gtk4::glib::MainContext::default();
    while context.pending() {
        context.iteration(false);
    }
}
fn main() {
    gtk4::init().expect("native selection test needs a display");
    let events = Rc::new(RefCell::new(Vec::new()));
    let mut toolkit = Gtk::new();
    toolkit.set_event_sink(Box::new({
        let events = events.clone();
        move |_, event| {
            if let Event::SelectionChanged(row) = event {
                events.borrow_mut().push(row);
            }
        }
    }));
    let host = toolkit.realize(
        kinds::LIST,
        &ListProps {
            selectable: true,
            ..Default::default()
        },
        NodeId(41),
    );
    let list = host
        .downcast_ref::<gtk4::ScrolledWindow>()
        .unwrap()
        .child()
        .unwrap()
        .downcast::<gtk4::ListView>()
        .unwrap();
    let selection = list
        .model()
        .unwrap()
        .downcast::<gtk4::SingleSelection>()
        .unwrap();
    let count = Rc::new(Cell::new(5));
    toolkit.update(&host, kinds::LIST, &ListPatch::Selected(vec![3]), None);
    toolkit.attach_list(
        &host,
        ListSource {
            first_visible: None,
            len: Rc::new({
                let count = count.clone();
                move || count.get()
            }),
            token_at: Rc::new(|row| row as u64),
            bind_row: Rc::new(|_, _| {}),
            recycle: Rc::new(|_| {}),
            layout_cell: Rc::new(|_, _| {}),
            reorder: None,
            delete: None,
            swipe: None,
        },
    );
    drain();
    assert_eq!(
        selection.selected(),
        3,
        "selection before source attachment survives the first reload"
    );
    assert!(
        events.borrow().is_empty(),
        "programmatic sync does not echo"
    );
    toolkit.update(&host, kinds::LIST, &ListPatch::Selected(vec![1]), None);
    drain();
    assert!(selection.is_selected(1));
    assert!(!selection.is_selected(3));
    toolkit.update(&host, kinds::LIST, &ListPatch::Reload, None);
    drain();
    assert_eq!(selection.selected(), 1, "reload preserves native selection");
    assert!(events.borrow().is_empty());
    selection.set_selected(4);
    assert_eq!(
        &*events.borrow(),
        &[4],
        "user selection reports exactly once"
    );
    events.borrow_mut().clear();
    toolkit.update(&host, kinds::LIST, &ListPatch::Reload, None);
    drain();
    assert_eq!(
        selection.selected(),
        4,
        "user selection also survives reload"
    );
    assert!(events.borrow().is_empty());
    count.set(2);
    toolkit.update(&host, kinds::LIST, &ListPatch::Reload, None);
    drain();
    assert_eq!(selection.selected(), gtk4::INVALID_LIST_POSITION);
    toolkit.update(&host, kinds::LIST, &ListPatch::Selected(vec![0]), None);
    toolkit.update(&host, kinds::LIST, &ListPatch::Selected(vec![]), None);
    drain();
    assert_eq!(
        selection.selected(),
        gtk4::INVALID_LIST_POSITION,
        "empty sync clears native selection"
    );
    assert!(events.borrow().is_empty());
    // The last request wins even when reload and multiple patches share one idle turn.
    count.set(5);
    toolkit.update(&host, kinds::LIST, &ListPatch::Reload, None);
    toolkit.update(&host, kinds::LIST, &ListPatch::Selected(vec![1]), None);
    toolkit.update(&host, kinds::LIST, &ListPatch::Selected(vec![4]), None);
    drain();
    assert_eq!(selection.selected(), 4);
    assert!(
        events.borrow().is_empty(),
        "batched selection sync never echoes"
    );
    selection.set_selected(2);
    assert_eq!(&*events.borrow(), &[2]);
    events.borrow_mut().clear();
    toolkit.update(&host, kinds::LIST, &ListPatch::Selected(vec![]), None);
    drain();
    assert_eq!(selection.selected(), gtk4::INVALID_LIST_POSITION);
    assert!(
        events.borrow().is_empty(),
        "clearing a user selection never echoes"
    );
    toolkit.release(host);
    println!("native list selection: sync, reload, clear, and echo suppression passed");
}
