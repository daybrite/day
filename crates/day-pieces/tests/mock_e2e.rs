//! M1 acceptance (DESIGN.md §21.2): end-to-end on the mock toolkit. The op log IS the
//! fine-grained-invalidation contract — "exactly one mutation op per state change" and
//! "bounded measure calls" are assertions, not aspirations.

use day_core::AnyPiece;
use day_mock::{MockHandle, MockProbe, MockToolkit};
use day_pieces::prelude::*;
use day_reactive::flush_sync;
use day_spec::{Event, NodeId, Size, WindowOptions};

/// Serializes boots against env mutation: `launch_with` reads process-global env
/// (DAY_DEEPLINK), and tests run on parallel threads.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn boot(root: impl FnOnce() -> AnyPiece + 'static) -> MockProbe {
    boot_with_env(None, root)
}

fn boot_with_env(
    env: Option<(&str, &str)>,
    root: impl FnOnce() -> AnyPiece + 'static,
) -> MockProbe {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((k, v)) = env {
        unsafe { std::env::set_var(k, v) };
    }
    day_core::uninstall_tree();
    let (mock, probe) = MockToolkit::new();
    let options = WindowOptions {
        title: "test".into(),
        size: Size::new(400.0, 600.0),
        ..Default::default()
    };
    day_core::launch_with(mock, options, root);
    if let Some((k, _)) = env {
        unsafe { std::env::remove_var(k) };
    }
    probe
}

fn node_id(probe: &MockProbe, kind: &str, index: usize) -> NodeId {
    let found = probe.find_by_kind(kind);
    NodeId(found[index].1.node)
}

#[test]
fn counter_updates_exactly_one_op_per_click() {
    let probe = boot(|| {
        let count = Signal::new(0);
        column((
            label(move || format!("Count: {}", count.get())),
            button("+").action(move || count.update(|c| *c += 1)),
        ))
        .spacing(8.0)
        .any()
    });
    let labels = probe.find_by_kind("day.label");
    assert_eq!(labels.len(), 1);
    assert_eq!(labels[0].1.text, "Count: 0");

    let btn = node_id(&probe, "day.button", 0);
    probe.clear_log();
    probe.emit(btn, Event::Pressed);

    // THE fine-grained guarantee: one native mutation for the click. "Count: 0"→"Count: 1"
    // has identical metrics, so zero frame ops.
    let muts: Vec<String> = probe
        .mutations()
        .into_iter()
        .filter(|m| !m.starts_with("a11y"))
        .collect();
    assert_eq!(
        muts.len(),
        1,
        "expected exactly one mutation, got: {muts:?}"
    );
    assert!(
        muts[0].contains("update day.label"),
        "unexpected op: {}",
        muts[0]
    );
    assert!(muts[0].contains("Count: 1"));

    // Bounded relayout: only the label's path re-measures (label + its ancestors' negotiation).
    assert!(
        probe.measure_calls() <= 6,
        "measure calls not bounded: {} ({:?})",
        probe.measure_calls(),
        probe.log()
    );
}

#[test]
fn layout_places_stack_children() {
    let probe = boot(|| {
        column((label("aa"), label("bbbb")))
            .spacing(10.0)
            .align(HAlign::Leading)
            .any()
    });
    let labels = probe.find_by_kind("day.label");
    // 8pt/char, 16pt line: "aa" = 16x16 at y=0; "bbbb" = 32x16 at y=26 (16 + spacing 10).
    assert_eq!(labels[0].1.frame, day_spec::Rect::new(0.0, 0.0, 16.0, 16.0));
    assert_eq!(
        labels[1].1.frame,
        day_spec::Rect::new(0.0, 26.0, 32.0, 16.0)
    );
}

#[test]
fn label_wraps_height_for_width() {
    let probe = boot(|| {
        // 30 chars * 8 = 240pt needed; window 400 - padding 2*150 = 100pt wide → 3 lines.
        column((label("abcdefghijklmnopqrstuvwxyz1234"),))
            .padding(Insets::symmetric(150.0, 0.0))
            .any()
    });
    let labels = probe.find_by_kind("day.label");
    assert_eq!(
        labels[0].1.frame.size,
        Size::new(100.0, 48.0),
        "expected 3 wrapped lines"
    );
}

#[test]
fn toggle_two_way() {
    let flag = Signal::new(false);
    let probe = boot(move || column((toggle(flag),)).any());
    let toggles = probe.find_by_kind("day.toggle");
    assert!(!toggles[0].1.flag);

    // native → signal
    probe.emit(node_id(&probe, "day.toggle", 0), Event::ToggleChanged(true));
    assert!(flag.get_untracked());

    // signal → native
    batch(|| flag.set(false));
    assert!(!probe.find_by_kind("day.toggle")[0].1.flag);
}

#[test]
fn text_field_controlled_echo_is_origin_tagged() {
    let name = Signal::new(String::new());
    let probe = boot(move || column((text_field(name).placeholder("Your name"),)).any());
    let tf = node_id(&probe, "day.text_field", 0);

    probe.clear_log();
    probe.emit(tf, Event::TextChanged("Ada".into()));
    assert_eq!(name.get_untracked(), "Ada");
    // The echo write-back must be origin-tagged so the widget's caret survives (§4.4).
    let echo: Vec<String> = probe
        .mutations()
        .into_iter()
        .filter(|m| m.contains("from_native=true"))
        .collect();
    assert_eq!(
        echo.len(),
        1,
        "expected one origin-tagged echo: {:?}",
        probe.mutations()
    );

    // Programmatic writes reach the widget.
    batch(|| name.set("Bob".into()));
    assert_eq!(probe.find_by_kind("day.text_field")[0].1.text, "Bob");
}

#[test]
fn slider_value_flows_both_ways() {
    let volume = Signal::new(40.0f64);
    let probe = boot(move || column((slider(volume).range(0.0..=100.0),)).any());
    probe.emit(node_id(&probe, "day.slider", 0), Event::ValueChanged(80.0));
    assert_eq!(volume.get_untracked(), 80.0);
    batch(|| volume.set(25.0));
    assert_eq!(probe.find_by_kind("day.slider")[0].1.value, 25.0);
}

#[test]
fn progress_tracks_signal_with_one_op_per_change() {
    let frac = Signal::new(0.25f64);
    let probe = boot(move || column((progress(move || frac.get()),)).any());

    let bars = probe.find_by_kind("day.progress");
    assert_eq!(bars.len(), 1);
    assert!(!bars[0].1.flag, "determinate bar is not indeterminate");
    assert_eq!(bars[0].1.value, 0.25);

    // One reactive write = exactly one native value patch (the fine-grained guarantee).
    probe.clear_log();
    batch(|| frac.set(0.75));
    flush_sync();
    assert_eq!(probe.find_by_kind("day.progress")[0].1.value, 0.75);
    let value_ops: Vec<String> = probe
        .mutations()
        .into_iter()
        .filter(|m| m.starts_with("update day.progress"))
        .collect();
    assert_eq!(value_ops.len(), 1, "exactly one value patch: {value_ops:?}");
    assert!(value_ops[0].ends_with("value=Some(0.75)"));
}

#[test]
fn progress_clamps_out_of_range_fractions() {
    let frac = Signal::new(2.0f64); // above 1.0
    let probe = boot(move || column((progress(move || frac.get()),)).any());
    assert_eq!(probe.find_by_kind("day.progress")[0].1.value, 1.0);
    batch(|| frac.set(-3.0)); // below 0.0
    flush_sync();
    assert_eq!(probe.find_by_kind("day.progress")[0].1.value, 0.0);
}

#[test]
fn spinner_is_indeterminate_and_static() {
    let probe = boot(|| column((spinner(),)).any());
    let bars = probe.find_by_kind("day.progress");
    assert_eq!(bars.len(), 1);
    assert!(bars[0].1.flag, "spinner is indeterminate");
    // An indeterminate spinner has no bound value, so no value patch is ever emitted.
    assert!(
        !probe
            .log()
            .iter()
            .any(|l| l.contains("day.progress") && l.contains("value=") && l.starts_with("update")),
        "spinner emits no value updates"
    );
}

#[test]
fn constant_progress_emits_no_updates() {
    let probe = boot(|| column((progress(0.5f64),)).any());
    assert_eq!(probe.find_by_kind("day.progress")[0].1.value, 0.5);
    // A constant fraction installs no binding: nothing to update after build.
    assert!(
        !probe
            .log()
            .iter()
            .any(|l| l.starts_with("update day.progress")),
        "constant progress never updates"
    );
}

#[test]
fn when_builds_and_disposes() {
    let show = Signal::new(false);
    let probe = boot(move || {
        column((
            label("always"),
            when(move || show.get(), || label("sometimes")),
        ))
        .any()
    });
    assert_eq!(probe.find_by_kind("day.label").len(), 1);

    batch(|| show.set(true));
    flush_sync();
    let labels = probe.find_by_kind("day.label");
    assert_eq!(labels.len(), 2);
    assert_eq!(labels[1].1.text, "sometimes");

    probe.clear_log();
    batch(|| show.set(false));
    assert_eq!(probe.find_by_kind("day.label").len(), 1);
    assert!(
        probe.log().iter().any(|l| l.starts_with("release")),
        "expected native release: {:?}",
        probe.log()
    );
}

#[test]
fn each_keyed_diff_touches_only_changes() {
    let items: Signal<Vec<(u64, String)>> = Signal::new(vec![(1, "one".into()), (2, "two".into())]);
    let probe = boot(move || {
        column((each(
            move || items.get(),
            |t| t.0,
            move |slot: ItemSlot<(u64, String), u64>| label(move || slot.field(|t| t.1.clone())),
        ),))
        .any()
    });
    assert_eq!(probe.find_by_kind("day.label").len(), 2);

    // Insert: exactly one new realize; survivors untouched.
    probe.clear_log();
    batch(|| items.update(|v| v.push((3, "three".into()))));
    let realizes: Vec<String> = probe
        .log()
        .into_iter()
        .filter(|l| l.starts_with("realize"))
        .collect();
    assert_eq!(
        realizes.len(),
        1,
        "one realize for the inserted row: {realizes:?}"
    );
    assert_eq!(probe.find_by_kind("day.label").len(), 3);

    // Item mutation: surviving row's slot propagates — an update, never a rebuild (§5.4).
    probe.clear_log();
    batch(|| items.update(|v| v[0].1 = "uno".into()));
    let log = probe.log();
    assert!(
        !log.iter().any(|l| l.starts_with("realize")),
        "no rebuild on value change: {log:?}"
    );
    assert!(
        log.iter().any(|l| l.contains("uno")),
        "slot write must reach the surviving row: {log:?}"
    );

    // Removal disposes exactly that row.
    probe.clear_log();
    batch(|| items.update(|v| v.retain(|t| t.0 != 2)));
    assert_eq!(probe.find_by_kind("day.label").len(), 2);
    assert!(probe.log().iter().any(|l| l.starts_with("release")));
}

#[test]
fn spacer_takes_remaining_space() {
    let probe = boot(|| {
        // Row inside a fixed 400-wide window: label 16 + spacer + label 24 → spacer 360.
        column((row((label("aa"), spacer(), label("bbb"))).frame(400.0, 30.0),)).any()
    });
    let labels = probe.find_by_kind("day.label");
    assert_eq!(labels[0].1.frame.origin.x, 0.0);
    assert_eq!(
        labels[1].1.frame.origin.x,
        400.0 - 24.0,
        "trailing label pinned to the end"
    );
}

#[test]
fn scroll_reports_content_size() {
    let probe = boot(|| {
        scroll(column((
            label("aaaaaaaaaa"),
            label("bbbbbbbbbb"),
            label("cccccccccc"),
        )))
        .any()
    });
    let scrolls = probe.find_by_kind("day.scroll");
    assert_eq!(scrolls.len(), 1);
    let content = scrolls[0].1.scroll_content;
    assert_eq!(content.width, 400.0, "content fills the viewport width");
    assert!(
        content.height >= 600.0,
        "content at least viewport height: {content:?}"
    );
    // Scroll children live in the scroll's native coordinate space.
    let labels = probe.find_by_kind("day.label");
    assert_eq!(labels[0].1.frame.origin.y, 0.0);
}

#[test]
fn ids_land_as_a11y_identifiers() {
    let probe = boot(|| column((button("go").id("go-button"),)).any());
    let buttons = probe.find_by_kind("day.button");
    assert_eq!(buttons[0].1.a11y.identifier.as_deref(), Some("go-button"));
}

// ---------------------------------------------------------------------------
// Navigation & tabs (docs/navigation.md, docs/tabs.md) — selector + stack
// ---------------------------------------------------------------------------

fn tabs_selector(sel: Signal<String>) -> AnyPiece {
    selector(sel)
        .style(SelectorStyle::Tabs)
        .item("one", "One", || label("one-content"))
        .item("two", "Two", || label("two-content"))
        .item("three", "Three", || label("three-content"))
        .id("main-tabs")
}

#[test]
fn selector_tabs_builds_all_pages_and_binds_selection() {
    let sel = Signal::new("one".to_string());
    let probe = boot(move || tabs_selector(sel));
    let hosts = probe.find_by_kind("day.tabs");
    assert_eq!(hosts.len(), 1);
    assert_eq!(hosts[0].1.value, 0.0);
    assert_eq!(probe.find_by_kind("day.tabs_page").len(), 3);
    for t in ["one-content", "two-content", "three-content"] {
        assert!(
            probe
                .find_by_kind("day.label")
                .iter()
                .any(|(_, w)| w.text == t),
            "{t} built eagerly"
        );
    }
    assert_eq!(day_core::current_route().as_deref(), Some("one"));

    // signal → native
    batch(|| sel.set("three".into()));
    flush_sync();
    assert_eq!(probe.find_by_kind("day.tabs")[0].1.value, 2.0);

    // route (string shim) → native + signal
    assert!(navigate("two"));
    flush_sync();
    assert_eq!(sel.get_untracked(), "two");
    assert_eq!(probe.find_by_kind("day.tabs")[0].1.value, 1.0);

    // native tap → signal
    probe.emit(node_id(&probe, "day.tabs", 0), Event::SelectionChanged(0));
    assert_eq!(sel.get_untracked(), "one");
    assert!(!navigate("nope"));
}

fn sidebar_selector(sel: Signal<String>) -> AnyPiece {
    selector(sel)
        .title("Home")
        .item("about", "About", || label("about-content"))
        .item("extra", "Extra", || label("extra-content"))
        .any()
}

#[test]
fn selector_sidebar_lists_items_and_navigates() {
    // Mock reports NavSplit=Unsupported → stack (mobile) presentation.
    let sel = Signal::new(String::new());
    let probe = boot(move || sidebar_selector(sel));
    assert_eq!(probe.find_by_kind("day.nav").len(), 1);
    assert_eq!(
        probe.find_by_kind("day.nav_page").len(),
        1,
        "root/list only"
    );
    let menus = probe.find_by_kind("day.nav_menu");
    assert_eq!(menus.len(), 1);
    assert_eq!(menus[0].1.text, "About|Extra");
    assert_eq!(day_core::current_route().as_deref(), Some(""));

    // native list tap → signal → detail shown + highlight synced
    probe.emit(NodeId(menus[0].1.node), Event::SelectionChanged(1));
    flush_sync();
    assert_eq!(sel.get_untracked(), "extra");
    assert_eq!(day_core::current_route().as_deref(), Some("extra"));
    assert_eq!(probe.find_by_kind("day.nav_page").len(), 2);
    assert!(
        probe
            .find_by_kind("day.label")
            .iter()
            .any(|(_, w)| w.text == "extra-content")
    );
    assert_eq!(probe.find_by_kind("day.nav_menu")[0].1.value, 1.0);

    // programmatic navigate resets the detail
    assert!(navigate("about"));
    flush_sync();
    assert_eq!(day_core::current_route().as_deref(), Some("about"));
    assert!(
        probe
            .find_by_kind("day.label")
            .iter()
            .any(|(_, w)| w.text == "about-content")
    );
    assert!(
        probe
            .find_by_kind("day.label")
            .iter()
            .all(|(_, w)| w.text != "extra-content")
    );

    // signal → detail directly
    batch(|| sel.set("extra".into()));
    flush_sync();
    assert!(
        probe
            .find_by_kind("day.label")
            .iter()
            .any(|(_, w)| w.text == "extra-content")
    );

    // back to root
    assert!(nav_back());
    flush_sync();
    assert_eq!(day_core::current_route().as_deref(), Some(""));
    assert_eq!(probe.find_by_kind("day.nav_page").len(), 1);
    assert!(!nav_back());
}

#[test]
fn selector_sidebar_deep_link_at_startup() {
    let sel = Signal::new(String::new());
    let probe = boot_with_env(Some(("DAY_DEEPLINK", "extra")), move || {
        sidebar_selector(sel)
    });
    flush_sync();
    assert_eq!(day_core::current_route().as_deref(), Some("extra"));
    assert!(
        probe
            .find_by_kind("day.label")
            .iter()
            .any(|(_, w)| w.text == "extra-content")
    );
}

fn stack_root(path: Signal<Vec<String>>) -> AnyPiece {
    stack(path, label("home-content"))
        .destination(|key| label(format!("detail:{key}")))
        .id("nav-stack")
}

#[test]
fn stack_pushes_pops_and_reconciles_to_path() {
    let path = Signal::new(Vec::<String>::new());
    let probe = boot(move || stack_root(path));
    assert_eq!(probe.find_by_kind("day.nav_page").len(), 1, "root only");
    assert!(
        probe
            .find_by_kind("day.label")
            .iter()
            .any(|(_, w)| w.text == "home-content")
    );
    assert_eq!(day_core::current_route().as_deref(), Some(""));

    // push two levels through the path signal
    batch(|| path.set(vec!["a".into(), "b".into()]));
    flush_sync();
    assert_eq!(probe.find_by_kind("day.nav_page").len(), 3);
    assert!(
        probe
            .find_by_kind("day.label")
            .iter()
            .any(|(_, w)| w.text == "detail:b")
    );
    assert_eq!(day_core::current_route().as_deref(), Some("b"));

    // nav_back pops one (through the string shim → path)
    assert!(nav_back());
    flush_sync();
    assert_eq!(path.get_untracked(), vec!["a".to_string()]);
    assert_eq!(probe.find_by_kind("day.nav_page").len(), 2);

    // divergent path: keep common prefix (none), pop the rest, push the new suffix
    batch(|| path.set(vec!["x".into()]));
    flush_sync();
    assert_eq!(probe.find_by_kind("day.nav_page").len(), 2);
    assert!(
        probe
            .find_by_kind("day.label")
            .iter()
            .any(|(_, w)| w.text == "detail:x")
    );
    assert!(
        probe
            .find_by_kind("day.label")
            .iter()
            .all(|(_, w)| w.text != "detail:a")
    );
}

#[test]
fn stack_native_back_writes_into_path() {
    let path = Signal::new(vec!["a".to_string()]);
    let probe = boot(move || stack_root(path));
    flush_sync();
    assert_eq!(probe.find_by_kind("day.nav_page").len(), 2);
    let host = node_id(&probe, "day.nav", 0);

    // iOS-style: the toolkit already popped natively.
    probe.emit(
        host,
        Event::NavBack {
            already_popped: true,
        },
    );
    flush_sync();
    assert_eq!(path.get_untracked(), Vec::<String>::new());
    assert_eq!(probe.find_by_kind("day.nav_page").len(), 1);
}

#[test]
fn nested_stack_in_selector_falls_through() {
    let section = Signal::new(String::new());
    let path = Signal::new(Vec::<String>::new());
    let probe = boot(move || {
        selector(section)
            .title("Root")
            .item("plain", "Plain", || label("plain-content"))
            .item("drill", "Drill", move || {
                stack(path, label("drill-root")).destination(|k| label(format!("drill:{k}")))
            })
            .any()
    });

    // Enter the drill section: the selector shows it and its inner stack registers on top.
    assert!(navigate("drill"));
    flush_sync();
    assert!(
        probe
            .find_by_kind("day.label")
            .iter()
            .any(|(_, w)| w.text == "drill-root")
    );
    // Innermost surface (the stack) is at its root.
    assert_eq!(day_core::current_route().as_deref(), Some(""));

    // Push onto the inner stack via its path (app state).
    batch(|| path.set(vec!["deep".into()]));
    flush_sync();
    assert_eq!(day_core::current_route().as_deref(), Some("deep"));
    assert!(
        probe
            .find_by_kind("day.label")
            .iter()
            .any(|(_, w)| w.text == "drill:deep")
    );

    // navigate a sibling section key: the stack doesn't own it, so it FALLS THROUGH to the
    // enclosing selector — which switches sections (disposing the stack).
    assert!(navigate("plain"));
    flush_sync();
    assert_eq!(section.get_untracked(), "plain");
    assert!(
        probe
            .find_by_kind("day.label")
            .iter()
            .any(|(_, w)| w.text == "plain-content")
    );
    assert!(
        probe
            .find_by_kind("day.label")
            .iter()
            .all(|(_, w)| w.text != "drill:deep")
    );
}

// ---------------------------------------------------------------------------
// Imperative presentation (docs/dialogs.md)
// ---------------------------------------------------------------------------

use day_spec::present::PresentResult;
use std::cell::RefCell;
use std::rc::Rc;

#[test]
fn confirm_true_when_confirm_button_chosen() {
    let out: Rc<RefCell<Option<bool>>> = Rc::default();
    let o2 = out.clone();
    let probe = boot(move || {
        let o2 = o2.clone();
        button("ask")
            .action(move || {
                let o2 = o2.clone();
                day_core::task(async move {
                    let ok = confirm("Quit?").await;
                    *o2.borrow_mut() = Some(ok);
                })
            })
            .id("ask")
            .any()
    });
    let btn = node_id(&probe, "day.button", 0);
    probe.emit(btn, Event::Pressed);
    // A modal is now pending; nothing resolved yet.
    assert!(out.borrow().is_none());
    let (req, spec) = day_core::pending_presentation().expect("a modal is pending");
    assert_eq!(spec.title(), "Quit?");
    // Answer the confirm button (index 1: [cancel, confirm]).
    assert!(day_core::respond_presentation(
        req,
        PresentResult::Button(1)
    ));
    flush_sync();
    assert_eq!(*out.borrow(), Some(true));
    assert!(day_core::pending_presentation().is_none());
}

#[test]
fn confirm_false_on_dismiss() {
    let out: Rc<RefCell<Option<bool>>> = Rc::default();
    let o2 = out.clone();
    let probe = boot(move || {
        let o2 = o2.clone();
        button("ask")
            .action(move || {
                let o2 = o2.clone();
                day_core::task(async move {
                    *o2.borrow_mut() = Some(confirm("Q").await);
                })
            })
            .id("ask")
            .any()
    });
    probe.emit(node_id(&probe, "day.button", 0), Event::Pressed);
    let (req, _) = day_core::pending_presentation().unwrap();
    assert!(day_core::respond_presentation(
        req,
        PresentResult::Dismissed
    ));
    flush_sync();
    assert_eq!(*out.borrow(), Some(false));
}

#[test]
fn prompt_returns_text_or_none() {
    let out: Rc<RefCell<Option<Option<String>>>> = Rc::default();
    let o2 = out.clone();
    let probe = boot(move || {
        let o2 = o2.clone();
        button("ask")
            .action(move || {
                let o2 = o2.clone();
                day_core::task(async move {
                    *o2.borrow_mut() = Some(prompt("Name").await);
                })
            })
            .id("ask")
            .any()
    });
    probe.emit(node_id(&probe, "day.button", 0), Event::Pressed);
    let (req, _) = day_core::pending_presentation().unwrap();
    day_core::respond_presentation(req, PresentResult::Text("Ada".into()));
    flush_sync();
    assert_eq!(*out.borrow(), Some(Some("Ada".to_string())));
}

#[test]
fn alert_returns_typed_payload_and_sequences() {
    #[derive(PartialEq, Debug, Clone, Copy)]
    enum Choice {
        Keep,
        Delete,
    }
    let out: Rc<RefCell<Vec<String>>> = Rc::default();
    let o2 = out.clone();
    let probe = boot(move || {
        let o2 = o2.clone();
        button("go")
            .action(move || {
                let o2 = o2.clone();
                day_core::task(async move {
                    let c = Alert::new("Title")
                        .button("Keep", Choice::Keep)
                        .destructive("Delete", Choice::Delete)
                        .cancel("Cancel")
                        .present()
                        .await;
                    if c == Some(Choice::Delete) {
                        // a SECOND awaited modal in the same flow
                        let name = prompt("Confirm name").await;
                        o2.borrow_mut().push(format!("deleted {name:?}"));
                    } else {
                        o2.borrow_mut().push(format!("chose {c:?}"));
                    }
                })
            })
            .id("go")
            .any()
    });
    probe.emit(node_id(&probe, "day.button", 0), Event::Pressed);
    // First modal: [Keep(0), Delete(1), Cancel(2)] — pick Delete.
    let (req, _) = day_core::pending_presentation().unwrap();
    day_core::respond_presentation(req, PresentResult::Button(1));
    flush_sync();
    // The flow chained into a second modal (the prompt).
    let (req2, spec2) = day_core::pending_presentation().expect("prompt pending");
    assert_eq!(spec2.title(), "Confirm name");
    day_core::respond_presentation(req2, PresentResult::Text("x".into()));
    flush_sync();
    assert_eq!(out.borrow().as_slice(), ["deleted Some(\"x\")"]);
}

// ---------------------------------------------------------------------------
// Native recycling `list` (docs/list.md, §10): the mock drives a simulated viewport through
// the real day-core driver, so these assert the whole build-once/rebind-on-recycle path.
// ---------------------------------------------------------------------------

fn five_item_list() -> AnyPiece {
    let items = Signal::new(
        ["a", "b", "c", "d", "e"]
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>(),
    );
    list(
        move || items.get(),
        |s: &String| s.clone(),
        |row: ItemSlot<String, String>| label(move || row.get()),
    )
    .row_height(RowHeight::Uniform(20.0))
    .any()
}

#[test]
fn list_builds_only_visible_rows() {
    let probe = boot(five_item_list);
    let host = probe.find_by_kind("day.list")[0].0;

    // The data-source sees all five rows…
    assert_eq!(probe.list_len(host), 5);
    // …but nothing is built until the native list pulls a cell (virtualization).
    assert_eq!(probe.find_by_kind("day.label").len(), 0);

    // A viewport of two physical cells shows rows 0 and 1.
    probe.list_bind(host, 0, MockHandle(9001));
    probe.list_bind(host, 1, MockHandle(9002));

    let labels = probe.find_by_kind("day.label");
    assert_eq!(labels.len(), 2, "only the visible rows are built");
    assert_eq!(labels[0].1.text, "a");
    assert_eq!(labels[1].1.text, "b");
}

#[test]
fn list_recycles_cells_with_a_slot_write_not_a_rebuild() {
    let probe = boot(five_item_list);
    let host = probe.find_by_kind("day.list")[0].0;
    let (cell_a, cell_b) = (MockHandle(9001), MockHandle(9002));

    probe.list_bind(host, 0, cell_a); // "a"
    probe.list_bind(host, 1, cell_b); // "b"
    assert_eq!(probe.find_by_kind("day.label").len(), 2);

    // Scroll: cell_a recycles to show row 2. This must REBIND (slot-write), not build a new row.
    probe.list_bind(host, 2, cell_a);

    let labels = probe.find_by_kind("day.label");
    assert_eq!(
        labels.len(),
        2,
        "recycling rebinds the existing cell — no new widget"
    );
    // The recycled cell's own label (lowest handle, built first) now shows row 2's content.
    assert_eq!(labels[0].1.text, "c");
    assert_eq!(labels[1].1.text, "b");

    // Scroll further: cell_b recycles to row 3.
    probe.list_bind(host, 3, cell_b);
    let labels = probe.find_by_kind("day.label");
    assert_eq!(labels.len(), 2);
    assert_eq!(labels[1].1.text, "d");
}

#[test]
fn list_reports_selection_by_key() {
    let picks = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    let sink = picks.clone();
    let probe = boot(move || {
        let items = Signal::new(vec!["a".to_string(), "b".into(), "c".into()]);
        list(
            move || items.get(),
            |s: &String| s.clone(),
            |row: ItemSlot<String, String>| label(move || row.get()),
        )
        .on_select(move |k| sink.borrow_mut().push(k))
        .any()
    });
    let list_node = node_id(&probe, "day.list", 0);
    probe.emit(list_node, Event::SelectionChanged(1));
    flush_sync();
    assert_eq!(picks.borrow().as_slice(), ["b".to_string()]);
}

// Imperative scroll-to-end (chat "stick to bottom"): a `Trigger` drives a `ListPatch::ScrollToEnd`
// that the mock records via the LIST host's `flag`. (Real backends scroll the native list.)
#[test]
fn list_scroll_to_end_follows_the_trigger() {
    let items = Signal::new((0..5).map(|i| i.to_string()).collect::<Vec<_>>());
    let scroll = Trigger::new();
    let probe = boot(move || {
        list(
            move || items.get(),
            |s: &String| s.clone(),
            |row: ItemSlot<String, String>| label(move || row.get()),
        )
        .row_height(RowHeight::Uniform(20.0))
        .scroll_to_end(scroll)
        .any()
    });
    let host = probe.find_by_kind("day.list")[0].0;

    // Building the list must NOT auto-scroll (watch never fires for the initial run).
    assert!(!probe.widget(host).flag);
    assert!(
        !probe
            .mutations()
            .iter()
            .any(|m| m.contains("scroll-to-end"))
    );

    // Firing the trigger scrolls the native list to its last row.
    probe.clear_log();
    batch(|| scroll.notify());
    flush_sync();
    assert!(probe.widget(host).flag, "trigger scrolled the list to end");
    assert!(
        probe
            .mutations()
            .iter()
            .any(|m| m.contains("scroll-to-end"))
    );
}

#[test]
fn list_scroll_to_end_is_a_noop_when_empty() {
    let items: Signal<Vec<String>> = Signal::new(Vec::new());
    let scroll = Trigger::new();
    let probe = boot(move || {
        list(
            move || items.get(),
            |s: &String| s.clone(),
            |row: ItemSlot<String, String>| label(move || row.get()),
        )
        .scroll_to_end(scroll)
        .any()
    });
    let host = probe.find_by_kind("day.list")[0].0;
    probe.clear_log();
    batch(|| scroll.notify());
    flush_sync();
    // day-core guards the empty case: no ScrollToEnd patch ever reaches the backend.
    assert!(!probe.widget(host).flag);
    assert!(
        !probe
            .mutations()
            .iter()
            .any(|m| m.contains("scroll-to-end"))
    );
}

#[test]
fn list_stick_to_bottom_scrolls_on_data_change() {
    let items = Signal::new(vec!["a".to_string(), "b".into()]);
    let probe = boot(move || {
        list(
            move || items.get(),
            |s: &String| s.clone(),
            |row: ItemSlot<String, String>| label(move || row.get()),
        )
        .row_height(RowHeight::Uniform(20.0))
        .stick_to_bottom(true)
        .any()
    });
    let host = probe.find_by_kind("day.list")[0].0;
    assert!(
        !probe.widget(host).flag,
        "initial build does not auto-scroll"
    );

    // A data change (a new message arriving) sticks to the bottom.
    probe.clear_log();
    batch(|| items.update(|v| v.push("c".into())));
    flush_sync();
    assert!(probe.widget(host).flag);
    assert!(
        probe
            .mutations()
            .iter()
            .any(|m| m.contains("scroll-to-end"))
    );
}

// ---------------------------------------------------------------------------
// Surface + grow decorators (background / corner_radius / grow*).
// ---------------------------------------------------------------------------

// The chat-bubble recipe: a padded label on a rounded colored surface. `background` and
// `corner_radius` each wrap the piece in a native container carrying the surface style.
#[test]
fn background_and_corner_radius_form_a_rounded_surface() {
    let probe = boot(|| {
        label("Hi")
            .padding(10.0)
            .background(Color::hex(0x2F6FDE))
            .corner_radius(12.0)
            .any()
    });
    assert_eq!(probe.find_by_kind("day.label")[0].1.text, "Hi");
    let containers = probe.find_by_kind("day.container");
    // Exactly one container carries the fill; exactly one rounds+clips.
    assert_eq!(
        containers
            .iter()
            .filter(|(_, w)| w.background == Some(Color::hex(0x2F6FDE)))
            .count(),
        1,
        "one colored surface"
    );
    assert_eq!(
        containers
            .iter()
            .filter(|(_, w)| w.corner_radius == 12.0 && w.clips)
            .count(),
        1,
        "one rounded clip"
    );
}

// A reactive background repaints the surface (one Background patch) when its signal changes.
#[test]
fn reactive_background_patches_the_surface() {
    let color = Signal::new(Color::hex(0x111111));
    let probe = boot(move || label("x").background(move || color.get()).any());
    let surface = probe
        .find_by_kind("day.container")
        .into_iter()
        .find(|(_, w)| w.background == Some(Color::hex(0x111111)))
        .expect("colored surface")
        .0;

    probe.clear_log();
    batch(|| color.set(Color::hex(0xEE0000)));
    flush_sync();
    assert_eq!(probe.widget(surface).background, Some(Color::hex(0xEE0000)));
    assert!(
        probe.mutations().iter().any(|m| m.contains("bg=")),
        "one background patch"
    );
}

// `grow_w` makes the surface fill the offered width (a filling pane) — the layout honours Flex.
#[test]
fn grow_w_fills_the_available_width() {
    let probe = boot(|| row((label("a").background(Color::hex(0x222222)).grow_w(),)).any());
    let surface = probe
        .find_by_kind("day.container")
        .into_iter()
        .find(|(_, w)| w.background == Some(Color::hex(0x222222)))
        .expect("colored surface")
        .0;
    // The 400pt-wide window: the growing surface takes the whole width, not the label's intrinsic.
    assert_eq!(probe.widget(surface).frame.size.width, 400.0);
}

// ---------------------------------------------------------------------------
// Shapes (docs/shapes.md): canvas-backed shape pieces, transforms, gestures.
// ---------------------------------------------------------------------------

#[test]
fn shape_records_fill_then_stroke() {
    let probe = boot(|| {
        circle()
            .fill(Color::hex(0xff0000))
            .stroke(Color::hex(0x0000ff), 2.0)
            .frame(100.0, 100.0)
            .any()
    });
    let canvases = probe.find_by_kind("day.canvas");
    assert_eq!(canvases.len(), 1);
    let ops = &canvases[0].1.ops;
    // A circle inscribes its frame → an Ellipse; fill records before stroke.
    assert!(
        matches!(ops[0], DrawOp::Fill(Shape::Ellipse(_), _)),
        "{ops:?}"
    );
    assert!(
        matches!(ops[1], DrawOp::Stroke(Shape::Ellipse(_), _, _)),
        "{ops:?}"
    );
}

#[test]
fn shape_rotate_wraps_geometry_in_a_transform() {
    let probe = boot(|| {
        rectangle()
            .fill(Color::hex(0x00ff00))
            .rotate(45.0)
            .frame(80.0, 80.0)
            .any()
    });
    let ops = &probe.find_by_kind("day.canvas")[0].1.ops;
    assert!(matches!(ops[0], DrawOp::Save), "{ops:?}");
    assert!(matches!(ops[1], DrawOp::Concat(_)), "{ops:?}");
    assert!(matches!(ops[2], DrawOp::Fill(Shape::Rect(_), _)), "{ops:?}");
    assert!(matches!(ops[3], DrawOp::Restore), "{ops:?}");
}

#[test]
fn shape_tap_enables_gesture_and_hit_tests_the_path() {
    let taps = std::rc::Rc::new(std::cell::Cell::new(0));
    let t2 = taps.clone();
    let probe = boot(move || {
        circle()
            .fill(Color::WHITE)
            .on_tap(move || t2.set(t2.get() + 1))
            .frame(100.0, 100.0)
            .any()
    });
    assert!(
        probe
            .log()
            .iter()
            .any(|l| l.contains("enable_gesture") && l.contains("Tap")),
        "shape must enable the Tap gesture"
    );
    let node = node_id(&probe, "day.canvas", 0);
    // Centre of the 100×100 frame is inside the inscribed circle → fires.
    probe.emit(node, Event::Tap(Point::new(50.0, 50.0)));
    flush_sync();
    assert_eq!(taps.get(), 1);
    // A corner is outside the circle → path-precise test rejects it.
    probe.emit(node, Event::Tap(Point::new(3.0, 3.0)));
    flush_sync();
    assert_eq!(taps.get(), 1, "corner tap must miss the circle");
}

#[test]
fn shape_fill_rebinds_reactively() {
    let on = Signal::new(false);
    let probe = boot(move || {
        circle()
            .fill(move || {
                if on.get() {
                    Color::hex(0xff0000)
                } else {
                    Color::hex(0x222222)
                }
            })
            .frame(60.0, 60.0)
            .any()
    });
    let node = probe.find_by_kind("day.canvas")[0].0;
    let red = |p: &MockProbe| {
        matches!(p.widget(node).ops.first(),
        Some(DrawOp::Fill(_, c)) if c.r > 0.5)
    };
    assert!(!red(&probe));
    batch(|| on.set(true));
    flush_sync();
    assert!(
        red(&probe),
        "fill colour must re-record when its signal flips"
    );
}

// ---------------------------------------------------------------------------
// File open / save (docs/files.md) — the FileUrl type + the picker round-trip.
// ---------------------------------------------------------------------------

#[test]
fn file_url_local_path_and_name() {
    // A filesystem path (and file:// URL) resolves to a PathBuf; a content:// URI does not.
    let p = FileUrl::new("/tmp/notes.txt");
    assert_eq!(
        p.local_path(),
        Some(std::path::PathBuf::from("/tmp/notes.txt"))
    );
    assert_eq!(p.file_name().as_deref(), Some("notes.txt"));

    let f = FileUrl::new("file:///tmp/a/b.md");
    assert_eq!(
        f.local_path(),
        Some(std::path::PathBuf::from("/tmp/a/b.md"))
    );

    let c = FileUrl::new("content://com.android.providers/doc/42");
    assert_eq!(c.local_path(), None); // not directly readable
    assert!(c.read_to_string().is_err());
}

#[test]
fn open_file_reads_the_chosen_path() {
    // Write a real file, then drive open_file → respond with its path → the app reads it back.
    let dir = std::env::temp_dir();
    let path = dir.join(format!("day-open-test-{}.txt", std::process::id()));
    std::fs::write(&path, b"opened contents").unwrap();

    let out: Rc<RefCell<Option<String>>> = Rc::default();
    let o2 = out.clone();
    let probe = boot(move || {
        let o2 = o2.clone();
        button("open")
            .action(move || {
                let o2 = o2.clone();
                day_core::task(async move {
                    if let Some(file) = open_file().filter("Text", &["txt"]).await {
                        *o2.borrow_mut() = file.read_to_string().ok();
                    }
                })
            })
            .id("open")
            .any()
    });
    probe.emit(node_id(&probe, "day.button", 0), Event::Pressed);
    let (req, spec) = day_core::pending_presentation().expect("open picker pending");
    assert!(matches!(
        spec,
        day_spec::present::PresentSpec::OpenFile { .. }
    ));
    day_core::respond_presentation(
        req,
        PresentResult::Files(vec![path.to_string_lossy().into_owned()]),
    );
    flush_sync();
    assert_eq!(out.borrow().as_deref(), Some("opened contents"));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn save_file_writes_data_to_the_chosen_path() {
    // Drive save_file → respond with a destination path → the bytes land there.
    let dir = std::env::temp_dir();
    let dest = dir.join(format!("day-save-test-{}.txt", std::process::id()));
    let _ = std::fs::remove_file(&dest);

    let saved: Rc<RefCell<Option<String>>> = Rc::default();
    let s2 = saved.clone();
    let probe = boot(move || {
        let s2 = s2.clone();
        button("save")
            .action(move || {
                let s2 = s2.clone();
                day_core::task(async move {
                    let dest = save_file(b"written by day".to_vec())
                        .suggested_name("out.txt")
                        .await;
                    *s2.borrow_mut() = dest.and_then(|d| d.file_name());
                })
            })
            .id("save")
            .any()
    });
    probe.emit(node_id(&probe, "day.button", 0), Event::Pressed);
    let (req, spec) = day_core::pending_presentation().expect("save picker pending");
    assert_eq!(spec.suggested_name(), "out.txt");
    assert!(
        !spec.src_path().is_empty(),
        "save spec stages a temp source file"
    );
    day_core::respond_presentation(
        req,
        PresentResult::Files(vec![dest.to_string_lossy().into_owned()]),
    );
    flush_sync();
    // The pieces layer copied the staged bytes to the chosen local destination.
    assert_eq!(std::fs::read(&dest).unwrap(), b"written by day");
    assert_eq!(
        saved.borrow().as_deref(),
        Some(dest.file_name().unwrap().to_str().unwrap())
    );
    let _ = std::fs::remove_file(&dest);
}
