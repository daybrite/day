// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

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

/// Boot a mock that CAN present split panes, in a window of `size` — so the launch size class
/// decides the presentation exactly as it does on a real toolkit (docs/size-classes.md).
fn boot_splittable(size: Size, root: impl FnOnce() -> AnyPiece + 'static) -> MockProbe {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    day_core::uninstall_tree();
    let (mock, probe) = MockToolkit::new();
    // Read during the build, so it has to be set before launching.
    probe.set_nav_split(true);
    let options = WindowOptions {
        title: "test".into(),
        size,
        ..Default::default()
    };
    day_core::launch_with(mock, options, root);
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

/// A wide window presents split; a narrow one stacks. The class decides, not the toolkit alone
/// (docs/size-classes.md).
#[test]
fn selector_presentation_follows_the_launch_size_class() {
    let sel = Signal::new(String::new());
    let probe = boot_splittable(Size::new(1000.0, 700.0), move || sidebar_selector(sel));
    let host = probe.find_by_kind("day.nav")[0].1.clone();
    assert!(host.flag, "expanded window → split");
    // Split never shows an empty detail: the first item is selected for us.
    assert_eq!(sel.get_untracked(), "about");
    assert_eq!(probe.find_by_kind("day.nav_page").len(), 2);

    let sel2 = Signal::new(String::new());
    let probe2 = boot_splittable(Size::new(390.0, 844.0), move || sidebar_selector(sel2));
    assert!(
        !probe2.find_by_kind("day.nav")[0].1.flag,
        "compact window → stack"
    );
    assert_eq!(sel2.get_untracked(), "", "a stack opens on its list");
    assert_eq!(probe2.find_by_kind("day.nav_page").len(), 1);
}

/// The morph, and the thing that makes it worth having: crossing a breakpoint RE-PRESENTS the
/// live host. The pages keep their node identities and the selection survives — a rebuild would
/// lose both, and would take every scroll offset and focused field with them.
#[test]
fn size_class_change_re_presents_without_rebuilding_pages() {
    let sel = Signal::new(String::new());
    let probe = boot_splittable(Size::new(1000.0, 700.0), move || sidebar_selector(sel));
    let host = probe.find_by_kind("day.nav")[0].0;
    batch(|| sel.set("extra".into()));
    flush_sync();
    let pages_before: Vec<u64> = probe
        .find_by_kind("day.nav_page")
        .iter()
        .map(|(_, w)| w.node)
        .collect();
    assert_eq!(pages_before.len(), 2, "sidebar + detail");
    assert!(probe.widget(host).flag, "split before");

    // Narrow the window past the 600dp breakpoint, as a backend would report it.
    day_core::set_size_class(day_spec::SizeClass::from_size(390.0, 844.0));
    flush_sync();

    assert!(!probe.widget(host).flag, "stacked after narrowing");
    let pages_after: Vec<u64> = probe
        .find_by_kind("day.nav_page")
        .iter()
        .map(|(_, w)| w.node)
        .collect();
    assert_eq!(
        pages_before, pages_after,
        "pages were re-homed, not rebuilt"
    );
    assert_eq!(
        sel.get_untracked(),
        "extra",
        "narrowing keeps the selection — the detail becomes the top of the stack"
    );
    assert_eq!(day_core::current_route().as_deref(), Some("extra"));

    // And back: widening re-presents again, still without rebuilding.
    day_core::set_size_class(day_spec::SizeClass::from_size(1000.0, 700.0));
    flush_sync();
    assert!(probe.widget(host).flag, "split again after widening");
    let pages_final: Vec<u64> = probe
        .find_by_kind("day.nav_page")
        .iter()
        .map(|(_, w)| w.node)
        .collect();
    assert_eq!(pages_before, pages_final);
    assert_eq!(sel.get_untracked(), "extra");
}

/// Widening with nothing selected has to pick something: a split presentation has no way to draw
/// an empty detail pane.
#[test]
fn widening_from_an_unselected_stack_selects_the_first_item() {
    let sel = Signal::new(String::new());
    let probe = boot_splittable(Size::new(390.0, 844.0), move || sidebar_selector(sel));
    assert_eq!(sel.get_untracked(), "");
    assert_eq!(probe.find_by_kind("day.nav_page").len(), 1);

    day_core::set_size_class(day_spec::SizeClass::from_size(1000.0, 700.0));
    flush_sync();
    assert_eq!(sel.get_untracked(), "about");
    assert_eq!(probe.find_by_kind("day.nav_page").len(), 2);
}

/// A pinned presentation ignores the window entirely — including the breakpoint it would
/// otherwise cross.
#[test]
fn a_pinned_presentation_does_not_morph() {
    let sel = Signal::new(String::new());
    let probe = boot_splittable(Size::new(390.0, 844.0), move || {
        selector(sel)
            .presentation(day_spec::props::NavPresentation::Split)
            .item("about", "About", || label("about-content"))
            .item("extra", "Extra", || label("extra-content"))
            .any()
    });
    let host = probe.find_by_kind("day.nav")[0].0;
    assert!(
        probe.widget(host).flag,
        "pinned split despite a compact window"
    );

    day_core::set_size_class(day_spec::SizeClass::from_size(1000.0, 700.0));
    flush_sync();
    assert!(probe.widget(host).flag);
    day_core::set_size_class(day_spec::SizeClass::from_size(390.0, 844.0));
    flush_sync();
    assert!(probe.widget(host).flag, "still pinned");
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
fn selector_data_driven_items_reconcile() {
    // A sidebar whose rows come from a signal: adding/removing rooms re-patches the menu, and
    // navigating a data-driven key shows its .destination page.
    let rooms = Signal::new(vec!["general".to_string(), "random".to_string()]);
    let current = Signal::new(Option::<String>::None);
    let rooms_r = rooms;
    let probe = boot(move || {
        selector(current)
            .style(SelectorStyle::Sidebar)
            .items(
                move || rooms_r.get(),
                |r: &String| item(r.clone(), r.clone()),
            )
            .destination(|k: &Option<String>| {
                label(format!("room:{}", k.clone().unwrap_or_default()))
            })
            .any()
    });
    let menu = probe.find_by_kind("day.nav_menu")[0].0;
    assert_eq!(probe.widget(menu).text, "general|random", "initial rows");

    // Add a room → the menu re-patches.
    batch(|| rooms.set(vec!["general".into(), "random".into(), "help".into()]));
    flush_sync();
    assert_eq!(probe.widget(menu).text, "general|random|help", "row added");

    // Navigate a data-driven key → its destination shows.
    assert!(navigate("help"));
    flush_sync();
    assert_eq!(day_core::current_route().as_deref(), Some("help"));
    assert!(
        probe
            .find_by_kind("day.label")
            .iter()
            .any(|(_, w)| w.text == "room:help"),
        "destination built for the data-driven key"
    );

    // Remove the selected room → selection resets to None (Option key), menu shrinks.
    batch(|| rooms.set(vec!["general".into(), "random".into()]));
    flush_sync();
    assert_eq!(probe.widget(menu).text, "general|random", "row removed");
    assert_eq!(
        current.get_untracked(),
        None,
        "selection reset when its item vanished"
    );
}

#[test]
fn selector_filtered_rows_keep_a_live_detail() {
    // A search-filtered sidebar (docs/navigation.md): the row set and the selection change in
    // the SAME batch, which used to leave the detail pane empty for good — the selection bind
    // is created before the derive effect, so it ran against the pre-filter rows, found no
    // index for the key, and gave up with nothing left to re-trigger it.
    let query = Signal::new(String::new());
    let current = Signal::new(Option::<String>::None);
    let all = ["canvas", "controls", "sensors"];
    let q = query;
    let probe = boot(move || {
        selector(current)
            .style(SelectorStyle::Sidebar)
            .items(
                move || {
                    let needle = q.get();
                    all.iter()
                        .filter(|t| day_l10n::matches_search_in("en", t, &needle))
                        .map(|t| t.to_string())
                        .collect::<Vec<_>>()
                },
                |r: &String| item(r.clone(), r.clone()),
            )
            .destination(|k: &Option<String>| {
                label(format!("page:{}", k.clone().unwrap_or_default()))
            })
            .any()
    });
    let menu = probe.find_by_kind("day.nav_menu")[0].0;
    assert_eq!(probe.widget(menu).text, "canvas|controls|sensors");

    let shows = |key: &str| {
        probe
            .find_by_kind("day.label")
            .iter()
            .any(|(_, w)| w.text == format!("page:{key}"))
    };

    // Narrow to one row: only the word-prefix match survives.
    batch(|| query.set("s".into()));
    flush_sync();
    assert_eq!(probe.widget(menu).text, "sensors");

    // THE HAZARD: widen the filter and select a row that reappears, in ONE batch. The selection
    // bind runs first, against the still-narrow row set, and finds no index for "canvas".
    batch(|| {
        query.set(String::new());
        current.set(Some("canvas".into()));
    });
    flush_sync();
    assert_eq!(probe.widget(menu).text, "canvas|controls|sensors");
    assert!(
        shows("canvas"),
        "the detail follows a selection made in the same batch as the filter that revealed it"
    );

    // A surviving key keeps its page across a re-filter.
    batch(|| query.set("can".into()));
    flush_sync();
    assert_eq!(probe.widget(menu).text, "canvas");
    assert_eq!(current.get_untracked(), Some("canvas".to_string()));
    assert!(shows("canvas"), "surviving key keeps its page");

    // Reset for the removal case below.
    batch(|| query.set(String::new()));
    flush_sync();
    batch(|| current.set(Some("sensors".into())));
    flush_sync();
    assert!(shows("sensors"));

    // Filtering the SELECTED row away resets the selection rather than stranding the pane on a
    // row that is no longer in the list.
    batch(|| query.set("canv".into()));
    flush_sync();
    assert_eq!(probe.widget(menu).text, "canvas");
    assert_eq!(
        current.get_untracked(),
        None,
        "selection cleared when its row was filtered out"
    );
    assert!(
        !shows("sensors"),
        "the filtered-out page is gone, not left on screen"
    );
}

#[test]
fn stack_on_back_guard_intercepts_and_defers() {
    use std::cell::Cell;
    use std::rc::Rc;
    // The guard consumes back-like events (nav_back / native NavBack) but NEVER a programmatic
    // path write, and BackRequest::proceed performs the deferred pop.
    let path = Signal::new(Vec::<String>::new());
    let held: Rc<RefCell<Option<BackRequest>>> = Rc::default();
    let block = Rc::new(Cell::new(true)); // guard consumes while true
    let (held_c, block_c) = (held.clone(), block.clone());
    let probe = boot(move || {
        stack(path, label("root"))
            .destination(|k: &String| label(format!("d:{k}")))
            .on_back(move |req| {
                if block_c.get() {
                    *held_c.borrow_mut() = Some(req);
                    BackResponse::Handled
                } else {
                    BackResponse::Proceed
                }
            })
            .any()
    });
    let host = probe.find_by_kind("day.nav")[0].0;

    // Push two levels (programmatic — never guarded).
    batch(|| path.set(vec!["a".into(), "b".into()]));
    flush_sync();
    assert_eq!(day_core::current_route().as_deref(), Some("a/b"));
    // GuardTop(true) armed the host (mock records it in `flag`).
    assert!(probe.widget(host).flag, "guard armed while above root");

    // A back-like event: nav_back() is GUARDED — the guard returns Handled, so no pop.
    assert!(nav_back());
    flush_sync();
    assert_eq!(
        day_core::current_route().as_deref(),
        Some("a/b"),
        "guarded back must not pop"
    );
    assert!(held.borrow().is_some(), "guard received the BackRequest");

    // The app proceeds the stashed request → the deferred pop lands.
    held.borrow().as_ref().unwrap().proceed();
    flush_sync();
    assert_eq!(day_core::current_route().as_deref(), Some("a"));

    // A PROGRAMMATIC path write is never guarded (even while block=true).
    batch(|| path.set(vec![]));
    flush_sync();
    assert_eq!(day_core::current_route().as_deref(), Some(""));
    assert!(!probe.widget(host).flag, "guard disarmed at root");

    // With the guard passing through, a back proceeds immediately.
    batch(|| path.set(vec!["x".into()]));
    flush_sync();
    block.set(false);
    assert!(nav_back());
    flush_sync();
    assert_eq!(day_core::current_route().as_deref(), Some(""));
}

#[test]
fn shown_page_retitles_native_bar_live() {
    // A page title that reads a signal (the locale case: `tr()` reads the locale signal). The
    // shown page must re-resolve it and retitle the host via NavPatch::Title — before this,
    // every backend's native bar kept the push-time title forever.
    let section = Signal::new(String::new());
    let name = Signal::new(String::from("Inbox"));
    let title = name;
    let probe = boot(move || {
        selector(section)
            .title("Root")
            .item("mail", move || title.get(), || label("mail-content"))
            .any()
    });

    assert!(navigate("mail"));
    flush_sync();
    let nav = probe.find_by_kind("day.nav")[0].0;
    assert_eq!(probe.widget(nav).text, "Inbox", "push-time title");

    batch(|| name.set("Inbox (3)".into()));
    flush_sync();
    assert_eq!(
        probe.widget(nav).text,
        "Inbox (3)",
        "NavPatch::Title must follow the live title source"
    );
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

// Teardown (docs/list.md): a list going away takes its bound rows with it — the row subtrees
// hang off the cells, OUTSIDE the node tree, so nothing else would collect them. But the cells
// themselves are the native host's, only borrowed through `adopt` (§15.3): the host frees its own
// pool, so day must NOT release them too. It did briefly, and the second delete corrupted the
// heap on the raw-pointer backends — the xaml showcase walkthrough died leaving the list page.
#[test]
fn list_teardown_releases_row_content_but_never_the_adopted_cells() {
    let shown = Signal::new(true);
    let probe = boot(move || when(move || shown.get(), five_item_list));
    let host = probe.find_by_kind("day.list")[0].0;

    let (cell_a, cell_b) = (MockHandle(9001), MockHandle(9002));
    probe.list_bind(host, 0, cell_a);
    probe.list_bind(host, 1, cell_b);
    let rows: Vec<MockHandle> = probe
        .find_by_kind("day.label")
        .iter()
        .map(|(h, _)| *h)
        .collect();
    assert_eq!(rows.len(), 2, "two cells bound");

    probe.clear_log();
    batch(|| shown.set(false));
    flush_sync();
    let log = probe.log();

    // No zombie rows: the cells' subtrees went with the list.
    assert_eq!(
        probe.find_by_kind("day.label").len(),
        0,
        "row nodes are gone: {log:?}"
    );
    for r in rows {
        assert!(
            log.contains(&format!("release #{}", r.0)),
            "row content #{} released: {log:?}",
            r.0
        );
    }
    // The cells are the host's — releasing them here would be a double free.
    for cell in [cell_a, cell_b] {
        assert!(
            !log.contains(&format!("release #{}", cell.0)),
            "adopted cell #{} must be left to the list host: {log:?}",
            cell.0
        );
    }
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

// ---------------------------------------------------------------------------
// Drag-to-reorder (docs/list.md): the probe drives the same sync guard → commit seam a native
// backend does, so these assert the whole path — guard verdicts, snapshot rotation before any
// rebind, the deferred app callback, and the echo skip (no redundant reload after the commit).
// ---------------------------------------------------------------------------

/// A reorderable five-row list whose app-side data lives in `order` (mirrored out for asserts)
/// and whose committed moves are recorded in `moves`.
fn reorderable_list(
    moves: std::rc::Rc<std::cell::RefCell<Vec<(usize, usize)>>>,
    order: std::rc::Rc<std::cell::RefCell<Vec<String>>>,
    guard: Option<fn(usize, usize) -> Reorder>,
) -> AnyPiece {
    let items = Signal::new(order.borrow().clone());
    let mut l = list(
        move || items.get(),
        |s: &String| s.clone(),
        |row: ItemSlot<String, String>| label(move || row.get()),
    )
    .row_height(RowHeight::Uniform(20.0))
    .reorderable(true)
    .on_reorder(move |from, to| {
        moves.borrow_mut().push((from, to));
        items.update(|v| {
            let it = v.remove(from);
            v.insert(to, it);
        });
        *order.borrow_mut() = items.get_untracked();
    });
    if let Some(g) = guard {
        l = l.reorder_guard(g);
    }
    l.any()
}

fn seed() -> Vec<String> {
    ["a", "b", "c", "d", "e"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

fn reload_count(probe: &MockProbe) -> usize {
    probe
        .log()
        .iter()
        .filter(|l| l.contains("list reload"))
        .count()
}

#[test]
fn list_reorder_commits_rotates_and_defers_callback() {
    let moves = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let order = std::rc::Rc::new(std::cell::RefCell::new(seed()));
    let (m, o) = (moves.clone(), order.clone());
    let probe = boot(move || reorderable_list(m, o, None));
    let host = probe.find_by_kind("day.list")[0].0;
    assert_eq!(
        reload_count(&probe),
        1,
        "one reload from the initial refresh"
    );

    // No guard: every move is accepted where proposed.
    assert_eq!(probe.list_can_move(host, 0, 2), 2);

    // A native drop: commit 0 -> 2. The app callback runs (deferred), the data follows, and the
    // echo of that data change must NOT re-reload the already-moved native rows.
    assert!(probe.list_move(host, 0, 2));
    assert_eq!(moves.borrow().as_slice(), [(0, 2)]);
    assert_eq!(
        order.borrow().as_slice(),
        [
            "b".to_string(),
            "c".into(),
            "a".into(),
            "d".into(),
            "e".into()
        ]
    );
    assert_eq!(reload_count(&probe), 1, "the commit echo skips the reload");

    // The rotated snapshot serves any bind that arrives after the drop.
    probe.list_bind(host, 0, MockHandle(9101));
    let labels = probe.find_by_kind("day.label");
    assert_eq!(labels[0].1.text, "b");
}

#[test]
fn list_reorder_denied_by_guard_and_unsupported_without_optin() {
    let moves = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let order = std::rc::Rc::new(std::cell::RefCell::new(seed()));
    let (m, o) = (moves.clone(), order.clone());
    let probe = boot(move || reorderable_list(m, o, Some(|_, _| Reorder::Deny)));
    let host = probe.find_by_kind("day.list")[0].0;

    assert_eq!(probe.list_can_move(host, 1, 3), -1);
    assert!(!probe.list_move(host, 1, 3));
    assert!(
        moves.borrow().is_empty(),
        "a denied move never reaches the app"
    );
    assert_eq!(order.borrow().as_slice(), seed().as_slice());
    assert!(
        probe
            .log()
            .iter()
            .any(|l| l.contains("list move denied 1->3"))
    );

    // A list that never opted in has no reorder seam at all.
    let probe = boot(five_item_list);
    let host = probe.find_by_kind("day.list")[0].0;
    assert_eq!(probe.list_can_move(host, 0, 1), i64::MIN);
    assert!(!probe.list_move(host, 0, 1));
}

#[test]
fn list_reorder_guard_retargets_the_drop() {
    let moves = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let order = std::rc::Rc::new(std::cell::RefCell::new(seed()));
    let (m, o) = (moves.clone(), order.clone());
    // Every drop lands at row 0, wherever it was proposed (the "pinned target" pattern).
    let probe = boot(move || reorderable_list(m, o, Some(|_, _| Reorder::Retarget(0))));
    let host = probe.find_by_kind("day.list")[0].0;

    assert_eq!(
        probe.list_can_move(host, 2, 4),
        0,
        "the guard retargets 4 -> 0"
    );
    assert!(probe.list_move(host, 2, 4));
    assert_eq!(
        moves.borrow().as_slice(),
        [(2, 0)],
        "the app sees the ACCEPTED target"
    );
    assert_eq!(
        order.borrow().as_slice(),
        [
            "c".to_string(),
            "a".into(),
            "b".into(),
            "d".into(),
            "e".into()
        ]
    );
}

#[test]
fn list_try_reorder_drives_the_scripted_path() {
    let moves = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let order = std::rc::Rc::new(std::cell::RefCell::new(seed()));
    let (m, o) = (moves.clone(), order.clone());
    let probe = boot(move || {
        reorderable_list(
            m,
            o,
            Some(|from, _| {
                if from == 0 {
                    Reorder::Deny
                } else {
                    Reorder::Allow
                }
            }),
        )
    });
    let node = day_core::id_to_rnode(node_id(&probe, "day.list", 0));

    // The dayscript path: guard consulted, committed, and — with no native animation — reloaded.
    assert_eq!(day_core::list_try_reorder(node, 1, 4), Ok(4));
    assert_eq!(moves.borrow().as_slice(), [(1, 4)]);
    assert_eq!(
        reload_count(&probe),
        2,
        "initial + the scripted reorder's reload"
    );

    // Denied and out-of-bounds report errors the runner can surface.
    assert!(day_core::list_try_reorder(node, 0, 2).is_err());
    assert!(day_core::list_try_reorder(node, 1, 99).is_err());
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

// `grow_w` makes the surface fill the offered width (a filling pane) — the layout honors Flex.
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
    // Center of the 100×100 frame is inside the inscribed circle → fires.
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
        "fill color must re-record when its signal flips"
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
            day_spec::Paint::Solid(Color::hex(0xffffff)),
            day_spec::StrokeStyle::width(2.0)
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
fn a_tint_picks_a_readable_label_color() {
    use day_spec::props::ButtonStyleSpec as S;
    // The showcase palette, which is what this rule is judged on in practice.
    assert_eq!(S::on_tint(Color::hex(0x2F6FDE)), Color::WHITE, "sky");
    assert_eq!(S::on_tint(Color::hex(0xC2491D)), Color::WHITE, "rust");
    assert_eq!(S::on_tint(Color::hex(0x7C5CD6)), Color::WHITE, "violet");
    // The one that a luminance-over-half test gets WRONG: amber is 0.44, so that test calls it
    // dark and puts white on it at 2.2:1. Against black it is 9.7:1.
    assert_eq!(S::on_tint(Color::hex(0xF0A64C)), Color::BLACK, "amber");
    assert_eq!(S::on_tint(Color::WHITE), Color::BLACK);
    assert_eq!(S::on_tint(Color::BLACK), Color::WHITE);
    // Either choice must clear WCAG AA for large text (3:1) on every color above.
    for hex in [0x2F6FDE, 0xC2491D, 0x7C5CD6, 0xF0A64C, 0x3AA76D] {
        let fill = Color::hex(hex);
        let lin = |c: f64| {
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        let l = 0.2126 * lin(fill.r) + 0.7152 * lin(fill.g) + 0.0722 * lin(fill.b);
        let ratio = if S::on_tint(fill) == Color::BLACK {
            (l + 0.05) / 0.05
        } else {
            1.05 / (l + 0.05)
        };
        assert!(
            ratio >= 3.0,
            "{hex:#08x} contrast {ratio:.2}:1 is below 3:1"
        );
    }
}

/// The invariant: `button()` ALWAYS realizes a native button leaf. A tint changes its color and
/// nothing else — it must never be composed into a container with a tap handler, which would
/// cost the platform's focus ring, its pressed rendering and its accessibility role.
#[test]
fn a_tinted_button_is_still_a_native_button() {
    let clicks = std::rc::Rc::new(std::cell::Cell::new(0));
    let c2 = clicks.clone();
    let probe = boot(move || {
        button("Go")
            .action(move || c2.set(c2.get() + 1))
            .tint(Color::hex(0x2F6FDE))
            .any()
    });
    let buttons = probe.find_by_kind("day.button");
    assert_eq!(buttons.len(), 1, "a native button leaf, not a composition");
    assert_eq!(buttons[0].1.text, "Go");
    // No stand-in surface: a container PAINTED with the tint is exactly what this guarantees
    // against. (The root container the harness mounts into is expected and carries no fill.)
    assert!(
        probe
            .find_by_kind("day.container")
            .iter()
            .all(|(_, w)| w.background.is_none()),
        "no painted surface stands in for the button"
    );
    // And the label is the button's own, not a separate label piece inside a composition.
    assert!(
        probe.find_by_kind("day.label").is_empty(),
        "the title belongs to the native button"
    );
    // And it still fires as a button does.
    probe.emit(NodeId(buttons[0].1.node), Event::Pressed);
    flush_sync();
    assert_eq!(clicks.get(), 1);
}

/// A tint wins over `prominent`, and says so rather than silently dropping one of them.
#[test]
fn a_tint_overrides_prominent_and_stays_native() {
    let probe = boot(|| button("Go").prominent().tint(Color::hex(0x2F6FDE)).any());
    assert_eq!(probe.find_by_kind("day.button").len(), 1);
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

// ── Baseline alignment (docs/baseline.md) ──────────────────────────────────────────────────
// The mock's text sits 12pt below the top of a bare label and (box - 16)/2 + 12 below the top of
// a framed control, which is the same fact every real toolkit reports: a field insets its text.
// Centering the two BOXES leaves those two text lines apart; these pin that they meet.

#[test]
fn labeled_rows_put_their_label_and_control_on_one_baseline() {
    let name = Signal::new(String::new());
    let probe = boot(move || form((section((labeled("Name", text_field(name).id("f1")),)),)));
    flush_sync();

    let (_, lbl) = probe
        .find_by_kind("day.label")
        .into_iter()
        .find(|(_, w)| w.text == "Name")
        .expect("the label");
    let (_, field) = probe.find_by_kind("day.text_field")[0].clone();

    // Label: 16 tall, baseline 12 from its top. Field: 24 tall, baseline (24-16)/2 + 12 = 16.
    // So the label has to sit 4pt lower than the field for the text to line up.
    let label_baseline = lbl.frame.origin.y + 12.0;
    let field_baseline = field.frame.origin.y + (field.frame.size.height - 16.0) / 2.0 + 12.0;
    assert!(
        (label_baseline - field_baseline).abs() < 0.01,
        "label baseline {label_baseline} vs field baseline {field_baseline} \
         (label at y={}, field at y={})",
        lbl.frame.origin.y,
        field.frame.origin.y
    );
    assert!(
        lbl.frame.origin.y > field.frame.origin.y,
        "the shorter label drops to meet the framed field's inset text"
    );
}

#[test]
fn a_control_with_no_baseline_keeps_its_row_centered() {
    // A toggle has no text, so the mock reports no baseline for it and the row must fall back
    // to centering — the guarantee that makes baseline-by-default safe on every backend.
    let on = Signal::new(true);
    let probe = boot(move || form((section((labeled("Sound", toggle(on).id("t1")),)),)));
    flush_sync();

    let (_, lbl) = probe
        .find_by_kind("day.label")
        .into_iter()
        .find(|(_, w)| w.text == "Sound")
        .expect("the label");
    let (_, tog) = probe.find_by_kind("day.toggle")[0].clone();
    let label_mid = lbl.frame.origin.y + lbl.frame.size.height / 2.0;
    let toggle_mid = tog.frame.origin.y + tog.frame.size.height / 2.0;
    assert!(
        (label_mid - toggle_mid).abs() < 0.01,
        "no baseline on either side ⇒ centered: label mid {label_mid}, toggle mid {toggle_mid}"
    );
}

#[test]
fn decorated_children_keep_their_baseline() {
    // `.width(..)`, `.padding(..)` and friends wrap the piece in a layout-only node. If those
    // wrappers reported no baseline the row would silently center the very children the author
    // asked to align — and because a decorator is invisible at the call site (`.width(90)` on a
    // label still reads as "a label"), the failure looks like the feature simply not working.
    let name = Signal::new(String::new());
    let probe = boot(move || {
        row((
            label("Qty").width(90.0),
            text_field(name).width(70.0).id("d-field"),
            label("items").padding(4.0),
        ))
        .align(VAlign::FirstBaseline)
        .any()
    });
    flush_sync();

    let by = |text: &str| {
        probe
            .find_by_kind("day.label")
            .into_iter()
            .find(|(_, w)| w.text == text)
            .map(|(_, w)| w.frame)
            .expect("label present")
    };
    let lead = by("Qty");
    let unit = by("items");
    let field = probe.find_by_kind("day.text_field")[0].1.frame;

    // All three carry the mock's 12pt ascent; the field's box adds its own inset, and the
    // padded label starts 4pt into its wrapper — every one of those has to be accounted for.
    let lead_baseline = lead.origin.y + 12.0;
    let field_baseline = field.origin.y + (field.size.height - 16.0) / 2.0 + 12.0;
    let unit_baseline = unit.origin.y + 12.0;
    assert!(
        (lead_baseline - field_baseline).abs() < 0.01
            && (unit_baseline - field_baseline).abs() < 0.01,
        "decorated children share the row's baseline: lead {lead_baseline}, \
         field {field_baseline}, unit {unit_baseline}"
    );
}

#[test]
fn a_baseline_row_aligns_text_and_leaves_baseline_less_children_centered() {
    // The public opt-in: `row(..).align(VAlign::FirstBaseline)`. A label, a framed field whose
    // text is inset, and an image with no text at all.
    let name = Signal::new(String::new());
    let probe = boot(move || {
        row((
            label("Qty").id("b-label"),
            text_field(name).id("b-field"),
            image("icon".to_string()).id("b-image"),
        ))
        .align(VAlign::FirstBaseline)
        .any()
    });
    flush_sync();

    let (_, lbl) = probe.find_by_kind("day.label")[0].clone();
    let (_, field) = probe.find_by_kind("day.text_field")[0].clone();
    let (_, img) = probe.find_by_kind("day.image")[0].clone();

    let label_baseline = lbl.frame.origin.y + 12.0;
    let field_baseline = field.frame.origin.y + (field.frame.size.height - 16.0) / 2.0 + 12.0;
    assert!(
        (label_baseline - field_baseline).abs() < 0.01,
        "row baselines meet: {label_baseline} vs {field_baseline}"
    );
    // The image reports no baseline, so it keeps the centered placement it always had.
    assert!(
        img.frame.origin.y >= 0.0 && img.frame.size.height > 0.0,
        "the baseline-less child is still placed"
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
/// disposed only after the backend reports the hide finished (`CoverHidden`).
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
    probe.emit(cover_id, Event::CoverHidden);
    flush_sync();
    assert!(
        !probe
            .find_by_kind("day.label")
            .iter()
            .any(|(_, w)| w.text == "game-breakout"),
        "content disposed after the hide finished"
    );
}

// ── the superapp lifecycle: siblings must survive a cover cycle, and a second present must
//    work — including with adversarial `CoverHidden` orderings (double emit, late emit).

/// (rev, taps, open) — the signals `cover_cycle_root` publishes for the test body.
type CycleSignals = (Signal<f64>, Signal<f64>, Signal<Option<String>>);

thread_local! {
    static CYCLE: std::cell::RefCell<Option<CycleSignals>> =
        const { std::cell::RefCell::new(None) };
}

fn cover_cycle_root() -> AnyPiece {
    // rev drives an `each` of "rows" (a catalog-list shape); taps counts row-button
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
    let (rev, _taps, open) = CYCLE.with(|c| *c.borrow()).expect("cycle state");

    // Rebuild the rows once BEFORE any cover (the install-confirm shape).
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
    probe.emit(cover_id, Event::CoverHidden);
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

    // 4) Adversarial orderings: a DOUBLE `CoverHidden` after dismissal must be harmless…
    open.set(None);
    flush_sync();
    probe.emit(cover_id, Event::CoverHidden);
    probe.emit(cover_id, Event::CoverHidden);
    flush_sync();
    tap_button(&probe, "row-b:2");
    assert_eq!(
        tap_count(&probe),
        "taps 4",
        "double CoverHidden is harmless"
    );

    // …and a LATE `CoverHidden` from the previous dismissal, arriving after the next
    // present, must not dispose the new content.
    open.set(Some("wx".into()));
    flush_sync();
    open.set(None);
    flush_sync();
    open.set(Some("wx2".into()));
    flush_sync();
    probe.emit(cover_id, Event::CoverHidden); // belated, for the wx dismissal
    flush_sync();
    assert!(
        probe
            .find_by_kind("day.label")
            .iter()
            .any(|(_, w)| w.text.starts_with("game-wx2")),
        "late CoverHidden does not kill the re-presented content"
    );
}

// ── text_area attributes (editable / selectable / spell-check) + Toggle::enabled ─────────────

/// (editable, selectable, spellcheck) signals `text_area_attr_root` publishes for the test body.
type TaAttrs = (Signal<bool>, Signal<bool>, Signal<bool>);

thread_local! {
    static TA_ATTRS: std::cell::RefCell<Option<TaAttrs>> = const { std::cell::RefCell::new(None) };
}

fn text_area_attr_root() -> AnyPiece {
    let content = Signal::new("hello".to_string());
    let editable = Signal::new(true);
    let selectable = Signal::new(true);
    let spellcheck = Signal::new(true);
    TA_ATTRS.with(|c| *c.borrow_mut() = Some((editable, selectable, spellcheck)));
    text_area(content)
        .editable(editable)
        .selectable(selectable)
        .spellcheck(spellcheck)
        .id("ta")
        .any()
}

fn textarea(probe: &MockProbe) -> day_mock::MockWidget {
    probe
        .find_by_kind("day.text_area")
        .into_iter()
        .next()
        .expect("a text_area")
        .1
}

#[test]
fn text_area_attributes_realize_and_patch_reactively() {
    let probe = boot(text_area_attr_root);
    flush_sync();
    // Defaults: all three attributes are on.
    let w = textarea(&probe);
    assert!(
        w.editable && w.selectable && w.spellcheck,
        "defaults all true"
    );

    let (editable, selectable, spellcheck) = TA_ATTRS.with(|c| *c.borrow()).expect("attr signals");

    // Flipping each reactive attribute patches the widget (one live update per change).
    editable.set(false);
    flush_sync();
    assert!(!textarea(&probe).editable, "editable patched off");

    selectable.set(false);
    flush_sync();
    assert!(!textarea(&probe).selectable, "selectable patched off");

    spellcheck.set(false);
    flush_sync();
    let w = textarea(&probe);
    assert!(
        !w.spellcheck && !w.editable && !w.selectable,
        "all off after toggling"
    );
}

#[test]
fn toggle_enabled_false_renders_disabled() {
    let probe = boot(|| toggle(Signal::new(false)).enabled(false).id("t").any());
    flush_sync();
    let t = probe
        .find_by_kind("day.toggle")
        .into_iter()
        .next()
        .expect("a toggle")
        .1;
    assert!(
        !t.enabled,
        "Toggle::enabled(false) disables the native control"
    );
}

#[test]
fn selectable_modifier_marks_the_node_and_is_opt_in() {
    // `.selectable()` calls the backend's set_selectable exactly once, on the label's own node.
    let probe = boot(|| label("copy me").selectable().id("sel").any());
    flush_sync();
    let sel_ops: Vec<String> = probe
        .log()
        .into_iter()
        .filter(|o| o.starts_with("set_selectable"))
        .collect();
    assert_eq!(sel_ops.len(), 1, "one set_selectable, got {sel_ops:?}");
    assert!(
        sel_ops[0].ends_with(" true"),
        "selectable = true: {sel_ops:?}"
    );

    // A plain label is NOT selectable — the modifier is strictly opt-in.
    let probe2 = boot(|| label("plain").id("plain").any());
    flush_sync();
    assert!(
        !probe2.log().iter().any(|o| o.starts_with("set_selectable")),
        "a plain label must not be selectable by default"
    );
}

// --- .restore() (docs/navigation.md) -------------------------------------------------------
// An in-memory NavStore standing in for `day_part_prefs::install_nav_store`, so these tests
// exercise the pieces' restore/persist wiring without touching the platform prefs facility.
// A test that doesn't call `.restore()` never consults the store, so a store left installed on a
// reused test thread can't affect another test.
#[derive(Clone, Default)]
struct MemStore(std::rc::Rc<std::cell::RefCell<std::collections::HashMap<String, String>>>);

impl day_core::NavStore for MemStore {
    fn load(&self, key: &str) -> Option<String> {
        self.0.borrow().get(key).cloned()
    }
    fn save(&self, key: &str, value: &str) {
        self.0
            .borrow_mut()
            .insert(key.to_string(), value.to_string());
    }
}

/// Install a fresh MemStore seeded with `pairs`, returning a handle to inspect it afterward.
fn install_store(pairs: &[(&str, &str)]) -> MemStore {
    let store = MemStore::default();
    for (k, v) in pairs {
        store
            .0
            .borrow_mut()
            .insert((*k).to_string(), (*v).to_string());
    }
    day_core::set_nav_store(std::rc::Rc::new(store.clone()));
    store
}

#[test]
fn selector_restore_reopens_last_tab_and_persists() {
    // A store already holding a last-selected tab: the selector reopens on it, and a later
    // selection is written back through the store.
    let store = install_store(&[("day.nav.tabs", "three")]);
    let sel = Signal::new("one".to_string());
    let probe = boot(move || {
        selector(sel)
            .style(SelectorStyle::Tabs)
            .restore("day.nav.tabs")
            .item("one", "One", || label("one-content"))
            .item("two", "Two", || label("two-content"))
            .item("three", "Three", || label("three-content"))
            .any()
    });
    flush_sync();
    assert_eq!(
        day_core::current_route().as_deref(),
        Some("three"),
        "restored"
    );
    assert_eq!(probe.find_by_kind("day.tabs")[0].1.value, 2.0);

    // A later selection is persisted.
    assert!(navigate("two"));
    flush_sync();
    assert_eq!(
        store.0.borrow().get("day.nav.tabs").map(String::as_str),
        Some("two"),
        "selection persisted through the store"
    );
}

#[test]
fn selector_restore_ignores_stale_key() {
    // A saved key whose item no longer exists is ignored — the selector opens on the app default.
    install_store(&[("day.nav.tabs", "gone")]);
    let sel = Signal::new("one".to_string());
    let probe = boot(move || {
        selector(sel)
            .style(SelectorStyle::Tabs)
            .restore("day.nav.tabs")
            .item("one", "One", || label("one-content"))
            .item("two", "Two", || label("two-content"))
            .any()
    });
    flush_sync();
    assert_eq!(
        day_core::current_route().as_deref(),
        Some("one"),
        "app default kept"
    );
    assert_eq!(probe.find_by_kind("day.tabs")[0].1.value, 0.0);
}

#[test]
fn stack_restore_reopens_saved_path_and_persists() {
    // A store holding a two-deep path: the stack rebuilds it at launch, and a pop is written back.
    let store = install_store(&[("day.nav.stack", "a/b")]);
    let path = Signal::new(Vec::<String>::new());
    let probe = boot(move || {
        stack(path, label("home-content"))
            .destination(|key| label(format!("detail:{key}")))
            .restore("day.nav.stack")
            .any()
    });
    flush_sync();
    assert_eq!(
        day_core::current_route().as_deref(),
        Some("a/b"),
        "path restored"
    );
    assert_eq!(probe.find_by_kind("day.nav_page").len(), 3);
    assert!(
        probe
            .find_by_kind("day.label")
            .iter()
            .any(|(_, w)| w.text == "detail:b")
    );

    // A back writes the shorter path back through the store.
    assert!(nav_back());
    flush_sync();
    assert_eq!(
        store.0.borrow().get("day.nav.stack").map(String::as_str),
        Some("a"),
        "shortened path persisted"
    );
}

#[test]
fn stack_restore_round_trips_a_key_containing_a_slash() {
    // A `String` stack key that itself contains the path separator must survive persist→restore:
    // it is percent-encoded on the way out (like the rest of nav), not split into two segments.
    let store = install_store(&[]);
    let path = Signal::new(Vec::<String>::new());
    let probe = boot(move || {
        stack(path, label("home"))
            .destination(|k| label(format!("d:{k}")))
            .restore("np.stack")
            .any()
    });
    flush_sync();
    // Push ONE key that contains '/'.
    batch(|| path.set(vec!["a/b".to_string()]));
    flush_sync();
    let saved = store.0.borrow().get("np.stack").cloned().unwrap();
    assert!(
        !saved.contains('/'),
        "the slash must be percent-encoded, got {saved:?}"
    );

    // A fresh launch restores the SAME single key — one pushed page, not two.
    let path2 = Signal::new(Vec::<String>::new());
    let probe2 = boot(move || {
        stack(path2, label("home"))
            .destination(|k| label(format!("d:{k}")))
            .restore("np.stack")
            .any()
    });
    flush_sync();
    assert_eq!(
        path2.get_untracked(),
        vec!["a/b".to_string()],
        "one key restored"
    );
    assert_eq!(
        probe2.find_by_kind("day.nav_page").len(),
        2,
        "root + one pushed page (not split into two)"
    );
    let _ = probe;
}

#[test]
fn restore_yields_to_launch_deeplink() {
    // A launch deep link outranks restored state: the saved tab is ignored and the deep link wins.
    install_store(&[("day.nav.dl", "three")]);
    let sel = Signal::new("one".to_string());
    let probe = boot_with_env(Some(("DAY_DEEPLINK", "two")), move || {
        selector(sel)
            .style(SelectorStyle::Tabs)
            .restore("day.nav.dl")
            .item("one", "One", || label("one-content"))
            .item("two", "Two", || label("two-content"))
            .item("three", "Three", || label("three-content"))
            .any()
    });
    flush_sync();
    assert_eq!(
        day_core::current_route().as_deref(),
        Some("two"),
        "deep link wins"
    );
    assert_eq!(probe.find_by_kind("day.tabs")[0].1.value, 1.0);
}

// --- .local() and the sibling-collision footgun (docs/navigation.md) ------------------------

#[test]
fn local_selector_stays_out_of_the_route() {
    // Two one-of-N surfaces at the SAME level: the second is `.local()`, so only the first
    // contributes to current_route() and `navigate` addresses the first. This is the fix for the
    // sibling collision the debug warning flags.
    let a = Signal::new("a1".to_string());
    let b = Signal::new("b1".to_string());
    let probe = boot(move || {
        column((
            selector(a)
                .style(SelectorStyle::Tabs)
                .item("a1", "A1", || label("a1"))
                .item("a2", "A2", || label("a2")),
            selector(b)
                .style(SelectorStyle::Tabs)
                .local()
                .item("b1", "B1", || label("b1"))
                .item("b2", "B2", || label("b2")),
        ))
        .any()
    });
    flush_sync();
    // Only the routed selector's key is in the route.
    assert_eq!(day_core::current_route().as_deref(), Some("a1"));
    // `navigate` addresses the routed one; the local one is untouched by it.
    assert!(navigate("a2"));
    flush_sync();
    assert_eq!(a.get_untracked(), "a2");
    assert_eq!(
        b.get_untracked(),
        "b1",
        "the .local() selector is not routable"
    );
    let _ = probe;
}

#[test]
fn two_routed_siblings_concatenate_into_the_route() {
    // Documents WHY `.local()` exists: two routed one-of-N surfaces at one level both feed
    // current_route(), so you get a concatenated `a1/b1`. (In a debug build this also emits the
    // sibling warning; behavior is unchanged either way.)
    let a = Signal::new("a1".to_string());
    let b = Signal::new("b1".to_string());
    let _probe = boot(move || {
        column((
            selector(a)
                .style(SelectorStyle::Tabs)
                .item("a1", "A1", || label("a1"))
                .item("a2", "A2", || label("a2")),
            selector(b)
                .style(SelectorStyle::Tabs)
                .item("b1", "B1", || label("b1"))
                .item("b2", "B2", || label("b2")),
        ))
        .any()
    });
    flush_sync();
    let route = day_core::current_route().unwrap_or_default();
    assert!(
        route.contains("a1") && route.contains("b1") && route.contains('/'),
        "both routed siblings concatenate, got {route:?}"
    );
}

// ---------------------------------------------------------------------------
// Secondary windows (docs/windows.md): the open/close/focus seam, the async
// (Pending) completion path, and the cover fallback tier.
// ---------------------------------------------------------------------------

fn win_options(title: &str, w: f64, h: f64) -> WindowOptions {
    WindowOptions {
        title: title.into(),
        size: Size::new(w, h),
        ..Default::default()
    }
}

#[test]
fn open_window_builds_and_lays_out_at_its_own_size() {
    let probe = boot(|| label("main").any());
    let handle = day_core::open_window(
        None,
        win_options("second", 300.0, 200.0),
        day_spec::WindowKind::Normal,
        || column((label("in window").id("w2-label"),)).grow().any(),
    );
    flush_sync();

    assert!(handle.is_open());
    let wins = probe.windows();
    assert_eq!(wins.len(), 1);
    assert_eq!(wins[0].title, "second");
    assert_eq!(wins[0].kind, "normal");
    assert!(wins[0].open);
    assert!(
        probe.log().iter().any(|l| l.starts_with("open_window #")),
        "open_window duty not called: {:?}",
        probe.log()
    );
    // The window's content lays out at ITS size, not the primary's 400×600.
    assert!(
        probe
            .find_by_kind("day.container")
            .iter()
            .any(|(_, w)| w.frame.size == Size::new(300.0, 200.0)),
        "no container laid out at the window size"
    );
}

#[test]
fn cross_window_find_and_tap_by_id() {
    let clicks = day_reactive::Scope::root().enter(|| day_reactive::Signal::new(0i64));
    let probe = boot(move || label(move || format!("clicks {}", clicks.get())).any());
    day_core::open_window(
        None,
        win_options("second", 300.0, 200.0),
        day_spec::WindowKind::Normal,
        move || {
            button("press")
                .action(move || clicks.set(clicks.get() + 1))
                .id("w2-btn")
                .any()
        },
    );
    flush_sync();

    // The one tree spans windows: the id resolves without any window scoping.
    assert!(day_core::with_tree(|t| t.find_by_id("w2-btn")).is_some());
    let btn = node_id(&probe, "day.button", 0);
    probe.emit(btn, Event::Pressed);
    flush_sync();
    let texts: Vec<String> = probe
        .find_by_kind("day.label")
        .iter()
        .map(|(_, w)| w.text.clone())
        .collect();
    assert!(
        texts.iter().any(|t| t == "clicks 1"),
        "primary-window label did not react to the secondary-window press: {texts:?}"
    );
}

#[test]
fn window_resize_relayouts_only_that_window() {
    let probe = boot(|| column((label("main"),)).grow().any());
    day_core::open_window(
        None,
        win_options("second", 300.0, 200.0),
        day_spec::WindowKind::Normal,
        || column((label("w2"),)).grow().any(),
    );
    flush_sync();
    let node = probe.windows()[0].node;

    let mark = probe.log_len();
    probe.resize_window(node, Size::new(350.0, 250.0));
    flush_sync();
    assert!(
        probe
            .find_by_kind("day.container")
            .iter()
            .any(|(_, w)| w.frame.size == Size::new(350.0, 250.0)),
        "window content did not relayout to the new size"
    );
    // The primary's content kept its 400×600 frame — no cross-window relayout ops.
    assert!(
        probe
            .find_by_kind("day.container")
            .iter()
            .any(|(_, w)| w.frame.size == Size::new(400.0, 600.0)),
        "primary content frame disturbed by a secondary resize"
    );
    let since = probe.log_since(mark);
    assert!(
        !since.iter().any(|l| l.contains("main")),
        "primary widgets were touched by a secondary-window resize: {since:?}"
    );
}

#[test]
fn programmatic_close_round_trips_and_tears_down() {
    let probe = boot(|| label("main").any());
    let closed = day_reactive::Scope::root().enter(|| day_reactive::Signal::new(false));
    let handle = day_core::open_window(
        Some("second"),
        win_options("second", 300.0, 200.0),
        day_spec::WindowKind::Normal,
        || label("in window").id("w2-label").any(),
    );
    handle.on_close(move || closed.set(true));
    flush_sync();
    assert!(day_core::with_tree(|t| t.find_by_id("w2-label")).is_some());

    handle.close();
    flush_sync();

    assert!(
        probe.log().iter().any(|l| l.starts_with("close_window #")),
        "close_window duty not called"
    );
    assert!(!handle.is_open());
    assert!(day_core::window_by_key("second").is_none());
    // Leak canary: the window's content is gone from the widget table and the tree.
    assert!(day_core::with_tree(|t| t.find_by_id("w2-label")).is_none());
    assert!(
        !probe
            .find_by_kind("day.label")
            .iter()
            .any(|(_, w)| w.text == "in window"),
        "window content leaked past close"
    );
    assert!(closed.get_untracked(), "on_close did not run");
    // Idempotent: a second close is a no-op.
    let mark = probe.log_len();
    handle.close();
    flush_sync();
    assert_eq!(probe.log_since(mark), Vec::<String>::new());
}

#[test]
fn native_close_tears_down_and_fires_on_close() {
    let probe = boot(|| label("main").any());
    let closed = day_reactive::Scope::root().enter(|| day_reactive::Signal::new(false));
    let handle = day_core::open_window(
        None,
        win_options("second", 300.0, 200.0),
        day_spec::WindowKind::Normal,
        || label("in window").id("w2-label").any(),
    );
    handle.on_close(move || closed.set(true));
    flush_sync();

    // The title-bar path: the platform reports the close; day-core tears down on receipt.
    probe.close_window_natively(probe.windows()[0].node);
    flush_sync();
    assert!(!handle.is_open());
    assert!(day_core::with_tree(|t| t.find_by_id("w2-label")).is_none());
    assert!(closed.get_untracked(), "on_close did not run");
}

/// The close policy (docs/windows.md): the app's life is the life of its PRIMARY windows, and
/// a settings panel is not one of them. Closing the last primary quits even with a preferences
/// window still open — and the panel goes with it rather than stranding a windowless process.
///
/// Except on macOS, where the windowless state is the convention rather than a stranding:
/// `applicationShouldTerminateAfterLastWindowClosed` defaults to false, the menu bar stays live,
/// and a Settings window is independent of the documents — closing the last document there
/// leaves Settings open, so this asserts that it survives.
///
/// The INITIAL window closes FIRST here, and must be no different from any other window: it is
/// an ordinary registry record, so the app carries on while another primary is open.
#[test]
fn last_primary_close_quits_even_with_a_secondary_window_open() {
    let probe = boot(|| label("main").any());
    let initial = day_core::windows::initial_window().expect("initial window adopted at boot");
    let extra = day_core::open_window(
        None,
        win_options("Second", 800.0, 600.0),
        day_spec::WindowKind::Normal,
        || label("second body").any(),
    );
    let prefs = day_core::open_window(
        Some("prefs"),
        win_options("Settings", 520.0, 640.0),
        day_spec::WindowKind::Preferences,
        || label("prefs body").any(),
    );
    flush_sync();
    assert_eq!(
        day_core::windows::primary_window_count(),
        2,
        "the initial window counts like any other primary"
    );

    // The INITIAL window goes first: the app must NOT end — another primary is still open.
    let mark0 = probe.log_len();
    probe.close_window_natively(day_core::windows::window_node_id(&initial));
    flush_sync();
    assert!(!initial.is_open());
    assert!(
        !probe.log_since(mark0).iter().any(|l| l == "quit_app"),
        "closing the FIRST window ended the app while another primary was open"
    );
    assert_eq!(day_core::windows::primary_window_count(), 1);

    let mark = probe.log_len();
    probe.close_window_natively(day_core::windows::window_node_id(&extra));
    flush_sync();
    // …but that WAS the last primary, so the app ends and the settings panel closes with it —
    // everywhere the app actually ends. macOS keeps running, and keeps its Settings window.
    assert!(!extra.is_open());
    assert_eq!(
        prefs.is_open(),
        cfg!(target_os = "macos"),
        "a secondary window must go with the app, and must survive an app that stays up"
    );
    assert_eq!(day_core::windows::primary_window_count(), 0);
    let quit = probe.log_since(mark).iter().any(|l| l == "quit_app");
    // macOS keeps a windowless app alive on purpose (its menu bar stays live), so the policy
    // is platform-conditional and the assertion follows it.
    assert_eq!(
        quit,
        !cfg!(target_os = "macos"),
        "quit_app reached the toolkit"
    );
}

/// The other half: closing a SECONDARY window never ends the app, however few windows remain.
#[test]
fn closing_a_secondary_window_never_quits() {
    let probe = boot(|| label("main").any());
    let prefs = day_core::open_window(
        Some("prefs"),
        win_options("Settings", 520.0, 640.0),
        day_spec::WindowKind::Preferences,
        || label("prefs body").any(),
    );
    flush_sync();
    let mark = probe.log_len();
    probe.close_window_natively(day_core::windows::window_node_id(&prefs));
    flush_sync();
    assert!(!prefs.is_open());
    assert!(
        !probe.log_since(mark).iter().any(|l| l == "quit_app"),
        "closing a settings panel ended the app"
    );
}

#[test]
fn singleton_key_opens_once_and_refocuses() {
    let probe = boot(|| label("main").any());
    let first = day_core::open_window(
        Some("prefs"),
        win_options("Settings", 520.0, 640.0),
        day_spec::WindowKind::Preferences,
        || label("prefs body").any(),
    );
    flush_sync();
    let mark = probe.log_len();
    let second = day_core::open_window(
        Some("prefs"),
        win_options("Settings", 520.0, 640.0),
        day_spec::WindowKind::Preferences,
        || label("SHOULD NOT BUILD").any(),
    );
    flush_sync();

    assert_eq!(probe.windows().len(), 1, "singleton key opened twice");
    assert_eq!(probe.windows()[0].kind, "preferences");
    assert!(first.is_open() && second.is_open());
    assert!(
        probe
            .log_since(mark)
            .iter()
            .any(|l| l.starts_with("focus_window #")),
        "reopen did not focus the existing window"
    );
    assert!(
        !probe
            .find_by_kind("day.label")
            .iter()
            .any(|(_, w)| w.text == "SHOULD NOT BUILD"),
        "singleton reopen ran the builder"
    );
    assert!(probe.windows()[0].focused);
    assert!(day_core::focused_window().is_some());
}

#[test]
fn set_title_reaches_the_backend() {
    let probe = boot(|| label("main").any());
    let handle = day_core::open_window(
        None,
        win_options("before", 300.0, 200.0),
        day_spec::WindowKind::Normal,
        || label("w2").any(),
    );
    flush_sync();
    handle.set_title("after");
    assert_eq!(probe.windows()[0].title, "after");
    assert!(
        probe
            .log()
            .iter()
            .any(|l| l.starts_with("set_window_title #") && l.contains("\"after\"")),
        "set_window_title duty not called"
    );
}

#[test]
fn fallback_presents_as_cover_and_close_dismisses() {
    let probe = boot(|| label("main").any());
    probe.set_no_multi_window(true);
    assert_eq!(
        day_core::capability(day_spec::Cap::MultiWindow),
        day_spec::Support::Unsupported
    );

    let handle = day_core::open_window(
        Some("prefs"),
        win_options("Settings", 520.0, 640.0),
        day_spec::WindowKind::Preferences,
        || label("prefs body").id("prefs-label").any(),
    );
    flush_sync();

    // No native window — a COVER presented in the primary instead.
    assert!(probe.windows().is_empty());
    let covers = probe.find_by_kind("day.cover");
    assert_eq!(covers.len(), 1, "no cover realized for the fallback tier");
    assert!(covers[0].1.flag, "cover not presented");
    assert!(
        probe.log().iter().any(|l| l.contains("cover present")),
        "no present patch: {:?}",
        probe.log()
    );
    // The native surface reports its size; the content lays out inside it.
    let cover_node = NodeId(covers[0].1.node);
    probe.emit(cover_node, Event::FrameChanged(Size::new(400.0, 600.0)));
    flush_sync();
    assert!(day_core::with_tree(|t| t.find_by_id("prefs-label")).is_some());
    // And LAID OUT at the reported size — the primary root's PassThrough never descends
    // into a second child, so the fallback surface must drive its own layout entry (the
    // regression the first iOS run caught: content present but frameless).
    assert!(
        probe
            .find_by_kind("day.label")
            .iter()
            .any(|(_, w)| w.text == "prefs body" && w.frame.size.width > 0.0),
        "cover-fallback content not laid out"
    );
    // The singleton key still holds on this tier.
    assert!(day_core::window_by_key("prefs").is_some());

    handle.close();
    flush_sync();
    assert!(
        probe.log().iter().any(|l| l.ends_with("cover dismiss")),
        "close did not dismiss the cover"
    );
    // Content survives until the hide transition confirms…
    assert!(day_core::with_tree(|t| t.find_by_id("prefs-label")).is_some());
    probe.emit(cover_node, Event::CoverHidden);
    flush_sync();
    // …then everything goes.
    assert!(!handle.is_open());
    assert!(day_core::with_tree(|t| t.find_by_id("prefs-label")).is_none());
    assert!(day_core::window_by_key("prefs").is_none());
}

#[test]
fn pending_open_completes_and_builds() {
    let probe = boot(|| label("main").any());
    probe.set_pending_windows(true);
    let handle = day_core::open_window(
        Some("detail"),
        win_options("Detail", 300.0, 200.0),
        day_spec::WindowKind::Normal,
        || label("detail body").id("detail-label").any(),
    );
    flush_sync();

    // Parked: record exists, nothing built, no live window yet.
    assert!(handle.is_open());
    assert!(probe.windows().is_empty());
    assert!(day_core::with_tree(|t| t.find_by_id("detail-label")).is_none());

    // The native side finishes creation (the scene/activity/ability connecting).
    let node = day_core::windows::window_node_id(&handle);
    let raw = probe
        .complete_window(node, Size::new(300.0, 200.0))
        .expect("no pending open recorded");
    assert!(day_core::finish_window_open(
        node,
        raw,
        Size::new(300.0, 200.0)
    ));
    flush_sync();

    assert_eq!(probe.windows().len(), 1);
    assert!(day_core::with_tree(|t| t.find_by_id("detail-label")).is_some());
    // The parked title applied at completion.
    assert!(
        probe
            .log()
            .iter()
            .any(|l| l.starts_with("set_window_title #") && l.contains("\"Detail\"")),
        "parked title not applied at completion"
    );
}

#[test]
fn pending_close_before_completion_cancels() {
    let probe = boot(|| label("main").any());
    probe.set_pending_windows(true);
    let handle = day_core::open_window(
        None,
        win_options("Detail", 300.0, 200.0),
        day_spec::WindowKind::Normal,
        || label("detail body").id("detail-label").any(),
    );
    flush_sync();
    handle.close();
    flush_sync();
    assert!(!handle.is_open());

    // The native side finishes anyway — completion must answer false so the backend
    // drops the window it just created.
    let node = day_core::windows::window_node_id(&handle);
    let raw = probe
        .complete_window(node, Size::new(300.0, 200.0))
        .expect("no pending open recorded");
    assert!(!day_core::finish_window_open(
        node,
        raw,
        Size::new(300.0, 200.0)
    ));
    flush_sync();
    assert!(day_core::with_tree(|t| t.find_by_id("detail-label")).is_none());
}

#[test]
fn register_preferences_injects_menu_item_and_dispatch_opens_singleton() {
    let probe = boot(|| label("main").any());
    // App menu installed BEFORE registration — the retained-model re-forward self-heals.
    app_menu(vec![sub_menu("File", vec![menu_item("Save").key("s")])]);
    day_core::register_preferences(|| label("prefs body").id("prefs-label").any());
    flush_sync();

    // The injection appended a live Preferences item to the File menu.
    let model = day_core::menu::app_menu_model();
    let found = {
        fn find_prefs(items: &[day_spec::MenuItem]) -> Option<u64> {
            items.iter().find_map(|it| match it {
                day_spec::MenuItem::Action { id, role, .. }
                    if *role == Some(day_spec::MenuRole::Preferences) =>
                {
                    Some(*id)
                }
                day_spec::MenuItem::Submenu { items, .. } => find_prefs(items),
                _ => None,
            })
        }
        find_prefs(&model)
    };
    let prefs_id = found.expect("no Preferences item injected");
    assert_ne!(prefs_id, 0, "injected item is inert");

    // Dispatching the action opens the singleton preferences window…
    day_core::dispatch_menu_action(prefs_id);
    flush_sync();
    assert_eq!(probe.windows().len(), 1);
    assert_eq!(probe.windows()[0].kind, "preferences");
    assert!(day_core::with_tree(|t| t.find_by_id("prefs-label")).is_some());
    // …and dispatching again focuses instead of duplicating.
    day_core::dispatch_menu_action(prefs_id);
    flush_sync();
    assert_eq!(probe.windows().len(), 1);
    assert!(
        probe.log().iter().any(|l| l.starts_with("focus_window #")),
        "second open did not focus"
    );
    // open_preferences() is the same path for a toolbar gear.
    assert!(day_core::open_preferences());
    assert_eq!(probe.windows().len(), 1);
}

/// `.searchable()` is declared on the SURFACE, and the query stays an app-owned signal
/// (docs/search.md). That is what will let the field move between the toolbar and the navigation
/// list without the state moving with it, so the binding has to run in both directions against
/// the signal — never against the widget.
#[test]
fn searchable_binds_the_query_both_ways() {
    let section = Signal::new(Option::<String>::None);
    let query = Signal::new(String::new());
    let scope = Signal::new(0usize);
    let rows = ["alpha".to_string(), "beta".to_string()];
    let q_r = query;
    let probe = boot(move || {
        selector(section)
            .style(SelectorStyle::Sidebar)
            .searchable(q_r)
            .search_prompt("Find")
            .search_scopes(scope, vec!["All", "Recent"])
            .items(
                move || {
                    // TRACKED: the row set narrows as the query changes, which is the whole point
                    // of binding search to the surface the rows come from.
                    let q = q_r.get().to_lowercase();
                    rows.iter()
                        .filter(|r| q.is_empty() || r.starts_with(&q))
                        .cloned()
                        .collect::<Vec<_>>()
                },
                |r: &String| item(r.clone(), r.clone()),
            )
            .destination(|_: &Option<String>| label("detail"))
            .any()
    });
    let menu = probe.find_by_kind("day.nav_menu")[0].0;
    let host = node_id(&probe, "day.nav", 0);
    assert_eq!(probe.widget(menu).text, "alpha|beta", "unfiltered to start");

    // Backend → app: the user typing writes the app's signal, and the rows re-derive from it.
    probe.emit(host, Event::SearchChanged("be".into()));
    flush_sync();
    assert_eq!(query.get_untracked(), "be", "typing wrote the app signal");
    assert_eq!(
        probe.widget(menu).text,
        "beta",
        "rows narrowed to the query"
    );

    // App → backend: the app clearing its own signal restores the rows. The field follows through
    // a targeted patch rather than a rebuild, so this direction must work without touching it.
    batch(|| query.set(String::new()));
    flush_sync();
    assert_eq!(
        probe.widget(menu).text,
        "alpha|beta",
        "cleared restores rows"
    );

    // Scopes are one-of-N over an app signal, same discipline as the query.
    probe.emit(host, Event::SearchScopeChanged(1));
    flush_sync();
    assert_eq!(
        scope.get_untracked(),
        1,
        "scope choice wrote the app signal"
    );
}

/// A `.deletable()` list over `seed()`, recording each committed delete and mirroring the
/// removal into the backing signal the way an app's `on_delete` would.
fn deletable_list(
    deletes: std::rc::Rc<std::cell::RefCell<Vec<usize>>>,
    order: std::rc::Rc<std::cell::RefCell<Vec<String>>>,
    guard: Option<fn(usize) -> bool>,
) -> AnyPiece {
    let items = Signal::new(seed());
    let mut l = list(
        move || items.get(),
        |s: &String| s.clone(),
        |row: ItemSlot<String, String>| label(move || row.get()),
    )
    .row_height(RowHeight::Uniform(20.0))
    .deletable(true)
    .on_delete(move |index| {
        deletes.borrow_mut().push(index);
        items.update(|v| {
            v.remove(index);
        });
        *order.borrow_mut() = items.get_untracked();
    });
    if let Some(g) = guard {
        l = l.delete_guard(g);
    }
    l.any()
}

#[test]
fn list_delete_commits_shortens_and_defers_callback() {
    let deletes = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let order = std::rc::Rc::new(std::cell::RefCell::new(seed()));
    let (d, o) = (deletes.clone(), order.clone());
    let probe = boot(move || deletable_list(d, o, None));
    let host = probe.find_by_kind("day.list")[0].0;

    // No guard: every row is offered.
    assert_eq!(probe.list_can_delete(host, 1), Some(true));

    assert!(probe.list_delete(host, 1));
    // The snapshot is ALREADY shorter when the commit returns — that is the seam's contract, so
    // a backend animating the removal reads the new length while the animation runs.
    assert_eq!(probe.list_len(host), 4);
    // The app's callback rides the event queue (never the swipe callback itself); the probe
    // pumps it, exactly as the reorder commit above does.
    assert_eq!(deletes.borrow().as_slice(), [1]);
    assert_eq!(
        order.borrow().as_slice(),
        ["a".to_string(), "c".into(), "d".into(), "e".into()]
    );
}

#[test]
fn list_delete_refused_by_guard_and_unsupported_without_optin() {
    let deletes = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let order = std::rc::Rc::new(std::cell::RefCell::new(seed()));
    let (d, o) = (deletes.clone(), order.clone());
    // The pinned-first-row pattern the Showcase demonstrates.
    let probe = boot(move || deletable_list(d, o, Some(|i: usize| i != 0)));
    let host = probe.find_by_kind("day.list")[0].0;

    // The guard answers BEFORE the affordance is offered, so row 0 shows no action at all.
    assert_eq!(probe.list_can_delete(host, 0), Some(false));
    assert!(!probe.list_delete(host, 0));
    assert!(deletes.borrow().is_empty());
    assert_eq!(probe.list_len(host), 5, "a refused delete changes nothing");

    // A list that never opted in has no seam at all — a backend must not offer the gesture.
    let probe2 = boot(|| {
        let items = Signal::new(seed());
        list(
            move || items.get(),
            |s: &String| s.clone(),
            |row: ItemSlot<String, String>| label(move || row.get()),
        )
        .row_height(RowHeight::Uniform(20.0))
        .any()
    });
    let host2 = probe2.find_by_kind("day.list")[0].0;
    assert_eq!(probe2.list_can_delete(host2, 0), None);
    assert!(!probe2.list_delete(host2, 0));
}
