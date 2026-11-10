// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Mock e2e for a text field's selection requests and its claimed keys (docs/textfield.md
//! "Selection" and "Keys"): the address-bar behaviors, inline completion, select-all on
//! focus, and the arrows and Escape walking a suggestion list.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use day_mock::{MockProbe, MockToolkit};
use day_pieces::prelude::*;
use day_reactive::flush_sync;
use day_spec::props::TextSelection;
use day_spec::{Event, KeyEvent, NodeId, WindowOptions};

/// Where a page built inside `boot` hands its signals back to the test.
type Slot<T> = Rc<Cell<Option<T>>>;

fn boot(root: impl FnOnce() -> AnyPiece + 'static) -> MockProbe {
    day_core::uninstall_tree();
    let (toolkit, probe) = MockToolkit::new();
    day_core::launch_with(toolkit, WindowOptions::default(), root);
    probe
}

fn field(probe: &MockProbe) -> NodeId {
    let found = probe.find_by_kind("day.text_field");
    assert_eq!(found.len(), 1, "exactly one text field realized");
    NodeId(found[0].1.node)
}

/// The text field's updates, in order, after the last `clear_log`.
fn field_updates(probe: &MockProbe) -> Vec<String> {
    probe
        .log()
        .into_iter()
        .filter(|l| l.starts_with("update day.text_field"))
        .map(|l| {
            l.split_once(' ')
                .unwrap()
                .1
                .split_once(' ')
                .unwrap()
                .1
                .to_owned()
        })
        .map(|l| l.split_once(' ').unwrap().1.to_owned())
        .collect()
}

fn selection(probe: &MockProbe) -> Option<TextSelection> {
    probe.find_by_kind("day.text_field")[0].1.selection
}

struct Bar {
    probe: MockProbe,
    address: Signal<String>,
    typed: Signal<usize>,
    complete: Trigger,
}

/// An address bar with inline completion over one known host.
fn address_bar() -> Bar {
    let slots: Slot<(Signal<String>, Signal<usize>, Trigger)> = Rc::new(Cell::new(None));
    let out = slots.clone();
    let probe = boot(move || {
        let address = Signal::new(String::new());
        let typed = Signal::new(0usize);
        let complete = Trigger::new();
        out.set(Some((address, typed, complete)));
        text_field(address)
            .select(complete, move || typed.get_untracked()..)
            .any()
    });
    let (address, typed, complete) = slots.get().unwrap();
    Bar {
        probe,
        address,
        typed,
        complete,
    }
}

#[test]
fn completion_text_lands_before_its_selection() {
    let bar = address_bar();
    let node = field(&bar.probe);
    bar.probe.clear_log();

    // The user types "git"; the app completes it in the same turn (a batch, as a handler
    // runs), firing the selection before it writes the text, which must not matter.
    bar.probe.emit(node, Event::TextChanged("git".into()));
    flush_sync();
    day_reactive::batch(|| {
        bar.complete.notify();
        bar.typed.set(3);
        bar.address.set("github.com".into());
    });
    flush_sync();

    assert_eq!(
        field_updates(&bar.probe),
        [
            "text=\"git\" from_native=true",
            "text=\"github.com\" from_native=false",
            "select=3..10",
        ]
    );
    assert_eq!(selection(&bar.probe), Some(TextSelection::new(3, 10)));
}

#[test]
fn the_same_range_applies_on_every_request() {
    let bar = address_bar();
    bar.address.set("example.org".into());
    flush_sync();
    bar.probe.clear_log();

    bar.complete.notify();
    flush_sync();
    bar.complete.notify();
    flush_sync();
    assert_eq!(field_updates(&bar.probe), ["select=0..11", "select=0..11"]);
}

#[test]
fn a_build_selects_nothing_until_the_trigger_fires() {
    let bar = address_bar();
    assert!(
        !bar.probe.log().iter().any(|l| l.contains("select=")),
        "no selection before the first request"
    );
    assert_eq!(selection(&bar.probe), None);
}

#[test]
fn ranges_are_held_to_the_text_in_characters() {
    let range = Signal::new((0usize, usize::MAX));
    let slots: Slot<(Signal<String>, Trigger)> = Rc::new(Cell::new(None));
    let out = slots.clone();
    let probe = boot(move || {
        let text = Signal::new(String::new());
        let fire = Trigger::new();
        out.set(Some((text, fire)));
        text_field(text)
            .select(fire, move || {
                let (a, b) = range.get_untracked();
                a..b
            })
            .any()
    });
    let (text, fire) = slots.get().unwrap();
    // Two characters past ASCII: the range counts characters, not bytes or UTF-16 units.
    text.set("né😀x".into());
    flush_sync();
    probe.clear_log();

    for (want, r) in [
        ("select=0..4", (0, usize::MAX)),
        ("select=2..4", (2, 40)),
        ("select=4..4", (9, 12)),
        ("select=1..1", (1, 1)),
    ] {
        range.set(r);
        fire.notify();
        flush_sync();
        assert_eq!(field_updates(&probe).last().map(String::as_str), Some(want));
    }
}

#[test]
fn every_range_form_is_accepted() {
    let which = Rc::new(Cell::new(0u8));
    let pick = which.clone();
    let slots: Slot<(Signal<String>, Trigger)> = Rc::new(Cell::new(None));
    let out = slots.clone();
    let probe = boot(move || {
        let text = Signal::new(String::new());
        let fire = Trigger::new();
        out.set(Some((text, fire)));
        // One closure type per call, so the forms take turns through `Bound`s.
        text_field(text)
            .select(fire, move || {
                use std::ops::Bound::*;
                match pick.get() {
                    0 => (Unbounded, Unbounded),
                    1 => (Included(2), Included(3)),
                    _ => (Excluded(1), Unbounded),
                }
            })
            .any()
    });
    let (text, fire) = slots.get().unwrap();
    text.set("abcdef".into());
    flush_sync();
    probe.clear_log();
    for (n, want) in [(0, "select=0..6"), (1, "select=2..4"), (2, "select=2..6")] {
        which.set(n);
        fire.notify();
        flush_sync();
        assert_eq!(field_updates(&probe).last().map(String::as_str), Some(want));
    }
}

#[test]
fn focus_selects_everything() {
    let slots: Slot<Signal<bool>> = Rc::new(Cell::new(None));
    let out = slots.clone();
    let probe = boot(move || {
        let address = Signal::new("https://daybrite.dev/".to_owned());
        let editing = Signal::new(false);
        let select_all = Trigger::new();
        out.set(Some(editing));
        watch(
            move || editing.get(),
            move |&on, _| {
                if on {
                    select_all.notify();
                }
            },
        );
        text_field(address)
            .select(select_all, || ..)
            .focused(editing)
            .any()
    });
    let node = field(&probe);
    probe.clear_log();
    for _ in 0..2 {
        probe.emit(node, Event::FocusChanged(true));
        flush_sync();
        assert_eq!(
            field_updates(&probe).last().map(String::as_str),
            Some("select=0..21")
        );
        probe.emit(node, Event::FocusChanged(false));
        flush_sync();
    }
    assert_eq!(slots.get().map(|s| s.get_untracked()), Some(false));
}

#[test]
fn a_field_with_a_key_handler_claims_the_arrows_and_escape() {
    let heard: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let log = heard.clone();
    let probe = boot(move || {
        column((
            text_field(Signal::new(String::new()))
                .on_key(move |k| log.borrow_mut().push(k.key.clone()))
                .id(day_core::id!(bar)),
            text_field(Signal::new(String::new())).id(day_core::id!(plain)),
        ))
        .any()
    });
    let fields = probe.find_by_kind("day.text_field");
    let bar = day_core::with_tree(|t| day_core::rnode_to_id(t.find_by_id("bar").unwrap()));
    let plain = fields
        .iter()
        .map(|(_, w)| NodeId(w.node))
        .find(|n| *n != bar)
        .unwrap();

    use day_spec::keys::field_claims;
    for key in ["ArrowUp", "ArrowDown", "Escape"] {
        assert!(field_claims(bar, key, 0), "{key} is the bar's");
        assert!(
            !field_claims(plain, key, 0),
            "{key} stays with a plain field"
        );
        assert!(
            !field_claims(bar, key, KeyEvent::SHIFT),
            "shift+{key} stays the field's"
        );
    }
    for key in ["Enter", "Tab", "ArrowLeft", "a", "Backspace"] {
        assert!(!field_claims(bar, key, 0), "{key} stays the field's");
    }

    for key in ["ArrowDown", "ArrowDown", "ArrowUp", "Escape"] {
        probe.emit(bar, Event::Key(KeyEvent::named(key, 0)));
    }
    flush_sync();
    assert_eq!(
        *heard.borrow(),
        ["ArrowDown", "ArrowDown", "ArrowUp", "Escape"]
    );
}

#[test]
fn completion_from_an_effect_on_typing() {
    // The browser's shape: an effect watching the bound text completes it as the user types,
    // and leaves a deletion alone so Backspace can remove the completed part.
    let slots: Slot<Signal<String>> = Rc::new(Cell::new(None));
    let out = slots.clone();
    let probe = boot(move || {
        let address = Signal::new(String::new());
        let typed = Signal::new(0usize);
        let complete = Trigger::new();
        out.set(Some(address));
        watch(
            move || address.get(),
            move |now, before| {
                let grew = now.chars().count() > before.map_or(0, |b| b.chars().count());
                let host = "github.com";
                if grew && now.len() < host.len() && host.starts_with(now.as_str()) {
                    typed.set(now.chars().count());
                    address.set(host.to_owned());
                    complete.notify();
                }
            },
        );
        text_field(address)
            .select(complete, move || typed.get_untracked()..)
            .any()
    });
    let node = field(&probe);
    probe.clear_log();
    probe.emit(node, Event::TextChanged("git".into()));
    flush_sync();
    assert_eq!(
        field_updates(&probe),
        [
            "text=\"git\" from_native=true",
            "text=\"github.com\" from_native=false",
            "select=3..10",
        ]
    );
    // Backspace deletes the selected completion: the field reports "git" and stays so.
    probe.clear_log();
    probe.emit(node, Event::TextChanged("git".into()));
    flush_sync();
    assert_eq!(field_updates(&probe), ["text=\"git\" from_native=true"]);
    assert_eq!(
        slots.get().map(|a| a.get_untracked()),
        Some("git".to_owned())
    );
}

#[test]
fn focus_requested_in_the_same_turn_lands_first() {
    // ⌘L in a browser: focus the address bar and select it in one handler. AppKit and UIKit
    // select only in a focused field, so the focus request must reach the toolkit first.
    let slots: Slot<(Signal<bool>, Trigger)> = Rc::new(Cell::new(None));
    let out = slots.clone();
    let probe = boot(move || {
        let address = Signal::new("https://daybrite.dev/".to_owned());
        let editing = Signal::new(false);
        let select_all = Trigger::new();
        out.set(Some((editing, select_all)));
        text_field(address)
            .select(select_all, || ..)
            .focused(editing)
            .any()
    });
    let (editing, select_all) = slots.get().unwrap();
    probe.clear_log();
    day_reactive::batch(|| {
        select_all.notify();
        editing.set(true);
    });
    flush_sync();
    let log = probe.log();
    let focus = log
        .iter()
        .position(|l| l.starts_with("focus #") && l.ends_with(" true"));
    let select = log.iter().position(|l| l.ends_with("select=0..21"));
    assert!(
        matches!((focus, select), (Some(f), Some(s)) if f < s),
        "focus, then select: {log:?}"
    );
}

#[test]
fn an_app_write_after_typing_survives_focus_loss() {
    // Escape in an address bar: the app puts the page's URL back and drops focus. The typed
    // text must not be committed over it as the field loses focus.
    let slots: Slot<Signal<String>> = Rc::new(Cell::new(None));
    let out = slots.clone();
    let probe = boot(move || {
        let address = Signal::new("https://daybrite.dev/".to_owned());
        out.set(Some(address));
        text_field(address).any()
    });
    let address = slots.get().unwrap();
    let node = field(&probe);
    probe.emit(node, Event::FocusChanged(true));
    probe.emit(node, Event::TextChanged("gi".into()));
    flush_sync();
    assert_eq!(address.get_untracked(), "gi");
    address.set("https://daybrite.dev/".into());
    flush_sync();
    probe.emit(node, Event::FocusChanged(false));
    flush_sync();
    assert_eq!(address.get_untracked(), "https://daybrite.dev/");
    // Typing and leaving still commits what was typed.
    probe.emit(node, Event::FocusChanged(true));
    probe.emit(node, Event::TextChanged("docs.rs".into()));
    probe.emit(node, Event::FocusChanged(false));
    flush_sync();
    assert_eq!(address.get_untracked(), "docs.rs");
}
