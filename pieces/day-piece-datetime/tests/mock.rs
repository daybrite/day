//! Mock e2e for the date/time pickers (the day-pieces mock_e2e pattern): both pieces realize
//! their kind, native picks (`Event::Custom` — ISO text in-process, epoch numbers across a
//! JNI/C-ABI boundary) and dayscript's `input:` step (`Event::TextChanged`) drive the bound
//! signal, app writes patch through to the native control, and out-of-range picks clamp.

use std::cell::Cell;
use std::rc::Rc;

use day_core::AnyPiece;
use day_mock::{MockProbe, MockToolkit};
use day_piece_datetime::{DATE_KIND, DayDate, DayTime, TIME_KIND, date_picker, time_picker};
use day_pieces::prelude::*;
use day_reactive::{Signal, flush_sync};
use day_spec::{Event, NodeId, Size, WindowOptions};

fn boot(root: impl FnOnce() -> AnyPiece + 'static) -> MockProbe {
    day_core::uninstall_tree();
    let (mock, probe) = MockToolkit::new();
    let options = WindowOptions {
        title: "test".into(),
        size: Size::new(400.0, 600.0),
        ..Default::default()
    };
    day_core::launch_with(mock, options, root);
    probe
}

fn picker_node(probe: &MockProbe, kind: &str) -> NodeId {
    let found = probe.find_by_kind(kind);
    assert_eq!(found.len(), 1, "exactly one {kind} realized");
    NodeId(found[0].1.node)
}

fn d(y: i32, m: u8, day: u8) -> DayDate {
    DayDate::new(y, m, day).unwrap()
}

#[test]
fn date_events_drive_signal() {
    let cell: Rc<Cell<Option<Signal<DayDate>>>> = Rc::new(Cell::new(None));
    let cell2 = cell.clone();
    let probe = boot(move || {
        let date = Signal::new(d(2026, 7, 18));
        cell2.set(Some(date));
        date_picker(date).any()
    });
    let date = cell.get().unwrap();
    let node = picker_node(&probe, DATE_KIND);

    // A native pick crossing a JNI/C-ABI boundary: empty tag/text, epoch days in `num`.
    probe.emit(
        node,
        Event::Custom {
            tag: "",
            num: d(2027, 1, 2).to_epoch_days() as f64,
            text: String::new(),
        },
    );
    flush_sync();
    assert_eq!(date.get_untracked(), d(2027, 1, 2));

    // An in-process native pick: ISO text.
    probe.emit(node, Event::custom("datepicker:value", "2025-12-31"));
    flush_sync();
    assert_eq!(date.get_untracked(), d(2025, 12, 31));

    // dayscript's `input:` step → TextChanged with ISO text.
    probe.emit(node, Event::TextChanged("2026-02-14".into()));
    flush_sync();
    assert_eq!(date.get_untracked(), d(2026, 2, 14));

    // Garbage is ignored.
    probe.emit(node, Event::TextChanged("not-a-date".into()));
    flush_sync();
    assert_eq!(date.get_untracked(), d(2026, 2, 14));
}

#[test]
fn app_writes_patch_native() {
    let cell: Rc<Cell<Option<Signal<DayDate>>>> = Rc::new(Cell::new(None));
    let cell2 = cell.clone();
    let probe = boot(move || {
        let date = Signal::new(d(2026, 7, 18));
        cell2.set(Some(date));
        date_picker(date).any()
    });
    let date = cell.get().unwrap();
    let _ = picker_node(&probe, DATE_KIND);

    let mark = probe.log_len();
    date.set(d(2030, 1, 1)); // app-initiated (e.g. a "jump to date" button)
    flush_sync();
    assert!(
        probe
            .log_since(mark)
            .iter()
            .any(|l| l.starts_with("update day.piece.datepicker")),
        "signal write patches the native control"
    );
}

#[test]
fn date_picks_clamp_to_range() {
    let cell: Rc<Cell<Option<Signal<DayDate>>>> = Rc::new(Cell::new(None));
    let cell2 = cell.clone();
    let probe = boot(move || {
        let date = Signal::new(d(2026, 6, 15));
        cell2.set(Some(date));
        date_picker(date)
            .min(d(2026, 1, 1))
            .max(d(2026, 12, 31))
            .any()
    });
    let date = cell.get().unwrap();
    let node = picker_node(&probe, DATE_KIND);

    probe.emit(node, Event::TextChanged("2027-06-15".into()));
    flush_sync();
    assert_eq!(date.get_untracked(), d(2026, 12, 31), "clamped to max");

    probe.emit(node, Event::TextChanged("2020-01-01".into()));
    flush_sync();
    assert_eq!(date.get_untracked(), d(2026, 1, 1), "clamped to min");
}

#[test]
fn out_of_range_seed_clamps_signal() {
    let cell: Rc<Cell<Option<Signal<DayDate>>>> = Rc::new(Cell::new(None));
    let cell2 = cell.clone();
    let _probe = boot(move || {
        let date = Signal::new(d(2020, 1, 1)); // before min
        cell2.set(Some(date));
        date_picker(date).min(d(2026, 1, 1)).any()
    });
    flush_sync();
    let date = cell.get().unwrap();
    assert_eq!(
        date.get_untracked(),
        d(2026, 1, 1),
        "build clamps an out-of-range initial value and reflects it into the signal"
    );
}

#[test]
fn time_events_drive_signal() {
    let cell: Rc<Cell<Option<Signal<DayTime>>>> = Rc::new(Cell::new(None));
    let cell2 = cell.clone();
    let probe = boot(move || {
        let time = Signal::new(DayTime::new(9, 30, 0).unwrap());
        cell2.set(Some(time));
        time_picker(time).any()
    });
    let time = cell.get().unwrap();
    let node = picker_node(&probe, TIME_KIND);

    // Across a native boundary: seconds-of-day in `num`.
    probe.emit(
        node,
        Event::Custom {
            tag: "",
            num: (14 * 3600 + 45 * 60) as f64,
            text: String::new(),
        },
    );
    flush_sync();
    assert_eq!(time.get_untracked(), DayTime::new(14, 45, 0).unwrap());

    // dayscript / in-process: ISO text (with seconds).
    probe.emit(node, Event::TextChanged("23:59:59".into()));
    flush_sync();
    assert_eq!(time.get_untracked(), DayTime::new(23, 59, 59).unwrap());

    probe.emit(node, Event::TextChanged("25:00".into()));
    flush_sync();
    assert_eq!(
        time.get_untracked(),
        DayTime::new(23, 59, 59).unwrap(),
        "invalid time ignored"
    );
}

#[test]
fn readout_label_follows_picks() {
    // The showcase pattern: an ISO readout label bound to the signal (what the walkthrough
    // asserts cross-platform).
    let probe = boot(|| {
        let date = Signal::new(d(2026, 7, 18));
        column((
            date_picker(date),
            label(move || date.get().to_string()).id("date-value"),
        ))
        .any()
    });
    let node = picker_node(&probe, DATE_KIND);
    probe.emit(node, Event::TextChanged("2026-11-05".into()));
    flush_sync();
    let labels = probe.find_by_kind("day.label");
    assert!(
        labels.iter().any(|(_, w)| w.text == "2026-11-05"),
        "readout shows the picked ISO date; labels = {:?}",
        labels
            .iter()
            .map(|(_, w)| w.text.clone())
            .collect::<Vec<_>>()
    );
}
