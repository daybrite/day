// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Typed queries against a live container: the §15 cost tiers through the real change sink,
//! tracked reads, the reactive-fetch form, raw SQL, and the connection escape hatch.

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

fn seeded(n: u32) -> ModelContainer {
    let container = ModelContainer::open(Sqlite::memory(), schema![Trip]).expect("open");
    let store = container.store::<Trip>();
    // A wholesale update: the fold resyncs the whole table, which is the honest way to seed
    // many rows at once (a single restructure op names ONE key, not three hundred).
    store.update("seed", move |k| {
        *k = day_model::Keyed::new(
            (1..=n)
                .map(|i| Trip {
                    id: i,
                    name: format!("trip {i:03}"),
                    start_day: i as i64,
                    done: i % 4 == 0,
                    notes: String::new(),
                    rating: None,
                })
                .collect(),
        );
    });
    container.save().expect("seed flush");
    container
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
    assert_eq!(q.first(), Some(10));
}

#[test]
fn a_column_the_query_never_mentions_wakes_nothing() {
    let c = seeded(50);
    let store = c.store::<Trip>();
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
    let evals = q.evaluations();

    for i in 1..=50u64 {
        store.elem(i).notes().write(format!("note {i}"));
    }
    day_reactive::flush_sync();

    assert_eq!(runs.get(), 1, "fifty notes edits woke the query zero times");
    assert_eq!(q.evaluations(), evals, "and evaluated zero predicates");

    store.elem(1).done().write(true);
    day_reactive::flush_sync();
    assert_eq!(runs.get(), 2, "a predicate column wakes it exactly once");
    assert!(!q.ids_untracked().contains(&1));
}

#[test]
fn deltas_feed_through_take_events() {
    let c = seeded(6);
    let store = c.store::<Trip>();
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
    day_reactive::flush_sync();
    assert_eq!(q.ids(), [1, 2, 3, 4, 5, 6, 7, 8, 9], "trip 001..009 match");

    term.write("TRIP 003".into());
    day_reactive::flush_sync();
    assert_eq!(q.ids(), [3], "case-insensitive");

    term.write(String::new());
    day_reactive::flush_sync();
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
    day_reactive::flush_sync();
    let filtered = q.ids();
    assert_eq!(filtered, [1, 2, 3, 5, 6, 7, 9, 10, 11]);
    assert!(
        filtered.iter().all(|id| before.contains(id)),
        "same rows, fewer of them"
    );

    all.write(true);
    day_reactive::flush_sync();
    assert_eq!(q.ids(), before, "flip back restores the exact set");
}

#[test]
fn option_columns_compare_null_correctly() {
    let c = seeded(4);
    let store = c.store::<Trip>();
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
    let store = c.store::<Trip>();
    let q = c
        .query::<Trip>()
        .filter(Trip::done().eq(false))
        .sort(Trip::start_day().asc())
        .limit(3)
        .live();
    assert_eq!(q.ids(), [1, 2, 3]);

    // Deleting inside the window pulls the next row in — the requery tier, still correct.
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
fn raw_queries_rerun_when_their_table_flushes() {
    let c = seeded(8);
    let store = c.store::<Trip>();
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
    // Not yet: raw queries wait for the COMMIT (the flush), by design.
    assert_eq!(q.ids_untracked(), [2, 4, 6, 8]);
    c.save().expect("flush");
    assert_eq!(q.ids_untracked(), [2, 4, 6, 8, 12]);
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
    assert_eq!(c.store::<Trip>().elem(99).name().peek(), "smuggled");
    // And the rescan's reload did not mark everything dirty for a pointless write-back.
    let sql = c.record_sql(|| {}).expect("empty flush");
    assert_eq!(sql, Vec::<String>::new());
}

#[test]
fn queries_work_against_the_recorder_too() {
    let (driver, _log) = Recorder::new();
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
    assert_eq!(
        q.ids(),
        [1],
        "in-memory queries never need the database at all"
    );
}

#[test]
fn dropping_the_query_unregisters_it() {
    let c = seeded(5);
    let store = c.store::<Trip>();
    {
        let _q = c.query::<Trip>().filter(Trip::done().eq(false)).live();
    }
    // The dead weak is pruned on the next dispatch; nothing panics, nothing leaks work.
    store.elem(1).done().write(true);
    store.elem(1).done().write(false);
}
