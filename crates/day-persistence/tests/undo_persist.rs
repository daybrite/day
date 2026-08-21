// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Undo against the container: the plan's exit assertions — an undone delete is ONE INSERT, a
//! sixty-move drag is ONE UPDATE, and the agreement property holds with undos interleaved.

use day_macros::Model;
use day_model::Op;
use day_persistence::{Fetch, ModelContainer, Pred, Recorder, Sort, Sqlite, Value, schema};
use day_reactive::Binding;

#[derive(Model, Clone, Default, PartialEq, Debug)]
#[model(table = "trips")]
struct Trip {
    #[model(id)]
    id: u32,
    name: String,
    start_day: i64,
    done: bool,
    notes: String,
}

fn recorder_seeded() -> (ModelContainer, day_persistence::RecorderLog) {
    let (driver, log) = Recorder::new();
    let driver = driver.with_table(
        "trips",
        vec![vec![
            Value::Int(1),
            Value::Text("kyoto".into()),
            Value::Int(7),
            Value::Int(0),
            Value::Text("bring a camera".into()),
        ]],
    );
    let c = ModelContainer::open(driver, schema![Trip]).expect("open");
    log.clear();
    (c, log)
}

#[test]
fn an_undone_delete_is_one_insert() {
    let (c, _log) = recorder_seeded();
    let stack = c.undo(100);
    let store = c.store::<Trip>();

    let sql = c
        .record_sql(|| {
            store.restructure("remove", Op::Delete, 1, |v| {
                v.remove(1);
            });
        })
        .expect("save");
    assert_eq!(sql, ["DELETE FROM trips WHERE id = ?"]);

    let sql = c
        .record_sql(|| {
            assert!(stack.undo());
        })
        .expect("save");
    assert_eq!(sql.len(), 1, "{sql:?}");
    assert!(
        sql[0].starts_with("INSERT INTO trips"),
        "one INSERT: {sql:?}"
    );
    assert_eq!(
        store.elem(1).name().peek(),
        "kyoto",
        "the row came back whole"
    );

    let sql = c
        .record_sql(|| {
            assert!(stack.redo());
        })
        .expect("save");
    assert_eq!(sql, ["DELETE FROM trips WHERE id = ?"], "redo re-deletes");
}

#[test]
fn a_sixty_move_drag_is_one_update() {
    let (c, _log) = recorder_seeded();
    let stack = c.undo(100);
    let store = c.store::<Trip>();
    let field = store.elem(1).start_day();

    let sql = c
        .record_sql(|| {
            for i in 0..60 {
                field.write_preview(i);
            }
            field.write_commit(42);
        })
        .expect("save");
    assert_eq!(
        sql,
        ["UPDATE trips SET start_day = ? WHERE id = ?"],
        "sixty thumb positions, one UPDATE"
    );

    day_reactive::flush_sync();
    let sql = c
        .record_sql(|| {
            assert!(stack.undo());
        })
        .expect("save");
    assert_eq!(sql, ["UPDATE trips SET start_day = ? WHERE id = ?"]);
    assert_eq!(field.peek(), 7, "back to the pre-drag value in one step");
}

#[test]
fn an_undone_field_edit_is_one_update_with_the_prior_value() {
    let (c, log) = recorder_seeded();
    let stack = c.undo(100);
    let store = c.store::<Trip>();

    store.elem(1).name().write("osaka".into());
    day_reactive::flush_sync();
    c.save().expect("flush");
    log.clear();

    assert!(stack.undo());
    c.save().expect("flush");
    let update = log
        .entries()
        .into_iter()
        .find(|(sql, _)| sql.starts_with("UPDATE"))
        .expect("an UPDATE was issued");
    assert_eq!(
        update.1,
        vec![Value::Text("kyoto".into()), Value::Int(1)],
        "the statement carries the RESTORED value"
    );
}

#[test]
fn agreement_holds_with_undos_interleaved() {
    // The §15 agreement property, with undo/redo woven through the edit stream: after 600
    // steps the live query's ids equal a fresh evaluation of the same store.
    let c = ModelContainer::open(Sqlite::memory(), schema![Trip]).expect("open");
    let store = c.store::<Trip>();
    store.update("seed", |k| {
        *k = day_model::Keyed::new(
            (1..=40u32)
                .map(|i| Trip {
                    id: i,
                    name: format!("trip {i}"),
                    start_day: i as i64,
                    done: false,
                    notes: String::new(),
                })
                .collect(),
        );
    });
    c.save().expect("seed");
    let stack = c.undo(200);
    let q = c
        .query::<Trip>()
        .filter(Trip::done().eq(false))
        .sort(Trip::start_day().asc())
        .live();

    let mut rng: u64 = 0x2026_08_21;
    for step in 0..600u64 {
        rng = rng
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let key = (rng >> 33) % 40 + 1;
        match step % 6 {
            0 => {
                let f = store.elem(key).done();
                f.write(!f.peek());
            }
            1 => store
                .elem(key)
                .start_day()
                .write(((rng >> 20) % 100) as i64),
            2 => store.elem(key).notes().write(format!("note {step}")),
            3 => store.elem(key).name().write(format!("renamed {step}")),
            4 => {
                stack.undo();
            }
            _ => {
                if step % 2 == 0 {
                    stack.redo();
                } else {
                    stack.undo();
                }
            }
        }
        day_reactive::flush_sync();
    }

    let fresh = c
        .query::<Trip>()
        .filter(Pred::Eq("done", Value::Int(0)))
        .sort(Sort::asc("start_day"))
        .live();
    assert_eq!(
        q.ids_untracked(),
        fresh.ids_untracked(),
        "600 edits with undos interleaved land exactly where a fresh fetch does"
    );

    // And the database agrees with the store after a final flush.
    c.save().expect("final flush");
    let raw = c.query_raw::<Trip>(
        "SELECT id FROM trips WHERE done = 0 ORDER BY start_day, id",
        vec![],
        &["trips"],
    );
    assert_eq!(raw.ids_untracked(), q.ids_untracked(), "SQLite agrees too");
}

#[test]
fn container_undo_covers_every_store_and_labels_resolve() {
    let (c, _log) = recorder_seeded();
    let stack = c.undo(10);
    stack.set_label_resolver(|label| format!("Undo {label}"));
    let store = c.store::<Trip>();
    store.elem(1).notes().write("packed".into());
    day_reactive::flush_sync();
    assert_eq!(stack.undo_label().get_untracked(), "Undo notes");

    let _ = Fetch::new(); // keep the import honest
}
