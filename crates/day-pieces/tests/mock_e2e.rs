//! M1 acceptance (DESIGN.md §21.2): end-to-end on the mock toolkit. The op log IS the
//! fine-grained-invalidation contract — "exactly one mutation op per state change" and
//! "bounded measure calls" are assertions, not aspirations.

use day_core::AnyPiece;
use day_mock::{MockProbe, MockToolkit};
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
        min_size: None,
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
// Navigation (docs/navigation.md)
// ---------------------------------------------------------------------------

fn nav_root() -> AnyPiece {
    nav(
        "Home",
        column((label("root-content"), nav_link("Go", "about"), nav_menu())),
    )
    .route("about", "About", || label("about-content"))
    .route("extra", "Extra", || label("extra-content"))
    .any()
}

#[test]
fn nav_push_builds_lazily_and_pop_disposes() {
    let probe = boot(nav_root);
    // Root page only; destinations are unbuilt.
    assert_eq!(probe.find_by_kind("day.nav").len(), 1);
    assert_eq!(probe.find_by_kind("day.nav_page").len(), 1);
    assert!(
        probe
            .find_by_kind("day.label")
            .iter()
            .all(|(_, w)| w.text != "about-content")
    );
    assert_eq!(day_core::current_route().as_deref(), Some(""));

    probe.clear_log();
    assert!(navigate("about"));
    assert_eq!(day_core::current_route().as_deref(), Some("about"));
    assert_eq!(probe.find_by_kind("day.nav_page").len(), 2);
    assert!(
        probe
            .find_by_kind("day.label")
            .iter()
            .any(|(_, w)| w.text == "about-content"),
        "destination content built on push"
    );
    assert!(
        probe
            .log()
            .iter()
            .any(|l| l.contains("nav pushed title=\"About\"")),
        "host received Pushed patch: {:?}",
        probe.log()
    );

    probe.clear_log();
    assert!(nav_back());
    assert_eq!(day_core::current_route().as_deref(), Some(""));
    assert_eq!(probe.find_by_kind("day.nav_page").len(), 1);
    assert!(
        probe
            .find_by_kind("day.label")
            .iter()
            .all(|(_, w)| w.text != "about-content")
    );
    assert!(probe.log().iter().any(|l| l.contains("nav popped")));
    // Nothing left to pop.
    assert!(!nav_back());
}

#[test]
fn navigate_has_reset_semantics_and_rejects_unknown() {
    let probe = boot(nav_root);
    assert!(navigate("about"));
    assert!(navigate("extra"));
    // Reset-to: the stack is replaced, not deepened.
    assert_eq!(day_core::current_route().as_deref(), Some("extra"));
    assert_eq!(probe.find_by_kind("day.nav_page").len(), 2);
    assert!(!navigate("nope"));
    assert_eq!(day_core::current_route().as_deref(), Some("extra"));
    // "" returns to the root.
    assert!(navigate(""));
    assert_eq!(day_core::current_route().as_deref(), Some(""));
}

#[test]
fn nav_link_navigates_through_the_event_path() {
    let probe = boot(nav_root);
    let link = probe
        .find_by_kind("day.button")
        .into_iter()
        .find(|(_, w)| w.text == "Go")
        .expect("nav_link renders a button");
    probe.emit(NodeId(link.1.node), Event::Pressed);
    assert_eq!(day_core::current_route().as_deref(), Some("about"));
}

#[test]
fn native_back_syncs_without_reissuing_pop_patch() {
    let probe = boot(nav_root);
    assert!(navigate("about"));
    let host = node_id(&probe, "day.nav", 0);
    probe.clear_log();
    // iOS-style: the toolkit popped natively already.
    probe.emit(
        host,
        Event::NavBack {
            already_popped: true,
        },
    );
    assert_eq!(day_core::current_route().as_deref(), Some(""));
    assert!(
        !probe.log().iter().any(|l| l.contains("nav popped")),
        "already_popped must not re-issue the Popped patch: {:?}",
        probe.log()
    );
    // Android-style: day drives the native pop.
    assert!(navigate("about"));
    probe.clear_log();
    probe.emit(
        host,
        Event::NavBack {
            already_popped: false,
        },
    );
    assert_eq!(day_core::current_route().as_deref(), Some(""));
    assert!(probe.log().iter().any(|l| l.contains("nav popped")));
}

#[test]
fn deep_link_env_navigates_at_startup() {
    let probe = boot_with_env(Some(("DAY_DEEPLINK", "about")), nav_root);
    flush_sync();
    assert_eq!(day_core::current_route().as_deref(), Some("about"));
    assert!(
        probe
            .find_by_kind("day.label")
            .iter()
            .any(|(_, w)| w.text == "about-content")
    );
}

#[test]
fn nav_menu_lists_routes_selects_and_syncs() {
    let probe = boot(nav_root);
    let menus = probe.find_by_kind("day.nav_menu");
    assert_eq!(menus.len(), 1);
    assert_eq!(menus[0].1.text, "About|Extra");
    assert_eq!(menus[0].1.value, -1.0, "nothing selected at root");

    // Native selection (row tap) navigates to the route...
    probe.emit(NodeId(menus[0].1.node), Event::SelectionChanged(1));
    assert_eq!(day_core::current_route().as_deref(), Some("extra"));
    // ...and the highlight syncs back to the menu.
    assert_eq!(probe.find_by_kind("day.nav_menu")[0].1.value, 1.0);

    // Programmatic navigation also highlights; popping to root clears it.
    assert!(navigate("about"));
    assert_eq!(probe.find_by_kind("day.nav_menu")[0].1.value, 0.0);
    assert!(nav_back());
    assert_eq!(probe.find_by_kind("day.nav_menu")[0].1.value, -1.0);
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
