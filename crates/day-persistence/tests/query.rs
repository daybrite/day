// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Typed queries against a live container: the cost tiers through the real change sink,
//! tracked reads, the reactive-fetch form, raw SQL, and the connection escape hatch. The
//! engine answers every fetch; these tests watch the SQL trace to prove which changes cost a
//! requery and which cost nothing.

use std::cell::Cell;
use std::rc::Rc;

use day_macros::Model;
use day_model::Op;
use day_persistence::{Fetch, ModelContainer, QueryEvents, Recorder, Sort, Sqlite, Value, schema};
use day_reactive::{Binding, Signal};

#[derive(Model, Clone, Default, PartialEq, Debug)]
#[model(table = "trips")]
struct Trip {
    #[model(id)]
    id: u32,
    name: String,
    start_day: i64,
    done: bool,
    notes: String,
    rating: Option<f64>,
}

fn trips(n: u32) -> Vec<Trip> {
    (1..=n)
        .map(|i| Trip {
            id: i,
            name: format!("trip {i:03}"),
            start_day: i as i64,
            done: i % 4 == 0,
            notes: String::new(),
            rating: None,
        })
        .collect()
}

fn seed(container: &ModelContainer, rows: Vec<Trip>) {
    let store = container.cache::<Trip>();
    store.update("seed", move |k| {
        *k = day_model::Keyed::new(rows);
    });
    container.save().expect("seed flush");
}

fn seeded(n: u32) -> ModelContainer {
    let container = ModelContainer::open(Sqlite::memory(), schema![Trip]).expect("open");
    seed(&container, trips(n));
    container
}

/// A drain plus the turn-end requeries it triggers: staleness resolves at turn end, and the
/// version bumps there schedule one more drain.
fn settle() {
    day_reactive::flush_sync();
    day_reactive::flush_sync();
}

#[test]
fn the_builder_filters_sorts_and_limits() {
    let c = seeded(10);
    let q = c
        .query::<Trip>()
        .filter(Trip::done().eq(false))
        .sort(Trip::start_day().desc())
        .limit(3)
        .live();
    assert_eq!(
        q.ids(),
        [10, 9, 7],
        "not-done, start descending, first three"
    );
    assert_eq!(q.count(), 3);
    assert_eq!(q.first().map(|i| i.handle()), Some(10));
}

#[test]
fn a_column_the_query_never_mentions_costs_nothing() {
    let trace: Rc<std::cell::RefCell<Vec<String>>> = Rc::new(std::cell::RefCell::new(Vec::new()));
    let sink = trace.clone();
    let driver = Sqlite::memory().trace_sql(move |sql| sink.borrow_mut().push(sql.to_string()));
    let c = ModelContainer::open(driver, schema![Trip]).expect("open");
    seed(&c, trips(50));
    let store = c.cache::<Trip>();
    let q = c
        .query::<Trip>()
        .filter(Trip::done().eq(false))
        .sort(Trip::start_day().asc())
        .live();

    let runs = Rc::new(Cell::new(0));
    let (q2, runs2) = (q.clone(), runs.clone());
    day_reactive::Effect::new(move || {
        let _ = q2.ids();
        runs2.set(runs2.get() + 1);
    });
    assert_eq!(runs.get(), 1);

    let requeries = |t: &[String]| {
        t.iter()
            .filter(|s| s.starts_with("SELECT trips.id FROM trips"))
            .count()
    };
    let baseline = requeries(&trace.borrow());

    for i in 1..=50u64 {
        store.elem(i).notes().write(format!("note {i}"));
    }
    settle();
    assert_eq!(runs.get(), 1, "fifty notes edits woke the query zero times");
    assert_eq!(
        requeries(&trace.borrow()),
        baseline,
        "and re-ran zero SQL: the dependency gate held"
    );

    store.elem(1).done().write(true);
    settle();
    assert_eq!(runs.get(), 2, "a predicate column wakes it exactly once");
    assert_eq!(
        requeries(&trace.borrow()),
        baseline + 1,
        "one requery, after the flush"
    );
    assert!(!q.ids_untracked().iter().any(|i| *i == 1));
}

#[test]
fn deltas_feed_through_take_events() {
    let c = seeded(6);
    let store = c.cache::<Trip>();
    let q = c
        .query::<Trip>()
        .filter(Trip::done().eq(false))
        .sort(Trip::start_day().asc())
        .live();
    let _ = q.take_events();

    store.elem(2).done().write(true); // leaves the set
    let events = q.take_events();
    assert!(
        matches!(events, QueryEvents::Deltas(ref d) if d.len() == 1),
        "{events:?}"
    );

    store.elem(1).start_day().write(99); // moves to the end
    store.elem(3).done().write(true); // and another leaves
    let events = q.take_events();
    assert!(
        matches!(events, QueryEvents::Deltas(ref d) if d.len() == 2),
        "coalesced into one drain: {events:?}"
    );
    assert_eq!(q.take_events(), QueryEvents::None);
}

#[test]
fn query_fn_follows_its_signals() {
    let c = seeded(30);
    let term = Signal::new(String::new());
    let q = c.query_fn::<Trip>(move || {
        let t = term.get();
        let mut f = Fetch::new().sort(Sort::asc("start_day"));
        if !t.is_empty() {
            f = f.filter(Trip::name().contains_ci(t));
        }
        f
    });
    assert_eq!(q.count(), 30);

    term.write("trip 00".into());
    settle();
    assert_eq!(q.ids(), [1, 2, 3, 4, 5, 6, 7, 8, 9], "trip 001..009 match");

    term.write("TRIP 003".into());
    settle();
    assert_eq!(q.ids(), [3], "case-insensitive, folded by day_fold in SQL");

    term.write(String::new());
    settle();
    assert_eq!(q.count(), 30);
}

#[test]
fn a_filter_flip_keeps_row_identity() {
    // The walkthrough promise, headless: the ids in both states name the same rows, and the
    // set after flipping back equals the set before.
    let c = seeded(12);
    let all = Signal::new(true);
    let q = c.query_fn::<Trip>(move || {
        let mut f = Fetch::new().sort(Sort::asc("start_day"));
        if !all.get() {
            f = f.filter(Trip::done().eq(false));
        }
        f
    });
    let before = q.ids();
    assert_eq!(before.len(), 12);

    all.write(false);
    settle();
    let filtered = q.ids();
    assert_eq!(filtered, [1, 2, 3, 5, 6, 7, 9, 10, 11]);
    assert!(
        filtered.iter().all(|id| before.contains(id)),
        "same rows, fewer of them"
    );

    all.write(true);
    settle();
    assert_eq!(q.ids(), before, "flip back restores the exact set");
}

#[test]
fn option_columns_compare_null_correctly() {
    let c = seeded(4);
    let store = c.cache::<Trip>();
    store.elem(2).rating().write(Some(4.5));

    let unrated = c
        .query::<Trip>()
        .filter(Trip::rating().eq(None))
        .sort(Trip::start_day().asc())
        .live();
    assert_eq!(unrated.ids(), [1, 3, 4]);

    let rated = c.query::<Trip>().filter(Trip::rating().ne(None)).live();
    assert_eq!(rated.ids(), [2]);
}

#[test]
fn a_windowed_query_stays_correct_across_the_boundary() {
    let c = seeded(10);
    let store = c.cache::<Trip>();
    let q = c
        .query::<Trip>()
        .filter(Trip::done().eq(false))
        .sort(Trip::start_day().asc())
        .limit(3)
        .live();
    assert_eq!(q.ids(), [1, 2, 3]);

    // Deleting inside the window pulls the next row in — the engine re-answers the window.
    store.restructure("remove", Op::Delete, 2, |v| {
        v.remove(2);
    });
    assert_eq!(
        q.ids_untracked(),
        [1, 3, 5],
        "4 is done; 5 fills the window"
    );
}

#[test]
fn every_read_settles_pending_writes() {
    // Reads are current: a dependency-touching edit flushes and requeries on the next read,
    // so imperative same-turn code never sees yesterday's answer.
    let c = seeded(8);
    let store = c.cache::<Trip>();
    let q = c.query_raw::<Trip>(
        "SELECT id FROM trips WHERE id % 2 = 0 ORDER BY id",
        vec![],
        &["trips"],
    );
    assert_eq!(q.ids(), [2, 4, 6, 8]);

    store.restructure("add", Op::Insert, 12, |v| {
        v.push(Trip {
            id: 12,
            ..Default::default()
        });
    });
    assert_eq!(
        q.ids_untracked(),
        [2, 4, 6, 8, 12],
        "the read flushed the insert and re-ran the statement"
    );
}

#[test]
fn with_connection_plus_rescan_recovers() {
    let c = seeded(3);
    let q = c
        .query::<Trip>()
        .filter(Trip::done().eq(false))
        .sort(Trip::start_day().asc())
        .live();
    assert_eq!(q.ids(), [1, 2, 3]);

    // A write that bypasses the change log entirely…
    c.with_connection(|conn| {
        conn.execute(
            "INSERT INTO trips (id, name, start_day, done, notes, rating) \
             VALUES (99, 'smuggled', 0, 0, '', NULL)",
            &[],
        )
        .expect("raw insert");
    });
    assert_eq!(
        q.ids_untracked(),
        [1, 2, 3],
        "invisible until rescan — the declared price"
    );

    c.rescan().expect("rescan");
    assert_eq!(q.ids_untracked(), [99, 1, 2, 3], "start 0 sorts first");
    let row = c.get::<Trip>(99u32).expect("faults in on demand");
    assert_eq!(row.name().peek(), "smuggled");
    // And the rescan did not mark everything dirty for a pointless write-back.
    let sql = c.record_sql(|| {}).expect("empty flush");
    assert_eq!(sql, Vec::<String>::new());
}

#[test]
fn queries_work_against_the_recorder_too() {
    let (driver, log) = Recorder::new();
    let driver = driver.with_table(
        "trips",
        vec![vec![
            Value::Int(1),
            Value::Text("kyoto".into()),
            Value::Int(7),
            Value::Int(0),
            Value::Text(String::new()),
            Value::Null,
        ]],
    );
    let c = ModelContainer::open(driver, schema![Trip]).expect("open");
    let q = c
        .query::<Trip>()
        .filter(Trip::name().contains("kyo"))
        .live();
    assert_eq!(q.ids(), [1], "fixtures answer the compiled SELECT");
    // The Recorder's real value now: the compiled SQL is assertable, headlessly.
    assert!(
        log.sql()
            .iter()
            .any(|s| s.starts_with("SELECT trips.id FROM trips WHERE instr(trips.name, ?) > 0")),
        "recorded: {:?}",
        log.sql()
    );
}

#[test]
fn dropping_the_query_unregisters_it() {
    let c = seeded(5);
    let store = c.cache::<Trip>();
    {
        let _q = c.query::<Trip>().filter(Trip::done().eq(false)).live();
    }
    // The dead weak is pruned on the next dispatch; nothing panics, nothing leaks work.
    store.elem(1).done().write(true);
    store.elem(1).done().write(false);
}

// --- count-only queries ---------------------------------------------------------------------

#[test]
fn a_count_query_stays_live_without_holding_ids() {
    let trace: Rc<std::cell::RefCell<Vec<String>>> = Rc::new(std::cell::RefCell::new(Vec::new()));
    let sink = trace.clone();
    let driver = Sqlite::memory().trace_sql(move |sql| sink.borrow_mut().push(sql.to_string()));
    let c = ModelContainer::open(driver, schema![Trip]).expect("open");
    seed(&c, trips(50));
    let store = c.cache::<Trip>();

    let unread = c
        .query::<Trip>()
        .filter(Trip::done().eq(false))
        .live_count();
    assert_eq!(unread.get(), 38, "50 minus the twelve done");
    assert!(
        trace
            .borrow()
            .iter()
            .any(|s| s.starts_with("SELECT COUNT(*) FROM trips")),
        "the badge form is one COUNT: {:?}",
        trace.borrow()
    );

    let counts = |t: &[String]| {
        t.iter()
            .filter(|s| s.starts_with("SELECT COUNT(*) FROM trips"))
            .count()
    };
    let baseline = counts(&trace.borrow());

    // The dependency gate holds for counts too.
    for i in 1..=50u64 {
        store.elem(i).notes().write(format!("note {i}"));
    }
    assert_eq!(unread.get(), 38);
    assert_eq!(
        counts(&trace.borrow()),
        baseline,
        "notes edits re-count nothing"
    );

    store.elem(1).done().write(true);
    assert_eq!(unread.get(), 37, "a predicate write re-counts once");
    assert_eq!(counts(&trace.borrow()), baseline + 1);
}

#[test]
fn a_count_query_wakes_its_readers_exactly_when_the_count_moves() {
    let c = seeded(10);
    let store = c.cache::<Trip>();
    let q = c.query::<Trip>().filter(Trip::done().eq(true)).live_count();

    let runs = Rc::new(Cell::new(0));
    let seen = Rc::new(Cell::new(0usize));
    let (q2, runs2, seen2) = (q.clone(), runs.clone(), seen.clone());
    day_reactive::Effect::new(move || {
        seen2.set(q2.get());
        runs2.set(runs2.get() + 1);
    });
    assert_eq!((runs.get(), seen.get()), (1, 2), "4 and 8 are done");

    store.elem(1).done().write(true);
    settle();
    assert_eq!((runs.get(), seen.get()), (2, 3));

    // A write that leaves the count unchanged wakes nothing: 1 was already done.
    store.elem(1).done().write(true);
    settle();
    assert_eq!(runs.get(), 2, "same count, no wake");
}

#[test]
fn a_count_respects_the_window_and_the_reactive_fetch_form() {
    let c = seeded(30);
    let capped = c
        .query::<Trip>()
        .filter(Trip::done().eq(false))
        .limit(10)
        .live_count();
    assert_eq!(capped.get(), 10, "a limited set counts its window");

    let floor = Signal::new(20i64);
    let q = c.count_fn::<Trip>(move || Fetch::new().filter(Trip::start_day().gt(floor.get())));
    assert_eq!(q.get(), 10, "days 21..=30");
    floor.write(25);
    settle();
    assert_eq!(q.get(), 5, "the fetch re-derived from its signal");
}
