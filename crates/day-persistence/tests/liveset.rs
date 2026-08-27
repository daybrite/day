// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The result-set differ, and the property that makes it trustworthy: a consumer applying
//! the narrated deltas in order ALWAYS lands on the new set (`adopt` verifies by simulation
//! and reloads rather than mis-narrate). Ends with the end-to-end agreement test: a long
//! random edit stream against a live container, mirrored purely from the delta feed.

use day_macros::Model;
use day_persistence::{
    Delta, Fetch, ModelContainer, Pred, QueryEvents, ResultSet, SetChange, Sort, Sqlite, schema,
};
use day_reactive::Binding;

fn set(ids: &[u64]) -> ResultSet {
    let mut s = ResultSet::new(Fetch::new());
    s.reset(ids.to_vec());
    s
}

/// Apply a delta list the way a list consumer does — sequentially.
fn apply(ids: &mut Vec<u64>, deltas: &[Delta]) {
    for d in deltas {
        match *d {
            Delta::Remove(i, _) => {
                ids.remove(i);
            }
            Delta::Insert(i, k) => ids.insert(i, k),
            Delta::Move(from, to, _) => {
                let k = ids.remove(from);
                ids.insert(to, k);
            }
        }
    }
}

#[test]
fn an_identical_answer_narrates_nothing() {
    let mut s = set(&[1, 2, 3]);
    assert_eq!(s.adopt(vec![1, 2, 3]), SetChange::Same);
}

#[test]
fn one_departure_is_one_remove_at_its_index() {
    let mut s = set(&[1, 2, 3, 4]);
    assert_eq!(
        s.adopt(vec![1, 2, 4]),
        SetChange::Deltas(vec![Delta::Remove(2, 3)])
    );
}

#[test]
fn one_arrival_is_one_insert_at_its_final_index() {
    let mut s = set(&[1, 3, 4]);
    assert_eq!(
        s.adopt(vec![1, 2, 3, 4]),
        SetChange::Deltas(vec![Delta::Insert(1, 2)])
    );
}

#[test]
fn multiple_removals_come_descending_so_each_index_is_valid() {
    let mut s = set(&[1, 2, 3, 4, 5]);
    let SetChange::Deltas(d) = s.adopt(vec![2, 4]) else {
        panic!("narratable");
    };
    assert_eq!(
        d,
        [
            Delta::Remove(4, 5),
            Delta::Remove(2, 3),
            Delta::Remove(0, 1)
        ]
    );
    let mut ids = vec![1, 2, 3, 4, 5];
    apply(&mut ids, &d);
    assert_eq!(ids, [2, 4]);
}

#[test]
fn a_reposition_is_one_move() {
    let mut s = set(&[1, 2, 3, 4, 5]);
    assert_eq!(
        s.adopt(vec![2, 3, 4, 1, 5]),
        SetChange::Deltas(vec![Delta::Move(0, 3, 1)])
    );
    let mut s = set(&[1, 2, 3, 4, 5]);
    assert_eq!(
        s.adopt(vec![1, 5, 2, 3, 4]),
        SetChange::Deltas(vec![Delta::Move(4, 1, 5)])
    );
}

#[test]
fn a_remove_plus_a_move_still_narrates() {
    // The shape one turn's "edit a sort key, toggle another row out" produces.
    let mut s = set(&[1, 3, 5, 6]);
    let SetChange::Deltas(d) = s.adopt(vec![5, 6, 1]) else {
        panic!("narratable");
    };
    assert_eq!(d.len(), 2, "{d:?}");
    let mut ids = vec![1, 3, 5, 6];
    apply(&mut ids, &d);
    assert_eq!(ids, [5, 6, 1]);
}

#[test]
fn a_tangled_reorder_reloads_instead_of_guessing() {
    let mut s = set(&[1, 2, 3, 4, 5, 6]);
    assert_eq!(s.adopt(vec![6, 5, 4, 3, 2, 1]), SetChange::Reload);
}

#[test]
fn narrated_deltas_always_land_on_the_new_set() {
    // The property adopt() proves by simulation before answering: whatever the change, a
    // Deltas answer applied in order equals the new set — across a large random sample.
    let mut rng: u64 = 0x2026_0827;
    let mut next = move || {
        rng = rng
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        rng >> 33
    };
    for _case in 0..500 {
        let old_len = (next() % 12) as usize;
        let old: Vec<u64> = (0..old_len as u64).map(|i| i + 1).collect();
        // Mutate: drop some, add some, maybe move one.
        let mut new: Vec<u64> = old.iter().copied().filter(|_| next() % 4 != 0).collect();
        for _a in 0..(next() % 3) {
            let k = 100 + next() % 20;
            if !new.contains(&k) {
                let at = (next() as usize) % (new.len() + 1);
                new.insert(at, k);
            }
        }
        if !new.is_empty() && next() % 2 == 0 {
            let from = (next() as usize) % new.len();
            let to = (next() as usize) % new.len();
            let k = new.remove(from);
            new.insert(to, k);
        }
        let mut s = set(&old);
        match s.adopt(new.clone()) {
            SetChange::Same => assert_eq!(old, new),
            SetChange::Reload => {} // honest
            SetChange::Deltas(d) => {
                let mut ids = old.clone();
                apply(&mut ids, &d);
                assert_eq!(ids, new, "deltas {d:?} from {old:?}");
            }
        }
    }
}

// --- end to end: the delta feed alone reconstructs the query -------------------------------

#[derive(Model, Clone, Default, PartialEq, Debug)]
#[model(table = "trips_ls")]
struct Trip {
    #[model(id)]
    id: u32,
    name: String,
    start: i64,
    done: bool,
    notes: String,
}

#[test]
fn a_long_edit_stream_mirrors_through_the_delta_feed() {
    let c = ModelContainer::open(Sqlite::memory(), schema![Trip]).expect("open");
    let store = c.cache::<Trip>();
    store.update("seed", |k| {
        *k = day_model::Keyed::new(
            (1..=40u32)
                .map(|i| Trip {
                    id: i,
                    name: format!("trip {i:02}"),
                    start: i as i64,
                    done: false,
                    notes: String::new(),
                })
                .collect(),
        );
    });
    c.save().expect("seed");

    let q = c
        .query::<Trip>()
        .filter(Pred::Eq("done", day_persistence::Value::Int(0)))
        .sort(Sort::asc("start"))
        .live();
    let mut mirror: Vec<u64> = q.ids().iter().map(|i| i.handle()).collect();

    let mut rng: u64 = 0x2026_0818;
    for step in 0..600u64 {
        rng = rng
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let key = (rng >> 33) % 40 + 1;
        match step % 4 {
            0 => {
                let done = store.elem(key).done().peek();
                store.elem(key).done().write(!done);
            }
            1 => store.elem(key).start().write(((rng >> 20) % 100) as i64),
            2 => store.elem(key).notes().write(format!("note {step}")),
            _ => store.elem(key).name().write(format!("renamed {step}")),
        }
        // Drain and mirror — the only information a list consumer gets.
        match q.take_events() {
            QueryEvents::None => {}
            QueryEvents::Deltas(d) => apply(&mut mirror, &d),
            QueryEvents::Reload => {
                mirror = q.ids_untracked().iter().map(|i| i.handle()).collect();
            }
        }
        assert_eq!(
            mirror,
            q.ids_untracked()
                .iter()
                .map(|i| i.handle())
                .collect::<Vec<_>>(),
            "step {step}: the mirror drifted from the query"
        );
    }

    // And the final set equals a fresh fetch — the whole pipeline agrees with the engine.
    let fresh = c
        .query::<Trip>()
        .filter(Pred::Eq("done", day_persistence::Value::Int(0)))
        .sort(Sort::asc("start"))
        .live();
    assert_eq!(q.ids_untracked(), fresh.ids_untracked());
}
