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

/// The `day.container` that directly parents every `day.label` — the piece's own z-layering panel,
/// as opposed to the mock's window-root container. (`MockWidget::children` holds child handle ids.)
fn container_of_labels(probe: &MockProbe) -> day_mock::MockWidget {
    let label_handles: Vec<u64> = probe
        .find_by_kind("day.label")
        .iter()
        .map(|(h, _)| h.0)
        .collect();
    let mut found: Vec<_> = probe
        .find_by_kind("day.container")
        .into_iter()
        .filter(|(_, w)| label_handles.iter().all(|lh| w.children.contains(lh)))
        .collect();
    assert_eq!(
        found.len(),
        1,
        "expected exactly one container parenting the labels"
    );
    found.remove(0).1
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
    // current_route is the FULL path (docs/navigation.md).
    assert_eq!(day_core::current_route().as_deref(), Some("a/b"));

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
    // Full route: the selector's key; the inner stack is at its root and contributes nothing.
    assert_eq!(day_core::current_route().as_deref(), Some("drill"));

    // Push onto the inner stack via its path (app state).
    batch(|| path.set(vec!["deep".into()]));
    flush_sync();
    assert_eq!(day_core::current_route().as_deref(), Some("drill/deep"));
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

#[test]
fn absolute_route_descends_into_lazily_mounted_stack() {
    // navigate("drill/one/two?hint=linked"): the selector anchors "drill", the stack — which
    // only MOUNTS as the section switch takes effect — consumes "one","two" as it registers,
    // and the destination builders see the query params (docs/navigation.md).
    let section = Signal::new(String::new());
    let seen_params: Rc<RefCell<Vec<String>>> = Rc::default();
    let probe = boot({
        let seen = seen_params.clone();
        move || {
            selector(section)
                .title("Root")
                .item("plain", "Plain", || label("plain-content"))
                .item("drill", "Drill", {
                    let seen = seen.clone();
                    move || {
                        let path = Signal::new(Vec::<String>::new());
                        let seen = seen.clone();
                        stack(path, label("drill-root")).destination(move |k| {
                            seen.borrow_mut()
                                .push(format!("{k}:{}", route_param("hint").unwrap_or_default()));
                            label(format!("drill:{k}"))
                        })
                    }
                })
                .any()
        }
    });

    assert!(navigate("drill/one/two?hint=linked"));
    flush_sync();
    assert_eq!(section.get_untracked(), "drill");
    assert_eq!(day_core::current_route().as_deref(), Some("drill/one/two"));
    assert!(
        probe
            .find_by_kind("day.label")
            .iter()
            .any(|(_, w)| w.text == "drill:two")
    );
    // Both pushed destinations were built with the navigation's params in scope.
    assert_eq!(
        seen_params.borrow().as_slice(),
        ["one:linked".to_string(), "two:linked".to_string()]
    );

    // The full route round-trips: navigating to it again is a no-op reset to the same state.
    let route = day_core::current_route().unwrap();
    assert!(navigate(&route));
    flush_sync();
    assert_eq!(day_core::current_route().as_deref(), Some("drill/one/two"));

    // An absolute route to a sibling section resets the drill state entirely.
    assert!(navigate("plain"));
    flush_sync();
    assert_eq!(day_core::current_route().as_deref(), Some("plain"));
}

#[test]
fn absolute_route_resets_inner_surfaces_of_the_anchor() {
    // With "drill/deep" active, navigate("drill/other") must yield exactly drill/other — the
    // previously pushed "deep" page pops (absolute path = the whole state, set-semantics).
    let section = Signal::new(String::new());
    let probe = boot(move || {
        selector(section)
            .title("Root")
            .item("drill", "Drill", move || {
                let path = Signal::new(Vec::<String>::new());
                stack(path, label("drill-root")).destination(|k| label(format!("drill:{k}")))
            })
            .any()
    });

    assert!(navigate("drill/deep"));
    flush_sync();
    assert_eq!(day_core::current_route().as_deref(), Some("drill/deep"));

    assert!(navigate("drill/other"));
    flush_sync();
    assert_eq!(day_core::current_route().as_deref(), Some("drill/other"));
    let labels = probe.find_by_kind("day.label");
    assert!(labels.iter().any(|(_, w)| w.text == "drill:other"));
    assert!(labels.iter().all(|(_, w)| w.text != "drill:deep"));
}

/// The sidebar-over-stack fixture: mock reports `NavSplit=Unsupported`, so the sidebar collapses
/// to a push stack and a stack in its detail runs the merged path (docs/navigation.md).
fn merge_fixture(section: Signal<String>, path: Signal<Vec<String>>) -> AnyPiece {
    selector(section)
        .item("plain", "Plain", || label("plain-content"))
        .item("drill", "Drill", move || {
            stack(path, label("drill-root")).destination(|k| label(format!("drill:{k}")))
        })
        .any()
}

#[test]
fn nested_stack_merges_into_one_host() {
    let section = Signal::new(String::new());
    let path = Signal::new(Vec::<String>::new());
    let probe = boot(move || merge_fixture(section, path));

    assert!(navigate("drill"));
    flush_sync();
    // ONE native nav host, not two — the whole point of the merge (would be 2 before the fix).
    assert_eq!(
        probe.find_by_kind("day.nav").len(),
        1,
        "nested stack merges into the enclosing host"
    );
    // The stack's root renders inline in the detail page: root list + detail, no extra root page.
    assert_eq!(probe.find_by_kind("day.nav_page").len(), 2);
    assert!(
        probe
            .find_by_kind("day.label")
            .iter()
            .any(|(_, w)| w.text == "drill-root")
    );

    // A push lands as a page on that same host.
    batch(|| path.set(vec!["deep".into()]));
    flush_sync();
    assert_eq!(day_core::current_route().as_deref(), Some("drill/deep"));
    assert_eq!(probe.find_by_kind("day.nav").len(), 1);
    assert_eq!(probe.find_by_kind("day.nav_page").len(), 3);
    assert!(
        probe
            .find_by_kind("day.label")
            .iter()
            .any(|(_, w)| w.text == "drill:deep")
    );
}

#[test]
fn merged_stack_back_pops_inner_then_outer() {
    let section = Signal::new(String::new());
    let path = Signal::new(Vec::<String>::new());
    let probe = boot(move || merge_fixture(section, path));

    assert!(navigate("drill"));
    batch(|| path.set(vec!["deep".into()]));
    flush_sync();
    assert_eq!(day_core::current_route().as_deref(), Some("drill/deep"));
    let host = node_id(&probe, "day.nav", 0);

    // First native back on the shared host → the topmost owner is the stack page → pop the path.
    probe.emit(
        host,
        Event::NavBack {
            already_popped: true,
        },
    );
    flush_sync();
    assert_eq!(day_core::current_route().as_deref(), Some("drill"));
    assert_eq!(probe.find_by_kind("day.nav_page").len(), 2);
    assert!(
        probe
            .find_by_kind("day.label")
            .iter()
            .any(|(_, w)| w.text == "drill-root")
    );

    // Second back → now the topmost owner is the sidebar detail → deselect to the list.
    probe.emit(
        host,
        Event::NavBack {
            already_popped: true,
        },
    );
    flush_sync();
    assert_eq!(day_core::current_route().as_deref(), Some(""));
    assert_eq!(probe.find_by_kind("day.nav_page").len(), 1);
}

#[test]
fn merged_stack_cleanup_on_section_switch() {
    let section = Signal::new(String::new());
    let path = Signal::new(Vec::<String>::new());
    let probe = boot(move || merge_fixture(section, path));

    assert!(navigate("drill"));
    batch(|| path.set(vec!["deep".into()]));
    flush_sync();
    assert!(
        probe
            .find_by_kind("day.label")
            .iter()
            .any(|(_, w)| w.text == "drill:deep")
    );

    // Switch section via a sibling key: it falls through to the sidebar, which disposes the
    // detail — the merged stack's cleanup pops its pages off the shared host.
    assert!(navigate("plain"));
    flush_sync();
    assert_eq!(section.get_untracked(), "plain");
    assert_eq!(probe.find_by_kind("day.nav").len(), 1);
    assert_eq!(
        probe.find_by_kind("day.nav_page").len(),
        2,
        "only the root list + the new detail remain"
    );
    assert!(
        probe
            .find_by_kind("day.label")
            .iter()
            .all(|(_, w)| w.text != "drill:deep" && w.text != "drill-root")
    );
    assert!(
        probe
            .find_by_kind("day.label")
            .iter()
            .any(|(_, w)| w.text == "plain-content")
    );
}

#[test]
fn grandchild_stack_merges() {
    // A stack inside a stack's destination merges into the same enclosing host.
    let section = Signal::new(String::new());
    let outer = Signal::new(Vec::<String>::new());
    let inner = Signal::new(Vec::<String>::new());
    let probe = boot(move || {
        selector(section)
            .item("drill", "Drill", move || {
                stack(outer, label("outer-root")).destination(move |_k| {
                    stack(inner, label("inner-root")).destination(|k2| label(format!("g:{k2}")))
                })
            })
            .any()
    });

    assert!(navigate("drill"));
    batch(|| outer.set(vec!["mid".into()]));
    flush_sync();
    assert_eq!(
        probe.find_by_kind("day.nav").len(),
        1,
        "the destination stack merged too"
    );
    // Drive the grandchild stack.
    batch(|| inner.set(vec!["leaf".into()]));
    flush_sync();
    assert_eq!(probe.find_by_kind("day.nav").len(), 1, "still one host");
    assert!(
        probe
            .find_by_kind("day.label")
            .iter()
            .any(|(_, w)| w.text == "g:leaf")
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
                });
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
                });
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
                });
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
                });
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
        Some(DrawOp::Fill(_, Paint::Solid(c))) if c.r > 0.5)
    };
    assert!(!red(&probe));
    batch(|| on.set(true));
    flush_sync();
    assert!(
        red(&probe),
        "fill colour must re-record when its signal flips"
    );
}

#[test]
fn shape_fill_linear_records_gradient_paint() {
    let night = Signal::new(false);
    let probe = boot(move || {
        rectangle()
            .fill_linear(move || {
                if night.get() {
                    LinearGradient::vertical(Color::hex(0x0e1430), Color::hex(0x2c3a66))
                } else {
                    LinearGradient::vertical(Color::hex(0x2e6fb8), Color::hex(0x7fb2e5))
                }
            })
            .frame(60.0, 60.0)
            .any()
    });
    let node = probe.find_by_kind("day.canvas")[0].0;
    let top_red = |p: &MockProbe| match p.widget(node).ops.first() {
        Some(DrawOp::Fill(_, Paint::Linear(g))) => {
            assert_eq!(g.start, UnitPoint::TOP);
            assert_eq!(g.end, UnitPoint::BOTTOM);
            assert_eq!(g.stops.len(), 2);
            g.stops[0].1.r
        }
        other => panic!("expected a gradient fill, got {other:?}"),
    };
    assert!(top_red(&probe) > 0.15, "day sky top stop");
    batch(|| night.set(true));
    flush_sync();
    assert!(
        top_red(&probe) < 0.1,
        "gradient must re-record when its signal flips"
    );

    // The packed encoding round-trips the gradient: kind 14 precedes its fill record and the
    // stops ride the texts channel.
    let ops = probe.widget(node).ops.clone();
    let (nums, texts) = day_spec::encode_ops(&ops);
    assert_eq!(nums[0], 14.0, "set-gradient record first");
    assert_eq!(nums[9], 0.0, "fill-rect record second");
    assert!(
        texts[0].split(' ').count() == 2 && texts[0].contains(','),
        "two stops on the texts channel: {:?}",
        texts[0]
    );
}

#[test]
fn focus_two_way_bool_binding() {
    let editing = Signal::new(false);
    let probe = boot(move || text_field(Signal::new(String::new())).focused(editing));
    let node = node_id(&probe, "day.text_field", 0);

    // Native gain writes the signal; the echo cell must swallow the resulting bind apply
    // (no `focus` duty op for a state the widget already has).
    let before = probe.log_len();
    probe.emit(node, Event::FocusChanged(true));
    flush_sync();
    assert!(editing.get_untracked(), "native gain writes the signal");
    assert!(
        !probe
            .log_since(before)
            .iter()
            .any(|l| l.starts_with("focus #")),
        "a native focus change must not re-drive the toolkit"
    );

    // A programmatic resign drives the duty.
    batch(|| editing.set(false));
    flush_sync();
    assert!(
        probe
            .log()
            .iter()
            .any(|l| l.ends_with(" false") && l.starts_with("focus #")),
        "programmatic resign drives the focus duty: {:?}",
        probe.log()
    );
}

#[test]
fn focus_group_moves_without_none_blip() {
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    enum Field {
        A,
        B,
    }
    let focus = Signal::new(None::<Field>);
    let blipped = Rc::new(std::cell::Cell::new(false));
    let b2 = blipped.clone();
    let probe = boot(move || {
        // Watch for an observable None between A and B.
        let seen_a = std::cell::Cell::new(false);
        watch(
            move || focus.get(),
            move |new, _| {
                if *new == Some(Field::A) {
                    seen_a.set(true);
                } else if new.is_none() && seen_a.get() {
                    b2.set(true);
                }
            },
        );
        column((
            text_field(Signal::new(String::new())).focused((focus, Field::A)),
            text_field(Signal::new(String::new())).focused((focus, Field::B)),
        ))
        .any()
    });
    let (a, b) = (
        node_id(&probe, "day.text_field", 0),
        node_id(&probe, "day.text_field", 1),
    );

    probe.emit(a, Event::FocusChanged(true));
    flush_sync();
    assert_eq!(focus.get_untracked(), Some(Field::A));

    // Focus moves natively: the loss for A and the gain for B arrive in the same drain — the
    // pump dispatches the gain first (docs/focus.md), so the group signal never reads None.
    day_core::enqueue_events([
        (a, Event::FocusChanged(false)),
        (b, Event::FocusChanged(true)),
    ]);
    flush_sync();
    assert_eq!(focus.get_untracked(), Some(Field::B));
    assert!(!blipped.get(), "group signal must not blip through None");

    // Losing focus to a non-Day target clears the signal.
    probe.emit(b, Event::FocusChanged(false));
    flush_sync();
    assert_eq!(focus.get_untracked(), None);
}

#[test]
fn focus_initial_some_requests_focus_on_mount() {
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    enum Field {
        Name,
    }
    let focus = Signal::new(Some(Field::Name));
    let probe = boot(move || text_field(Signal::new(String::new())).focused((focus, Field::Name)));
    flush_sync();
    assert!(
        probe
            .log()
            .iter()
            .any(|l| l.starts_with("focus #") && l.ends_with(" true")),
        "a signal that already names the control requests focus at mount: {:?}",
        probe.log()
    );
}

#[test]
fn text_field_on_submit_fires() {
    let submitted = Signal::new(0i64);
    let probe = boot(move || {
        text_field(Signal::new(String::new()))
            .on_submit(move || submitted.update(|n| *n += 1))
            .any()
    });
    let node = node_id(&probe, "day.text_field", 0);
    probe.emit(node, Event::Submitted);
    flush_sync();
    assert_eq!(submitted.get_untracked(), 1);
}

#[test]
fn shape_fill_radial_records_gradient_paint() {
    let probe = boot(move || {
        circle()
            .fill_radial(RadialGradient::centered(
                Color::hex(0xfff2b0),
                Color::hex(0x3e86c9),
            ))
            .frame(60.0, 60.0)
            .any()
    });
    let node = probe.find_by_kind("day.canvas")[0].0;
    let ops = probe.widget(node).ops.clone();
    match ops.first() {
        Some(DrawOp::Fill(_, Paint::Radial(g))) => {
            assert_eq!(g.center, UnitPoint::CENTER);
            assert_eq!(g.radius, 0.5);
            assert_eq!(g.stops.len(), 2);
        }
        other => panic!("expected a radial fill, got {other:?}"),
    }
    // Encoding: one kind-14 set-gradient record with the radial discriminant (slot f = 1),
    // center in a,b and radius in c, then the fill-shape record.
    let (nums, texts) = day_spec::encode_ops(&ops);
    assert_eq!(nums[0], 14.0, "set-gradient record first");
    assert_eq!(nums[6], 1.0, "radial type discriminant in slot f");
    assert_eq!((nums[1], nums[2]), (0.5, 0.5), "center unit point");
    assert_eq!(nums[3], 0.5, "unit radius");
    assert_eq!(nums[9], 3.0, "fill-ellipse record second");
    assert!(
        texts[0].split(' ').count() == 2,
        "two stops on the texts channel: {:?}",
        texts[0]
    );
}

#[test]
fn line_records_stroke_only_at_unit_points() {
    let probe = boot(|| {
        line((0.16, 0.72), (0.84, 0.72))
            .fill(Color::WHITE) // ignored: a line has no interior
            .stroke(Color::hex(0xffffff), 2.0)
            .frame(100.0, 100.0)
            .any()
    });
    let ops = &probe.find_by_kind("day.canvas")[0].1.ops;
    assert_eq!(ops.len(), 1, "stroke only, no fill: {ops:?}");
    // No stroke-half inset for open kinds: endpoints resolve exactly at the unit points.
    assert_eq!(
        ops[0],
        DrawOp::Stroke(
            Shape::Line(Point::new(16.0, 72.0), Point::new(84.0, 72.0)),
            Color::hex(0xffffff),
            2.0
        ),
        "{ops:?}"
    );
}

#[test]
fn polygon_resolves_unit_points_and_allows_overflow() {
    let probe = boot(|| {
        polygon([(0.5, 0.0), (1.0, 1.0), (0.44, 1.02), (0.0, 1.0)])
            .fill(Color::WHITE)
            .frame(50.0, 50.0)
            .any()
    });
    let ops = &probe.find_by_kind("day.canvas")[0].1.ops;
    match &ops[0] {
        DrawOp::Fill(Shape::Polygon(pts), _) => {
            assert_eq!(pts[0], Point::new(25.0, 0.0));
            // Unit points resolve unclamped — 1.02 lands past the frame edge on purpose.
            assert_eq!(pts[2], Point::new(22.0, 51.0));
        }
        other => panic!("expected a polygon fill, got {other:?}"),
    }
}

#[test]
fn shape_at_places_fractional_subrect() {
    let probe = boot(|| {
        ellipse()
            .fill(Color::WHITE)
            .at(0.25, 0.25, 0.5, 0.5)
            .frame(100.0, 100.0)
            .any()
    });
    let ops = &probe.find_by_kind("day.canvas")[0].1.ops;
    assert_eq!(
        ops[0],
        DrawOp::Fill(
            Shape::Ellipse(Rect::new(25.0, 25.0, 50.0, 50.0)),
            Paint::Solid(Color::WHITE)
        ),
        "{ops:?}"
    );
}

#[test]
fn shape_group_flattens_to_one_canvas_leaf() {
    let probe = boot(|| {
        shape_group([
            rectangle().fill(Color::hex(0x111111)),
            circle().fill(Color::hex(0x222222)),
            line((0.0, 0.5), (1.0, 0.5)).stroke(Color::hex(0x333333), 1.0),
        ])
        .frame(80.0, 80.0)
        .any()
    });
    let canvases = probe.find_by_kind("day.canvas");
    assert_eq!(canvases.len(), 1, "a group is ONE canvas leaf");
    let ops = &canvases[0].1.ops;
    // Ops record in child order.
    assert!(matches!(ops[0], DrawOp::Fill(Shape::Rect(_), _)), "{ops:?}");
    assert!(
        matches!(ops[1], DrawOp::Fill(Shape::Ellipse(_), _)),
        "{ops:?}"
    );
    assert!(
        matches!(ops[2], DrawOp::Stroke(Shape::Line(_, _), _, _)),
        "{ops:?}"
    );
}

#[test]
fn shape_group_reactive_fill_rerecords() {
    let on = Signal::new(false);
    let probe = boot(move || {
        shape_group([
            rectangle().fill(Color::hex(0x000000)),
            circle().fill(move || {
                if on.get() {
                    Color::hex(0xff0000)
                } else {
                    Color::hex(0x222222)
                }
            }),
        ])
        .frame(60.0, 60.0)
        .any()
    });
    let node = probe.find_by_kind("day.canvas")[0].0;
    let red = |p: &MockProbe| {
        matches!(p.widget(node).ops.get(1),
        Some(DrawOp::Fill(_, Paint::Solid(c))) if c.r > 0.5)
    };
    assert!(!red(&probe));
    batch(|| on.set(true));
    flush_sync();
    assert!(
        red(&probe),
        "a child's reactive fill must re-record the group"
    );
}

#[test]
fn shape_group_fn_derives_children_from_size() {
    let probe = boot(|| {
        shape_group_fn(|size| {
            // A 10pt-wide bar expressed as a fraction of the laid-out width — only correct
            // if the closure really receives the final size.
            let f = 10.0 / size.width.max(1.0);
            vec![rectangle().fill(Color::WHITE).at(0.0, 0.0, f, 1.0)]
        })
        .frame(200.0, 20.0)
        .any()
    });
    let ops = &probe.find_by_kind("day.canvas")[0].1.ops;
    match &ops[0] {
        DrawOp::Fill(Shape::Rect(r), _) => {
            assert!(
                (r.size.width - 10.0).abs() < 1e-9 && (r.size.height - 20.0).abs() < 1e-9,
                "geometry must derive from the laid-out 200×20 size, got {r:?}"
            );
        }
        other => panic!("expected a rect fill, got {other:?}"),
    }
}

#[test]
fn polygon_tap_is_path_precise() {
    let taps = std::rc::Rc::new(std::cell::Cell::new(0));
    let t2 = taps.clone();
    let probe = boot(move || {
        polygon([(0.5, 0.0), (1.0, 1.0), (0.0, 1.0)])
            .fill(Color::WHITE)
            .on_tap(move || t2.set(t2.get() + 1))
            .frame(100.0, 100.0)
            .any()
    });
    let node = node_id(&probe, "day.canvas", 0);
    // Centroid of the triangle → inside.
    probe.emit(node, Event::Tap(Point::new(50.0, 70.0)));
    flush_sync();
    assert_eq!(taps.get(), 1);
    // The top-left corner is outside the triangle.
    probe.emit(node, Event::Tap(Point::new(5.0, 5.0)));
    flush_sync();
    assert_eq!(taps.get(), 1, "corner tap must miss the triangle");
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
                });
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

// ---------------------------------------------------------------------------
// Tier A.1 composition-first primitives: zstack / overlay / modifier / ButtonStyle / @Environment.
// ---------------------------------------------------------------------------

#[test]
fn zstack_sizes_to_union_and_centers() {
    // "aa" = 16x16, "bbbb" = 32x16 → the union is 32x16; children centered (default alignment).
    let probe = boot(|| zstack((label("aa"), label("bbbb"))).any());
    // The mock's window root is also a `day.container` (400x600); pick the z-stack's own panel.
    let stack = container_of_labels(&probe);
    assert_eq!(
        stack.frame.size,
        Size::new(32.0, 16.0),
        "z-stack sizes to the union of its children"
    );
    let labels = probe.find_by_kind("day.label");
    let aa = labels.iter().find(|(_, w)| w.text == "aa").unwrap();
    let bbbb = labels.iter().find(|(_, w)| w.text == "bbbb").unwrap();
    // Narrow child centered in the 32-wide union → x = 8; wide child fills it → x = 0.
    assert_eq!(aa.1.frame.origin.x, 8.0);
    assert_eq!(bbbb.1.frame.origin.x, 0.0);
    assert_eq!(aa.1.frame.origin.y, 0.0);
    assert_eq!(bbbb.1.frame.origin.y, 0.0);
}

#[test]
fn zstack_alignment_pins_to_corner() {
    let probe = boot(|| {
        zstack((label("aa"), label("bbbb")))
            .align(Alignment::TopTrailing)
            .any()
    });
    let labels = probe.find_by_kind("day.label");
    let aa = labels.iter().find(|(_, w)| w.text == "aa").unwrap();
    // "aa" (16 wide) pinned trailing in the 32-wide union → x = 16, top → y = 0.
    assert_eq!(aa.1.frame.origin.x, 16.0);
    assert_eq!(aa.1.frame.origin.y, 0.0);
}

#[test]
fn overlay_sizes_to_first_child() {
    // Content "aa" = 16x16; annotation "wwwwwwww" = 64x16. Sizing to the FIRST child gives a
    // 16x16 frame (a UNION would be 64x16) — the annotation does not grow the layout.
    let probe = boot(|| label("aa").overlay(label("wwwwwwww")).any());
    let overlay = container_of_labels(&probe);
    assert_eq!(
        overlay.frame.size,
        Size::new(16.0, 16.0),
        "overlay sizes to its content, not the annotation"
    );
    assert_eq!(
        probe.find_by_kind("day.label").len(),
        2,
        "both the content and the annotation are built"
    );
}

#[test]
fn modifier_closure_wraps_the_piece() {
    // A plain FnOnce(AnyPiece) -> AnyPiece is a Modifier (blanket impl): wrap the label in a surface.
    let probe = boot(|| label("m").modifier(|p: AnyPiece| p.background(Color::hex(0x445566))));
    assert_eq!(probe.find_by_kind("day.label")[0].1.text, "m");
    assert!(
        probe
            .find_by_kind("day.container")
            .iter()
            .any(|(_, w)| w.background == Some(Color::hex(0x445566))),
        "the modifier wrapped the label in a colored surface"
    );
}

#[test]
fn filled_button_style_tints_its_label_white() {
    assert_eq!(
        FilledButtonStyle {
            color: Color::hex(0x2F6FDE)
        }
        .label_color(),
        Some(Color::WHITE)
    );
}

#[test]
fn button_style_renders_a_composed_tappable_not_a_native_leaf() {
    let clicks = std::rc::Rc::new(std::cell::Cell::new(0));
    let c2 = clicks.clone();
    let probe = boot(move || {
        button("Go")
            .action(move || c2.set(c2.get() + 1))
            .style(FilledButtonStyle {
                color: Color::hex(0x2F6FDE),
            })
    });
    // A styled button is composed from pieces — no native button leaf is realized.
    assert!(
        probe.find_by_kind("day.button").is_empty(),
        "styled button uses no native leaf"
    );
    let labels = probe.find_by_kind("day.label");
    assert_eq!(labels.len(), 1);
    assert_eq!(labels[0].1.text, "Go");
    // The filled surface + rounded clip are present.
    let containers = probe.find_by_kind("day.container");
    assert!(
        containers
            .iter()
            .any(|(_, w)| w.background == Some(Color::hex(0x2F6FDE))),
        "colored fill surface"
    );
    let clip = containers
        .iter()
        .find(|(_, w)| w.clips && w.corner_radius == 8.0)
        .expect("rounded clip surface");
    // The action fires through the composed on_tap wired onto the outer surface.
    probe.emit(
        NodeId(clip.1.node),
        Event::Tap(day_spec::Point::new(1.0, 1.0)),
    );
    flush_sync();
    assert_eq!(clicks.get(), 1, "composed tap triggers the button action");
}

#[test]
fn with_environment_provides_to_descendants_only() {
    #[derive(Clone)]
    struct Tint(u32);
    let probe = boot(|| {
        column((
            with_environment(Tint(7), || {
                piece_fn(|cx| {
                    let v = environment::<Tint>().map(|t| t.0).unwrap_or(0);
                    label(format!("in={v}")).build(cx)
                })
            }),
            // A sibling OUTSIDE the environment scope must not see the value.
            piece_fn(|cx| {
                let v = environment::<Tint>().map(|t| t.0).unwrap_or(99);
                label(format!("out={v}")).build(cx)
            }),
        ))
        .any()
    });
    let texts: Vec<String> = probe
        .find_by_kind("day.label")
        .iter()
        .map(|(_, w)| w.text.clone())
        .collect();
    assert!(
        texts.contains(&"in=7".to_string()),
        "descendant reads the ambient value: {texts:?}"
    );
    assert!(
        texts.contains(&"out=99".to_string()),
        "sibling outside the scope reads None: {texts:?}"
    );
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
                });
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

// ---------------------------------------------------------------------------
// Tweaks (docs/tweaks.md): the mount hook, the NativeRef lifecycle, and size invalidation.
// ---------------------------------------------------------------------------

#[test]
fn tweak_runs_once_at_mount_with_live_downcastable_handle() {
    use std::cell::Cell;
    use std::rc::Rc;
    let runs = Rc::new(Cell::new(0u32));
    let typed = Rc::new(Cell::new(false));
    let _probe = boot({
        let (runs, typed) = (runs.clone(), typed.clone());
        move || {
            label("Hello")
                .tweak(move |n| {
                    runs.set(runs.get() + 1);
                    // The native handle exists at hook time and downcasts to the compiled
                    // backend's concrete Handle type — the tweaks-door contract.
                    let ok = day_core::with_tree(|t| t.node_handle_any(n))
                        .is_some_and(|h| h.downcast::<MockHandle>().is_ok());
                    typed.set(ok);
                })
                .any()
        }
    });
    assert_eq!(runs.get(), 1, "tweak must run exactly once, at mount");
    assert!(
        typed.get(),
        "handle must be live and downcast to MockHandle"
    );
}

#[test]
fn native_ref_tracks_mount_and_clears_on_disposal() {
    let r = NativeRef::new();
    assert!(r.node().is_none(), "unmounted ref resolves to None");
    let probe = boot({
        let r = r.clone();
        move || {
            let show = Signal::new(true);
            column((
                button("toggle").action(move || show.update(|s| *s = !*s)),
                when(move || show.get(), {
                    let r = r.clone();
                    move || label("tweaked").native_ref(&r)
                }),
            ))
            .any()
        }
    });
    let first = r.node().expect("mounted ref resolves");
    let btn = node_id(&probe, "day.button", 0);
    probe.emit(btn, Event::Pressed); // when-arm disposed → scope cleanup clears the ref
    assert!(r.node().is_none(), "disposal must clear the ref");
    assert!(r.with(|_| ()).is_none());
    probe.emit(btn, Event::Pressed); // arm rebuilt → ref points at the NEW node
    let second = r.node().expect("re-mounted ref resolves");
    assert_ne!(first, second, "rebuild yields a fresh node");
}

#[test]
fn invalidate_size_remeasures_the_tweaked_path() {
    let r = NativeRef::new();
    let probe = boot({
        let r = r.clone();
        move || label("resize me").native_ref(&r).any()
    });
    probe.clear_log();
    assert_eq!(probe.measure_calls(), 0);
    r.with(day_core::invalidate_size).expect("live node");
    flush_sync(); // turn boundary → layout re-enters at the boundary above the dirty node
    assert!(
        probe.measure_calls() > 0,
        "invalidate_size must trigger a re-measure of the node's path"
    );
}

#[test]
fn custom_font_flows_to_the_toolkit() {
    // A bundled custom font (§18.4) reaches the toolkit as `FontSpec { style: Font::Custom }`,
    // with weight/italic riding the same spec; an unstyled label stays on Font::Body.
    let probe = boot(|| {
        column((
            label("scripted")
                .font(Font::Custom("Pacifico", 24.0))
                .italic(),
            label("plain"),
        ))
        .any()
    });
    let labels = probe.find_by_kind("day.label");
    let custom = labels[0].1.font.expect("label carries a font spec");
    assert_eq!(custom.style, Font::Custom("Pacifico", 24.0));
    assert!(custom.italic);
    assert_eq!(labels[1].1.font.map(|f| f.style), Some(Font::Body));
}

// ---------------------------------------------------------------------------
// Typed routes (docs/navigation.md): Route enums over selector/stack.
// ---------------------------------------------------------------------------

day_pieces::routes! {
    /// Top-level sections for the typed-route tests.
    enum Area { Home => "home", Drill => "drill" }
}

/// A data-carrying stack route: `Leg(n)` ↔ `"leg-n"`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Leg(u32);
impl Route for Leg {
    fn key(&self) -> String {
        format!("leg-{}", self.0)
    }
    fn from_key(key: &str) -> Option<Self> {
        key.strip_prefix("leg-")?.parse().ok().map(Leg)
    }
}

#[test]
fn typed_route_encoding_round_trips() {
    assert_eq!(Area::from_key("drill"), Some(Area::Drill));
    assert_eq!(Area::from_key("nope"), None);
    assert_eq!(Option::<Area>::from_key(""), Some(None));
    assert_eq!(Option::<Area>::from_key("home"), Some(Some(Area::Home)));
    assert_eq!(Leg(7).key(), "leg-7");
    assert_eq!(Leg::from_key("leg-7"), Some(Leg(7)));
    assert_eq!(Leg::from_key("leg-x"), None);
    // RoutePath builds the encoded wire string, params percent-escaped.
    let p = route(&Area::Drill).then(&Leg(7)).param("q", "a/b");
    assert_eq!(p.to_route(), "drill/leg-7?q=a%2Fb");
    assert_eq!(format!("{p}"), "drill/leg-7?q=a%2Fb");
}

#[test]
fn typed_routes_drive_selector_and_stack() {
    // A Signal<Option<Area>> sidebar over a Signal<Vec<Leg>> stack: the same wire-format
    // routes drive them, but the app-facing state and destinations are typed values.
    let section = Signal::new(None::<Area>);
    let seen: Rc<RefCell<Vec<String>>> = Rc::default();
    let probe = boot({
        let seen = seen.clone();
        move || {
            selector(section)
                .title("Root")
                .item(Area::Home, "Home", || label("home-content"))
                .item(Area::Drill, "Drill", {
                    let seen = seen.clone();
                    move || {
                        let path = Signal::new(Vec::<Leg>::new());
                        let seen = seen.clone();
                        stack(path, label("drill-root")).destination(move |leg: &Leg| {
                            seen.borrow_mut().push(format!(
                                "{}:{}",
                                leg.0,
                                route_param("hint").unwrap_or_default()
                            ));
                            label(format!("leg:{}", leg.0))
                        })
                    }
                })
                .any()
        }
    });

    // A typed absolute path descends into the lazily-mounted stack; the destination builder
    // received the PARSED value (u32 payload), not a string to split.
    assert!(
        route(&Area::Drill)
            .then(&Leg(7))
            .param("hint", "x")
            .navigate()
    );
    flush_sync();
    assert_eq!(section.get_untracked(), Some(Area::Drill));
    assert_eq!(day_core::current_route().as_deref(), Some("drill/leg-7"));
    assert_eq!(seen.borrow().as_slice(), ["7:x".to_string()]);
    assert!(
        probe
            .find_by_kind("day.label")
            .iter()
            .any(|(_, w)| w.text == "leg:7")
    );

    // A typed stack VALIDATES absolute segments: "drill/bogus" anchors the section but the
    // unparseable segment is refused, so the stack stays at its root.
    assert!(navigate("drill/bogus"));
    flush_sync();
    assert_eq!(day_core::current_route().as_deref(), Some("drill"));

    // Relative typed navigation and the string wire format address the same items.
    assert!(navigate_to(&Area::Home));
    flush_sync();
    assert_eq!(section.get_untracked(), Some(Area::Home));
    assert_eq!(day_core::current_route().as_deref(), Some("home"));
    assert!(navigate("drill"));
    flush_sync();
    assert_eq!(section.get_untracked(), Some(Area::Drill));
}

// ---------------------------------------------------------------------------
// Forms (docs/forms.md): form / section / labeled.
// ---------------------------------------------------------------------------

#[test]
fn form_aligns_labels_and_sections_carry_the_card_surface() {
    let on = Signal::new(true);
    let level = Signal::new(0.5f64);
    let name = Signal::new(String::new());
    let probe = boot(move || {
        form((
            section((
                labeled("Short", toggle(on).id("t1")),
                labeled("A much longer label", slider(level).id("s1")),
            ))
            .title("Sound"),
            section((labeled("Name", text_field(name).id("f1")),)),
        ))
    });
    flush_sync();

    // Both sections realize as containers carrying the theme-adaptive card surface role.
    let cards: Vec<_> = probe
        .find_by_kind("day.container")
        .into_iter()
        .filter(|(_, w)| w.surface_role == Some(day_spec::SurfaceRole::SectionCard))
        .collect();
    assert_eq!(cards.len(), 2, "one card per section");
    assert!(cards.iter().all(|(_, w)| w.corner_radius > 0.0));

    // The label COLUMN is shared across the whole form: every label's right edge lines up,
    // and every control's left edge lines up — across sections, not just within one.
    let labels: Vec<_> = probe
        .find_by_kind("day.label")
        .into_iter()
        .filter(|(_, w)| ["Short", "A much longer label", "Name"].contains(&w.text.as_str()))
        .collect();
    assert_eq!(labels.len(), 3);
    let right_edges: Vec<i64> = labels
        .iter()
        .map(|(_, w)| (w.frame.origin.x + w.frame.size.width).round() as i64)
        .collect();
    assert!(
        right_edges.windows(2).all(|w| w[0] == w[1]),
        "label right edges align: {right_edges:?}"
    );

    let mut control_lefts = Vec::new();
    for kind in ["day.toggle", "day.slider", "day.text_field"] {
        for (_, w) in probe.find_by_kind(kind) {
            control_lefts.push(w.frame.origin.x.round() as i64);
        }
    }
    assert_eq!(control_lefts.len(), 3);
    assert!(
        control_lefts.windows(2).all(|w| w[0] == w[1]),
        "control left edges align: {control_lefts:?}"
    );
}

#[test]
fn scroll_target_signal_drives_offset() {
    // A 400x600 window; 40 rows of ~20+ tall labels overflow the viewport for sure.
    let jump: Signal<Option<ScrollTarget>> = Signal::new(None);
    let jump2 = jump;
    let probe = boot(move || {
        scroll(column(PieceVec(
            (0..100)
                .map(|i| label(format!("row {i}")).id(format!("mock-row-{i}")).any())
                .collect(),
        )))
        .scroll_target(jump2)
        .any()
    });
    let scrolls = probe.find_by_kind("day.scroll");
    let content_h = scrolls[0].1.scroll_content.height;
    let viewport_h = scrolls[0].1.frame.size.height;
    assert!(content_h > viewport_h, "content overflows: {content_h}");

    jump.set(Some(ScrollTarget::Bottom));
    flush_sync();
    let w = &probe.find_by_kind("day.scroll")[0].1;
    assert_eq!(
        w.scroll_offset.y,
        content_h - viewport_h,
        "Bottom lands at content minus viewport"
    );
    assert_eq!(jump.get_untracked(), None, "signal resets after consuming");

    jump.set(Some(ScrollTarget::Top));
    flush_sync();
    assert_eq!(
        probe.find_by_kind("day.scroll")[0].1.scroll_offset.y,
        0.0,
        "Top returns to zero"
    );

    jump.set(Some(ScrollTarget::Offset(Point::new(0.0, 123.0))));
    flush_sync();
    assert_eq!(
        probe.find_by_kind("day.scroll")[0].1.scroll_offset.y,
        123.0,
        "Offset pins the viewport origin"
    );

    // Reveal-by-id: a row far below the fold scrolls its enclosing scroll.
    jump.set(Some(ScrollTarget::Id("mock-row-90".into())));
    flush_sync();
    let y = probe.find_by_kind("day.scroll")[0].1.scroll_offset.y;
    assert!(y > 123.0, "revealing row 90 scrolled further down: {y}");
}

#[test]
fn picker_and_text_area_are_built_in() {
    // Both moved from satellite crates into core (2026-07): they realize as first-class
    // widgets on the mock backend, with probe-visible selection/text — no registry fallback.
    let choice = Signal::new(1usize);
    let draft = Signal::new(String::from("hi"));
    let choice2 = choice;
    let draft2 = draft;
    let probe = boot(move || {
        column((
            picker(["A", "B", "C"], choice2).segmented().id("pk"),
            text_area(draft2).placeholder("write…").id("ta"),
        ))
        .any()
    });

    let pk = probe.find_by_kind("day.picker");
    assert_eq!(pk.len(), 1, "picker realized as a native built-in");
    assert_eq!(pk[0].1.value, 1.0, "initial selection reached the widget");
    let ta = probe.find_by_kind("day.text_area");
    assert_eq!(ta.len(), 1, "text_area realized as a native built-in");
    assert_eq!(ta[0].1.text, "hi");

    // App → widget: writing the signals patches through to the mock widget.
    choice.set(2);
    draft.set("bye".into());
    flush_sync();
    assert_eq!(probe.find_by_kind("day.picker")[0].1.value, 2.0);
    assert_eq!(probe.find_by_kind("day.text_area")[0].1.text, "bye");

    // Widget → app: a native SelectionChanged / TextChanged flows back into the signals.
    let pk_id = node_id(&probe, "day.picker", 0);
    probe.emit(pk_id, Event::SelectionChanged(0));
    let ta_id = node_id(&probe, "day.text_area", 0);
    probe.emit(ta_id, Event::TextChanged("typed".into()));
    flush_sync();
    assert_eq!(choice.get_untracked(), 0);
    assert_eq!(draft.get_untracked(), "typed");
}

/// Cover (docs/cover.md): Some(route) presents + builds content, the native FrameChanged
/// report lays the content out at the reported size, nav_back dismisses, and the content is
/// disposed only after the backend reports the hide finished ("cover-hidden").
#[test]
fn cover_presents_lays_out_and_dismisses() {
    let probe = boot(|| {
        let open = Signal::new(None::<String>);
        zstack((
            label("home"),
            cover(open, |k: &String| label(format!("game-{k}")).any()),
        ))
        .any()
    });
    flush_sync();
    assert!(probe.find_by_kind("day.cover").len() == 1, "cover realized");

    // Present via the string-route adapter the cover registers.
    assert!(day_core::navigate("breakout"));
    flush_sync();
    assert!(
        probe
            .mutations()
            .iter()
            .any(|l| l.contains("cover present")),
        "present patch reached the backend: {:?}",
        probe.mutations()
    );
    assert!(
        probe
            .find_by_kind("day.label")
            .iter()
            .any(|(_, w)| w.text == "game-breakout"),
        "content built under the cover"
    );
    assert_eq!(day_core::current_route().as_deref(), Some("breakout"));

    // The native surface reports its content size; the content lays out inside it.
    let cover_id = node_id(&probe, "day.cover", 0);
    probe.emit(cover_id, Event::FrameChanged(Size::new(400.0, 600.0)));
    flush_sync();
    let game = probe
        .find_by_kind("day.label")
        .into_iter()
        .find(|(_, w)| w.text == "game-breakout")
        .expect("game label");
    assert!(
        game.1.frame.size.width > 0.0,
        "content laid out after the size report (frame {:?})",
        game.1.frame
    );

    // nav_back writes None; the backend gets the dismiss patch; content survives the hide
    // transition and is disposed on the hidden report.
    assert!(day_core::nav_back());
    flush_sync();
    assert!(
        probe
            .mutations()
            .iter()
            .any(|l| l.contains("cover dismiss")),
        "dismiss patch reached the backend"
    );
    assert!(
        probe
            .find_by_kind("day.label")
            .iter()
            .any(|(_, w)| w.text == "game-breakout"),
        "content stays mounted while the hide transition runs"
    );
    probe.emit(cover_id, Event::custom("cover-hidden", ""));
    flush_sync();
    assert!(
        !probe
            .find_by_kind("day.label")
            .iter()
            .any(|(_, w)| w.text == "game-breakout"),
        "content disposed after the hide finished"
    );
}

// ── the daylite lifecycle: siblings must survive a cover cycle, and a second present must
//    work — including with adversarial "cover-hidden" orderings (double emit, late emit).

thread_local! {
    static CYCLE: std::cell::RefCell<Option<(Signal<f64>, Signal<f64>, Signal<Option<String>>)>> =
        const { std::cell::RefCell::new(None) };
}

fn cover_cycle_root() -> AnyPiece {
    // rev drives an `each` of "rows" (the daylite catalog shape); taps counts row-button
    // presses; open drives the cover.
    let rev = Signal::new(0.0f64);
    let taps = Signal::new(0.0f64);
    let open = Signal::new(None::<String>);
    CYCLE.with(|c| *c.borrow_mut() = Some((rev, taps, open)));
    zstack((
        column((
            label(move || format!("taps {}", taps.get())),
            each(
                move || {
                    let generation = rev.get() as i64;
                    vec![format!("row-a:{generation}"), format!("row-b:{generation}")]
                },
                |item: &String| item.clone(),
                move |slot| {
                    let name = slot.get();
                    button(name.clone())
                        .action(move || taps.set(taps.get_untracked() + 1.0))
                        .id(name)
                },
            ),
        ))
        .any(),
        cover(open, |k: &String| {
            // FIRST-touch a lazily-allocated process-global signal from INSIDE the
            // presentation scope — the day-lite regression: the global must be allocated
            // in the root scope, not inherit this cover's, or it dies on dismissal and
            // every later read panics (day-l10n's locale signal was the observed case).
            let locale = day_l10n::locale().get_untracked();
            label(format!("game-{k}@{locale}")).any()
        }),
    ))
    .any()
}

fn tap_count(probe: &MockProbe) -> String {
    probe
        .find_by_kind("day.label")
        .into_iter()
        .map(|(_, w)| w.text)
        .find(|t| t.starts_with("taps "))
        .unwrap_or_default()
}

fn tap_button(probe: &MockProbe, text: &str) {
    let found = probe
        .find_by_kind("day.button")
        .into_iter()
        .find(|(_, w)| w.text == text)
        .unwrap_or_else(|| panic!("button {text} not found"));
    probe.emit(NodeId(found.1.node), Event::Pressed);
    flush_sync();
}

#[test]
fn cover_cycle_keeps_siblings_alive_and_represents() {
    let probe = boot(cover_cycle_root);
    flush_sync();
    let (rev, _taps, open) = CYCLE.with(|c| c.borrow().clone()).expect("cycle state");

    // Rebuild the rows once BEFORE any cover (the daylite install-confirm shape).
    rev.set(1.0);
    flush_sync();
    tap_button(&probe, "row-a:1");
    assert_eq!(tap_count(&probe), "taps 1", "pre-cover rows respond");

    // Present, size, dismiss, and finish the hide transition.
    open.set(Some("ttt".into()));
    flush_sync();
    let cover_id = node_id(&probe, "day.cover", 0);
    probe.emit(cover_id, Event::FrameChanged(Size::new(400.0, 600.0)));
    flush_sync();
    assert!(
        probe
            .find_by_kind("day.label")
            .iter()
            .any(|(_, w)| w.text.starts_with("game-ttt")),
        "cover content built"
    );
    open.set(None);
    flush_sync();
    probe.emit(cover_id, Event::custom("cover-hidden", ""));
    flush_sync();

    // 1) Siblings built BEFORE the cycle still respond.
    tap_button(&probe, "row-a:1");
    assert_eq!(
        tap_count(&probe),
        "taps 2",
        "pre-cycle sibling handler still fires after the cover cycle"
    );

    // 2) Rows rebuilt AFTER the cycle respond.
    rev.set(2.0);
    flush_sync();
    tap_button(&probe, "row-b:2");
    assert_eq!(
        tap_count(&probe),
        "taps 3",
        "post-cycle rebuilt rows respond"
    );

    // 3) A second present builds fresh content.
    open.set(Some("todo".into()));
    flush_sync();
    probe.emit(cover_id, Event::FrameChanged(Size::new(400.0, 600.0)));
    flush_sync();
    assert!(
        probe
            .find_by_kind("day.label")
            .iter()
            .any(|(_, w)| w.text.starts_with("game-todo")),
        "second present builds content"
    );

    // 4) Adversarial orderings: a DOUBLE cover-hidden after dismissal must be harmless…
    open.set(None);
    flush_sync();
    probe.emit(cover_id, Event::custom("cover-hidden", ""));
    probe.emit(cover_id, Event::custom("cover-hidden", ""));
    flush_sync();
    tap_button(&probe, "row-b:2");
    assert_eq!(
        tap_count(&probe),
        "taps 4",
        "double cover-hidden is harmless"
    );

    // …and a LATE cover-hidden from the previous dismissal, arriving after the next
    // present, must not dispose the new content.
    open.set(Some("wx".into()));
    flush_sync();
    open.set(None);
    flush_sync();
    open.set(Some("wx2".into()));
    flush_sync();
    probe.emit(cover_id, Event::custom("cover-hidden", "")); // belated, for the wx dismissal
    flush_sync();
    assert!(
        probe
            .find_by_kind("day.label")
            .iter()
            .any(|(_, w)| w.text.starts_with("game-wx2")),
        "late cover-hidden does not kill the re-presented content"
    );
}
