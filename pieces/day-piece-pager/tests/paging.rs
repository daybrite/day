//! Synthetic native events verify the shared commit/lifetime contract, not touch arbitration.
use day_mock::MockToolkit;
use day_piece_pager::{PagerState, pager};
use day_pieces::prelude::*;
use day_reactive::{Signal, flush_sync};
use day_spec::{Event, WindowOptions};

#[test]
fn settlement_freezes_identity_and_rejects_stale_callbacks_without_rebuilding_pages() {
    day_core::uninstall_tree();
    let (mock, probe) = MockToolkit::new();
    let state = Signal::new(PagerState {
        order: vec![0, 1],
        selected: 0,
        epoch: 1,
    });
    let busy = Signal::new(false);
    let commits = Signal::new(Vec::new());
    let mounted = Signal::new(true);
    day_core::launch_with(mock, WindowOptions::default(), move || {
        when(
            move || mounted.get(),
            move || {
                pager(
                    state,
                    busy,
                    vec![
                        label("fixture first").id(day_core::id!(first_page)).any(),
                        label("fixture second").id(day_core::id!(second_page)).any(),
                    ],
                    move |slot| {
                        let was_busy = busy.get_untracked();
                        commits.update(|v| v.push((slot, was_busy)));
                    },
                )
                .id(day_core::id!(fixture_pager))
            },
        )
        .any()
    });
    let pager = day_core::rnode_to_id(day_core::with_tree(|t| {
        t.find_by_id("fixture-pager").unwrap()
    }));
    let first = day_core::with_tree(|t| t.find_by_id("first-page").unwrap());
    probe.emit(pager, Event::custom("pager", "begin 1"));
    assert!(busy.get_untracked());
    probe.emit(pager, Event::SelectionChanged(1));
    assert!(
        commits.get_untracked().is_empty(),
        "programmatic selection cannot interrupt a drag"
    );
    probe.emit(pager, Event::custom("pager", "settle 1 0"));
    assert_eq!(
        commits.get_untracked(),
        vec![(0, true)],
        "cancelled gesture settles on its original page"
    );
    assert!(!busy.get_untracked());
    probe.emit(pager, Event::custom("pager", "begin 1"));
    probe.emit(pager, Event::custom("pager", "settle 1 1"));
    assert_eq!(commits.get_untracked(), vec![(0, true), (1, true)]);
    state.set(PagerState {
        order: vec![1, 0],
        selected: 1,
        epoch: 2,
    });
    flush_sync();
    assert_eq!(
        day_core::with_tree(|t| t.find_by_id("first-page")),
        Some(first)
    );
    probe.emit(pager, Event::custom("pager", "begin 1"));
    probe.emit(pager, Event::custom("pager", "settle 1 0"));
    assert!(!busy.get_untracked());
    assert_eq!(commits.get_untracked().len(), 2);
    probe.emit(pager, Event::SelectionChanged(0));
    assert_eq!(commits.get_untracked().last(), Some(&(1, false)));
    mounted.set(false);
    flush_sync();
    probe.emit(pager, Event::custom("pager", "settle 2 0"));
    assert_eq!(commits.get_untracked().len(), 3);
    day_core::uninstall_tree();
}
