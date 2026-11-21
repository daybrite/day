// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Lists and scroll views on the mock (docs/list.md, docs/scroll.md): keyed updates that
//! splice instead of reloading, following the end, reaching the end, the visible range, the
//! header, footer and empty state, row scrolls with an alignment, and a scroll view's props,
//! offset, phase and zoom. The mock plays the native host: a test reports the viewport and
//! plays the user's scrolling by hand, as a toolkit would.

use day_mock::{MockHandle, MockProbe, MockToolkit};
use day_pieces::Items;
use day_pieces::prelude::*;
use day_reactive::flush_sync;
use day_spec::{Event, ListViewport, NodeId, Size, Support, WindowOptions};

static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn boot_with<P: Piece>(
    setup: impl FnOnce(&MockProbe),
    root: impl FnOnce() -> P + 'static,
) -> MockProbe {
    let _guard = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    day_core::uninstall_tree();
    let (mock, probe) = MockToolkit::new();
    // Capabilities are read while the page builds.
    setup(&probe);
    let options = WindowOptions {
        title: "test".into(),
        size: Size::new(400.0, 600.0),
        ..Default::default()
    };
    day_core::launch_with(mock, options, move || root().any());
    probe
}

fn boot<P: Piece>(root: impl FnOnce() -> P + 'static) -> MockProbe {
    boot_with(|_| {}, root)
}

fn list_host(probe: &MockProbe) -> MockHandle {
    probe.find_by_kind("day.list")[0].0
}

fn scroll_host(probe: &MockProbe) -> (MockHandle, NodeId) {
    let (handle, widget) = probe.find_by_kind("day.scroll").remove(0);
    (handle, NodeId(widget.node))
}

/// The list patches the host received since the log was last cleared, without the prefix.
fn list_patches(probe: &MockProbe) -> Vec<String> {
    probe
        .mutations()
        .iter()
        .filter_map(|line| line.split_once(" list ").map(|(_, patch)| patch.to_owned()))
        .collect()
}

fn label_texts(probe: &MockProbe) -> Vec<String> {
    probe
        .find_by_kind("day.label")
        .into_iter()
        .map(|(_, w)| w.text)
        .collect()
}

fn report(probe: &MockProbe, first: usize, last: usize, end_distance: f64) {
    probe.list_viewport(
        list_host(probe),
        ListViewport {
            first,
            last,
            end_distance,
        },
    );
    day_core::pump_events();
    flush_sync();
}

type Row = (u32, String);

fn rows_of(ids: &[u32]) -> Vec<Row> {
    ids.iter().map(|id| (*id, format!("row {id}"))).collect()
}

fn keyed_list(rows: Signal<Vec<Row>>) -> List<Items<Row, u32>> {
    list(
        items(move || rows.get(), |r: &Row| r.0),
        |slot: ItemSlot<Row, u32>| label(move || slot.field(|r| r.1.clone())),
    )
    .row_height(RowHeight::Uniform(20.0))
}

// --- keyed updates -----------------------------------------------------------------------

#[test]
fn a_plain_items_list_splices_what_changed() {
    let rows = Signal::new(rows_of(&[1, 2, 3]));
    let probe = boot(move || keyed_list(rows));
    probe.clear_log();

    // An append is one insert at the end, with no reload.
    rows.update(|v| v.push((4, "row 4".into())));
    flush_sync();
    assert_eq!(list_patches(&probe), ["splice [Insert(3)]"]);

    // A removal from the middle, then a row moved to the top.
    probe.clear_log();
    rows.update(|v| {
        v.remove(1);
    });
    flush_sync();
    assert_eq!(list_patches(&probe), ["splice [Remove(1)]"]);
    probe.clear_log();
    rows.update(|v| {
        let last = v.remove(2);
        v.insert(0, last);
    });
    flush_sync();
    assert_eq!(list_patches(&probe), ["splice [Move(2, 0)]"]);

    // Rows that share nothing with the old ones are a different list: the host reloads.
    probe.clear_log();
    rows.set(rows_of(&[10, 11]));
    flush_sync();
    assert_eq!(list_patches(&probe), ["reload"]);
}

#[test]
fn a_changed_value_reaches_its_bound_row_without_a_native_patch() {
    let rows = Signal::new(rows_of(&[1, 2, 3]));
    let probe = boot(move || keyed_list(rows));
    let host = list_host(&probe);
    probe.list_bind(host, 0, MockHandle(9001));
    probe.list_bind(host, 1, MockHandle(9002));
    assert_eq!(label_texts(&probe), ["row 1", "row 2"]);

    // Same keys, one new value: nothing for the host to do, and the row shows it.
    probe.clear_log();
    rows.update(|v| v[1].1 = "edited".into());
    flush_sync();
    assert!(
        list_patches(&probe).is_empty(),
        "{:?}",
        list_patches(&probe)
    );
    assert_eq!(label_texts(&probe), ["row 1", "edited"]);

    // A row inserted above them: the bound rows keep showing their own items.
    probe.clear_log();
    rows.update(|v| v.insert(0, (9, "row 9".into())));
    flush_sync();
    assert_eq!(list_patches(&probe), ["splice [Insert(0)]"]);
    assert_eq!(label_texts(&probe), ["row 1", "edited"]);
}

#[test]
fn a_repeated_key_falls_back_to_a_reload() {
    let rows = Signal::new(rows_of(&[1, 2]));
    let probe = boot(move || keyed_list(rows));
    probe.clear_log();
    rows.set(rows_of(&[1, 2, 2]));
    flush_sync();
    assert_eq!(list_patches(&probe), ["reload"]);
}

#[test]
fn a_sectioned_list_splices_headers_with_its_rows() {
    let rows: Signal<Vec<(char, u32)>> = Signal::new(vec![('a', 1), ('a', 2), ('b', 3)]);
    let probe = boot(move || {
        list(
            items(move || rows.get(), |r: &(char, u32)| r.1),
            |slot: ItemSlot<(char, u32), u32>| label(move || slot.field(|r| r.1.to_string())),
        )
        .sections(
            move |i| rows.with(|r| r[i].0),
            |slot: ItemSlot<(char, u32), u32>| {
                label(move || format!("Section {}", slot.field(|r| r.0)))
            },
        )
    });
    let host = list_host(&probe);
    // Headers at display rows 0 and 3.
    assert_eq!(probe.list_len(host), 5);
    probe.clear_log();

    // A row joins the second section: one insert, in the host's own rows.
    rows.update(|v| v.push(('b', 4)));
    flush_sync();
    assert_eq!(list_patches(&probe), ["splice [Insert(5)]"]);
    assert_eq!(probe.list_len(host), 6);

    // A row that opens a third section brings its header with it.
    probe.clear_log();
    rows.update(|v| v.push(('c', 5)));
    flush_sync();
    assert_eq!(list_patches(&probe), ["splice [Insert(6), Insert(6)]"]);
    assert_eq!(probe.list_len(host), 8);
}

// --- following the end -------------------------------------------------------------------

#[test]
fn follow_end_follows_only_while_the_end_is_near() {
    let rows = Signal::new(rows_of(&[1, 2]));
    let probe = boot(move || keyed_list(rows).follow_end(40.0));
    let host = list_host(&probe);
    assert!(
        !probe.widget(host).flag,
        "building the list does not scroll"
    );

    // A list that has not reported is taken to be at its end.
    probe.clear_log();
    rows.update(|v| v.push((3, "row 3".into())));
    flush_sync();
    assert_eq!(
        list_patches(&probe),
        ["splice [Insert(2)]", "scroll-to-end animated=true"]
    );

    // The scroll is on its way: a distance reported in flight is not the reader leaving.
    report(&probe, 0, 1, 20.0 + 60.0);
    probe.clear_log();
    rows.update(|v| v.push((4, "row 4".into())));
    flush_sync();
    assert!(list_patches(&probe).contains(&"scroll-to-end animated=true".to_owned()));

    // Arrived, and then the reader scrolls up, further than the threshold: left alone.
    report(&probe, 2, 3, 0.0);
    report(&probe, 0, 1, 300.0);
    probe.clear_log();
    rows.update(|v| v.push((5, "row 5".into())));
    flush_sync();
    assert_eq!(list_patches(&probe), ["splice [Insert(4)]"]);

    // Back within the threshold: following again.
    report(&probe, 3, 4, 30.0);
    probe.clear_log();
    rows.update(|v| v.push((7, "row 7".into())));
    flush_sync();
    assert!(list_patches(&probe).contains(&"scroll-to-end animated=true".to_owned()));
}

/// A followed scroll the reader interrupts is theirs: the next row leaves them where they are,
/// though the list never reported arriving at the end (docs/list.md "Following the end").
#[test]
fn follow_end_stops_when_the_reader_takes_the_list() {
    let rows = Signal::new(rows_of(&[1, 2]));
    let probe = boot(move || keyed_list(rows).follow_end(40.0));
    let host = list_host(&probe);
    rows.update(|v| v.push((3, "row 3".into())));
    flush_sync();

    // Mid-flight, the reader grabs the list and holds it 300 points from the end.
    probe.list_reader_scroll(host);
    day_core::pump_events();
    report(&probe, 0, 1, 300.0);
    probe.clear_log();
    rows.update(|v| v.push((4, "row 4".into())));
    flush_sync();
    assert_eq!(list_patches(&probe), ["splice [Insert(3)]"]);

    // Even before the list says where they stopped, a new row leaves them alone.
    probe.list_reader_scroll(host);
    day_core::pump_events();
    probe.clear_log();
    rows.update(|v| v.push((5, "row 5".into())));
    flush_sync();
    assert_eq!(list_patches(&probe), ["splice [Insert(4)]"]);

    // A drag against the end itself: the list's next report repeats its last one, and still
    // counts, so the reader is at the end and followed.
    report(&probe, 3, 4, 0.0);
    probe.list_reader_scroll(host);
    day_core::pump_events();
    report(&probe, 3, 4, 0.0);
    probe.clear_log();
    rows.update(|v| v.push((6, "row 6".into())));
    flush_sync();
    assert!(list_patches(&probe).contains(&"scroll-to-end animated=true".to_owned()));

    // Back at the end by their own hand: following again.
    report(&probe, 4, 5, 0.0);
    probe.clear_log();
    rows.update(|v| v.push((6, "row 6".into())));
    flush_sync();
    assert!(list_patches(&probe).contains(&"scroll-to-end animated=true".to_owned()));
}

#[test]
fn follow_end_keeps_a_growing_row_in_view_without_animating() {
    let rows = Signal::new(rows_of(&[1, 2]));
    let probe = boot(move || keyed_list(rows).follow_end(40.0));
    probe.clear_log();
    // A streamed reply: the same rows, the last one's text longer.
    rows.update(|v| v[1].1.push_str(" and more"));
    flush_sync();
    assert_eq!(list_patches(&probe), ["scroll-to-end animated=false"]);
}

#[test]
fn scroll_to_end_follows_the_trigger_whatever_the_position() {
    let rows = Signal::new(rows_of(&[1, 2, 3]));
    let jump = Trigger::new();
    let probe = boot(move || keyed_list(rows).scroll_to_end(jump));
    probe.clear_log();
    batch(|| jump.notify());
    flush_sync();
    assert_eq!(list_patches(&probe), ["scroll-to-end animated=true"]);
}

// --- reaching the end --------------------------------------------------------------------

#[test]
fn end_reached_fires_once_per_approach_and_again_after_the_rows_change() {
    let rows = Signal::new(rows_of(&[1, 2, 3]));
    let fired = Signal::new(0_u32);
    let probe =
        boot(move || keyed_list(rows).on_end_reached(100.0, move || fired.update(|n| *n += 1)));

    // Far from the end: nothing.
    report(&probe, 0, 1, 500.0);
    assert_eq!(fired.get_untracked(), 0);
    // Within the threshold: once, however many reports follow.
    report(&probe, 1, 2, 80.0);
    report(&probe, 1, 2, 40.0);
    report(&probe, 1, 2, 0.0);
    assert_eq!(fired.get_untracked(), 1);
    // Away and back: once more.
    report(&probe, 0, 1, 400.0);
    report(&probe, 1, 2, 10.0);
    assert_eq!(fired.get_untracked(), 2);

    // The handler's rows arrive; the list is still at its end, by the same numbers, and
    // the reader has more to reach: it fires for the new end.
    rows.update(|v| v.push((4, "row 4".into())));
    flush_sync();
    report(&probe, 1, 2, 10.0);
    assert_eq!(fired.get_untracked(), 3);
    // The same report again, with nothing changed: no loop.
    report(&probe, 1, 2, 10.0);
    assert_eq!(fired.get_untracked(), 3);
}

#[test]
fn an_empty_list_reaches_no_end() {
    let rows: Signal<Vec<Row>> = Signal::new(Vec::new());
    let fired = Signal::new(0_u32);
    let probe =
        boot(move || keyed_list(rows).on_end_reached(100.0, move || fired.update(|n| *n += 1)));
    report(&probe, 0, 0, 0.0);
    assert_eq!(fired.get_untracked(), 0);
}

// --- the visible range -------------------------------------------------------------------

#[test]
fn visible_rows_follow_the_viewport_reports() {
    let rows = Signal::new(rows_of(&[1, 2, 3, 4, 5, 6]));
    let visible = Signal::new(0..0);
    let probe = boot(move || keyed_list(rows).visible_rows(visible));
    report(&probe, 1, 3, 40.0);
    assert_eq!(visible.get_untracked(), 1..4);
    // A report past the rows is clamped to them.
    report(&probe, 4, 9, 0.0);
    assert_eq!(visible.get_untracked(), 4..6);
}

// --- header, footer, empty state, padding ------------------------------------------------

#[test]
fn header_and_footer_are_rows_of_their_own_around_the_items() {
    let rows = Signal::new(rows_of(&[1, 2]));
    let picked = Signal::new(0_u32);
    let visible = Signal::new(0..0);
    let probe = boot(move || {
        keyed_list(rows)
            .header(|| label("Header"))
            .footer(|| label("Footer"))
            .on_select(move |key| picked.set(key))
            .visible_rows(visible)
    });
    let host = list_host(&probe);
    let node = NodeId(probe.widget(host).node);
    // Header, two items, footer.
    assert_eq!(probe.list_len(host), 4);
    let supplements: Vec<bool> = (0..4).map(|i| probe.list_is_supplement(host, i)).collect();
    assert_eq!(supplements, [true, false, false, true]);

    probe.list_bind(host, 0, MockHandle(9001));
    probe.list_bind(host, 1, MockHandle(9002));
    probe.list_bind(host, 3, MockHandle(9003));
    assert_eq!(label_texts(&probe), ["Header", "row 1", "Footer"]);

    // The host's row 2 is item 1; a supplementary row selects nothing.
    probe.emit(node, Event::SelectionChanged(2));
    probe.emit(node, Event::SelectionChanged(0));
    day_core::pump_events();
    flush_sync();
    assert_eq!(picked.get_untracked(), 2);

    // The header and the first item in view: the visible rows are the items only.
    report(&probe, 0, 1, 60.0);
    assert_eq!(visible.get_untracked(), 0..1);

    // An appended row is an insert below the header's offset.
    probe.clear_log();
    rows.update(|v| v.push((3, "row 3".into())));
    flush_sync();
    assert_eq!(list_patches(&probe), ["splice [Insert(3)]"]);
}

#[test]
fn the_empty_state_stands_in_for_the_rows_and_fills_the_viewport() {
    let rows: Signal<Vec<Row>> = Signal::new(Vec::new());
    let probe = boot(move || {
        keyed_list(rows)
            .empty(|| label("Nothing here"))
            .height(300.0)
    });
    let host = list_host(&probe);
    assert_eq!(probe.list_len(host), 1);
    assert!(probe.list_is_supplement(host, 0));
    probe.list_bind(host, 0, MockHandle(9001));
    assert_eq!(label_texts(&probe), ["Nothing here"]);
    // It takes the list's whole height, so its content can center there.
    let height = {
        let cell_height = probe.state.borrow().list_sources[&host.0]
            .cell_height
            .clone();
        cell_height(9001 as day_spec::RawHandle)
    };
    assert!((height - 300.0).abs() < 0.5, "empty state is {height} tall");

    // The first row replaces it.
    rows.set(rows_of(&[1]));
    flush_sync();
    assert_eq!(probe.list_len(host), 1);
    assert!(!probe.list_is_supplement(host, 0));
    // And it returns when the last row leaves.
    rows.set(Vec::new());
    flush_sync();
    assert!(probe.list_is_supplement(host, 0));
}

#[test]
fn content_padding_adds_a_row_above_and_below_and_insets_the_rest() {
    let rows = Signal::new(rows_of(&[1]));
    let probe = boot(move || {
        keyed_list(rows).content_padding(Insets {
            top: 12.0,
            leading: 16.0,
            bottom: 80.0,
            trailing: 16.0,
        })
    });
    let host = list_host(&probe);
    assert_eq!(probe.list_len(host), 3);
    let cell_height = probe.state.borrow().list_sources[&host.0]
        .cell_height
        .clone();
    probe.list_bind(host, 0, MockHandle(9001));
    probe.list_bind(host, 1, MockHandle(9002));
    probe.list_bind(host, 2, MockHandle(9003));
    assert!((cell_height(9001 as day_spec::RawHandle) - 12.0).abs() < 0.01);
    assert!((cell_height(9002 as day_spec::RawHandle) - 20.0).abs() < 0.01);
    assert!((cell_height(9003 as day_spec::RawHandle) - 80.0).abs() < 0.01);
    // The row's content starts past the leading inset.
    let (_, row_label) = probe.find_by_kind("day.label").remove(0);
    assert!(
        (row_label.frame.origin.x - 16.0).abs() < 0.01,
        "{:?}",
        row_label.frame
    );
}

// --- programmatic row scrolls ------------------------------------------------------------

#[test]
fn scroll_to_row_is_a_consumed_command_with_alignment_and_animation() {
    let rows = Signal::new(rows_of(&(0..50).collect::<Vec<_>>()));
    let jump: Signal<Option<RowTarget>> = Signal::new(None);
    let probe = boot(move || keyed_list(rows).scroll_to_row(jump));
    probe.clear_log();

    jump.set(Some(RowTarget::new(30).align(RowAlign::Start).instant()));
    flush_sync();
    day_core::pump_events();
    assert_eq!(jump.get_untracked(), None, "the command is consumed");
    assert_eq!(
        list_patches(&probe),
        ["scroll-to-row 30 align=Start animated=false"]
    );

    // The same row again re-fires, as a plain animated reveal.
    probe.clear_log();
    jump.set(Some(RowTarget::new(30)));
    flush_sync();
    day_core::pump_events();
    assert_eq!(
        list_patches(&probe),
        ["scroll-to-row 30 align=Nearest animated=true"]
    );

    // Past the end: clamped to the last row.
    probe.clear_log();
    jump.set(Some(RowTarget::new(900).align(RowAlign::End)));
    flush_sync();
    day_core::pump_events();
    assert_eq!(
        list_patches(&probe),
        ["scroll-to-row 49 align=End animated=true"]
    );
}

// --- scroll views ------------------------------------------------------------------------

fn tall_page() -> impl Piece {
    column(
        (0..30)
            .map(|i| label(format!("line {i}")).height(40.0))
            .collect::<Vec<_>>(),
    )
    .spacing(0.0)
}

#[test]
fn scroll_props_carry_every_preference() {
    let probe = boot(|| {
        scroll(tall_page())
            .horizontal()
            .hide_indicators()
            .no_bounce()
            .snap_interval(120.0)
            .keyboard_dismiss(KeyboardDismiss::OnDrag)
            .keyboard_taps(KeyboardTaps::Dismiss)
            .zoom(1.0..=4.0)
    });
    let (host, _) = scroll_host(&probe);
    let props = probe
        .widget(host)
        .scroll
        .expect("a scroll records its props");
    assert!(props.horizontal);
    assert_eq!(props.indicators, day_spec::props::ScrollIndicators::Hidden);
    assert_eq!(props.bounce, Some(false));
    assert_eq!(props.snap, day_spec::props::ScrollSnap::Interval(120.0));
    assert_eq!(props.keyboard_dismiss, KeyboardDismiss::OnDrag);
    assert_eq!(props.keyboard_taps, KeyboardTaps::Dismiss);
    assert_eq!(
        props.zoom,
        Some(day_spec::props::ZoomRange { min: 1.0, max: 4.0 })
    );
    // Nothing observes the view: it installs no scroll observer.
    assert!(!props.track_offset && !props.track_phase);

    // A plain scroll is the platform's own in every respect.
    let probe = boot(|| scroll(tall_page()));
    let (host, _) = scroll_host(&probe);
    assert_eq!(
        probe.widget(host).scroll,
        Some(day_spec::props::ScrollProps::default())
    );
}

#[test]
fn scroll_offset_follows_programmatic_and_user_scrolling() {
    let offset = Signal::new(Point::ZERO);
    let jump: Signal<Option<ScrollTarget>> = Signal::new(None);
    let probe = boot(move || {
        scroll(tall_page())
            .scroll_offset(offset)
            .scroll_target(jump)
            .frame(200.0, 400.0)
    });
    let (host, _) = scroll_host(&probe);
    assert!(probe.widget(host).scroll.is_some_and(|p| p.track_offset));

    // 30 lines of 40 points in a 400-point viewport: the bottom is at 800.
    jump.set(Some(ScrollTarget::Bottom));
    flush_sync();
    day_core::pump_events();
    flush_sync();
    assert_eq!(offset.get_untracked(), Point::new(0.0, 800.0));

    probe.scroll_by_user(host, Point::new(0.0, 120.0));
    day_core::pump_events();
    flush_sync();
    assert_eq!(offset.get_untracked(), Point::new(0.0, 120.0));
}

#[test]
fn scroll_phase_follows_the_users_scrolling() {
    let phase = Signal::new(ScrollPhase::Idle);
    let seen = Signal::new(Vec::<ScrollPhase>::new());
    let probe = boot(move || {
        watch(
            move || phase.get(),
            move |p: &ScrollPhase, _| seen.update(|v| v.push(*p)),
        );
        scroll(tall_page()).scroll_phase(phase)
    });
    let (host, node) = scroll_host(&probe);
    let props = probe.widget(host).scroll.expect("props");
    // The mock reports phases itself, so the offset is not needed for them.
    assert!(props.track_phase && !props.track_offset);

    probe.emit(node, Event::ScrollPhase(ScrollPhase::Dragging));
    probe.emit(node, Event::ScrollPhase(ScrollPhase::Coasting));
    probe.emit(node, Event::ScrollPhase(ScrollPhase::Coasting));
    probe.emit(node, Event::ScrollPhase(ScrollPhase::Idle));
    day_core::pump_events();
    flush_sync();
    assert_eq!(phase.get_untracked(), ScrollPhase::Idle);
    assert_eq!(
        seen.get_untracked(),
        [
            ScrollPhase::Dragging,
            ScrollPhase::Coasting,
            ScrollPhase::Idle
        ]
    );
}

#[test]
fn scroll_phase_is_derived_from_offsets_where_the_toolkit_reports_no_phases() {
    let phase = Signal::new(ScrollPhase::Idle);
    let seen = Signal::new(Vec::<ScrollPhase>::new());
    let probe = boot_with(
        |probe| probe.set_scroll_phases(Support::Emulated),
        move || {
            watch(
                move || phase.get(),
                move |p: &ScrollPhase, _| seen.update(|v| v.push(*p)),
            );
            scroll(tall_page()).scroll_phase(phase)
        },
    );
    let (host, node) = scroll_host(&probe);
    // The offset reports are what the phase comes from, so the view is asked for them.
    let props = probe.widget(host).scroll.expect("props");
    assert!(props.track_offset);

    // Movement starts the scrolling and a quiet moment ends it. The mock's timers fire at
    // once, so both writes land in one turn and the watch sees the phase come back to rest.
    probe.emit(node, Event::ScrollChanged(Point::new(0.0, 12.0)));
    day_core::pump_events();
    flush_sync();
    assert_eq!(seen.get_untracked(), [ScrollPhase::Idle]);
    // A phase event from a toolkit that says it has none is not trusted.
    probe.emit(node, Event::ScrollPhase(ScrollPhase::Coasting));
    day_core::pump_events();
    flush_sync();
    assert_eq!(phase.get_untracked(), ScrollPhase::Idle);
}

#[test]
fn keyboard_dismiss_on_drag_resigns_focus_where_the_toolkit_leaves_it_to_day() {
    let text = Signal::new(String::new());
    let probe = boot_with(
        |probe| probe.set_scroll_keyboard_dismiss(Support::Emulated),
        move || {
            scroll(column((
                text_field(text).id(day_core::id!(field)),
                tall_page(),
            )))
            .keyboard_dismiss(KeyboardDismiss::OnDrag)
        },
    );
    let (host, _) = scroll_host(&probe);
    // The dismissal rides the phase reports, which the view was therefore asked for.
    assert!(probe.widget(host).scroll.is_some_and(|p| p.track_phase));
    let (field, _) = probe.find_by_kind("day.text_field").remove(0);
    let field_node = day_core::with_tree(|t| t.find_by_id("field")).expect("the field");
    day_core::with_tree(|t| t.focus_node(field_node, true));
    day_core::pump_events();
    flush_sync();
    assert!(probe.widget(field).focused);

    probe.scroll_by_user(host, Point::new(0.0, 60.0));
    day_core::pump_events();
    flush_sync();
    assert!(
        !probe.widget(field).focused,
        "a drag dismisses the keyboard"
    );
}

#[test]
fn zoom_scale_is_two_way_and_clamped() {
    let scale = Signal::new(1.0_f64);
    let probe = boot(move || scroll(tall_page()).zoom(1.0..=4.0).zoom_scale(scale));
    let (host, _) = scroll_host(&probe);

    // The user pinches: the binding follows, and nothing is sent back to the view.
    probe.clear_log();
    probe.scroll_zoom_by_user(host, 2.5);
    day_core::pump_events();
    flush_sync();
    assert!((scale.get_untracked() - 2.5).abs() < 1e-9);
    assert!(!probe.mutations().iter().any(|m| m.contains("scroll zoom")));

    // The app writes a scale: the view zooms there, animated, clamped to its range.
    scale.set(9.0);
    flush_sync();
    day_core::pump_events();
    flush_sync();
    assert!(
        probe
            .mutations()
            .iter()
            .any(|m| m.ends_with("scroll zoom 4 animated=true")),
        "{:?}",
        probe.mutations()
    );
    assert!((probe.widget(host).zoom - 4.0).abs() < 1e-9);
    assert!((scale.get_untracked() - 4.0).abs() < 1e-9);
}

#[test]
fn content_padding_scrolls_with_the_content() {
    let probe = boot(|| {
        scroll(tall_page().id(day_core::id!(page)))
            .content_padding(Insets {
                top: 10.0,
                leading: 20.0,
                bottom: 30.0,
                trailing: 20.0,
            })
            .frame(200.0, 400.0)
    });
    let (host, _) = scroll_host(&probe);
    // 1200 points of lines plus 40 of padding; the content keeps the viewport's width.
    assert_eq!(probe.widget(host).scroll_content, Size::new(200.0, 1240.0));
    let page = day_core::with_tree(|t| t.find_by_id("page")).expect("the page");
    let frame = day_core::with_tree(|t| t.node_frame(page)).expect("laid out");
    assert_eq!((frame.origin.x, frame.origin.y), (20.0, 10.0));
    assert!((frame.size.width - 160.0).abs() < 0.01);
}

// --- pager -------------------------------------------------------------------------------

#[test]
fn pager_progress_steps_to_the_selected_page_and_follows_native_reports() {
    use day_pieces::{PagerState, pager};
    let state = Signal::new(PagerState {
        order: vec![0, 1, 2],
        selected: 0,
        epoch: 1,
    });
    let busy = Signal::new(false);
    let progress = Signal::new(-1.0_f64);
    let probe = boot(move || {
        pager(
            state,
            busy,
            vec![label("A").any(), label("B").any(), label("C").any()],
            move |slot| state.update(|s| s.selected = slot),
        )
        .progress(progress)
        .id(day_core::id!(pager))
    });
    // At rest the progress is the selected page's place in reading order.
    assert!((progress.get_untracked() - 0.0).abs() < 1e-9);
    state.update(|s| s.selected = 2);
    flush_sync();
    assert!((progress.get_untracked() - 2.0).abs() < 1e-9);
    // Reversed, the same slot is the first page.
    state.update(|s| {
        s.order.reverse();
        s.epoch += 1;
    });
    flush_sync();
    assert!((progress.get_untracked() - 0.0).abs() < 1e-9);

    // A native pager's continuous report, for the current epoch only.
    let node = day_core::with_tree(|t| t.find_by_id("pager")).expect("the pager");
    let node = day_core::rnode_to_id(node);
    probe.emit(node, Event::custom("", "progress 2 0.4"));
    day_core::pump_events();
    flush_sync();
    assert!((progress.get_untracked() - 0.4).abs() < 1e-9);
    probe.emit(node, Event::custom("", "progress 1 0.9"));
    day_core::pump_events();
    flush_sync();
    assert!(
        (progress.get_untracked() - 0.4).abs() < 1e-9,
        "a stale epoch is ignored"
    );
}

// --- layout ------------------------------------------------------------------------------

fn frame_of(id: &str) -> day_spec::Rect {
    let node = day_core::with_tree(|t| t.find_by_id(id)).unwrap_or_else(|| panic!("no node {id}"));
    day_core::with_tree(|t| t.node_frame(node)).unwrap_or_else(|| panic!("{id} not laid out"))
}

#[test]
fn growing_siblings_share_spare_space_by_weight() {
    boot(|| {
        row((
            rectangle()
                .height(10.0)
                .grow(Dimension::Width)
                .wrapper_id(day_core::id!(one)),
            rectangle()
                .height(10.0)
                .grow(Dimension::Width.weight(3.0))
                .wrapper_id(day_core::id!(three)),
        ))
        .spacing(0.0)
        .frame(400.0, 10.0)
    });
    assert!((frame_of("one").size.width - 100.0).abs() < 0.01);
    assert!((frame_of("three").size.width - 300.0).abs() < 0.01);
}

#[test]
fn a_wrapping_row_shares_each_line_among_its_growing_children() {
    boot(|| {
        let tile = |id: day_core::ElementId| {
            rectangle()
                .height(20.0)
                .min_width(50.0)
                .grow(Dimension::Width)
                .wrapper_id(id)
        };
        row((
            tile(day_core::id!(t0)),
            tile(day_core::id!(t1)),
            tile(day_core::id!(t2)),
            tile(day_core::id!(t3)),
            tile(day_core::id!(t4)),
        ))
        .spacing(10.0)
        .fit(RowFit::Wrap { run_spacing: 10.0 })
        .frame(230.0, 100.0)
    });
    // 230 points hold four 50-point tiles and their gaps (230), so the fifth wraps. The
    // first line has nothing spare; the lone tile on the second takes the whole line.
    for id in ["t0", "t1", "t2", "t3"] {
        assert!((frame_of(id).size.width - 50.0).abs() < 0.01, "{id}");
    }
    assert!((frame_of("t3").origin.x - 180.0).abs() < 0.01);
    let last = frame_of("t4");
    assert!((last.origin.y - 30.0).abs() < 0.01);
    assert!((last.size.width - 230.0).abs() < 0.01);
}

#[test]
fn a_wrapping_row_keeps_its_fixed_children_and_weights_the_rest() {
    boot(|| {
        row((
            rectangle().frame(40.0, 20.0).id(day_core::id!(fixed)),
            rectangle()
                .height(20.0)
                .min_width(30.0)
                .grow(Dimension::Width)
                .wrapper_id(day_core::id!(a)),
            rectangle()
                .height(20.0)
                .min_width(30.0)
                .grow(Dimension::Width.weight(2.0))
                .wrapper_id(day_core::id!(b)),
        ))
        .spacing(0.0)
        .fit(RowFit::Wrap { run_spacing: 0.0 })
        .frame(400.0, 20.0)
    });
    // 400 - 40 - 30 - 30 = 300 spare, shared one to two.
    assert!((frame_of("fixed").size.width - 40.0).abs() < 0.01);
    assert!((frame_of("a").size.width - 130.0).abs() < 0.01);
    assert!((frame_of("b").size.width - 230.0).abs() < 0.01);
}

#[test]
fn lanes_reassign_the_same_children_when_the_width_changes() {
    let built = Signal::new(0_u32);
    let width = Signal::new(700.0_f64);
    let probe = boot(move || {
        let card = move |id: day_core::ElementId, height: f64| {
            built.update(|n| *n += 1);
            rectangle()
                .height(height)
                .grow(Dimension::Width)
                .wrapper_id(id)
        };
        column((lanes((
            card(day_core::id!(c0), 100.0),
            card(day_core::id!(c1), 40.0),
            card(day_core::id!(c2), 60.0),
            card(day_core::id!(c3), 30.0),
        ))
        .lane_width(200.0)
        .max_lanes(3)
        .spacing(10.0)
        .id(day_core::id!(lanes)),))
        .align(HAlign::Leading)
        .width(move || width.get())
    });
    // 700 points hold three 200-point lanes: each 226.67 wide, dealt in turn.
    let lane = (700.0 - 20.0) / 3.0;
    assert!((frame_of("c0").size.width - lane).abs() < 0.01);
    assert!((frame_of("c1").origin.x - (lane + 10.0)).abs() < 0.01);
    assert!((frame_of("c2").origin.x - 2.0 * (lane + 10.0)).abs() < 0.01);
    // The fourth card starts the second round, under the first.
    let fourth = frame_of("c3");
    assert!((fourth.origin.x).abs() < 0.01);
    assert!((fourth.origin.y - 110.0).abs() < 0.01);
    assert!((frame_of("lanes").size.height - 140.0).abs() < 0.01);

    // Narrower: two lanes, the same four cards, none rebuilt.
    let widgets = probe.find_by_kind("day.shape").len();
    width.set(430.0);
    flush_sync();
    day_core::pump_events();
    flush_sync();
    assert_eq!(built.get_untracked(), 4);
    assert_eq!(probe.find_by_kind("day.shape").len(), widgets);
    assert!((frame_of("c0").size.width - 210.0).abs() < 0.01);
    assert!((frame_of("c1").origin.x - 220.0).abs() < 0.01);
    let third = frame_of("c2");
    assert!((third.origin.x).abs() < 0.01 && (third.origin.y - 110.0).abs() < 0.01);
    assert!((frame_of("c3").origin.y - 50.0).abs() < 0.01);

    // One lane on a phone.
    width.set(300.0);
    flush_sync();
    day_core::pump_events();
    flush_sync();
    assert!((frame_of("c3").origin.y - 230.0).abs() < 0.01);
    assert_eq!(built.get_untracked(), 4);
}

#[test]
fn balanced_lanes_put_each_child_in_the_shortest_lane() {
    boot(|| {
        let card = |id: day_core::ElementId, height: f64| {
            rectangle()
                .height(height)
                .grow(Dimension::Width)
                .wrapper_id(id)
        };
        lanes((
            card(day_core::id!(b0), 100.0),
            card(day_core::id!(b1), 20.0),
            card(day_core::id!(b2), 20.0),
            card(day_core::id!(b3), 20.0),
        ))
        .lane_width(200.0)
        .max_lanes(2)
        .spacing(0.0)
        .balanced()
        .frame(400.0, 200.0)
    });
    // The tall card holds the first lane; the three short ones stack in the second.
    assert!((frame_of("b1").origin.x - 200.0).abs() < 0.01);
    assert!((frame_of("b2").origin.x - 200.0).abs() < 0.01);
    assert!((frame_of("b2").origin.y - 20.0).abs() < 0.01);
    assert!((frame_of("b3").origin.y - 40.0).abs() < 0.01);
}

#[test]
fn scroll_to_row_takes_a_binding_of_plain_indices() {
    let rows = Signal::new(rows_of(&(0..50).collect::<Vec<_>>()));
    let jump = Signal::new(None);
    let probe = boot(move || keyed_list(rows).scroll_to_row(jump));
    probe.clear_log();
    jump.set(Some(12_usize));
    flush_sync();
    day_core::pump_events();
    assert_eq!(jump.get_untracked(), None);
    assert_eq!(
        list_patches(&probe),
        ["scroll-to-row 12 align=Nearest animated=true"]
    );
}
