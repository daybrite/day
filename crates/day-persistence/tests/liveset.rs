// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The live result-set maintainer: §15's cost table as assertions, ported from the design
//! prototype onto the crate's own Value type — ending with the agreement property that makes
//! skipping the database safe.

use std::collections::BTreeMap;

use day_model::Op;
use day_persistence::{Delta, Fetch, LiveSet, Outcome, Pred, RowView, RowsView, Sort, Value};

#[derive(Clone)]
struct Trip {
    name: String,
    start: i64,
    done: bool,
    notes: String,
}

struct TripView<'a>(&'a Trip);

impl RowView for TripView<'_> {
    fn col(&self, column: &str) -> Option<Value> {
        match column {
            "name" => Some(Value::Text(self.0.name.clone())),
            "start" => Some(Value::Int(self.0.start)),
            "done" => Some(Value::Int(self.0.done as i64)),
            "notes" => Some(Value::Text(self.0.notes.clone())),
            _ => None,
        }
    }
}

#[derive(Default)]
struct Table(BTreeMap<u64, Trip>);

impl RowsView for Table {
    fn row_view(&self, key: u64) -> Option<Box<dyn RowView + '_>> {
        self.0
            .get(&key)
            .map(|t| Box::new(TripView(t)) as Box<dyn RowView>)
    }
}

fn table(n: u64) -> Table {
    let mut t = Table::default();
    for i in 1..=n {
        t.0.insert(
            i,
            Trip {
                name: format!("trip {i:02}"),
                start: i as i64,
                done: false,
                notes: String::new(),
            },
        );
    }
    t
}

/// The query in every test: unfinished trips, by start date.
fn fetch() -> Fetch {
    Fetch::new()
        .filter(Pred::Eq("done", Value::Int(0)))
        .sort(Sort::asc("start"))
}

fn seeded(n: u64) -> (LiveSet, Table) {
    let t = table(n);
    let mut q = LiveSet::new(fetch());
    let keys: Vec<u64> = t.0.keys().copied().collect();
    q.seed(&keys, &t);
    (q, t)
}

#[test]
fn a_column_the_query_never_mentions_costs_nothing() {
    // The tier SwiftData does not have: editing notes on every row leaves the set untouched,
    // and the predicate is never evaluated even once.
    let (mut q, mut t) = seeded(100);
    let before = q.evaluations();

    for key in 1..=100u64 {
        t.0.get_mut(&key).unwrap().notes = "edited".into();
        assert_eq!(q.apply(key, "notes", Op::Set, &t), Outcome::Unaffected);
    }

    assert_eq!(q.ids().len(), 100);
    assert_eq!(q.evaluations() - before, 0, "100 writes, zero evaluations");
}

#[test]
fn a_predicate_column_removes_exactly_one_row() {
    let (mut q, mut t) = seeded(10);
    t.0.get_mut(&4).unwrap().done = true;

    let out = q.apply(4, "done", Op::Set, &t);
    assert_eq!(out, Outcome::Changed(vec![Delta::Remove(3, 4)]));
    assert_eq!(q.ids(), [1, 2, 3, 5, 6, 7, 8, 9, 10]);
    assert_eq!(q.evaluations(), 1, "one row evaluated, not ten");
}

#[test]
fn a_row_that_qualifies_again_comes_back_in_sorted_position() {
    let (mut q, mut t) = seeded(6);
    t.0.get_mut(&3).unwrap().done = true;
    q.apply(3, "done", Op::Set, &t);
    assert_eq!(q.ids(), [1, 2, 4, 5, 6]);

    t.0.get_mut(&3).unwrap().done = false;
    let out = q.apply(3, "done", Op::Set, &t);
    assert_eq!(out, Outcome::Changed(vec![Delta::Insert(2, 3)]));
    assert_eq!(q.ids(), [1, 2, 3, 4, 5, 6]);
}

#[test]
fn changing_the_sort_key_moves_the_row() {
    let (mut q, mut t) = seeded(5);
    t.0.get_mut(&1).unwrap().start = 99; // first becomes last

    let out = q.apply(1, "start", Op::Set, &t);
    assert_eq!(out, Outcome::Changed(vec![Delta::Move(0, 4, 1)]));
    assert_eq!(q.ids(), [2, 3, 4, 5, 1]);
}

#[test]
fn a_sort_key_that_does_not_reorder_moves_nothing() {
    let (mut q, mut t) = seeded(5);
    t.0.get_mut(&3).unwrap().start = 3; // rewritten, same value

    // The row still repaints itself through its own field path; the SET did not change.
    assert_eq!(q.apply(3, "start", Op::Set, &t), Outcome::Unaffected);
    assert_eq!(q.ids(), [1, 2, 3, 4, 5]);
}

#[test]
fn inserts_and_deletes_splice() {
    let (mut q, mut t) = seeded(4);

    t.0.insert(
        99,
        Trip {
            name: "new".into(),
            start: 3,
            done: false,
            notes: String::new(),
        },
    );
    let out = q.apply(99, "", Op::Insert, &t);
    assert_eq!(
        out,
        Outcome::Changed(vec![Delta::Insert(3, 99)]),
        "start=3 sorts after id 3 (stable id tie-break)"
    );

    let out = q.apply(2, "", Op::Delete, &t);
    assert_eq!(out, Outcome::Changed(vec![Delta::Remove(1, 2)]));
    assert_eq!(q.ids(), [1, 3, 99, 4]);
}

#[test]
fn an_inserted_row_that_fails_the_predicate_is_ignored() {
    let (mut q, mut t) = seeded(3);
    t.0.insert(
        50,
        Trip {
            name: "done".into(),
            start: 1,
            done: true,
            notes: String::new(),
        },
    );
    assert_eq!(q.apply(50, "", Op::Insert, &t), Outcome::Unaffected);
    assert_eq!(q.ids(), [1, 2, 3]);
}

#[test]
fn a_windowed_query_asks_again() {
    // Honest fallback: with LIMIT, a row leaving may let another in from beyond the window.
    let t = table(20);
    let mut q = LiveSet::new(fetch().limit(5));
    q.seed(&t.0.keys().copied().collect::<Vec<_>>(), &t);
    assert_eq!(q.ids(), [1, 2, 3, 4, 5]);
    assert_eq!(q.apply(2, "", Op::Delete, &t), Outcome::Requery);
}

#[test]
fn a_raw_sql_predicate_opts_out() {
    let t = table(5);
    let mut q = LiveSet::new(Fetch::new().filter(Pred::Raw("name GLOB 'trip 0*'".into(), vec![])));
    q.seed(&t.0.keys().copied().collect::<Vec<_>>(), &t);
    assert_eq!(q.apply(1, "name", Op::Set, &t), Outcome::Requery);
}

#[test]
fn incremental_maintenance_agrees_with_a_full_fetch() {
    // The correctness test: drive a long deterministic edit stream through the incremental
    // path, then compare against seeding a fresh set. They must be identical, id for id.
    let mut t = table(40);
    let mut q = LiveSet::new(fetch());
    let keys: Vec<u64> = t.0.keys().copied().collect();
    q.seed(&keys, &t);

    let mut rng: u64 = 0x2026_0818;
    for step in 0..600u64 {
        rng = rng
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let key = (rng >> 33) % 40 + 1;
        match step % 4 {
            0 => {
                let row = t.0.get_mut(&key).unwrap();
                row.done = !row.done;
                q.apply(key, "done", Op::Set, &t);
            }
            1 => {
                let row = t.0.get_mut(&key).unwrap();
                row.start = ((rng >> 20) % 100) as i64;
                q.apply(key, "start", Op::Set, &t);
            }
            2 => {
                t.0.get_mut(&key).unwrap().notes = format!("note {step}");
                q.apply(key, "notes", Op::Set, &t);
            }
            _ => {
                t.0.get_mut(&key).unwrap().name = format!("renamed {step}");
                q.apply(key, "name", Op::Set, &t);
            }
        }
    }

    let mut fresh = LiveSet::new(fetch());
    fresh.seed(&t.0.keys().copied().collect::<Vec<_>>(), &t);
    assert_eq!(
        q.ids(),
        fresh.ids(),
        "600 incremental edits land on exactly the set a fresh fetch returns"
    );
    assert!(
        q.evaluations() < 350,
        "roughly half the edits touched non-dependency columns and cost nothing: {}",
        q.evaluations()
    );
}

#[test]
fn within_evaluates_in_memory_and_matches_requeries() {
    struct Pin {
        lat: f64,
        lon: f64,
    }
    struct PinView<'a>(&'a Pin);
    impl RowView for PinView<'_> {
        fn col(&self, c: &str) -> Option<Value> {
            match c {
                "lat" => Some(Value::Real(self.0.lat)),
                "lon" => Some(Value::Real(self.0.lon)),
                _ => None,
            }
        }
    }
    struct Pins(BTreeMap<u64, Pin>);
    impl RowsView for Pins {
        fn row_view(&self, key: u64) -> Option<Box<dyn RowView + '_>> {
            self.0
                .get(&key)
                .map(|p| Box::new(PinView(p)) as Box<dyn RowView>)
        }
    }

    let mut pins = Pins(BTreeMap::new());
    for i in 1..=10u64 {
        pins.0.insert(
            i,
            Pin {
                lat: i as f64,
                lon: i as f64,
            },
        );
    }
    let within = Pred::Within {
        lat: "lat",
        lon: "lon",
        min_lat: 2.5,
        max_lat: 6.5,
        min_lon: 0.0,
        max_lon: 100.0,
    };
    let mut q = LiveSet::new(Fetch::new().filter(within));
    q.seed(&pins.0.keys().copied().collect::<Vec<_>>(), &pins);
    assert_eq!(q.ids(), [3, 4, 5, 6]);

    // A pin dragged out of the box: one evaluation, one Remove — no requery.
    pins.0.get_mut(&4).unwrap().lat = 50.0;
    assert_eq!(
        q.apply(4, "lat", Op::Set, &pins),
        Outcome::Changed(vec![Delta::Remove(1, 4)])
    );

    // An FTS predicate cannot be evaluated here; a change to an INDEXED column re-queries…
    let mut f = LiveSet::new(Fetch::new().filter(Pred::Matches {
        columns: &["name", "notes"],
        query: "kyoto".into(),
    }));
    f.reset(vec![1, 2]);
    let t = table(3);
    assert_eq!(f.apply(1, "name", Op::Set, &t), Outcome::Requery);
}

#[test]
fn nan_sort_keys_stay_deterministic() {
    // SQLite cannot store NaN (it becomes NULL), but an in-memory preview can hold one for a
    // moment; total_cmp keeps the order defined instead of corrupting the sort.
    struct N(f64);
    struct NV<'a>(&'a N);
    impl RowView for NV<'_> {
        fn col(&self, c: &str) -> Option<Value> {
            (c == "x").then_some(Value::Real(self.0.0))
        }
    }
    struct Ns(BTreeMap<u64, N>);
    impl RowsView for Ns {
        fn row_view(&self, key: u64) -> Option<Box<dyn RowView + '_>> {
            self.0
                .get(&key)
                .map(|n| Box::new(NV(n)) as Box<dyn RowView>)
        }
    }
    let mut ns = Ns(BTreeMap::new());
    ns.0.insert(1, N(1.0));
    ns.0.insert(2, N(f64::NAN));
    ns.0.insert(3, N(-1.0));
    let mut q = LiveSet::new(Fetch::new().sort(Sort::asc("x")));
    q.seed(&[1, 2, 3], &ns);
    assert_eq!(
        q.ids(),
        [3, 1, 2],
        "NaN sorts last under total_cmp, deterministically"
    );
}
