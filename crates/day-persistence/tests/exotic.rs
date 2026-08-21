// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The corner-case hunt: interleaved sessions, undo against vanished rows, id reuse, unicode
//! and megabyte payloads, malformed FTS syntax, sentinel keys, container churn — the places a
//! design is usually under-specified, pinned as behavior.

use day_macros::Model;
use day_model::Op;
use day_persistence::{GeoRect, ModelContainer, Sqlite, schema};
use day_reactive::Binding;

#[derive(Model, Clone, Default, PartialEq, Debug)]
#[model(table = "docs", fts("title", "body"))]
struct Doc {
    #[model(id)]
    id: u64,
    title: String,
    body: String,
    rank_no: i64,
}

fn open_seeded() -> ModelContainer {
    let c = ModelContainer::open(Sqlite::memory(), schema![Doc]).expect("open");
    c.store::<Doc>().update("seed", |k| {
        *k = day_model::Keyed::new(vec![
            Doc {
                id: 1,
                title: "Grüße aus Zürich".into(),
                body: "schöne Grüße".into(),
                rank_no: 1,
            },
            Doc {
                id: 2,
                title: "こんにちは世界".into(),
                body: "日本語のテスト".into(),
                rank_no: 2,
            },
            Doc {
                id: 3,
                title: "emoji 🎛️ knobs".into(),
                body: "🎚️🎛️🎚️".into(),
                rank_no: 3,
            },
        ]);
    });
    c.save().expect("flush");
    c
}

#[test]
fn two_interleaved_sessions_commit_independently() {
    let c = open_seeded();
    let store = c.store::<Doc>();
    let title = store.elem(1).title();
    let body = store.elem(1).body();

    let ((), changes) = day_model::record_changes(|| {
        title.write_preview("draft t1".into());
        body.write_preview("draft b1".into());
        title.write_preview("draft t2".into());
        body.write_commit("final body".into());
        title.write_commit("final title".into());
    });
    assert_eq!(changes.len(), 2, "one committed record per session");
    assert_eq!(
        changes[0].prior_as::<String>().map(String::as_str),
        Some("schöne Grüße"),
        "each prior predates ITS OWN session, not the other's"
    );
    assert_eq!(
        changes[1].prior_as::<String>().map(String::as_str),
        Some("Grüße aus Zürich")
    );
}

#[test]
fn a_session_on_a_row_deleted_mid_gesture_stays_harmless() {
    let c = open_seeded();
    let store = c.store::<Doc>();
    let title = store.elem(2).title();

    title.write_preview("half-typed".into());
    store.restructure("remove", Op::Delete, 2, |v| {
        v.remove(2);
    });
    // The commit lands on a gone row: nothing persists, nothing panics, and the pending
    // flush holds only the delete.
    let sql = c
        .record_sql(|| title.write_commit("never lands".into()))
        .expect("save");
    assert_eq!(sql, ["DELETE FROM docs WHERE id = ?"]);
    assert!(!store.elem(2).exists());
}

#[test]
fn undo_against_a_row_another_author_deleted_degrades_quietly() {
    let c = open_seeded();
    let stack = c.undo(10);
    let store = c.store::<Doc>();

    store.elem(1).rank_no().write(99);
    day_reactive::flush_sync();
    // Another author removes the row AFTER the edit was captured (capture suppressed so the
    // stack does not learn about it — the "another writer" shape).
    day_model::with_author("importer", || {
        store.restructure("remove", Op::Delete, 1, |v| {
            v.remove(1);
        });
    });
    day_reactive::flush_sync();

    // The inverse targets a gone row: last-writer-wins means the delete stands; no panic,
    // and the stack stays usable.
    stack.undo();
    assert!(
        !store.elem(1).exists(),
        "the other author's delete is not resurrected"
    );
    let _ = stack.undo();
}

#[test]
fn id_reuse_keeps_histories_distinct() {
    let c = open_seeded();
    let stack = c.undo(10);
    let store = c.store::<Doc>();

    store.restructure("remove", Op::Delete, 3, |v| {
        v.remove(3);
    });
    day_reactive::flush_sync();
    store.restructure("add", Op::Insert, 3, |v| {
        v.push(Doc {
            id: 3,
            title: "the impostor".into(),
            ..Default::default()
        });
    });
    day_reactive::flush_sync();

    stack.undo(); // remove the impostor
    assert!(!store.elem(3).exists());
    stack.undo(); // restore the original
    assert_eq!(store.elem(3).title().peek(), "emoji 🎛️ knobs");
    stack.redo(); // delete it again
    assert!(!store.elem(3).exists());
    stack.redo(); // impostor returns
    assert_eq!(store.elem(3).title().peek(), "the impostor");
}

#[test]
fn insert_edit_delete_of_one_row_in_one_turn_undoes_to_nothing() {
    let c = open_seeded();
    let stack = c.undo(10);
    let store = c.store::<Doc>();

    day_reactive::batch(|| {
        store.restructure("add", Op::Insert, 50, |v| {
            v.push(Doc {
                id: 50,
                ..Default::default()
            });
        });
        store.elem(50).title().write("ephemeral".into());
        store.restructure("remove", Op::Delete, 50, |v| {
            v.remove(50);
        });
    });
    day_reactive::flush_sync();
    assert!(!store.elem(50).exists());

    stack.undo();
    assert!(
        !store.elem(50).exists(),
        "the net of the turn was nothing; so is its undo"
    );
    stack.redo();
    assert!(!store.elem(50).exists());
}

#[test]
fn unicode_survives_the_whole_pipeline() {
    let c = open_seeded();
    let store = c.store::<Doc>();

    // contains_ci through Unicode case folding (ß/İ live in to_lowercase's world).
    let q = c
        .query::<Doc>()
        .filter(Doc::title().contains_ci("GRÜSSE"))
        .live();
    assert_eq!(
        q.ids(),
        Vec::<u64>::new(),
        "ẞ folds to ss, not SS — pinned, not assumed"
    );
    let q = c
        .query::<Doc>()
        .filter(Doc::title().contains_ci("grüße"))
        .live();
    assert_eq!(q.ids(), [1]);

    // FTS over CJK: the default tokenizer is unicode61 — treats CJK as one token per run,
    // so a whole-run query matches and a partial does not. Pinned so a future tokenizer
    // change is a visible decision.
    let q = c
        .query::<Doc>()
        .filter(Doc::fts().matches("こんにちは世界"))
        .live();
    assert_eq!(q.ids(), [2]);

    // Round-trip through SQLite bytes.
    store
        .elem(3)
        .body()
        .write("🎛️ / Grüße / 世界 / \u{200d} zero-width".into());
    c.save().expect("flush");
    c.rescan().expect("rescan");
    assert_eq!(
        c.store::<Doc>().elem(3).body().peek(),
        "🎛️ / Grüße / 世界 / \u{200d} zero-width"
    );
}

#[test]
fn a_megabyte_of_text_round_trips_and_indexes() {
    let c = open_seeded();
    let store = c.store::<Doc>();
    let big = "lorem 🎚️ ".repeat(120_000); // ~1.2 MB
    store.elem(1).body().write(big.clone());
    c.save().expect("flush");
    c.rescan().expect("rescan");
    assert_eq!(c.store::<Doc>().elem(1).body().peek().len(), big.len());

    let q = c.query::<Doc>().filter(Doc::fts().matches("lorem")).live();
    assert!(q.ids().contains(&1));
}

#[test]
fn malformed_fts_syntax_surfaces_instead_of_reading_as_empty() {
    let c = open_seeded();
    let err_signal = c.last_error();
    let q = c
        .query::<Doc>()
        .filter(Doc::fts().matches("AND OR ((("))
        .live();
    assert_eq!(q.ids(), Vec::<u64>::new());
    assert!(
        err_signal.get_untracked().is_some(),
        "a syntax error is an error, not an empty library"
    );
}

#[test]
fn sentinel_adjacent_keys_work_and_the_sentinel_is_reserved() {
    let c = ModelContainer::open(Sqlite::memory(), schema![Doc]).expect("open");
    let store = c.store::<Doc>();
    let near_max = u64::MAX - 1;
    store.restructure("add", Op::Insert, near_max, |v| {
        v.push(Doc {
            id: near_max,
            title: "edge of the keyspace".into(),
            ..Default::default()
        });
    });
    let sql = c.record_sql(|| {}).expect("save");
    assert_eq!(sql.len(), 1, "near-MAX keys persist normally: {sql:?}");
    // u64::MAX itself is day-model's STRUCTURE sentinel: a row using it as a key would alias
    // the collection's shape path. Documented reservation, pinned here.
    assert_eq!(day_model::STRUCTURE, u64::MAX);
}

#[test]
fn wholesale_emptying_deletes_every_row() {
    let c = open_seeded();
    let store = c.store::<Doc>();
    store.update("clear", |k| *k = day_model::Keyed::new(Vec::new()));
    c.save().expect("flush");
    let raw = c.query_raw::<Doc>("SELECT id FROM docs", vec![], &["docs"]);
    assert_eq!(raw.ids(), Vec::<u64>::new());
}

#[test]
fn container_churn_leaves_no_standing_sinks() {
    for _ in 0..50 {
        let c = ModelContainer::open(Sqlite::memory(), schema![Doc]).expect("open");
        let _ = c.store::<Doc>();
    }
    // All fifty containers are gone; a write to an unrelated store must dirty nothing and
    // panic nothing (dead sinks removed on drop).
    let lone = day_model::Store::new(day_model::Keyed::new(vec![Doc {
        id: 1,
        ..Default::default()
    }]));
    lone.elem(1).title().write("still fine".into());
    assert_eq!(lone.elem(1).title().peek(), "still fine");
}

#[test]
fn record_sql_reentrancy_is_defined() {
    let c = open_seeded();
    let store = c.store::<Doc>();
    let outer = c
        .record_sql(|| {
            store.elem(1).rank_no().write(10);
            // An inner record_sql flushes MID-outer: it takes the pending change with it.
            let inner = c
                .record_sql(|| store.elem(2).rank_no().write(20))
                .expect("inner");
            assert_eq!(
                inner.len(),
                2,
                "the inner flush carries both pending rows: {inner:?}"
            );
        })
        .expect("outer");
    assert_eq!(
        outer,
        Vec::<String>::new(),
        "nothing left for the outer flush"
    );
}

#[test]
fn a_viewport_with_inverted_bounds_matches_nothing() {
    let c = open_seeded();
    let q = c
        .query::<Doc>()
        .filter(day_persistence::Pred::Within {
            lat: "rank_no",
            lon: "rank_no",
            min_lat: 5.0,
            max_lat: 1.0, // inverted on purpose
            min_lon: 0.0,
            max_lon: 10.0,
        })
        .live();
    assert_eq!(
        q.ids(),
        Vec::<u64>::new(),
        "an empty box is empty, not everything"
    );
    let _ = GeoRect {
        min_lat: 0.0,
        max_lat: 0.0,
        min_lon: 0.0,
        max_lon: 0.0,
    };
}
