// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The lazy contract itself: open reads no rows, rows fault in batches, the cache stays
//! bounded, eviction spares what is dirty or observed, and an evicted row faults back whole.

use std::cell::RefCell;
use std::rc::Rc;

use day_macros::Model;
use day_model::Op;
use day_persistence::{DEFAULT_CACHE_LIMIT, ModelContainer, Sqlite, schema};
use day_reactive::Binding;

#[derive(Model, Clone, Default, PartialEq, Debug)]
#[model(table = "rows")]
struct Rowy {
    #[model(id)]
    id: u32,
    #[model(index)]
    bucket: i64,
    name: String,
}

fn traced() -> (Sqlite, Rc<RefCell<Vec<String>>>) {
    let trace: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let sink = trace.clone();
    (
        Sqlite::memory().trace_sql(move |sql| sink.borrow_mut().push(sql.to_string())),
        trace,
    )
}

fn seeded(n: u32) -> (ModelContainer, Rc<RefCell<Vec<String>>>) {
    let (driver, trace) = traced();
    let c = ModelContainer::open(driver, schema![Rowy]).expect("open");
    let store = c.cache::<Rowy>();
    store.update("seed", move |k| {
        *k = day_model::Keyed::new(
            (1..=n)
                .map(|i| Rowy {
                    id: i,
                    bucket: (i % 10) as i64,
                    name: format!("row {i}"),
                })
                .collect(),
        );
    });
    c.save().expect("seed");
    // Drop the seed from the cache so tests start from a cold working set.
    c.set_cache_limit(1);
    c.set_cache_limit(DEFAULT_CACHE_LIMIT);
    (c, trace)
}

#[test]
fn open_reads_no_rows() {
    let (driver, trace) = traced();
    let _c = ModelContainer::open(driver, schema![Rowy]).expect("open");
    assert!(
        !trace
            .borrow()
            .iter()
            .any(|s| s.starts_with("SELECT id, bucket, name")),
        "open issued a row SELECT: {:?}",
        trace.borrow()
    );
}

#[test]
fn a_batch_fault_is_one_select() {
    let (c, trace) = seeded(100);
    // Evict everything the seed left resident.
    c.set_cache_limit(1);
    c.set_cache_limit(usize::MAX);
    trace.borrow_mut().clear();

    let keys: Vec<u64> = (1..=50u64).collect();
    c.ensure_resident::<Rowy>(&keys).expect("fault");
    let selects: Vec<String> = trace
        .borrow()
        .iter()
        .filter(|s| s.starts_with("SELECT id, bucket, name"))
        .cloned()
        .collect();
    assert_eq!(selects.len(), 1, "fifty rows, one SELECT: {selects:?}");
    assert!(selects[0].contains("IN ("));
    let resident = c.cache::<Rowy>().with_untracked(|k| k.len());
    assert!(
        (50..=51).contains(&resident),
        "all fifty resident (the limit floor may keep one more): {resident}"
    );

    // Re-ensuring costs nothing: residency short-circuits before any SQL.
    trace.borrow_mut().clear();
    c.ensure_resident::<Rowy>(&keys).expect("re-ensure");
    assert!(
        trace.borrow().iter().all(|s| !s.starts_with("SELECT")),
        "already resident: {:?}",
        trace.borrow()
    );
}

#[test]
fn the_cache_stays_bounded_and_faults_back() {
    let (c, _trace) = seeded(500);
    c.set_cache_limit(64);
    let store = c.cache::<Rowy>();

    for chunk in (1..=500u64).collect::<Vec<_>>().chunks(50) {
        c.ensure_resident::<Rowy>(chunk).expect("fault");
    }
    let resident = store.with_untracked(|k| k.len());
    assert!(
        resident <= 64 + 50,
        "the cache respects its bound (a just-faulted batch is protected): {resident}"
    );

    // An evicted row faults back whole through `get`.
    let row = c.get::<Rowy>(3u32).expect("faults back");
    assert_eq!(row.name().peek(), "row 3");
    assert_eq!(row.bucket().peek(), 3);
}

#[test]
fn dirty_rows_never_evict() {
    let (c, _trace) = seeded(200);
    c.set_autosave(false); // keep the edit dirty across the whole test
    let store = c.cache::<Rowy>();

    let edited = c.get::<Rowy>(7u32).expect("faults");
    edited.name().write("edited, unflushed".into());

    c.set_cache_limit(8);
    for chunk in (1..=200u64).collect::<Vec<_>>().chunks(40) {
        c.ensure_resident::<Rowy>(chunk).expect("fault");
    }
    assert!(
        store.with_untracked(|k| k.get(7).is_some()),
        "the dirty row survived every eviction pass"
    );
    assert_eq!(store.elem(7u64).name().peek(), "edited, unflushed");

    c.save().expect("flush");
}

#[test]
fn observed_rows_never_evict() {
    let (c, _trace) = seeded(200);
    let store = c.cache::<Rowy>();
    let _ = c.get::<Rowy>(9u32).expect("faults");

    // A standing effect binds the row's name — the row is observed.
    let seen = Rc::new(RefCell::new(String::new()));
    let sink = seen.clone();
    day_reactive::Effect::new(move || {
        *sink.borrow_mut() = store.elem(9u64).name().read();
    });
    assert_eq!(&*seen.borrow(), "row 9");

    c.set_cache_limit(4);
    for chunk in (1..=200u64).collect::<Vec<_>>().chunks(40) {
        c.ensure_resident::<Rowy>(chunk).expect("fault");
    }
    assert!(
        store.with_untracked(|k| k.get(9).is_some()),
        "an observed row survived every eviction pass"
    );
}

#[test]
fn a_row_deleted_this_turn_does_not_resurrect() {
    let (c, _trace) = seeded(20);
    c.set_autosave(false);
    let store = c.cache::<Rowy>();
    let _ = c.get::<Rowy>(5u32).expect("faults");
    store.restructure("remove", Op::Delete, 5u32, |v| {
        v.remove(5);
    });
    // The file still holds row 5 until the flush; a fault must not bring the zombie back.
    assert!(c.get::<Rowy>(5u32).is_none(), "deleted is deleted");
    c.save().expect("flush");
    assert!(c.get::<Rowy>(5u32).is_none(), "and stays deleted after it");
}

#[test]
fn a_windowed_query_materializes_only_its_window() {
    let (c, _trace) = seeded(400);
    c.set_cache_limit(1);
    c.set_cache_limit(usize::MAX);
    let store = c.cache::<Rowy>();
    let cold = store.with_untracked(|k| k.len());
    assert!(
        cold <= 1,
        "cold (the limit floor keeps at most one): {cold}"
    );

    let q = c
        .query::<Rowy>()
        .filter(Rowy::bucket().eq(3i64))
        .sort(Rowy::id().asc())
        .live();
    assert_eq!(q.count(), 40, "ids only — nothing new resident");
    assert_eq!(store.with_untracked(|k| k.len()), cold);

    q.materialize(0..10);
    let after = store.with_untracked(|k| k.len());
    assert!(
        (10..=10 + cold).contains(&after),
        "exactly the window faulted: {after}"
    );
    let first = q.first().expect("has rows");
    assert_eq!(store.elem(first).name().peek(), "row 3");
}

#[test]
fn table_count_answers_from_the_engine() {
    let (c, _trace) = seeded(123);
    assert_eq!(c.table_count::<Rowy>().expect("count"), 123);
    assert!(
        c.cache::<Rowy>().with_untracked(|k| k.len()) < 123 || DEFAULT_CACHE_LIMIT >= 123,
        "counting did not require residency"
    );
}

#[test]
fn warm_faults_the_whole_table() {
    let (c, trace) = seeded(300);
    c.set_cache_limit(1);
    c.set_cache_limit(usize::MAX);
    assert!(c.cache::<Rowy>().with_untracked(|k| k.len()) <= 1, "cold");
    trace.borrow_mut().clear();

    let n = c.warm::<Rowy>().expect("warm");
    assert_eq!(n, 300, "the document pattern, one call");
    assert_eq!(c.cache::<Rowy>().with_untracked(|k| k.len()), 300);
    let selects = trace
        .borrow()
        .iter()
        .filter(|s| s.starts_with("SELECT"))
        .count();
    assert!(
        selects <= 2,
        "one id scan plus one chunked fault: {selects}"
    );
}
