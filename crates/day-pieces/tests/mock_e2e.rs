//! M1 acceptance (DESIGN.md §21.2): end-to-end on the mock toolkit. The op log IS the
//! fine-grained-invalidation contract — "exactly one mutation op per state change" and
//! "bounded measure calls" are assertions, not aspirations.

use day_core::AnyPiece;
use day_mock::{MockProbe, MockToolkit};
use day_pieces::prelude::*;
use day_reactive::flush_sync;
use day_spec::{Event, NodeId, Size, WindowOptions};

fn boot(root: impl FnOnce() -> AnyPiece + 'static) -> MockProbe {
    day_core::uninstall_tree();
    let (mock, probe) = MockToolkit::new();
    let options =
        WindowOptions { title: "test".into(), size: Size::new(400.0, 600.0), min_size: None };
    day_core::launch_with(mock, options, root);
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
    let muts: Vec<String> =
        probe.mutations().into_iter().filter(|m| !m.starts_with("a11y")).collect();
    assert_eq!(muts.len(), 1, "expected exactly one mutation, got: {muts:?}");
    assert!(muts[0].contains("update day.label"), "unexpected op: {}", muts[0]);
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
        column((label("aa"), label("bbbb"))).spacing(10.0).align(HAlign::Leading).any()
    });
    let labels = probe.find_by_kind("day.label");
    // 8pt/char, 16pt line: "aa" = 16x16 at y=0; "bbbb" = 32x16 at y=26 (16 + spacing 10).
    assert_eq!(labels[0].1.frame, day_spec::Rect::new(0.0, 0.0, 16.0, 16.0));
    assert_eq!(labels[1].1.frame, day_spec::Rect::new(0.0, 26.0, 32.0, 16.0));
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
    assert_eq!(labels[0].1.frame.size, Size::new(100.0, 48.0), "expected 3 wrapped lines");
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
    let echo: Vec<String> =
        probe.mutations().into_iter().filter(|m| m.contains("from_native=true")).collect();
    assert_eq!(echo.len(), 1, "expected one origin-tagged echo: {:?}", probe.mutations());

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
fn when_builds_and_disposes() {
    let show = Signal::new(false);
    let probe = boot(move || {
        column((label("always"), when(move || show.get(), || label("sometimes")))).any()
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
    let items: Signal<Vec<(u64, String)>> =
        Signal::new(vec![(1, "one".into()), (2, "two".into())]);
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
    let realizes: Vec<String> =
        probe.log().into_iter().filter(|l| l.starts_with("realize")).collect();
    assert_eq!(realizes.len(), 1, "one realize for the inserted row: {realizes:?}");
    assert_eq!(probe.find_by_kind("day.label").len(), 3);

    // Item mutation: surviving row's slot propagates — an update, never a rebuild (§5.4).
    probe.clear_log();
    batch(|| items.update(|v| v[0].1 = "uno".into()));
    let log = probe.log();
    assert!(!log.iter().any(|l| l.starts_with("realize")), "no rebuild on value change: {log:?}");
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
    assert_eq!(labels[1].1.frame.origin.x, 400.0 - 24.0, "trailing label pinned to the end");
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
    assert!(content.height >= 600.0, "content at least viewport height: {content:?}");
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
