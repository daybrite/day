// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Wide keys, stored: Uuid keys are 16-byte `BLOB` primary keys, String keys are `TEXT` —
//! round-tripping files, folding to correctly-typed parameters, merging another connection's
//! writes, and refusing the shapes SQLite's rowid-backed indexes cannot address.

use day_macros::Model;
use day_model::{ModelId, Op, Uuid};
use day_persistence::{
    ModelContainer, Recorder, Sqlite, SqliteConnection, SqliteDriver, Value, schema,
};
use day_reactive::Binding;

#[derive(Model, Clone, Default, PartialEq, Debug)]
#[model(table = "contacts")]
struct Contact {
    #[model(id)]
    id: Uuid,
    name: String,
    starred: bool,
}

#[derive(Model, Clone, Default, PartialEq, Debug)]
#[model(table = "wiki_pages")]
struct WikiPage {
    #[model(id)]
    slug: String,
    body: String,
}

#[derive(Model, Clone, Default, PartialEq, Debug)]
#[model(table = "counters")]
struct Counter {
    #[model(id)]
    id: u32,
    n: i64,
}

fn temp_db(name: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "day-persistence-widekeys-{}-{}.sqlite",
        std::process::id(),
        name
    ));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
    let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    path
}

fn contact(id: Uuid, name: &str) -> Contact {
    Contact {
        id,
        name: name.into(),
        starred: false,
    }
}

#[test]
fn uuid_rows_survive_a_reopen_and_store_as_blobs() {
    let path = temp_db("reopen");
    let (ada, grace) = (Uuid::now_v7(), Uuid::now_v7());
    {
        let c = ModelContainer::open(Sqlite::at(&path), schema![Contact]).expect("open");
        let store = c.store::<Contact>();
        store.restructure("add", Op::Insert, ada, |v| v.push(contact(ada, "Ada")));
        store.restructure("add", Op::Insert, grace, |v| {
            v.push(contact(grace, "Grace"))
        });
        store.elem(ada).name().write("Ada Lovelace".into());
        store.elem(grace).starred().write(true);
        c.save().expect("save");
    }
    {
        // The stored form is a 16-byte BLOB any SQLite tool can read.
        let conn = rusqlite::Connection::open(&path).expect("raw open");
        let ty: String = conn
            .query_row("SELECT typeof(id) FROM contacts LIMIT 1", [], |r| r.get(0))
            .expect("typeof");
        assert_eq!(ty, "blob");
        let name: String = conn
            .query_row(
                "SELECT name FROM contacts WHERE id = ?1",
                [rusqlite::types::Value::Blob(ada.as_bytes().to_vec())],
                |r| r.get(0),
            )
            .expect("row by uuid");
        assert_eq!(name, "Ada Lovelace");
    }
    {
        let c = ModelContainer::open(Sqlite::at(&path), schema![Contact]).expect("reopen");
        let store = c.store::<Contact>();
        assert_eq!(store.keys().len(), 2);
        assert_eq!(store.elem(ada).name().peek(), "Ada Lovelace");
        assert!(store.elem(grace).starred().peek());
        // The handle → key round trip survives the reload.
        assert_eq!(store.elem(ada).model_id().key().as_uuid(), Some(ada));
    }
    let _ = std::fs::remove_file(&path);
}

#[test]
fn the_fold_binds_typed_key_parameters() {
    let (driver, log) = Recorder::new();
    let c = ModelContainer::open(driver, schema![Contact]).expect("open");
    let store = c.store::<Contact>();
    let id = Uuid::now_v7();

    store.restructure("add", Op::Insert, id, |v| v.push(contact(id, "Alan")));
    c.save().expect("insert flush");
    let insert = log
        .entries()
        .into_iter()
        .find(|(sql, _)| sql.starts_with("INSERT INTO contacts"))
        .expect("the insert statement");
    assert_eq!(
        insert.1.first(),
        Some(&Value::Blob(id.as_bytes().to_vec())),
        "the key column binds as a 16-byte blob"
    );

    let sql = c
        .record_sql(|| store.elem(id).name().write("Alan Turing".into()))
        .expect("update flush");
    assert_eq!(sql, ["UPDATE contacts SET name = ? WHERE id = ?"]);
    let update = log
        .entries()
        .into_iter()
        .rfind(|(s, _)| s.starts_with("UPDATE contacts"))
        .expect("the update statement");
    assert_eq!(update.1.last(), Some(&Value::Blob(id.as_bytes().to_vec())));

    let sql = c
        .record_sql(|| {
            store.restructure("remove", Op::Delete, id, |v| {
                v.remove(ModelId::<Contact>::of(id).handle());
            })
        })
        .expect("delete flush");
    assert_eq!(sql, ["DELETE FROM contacts WHERE id = ?"]);
}

#[test]
fn string_keys_round_trip_as_text_primary_keys() {
    let path = temp_db("slugs");
    {
        let c = ModelContainer::open(Sqlite::at(&path), schema![WikiPage]).expect("open");
        let store = c.store::<WikiPage>();
        for (slug, body) in [("welcome", "hello"), ("faq", "answers"), ("about", "us")] {
            store.restructure("add", Op::Insert, slug, |v| {
                v.push(WikiPage {
                    slug: slug.into(),
                    body: body.into(),
                });
            });
        }
        store.elem("faq").body().write("better answers".into());
        c.save().expect("save");
    }
    {
        let conn = rusqlite::Connection::open(&path).expect("raw open");
        let ty: String = conn
            .query_row("SELECT typeof(slug) FROM wiki_pages LIMIT 1", [], |r| {
                r.get(0)
            })
            .expect("typeof");
        assert_eq!(ty, "text");
        let body: String = conn
            .query_row("SELECT body FROM wiki_pages WHERE slug = 'faq'", [], |r| {
                r.get(0)
            })
            .expect("row by slug");
        assert_eq!(body, "better answers");
    }
    {
        let c = ModelContainer::open(Sqlite::at(&path), schema![WikiPage]).expect("reopen");
        assert_eq!(c.store::<WikiPage>().elem("about").body().peek(), "us");
    }
    let _ = std::fs::remove_file(&path);
}

#[test]
fn another_connections_uuid_writes_merge_precisely() {
    let path = temp_db("external");
    let c = ModelContainer::open(Sqlite::at(&path), schema![Contact]).expect("open");
    let store = c.store::<Contact>();
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    store.restructure("add", Op::Insert, a, |v| v.push(contact(a, "Edsger")));
    store.restructure("add", Op::Insert, b, |v| v.push(contact(b, "Barbara")));
    c.save().expect("save");

    {
        let mut other = Sqlite::at(&path).open().expect("second connection");
        other
            .execute(
                "UPDATE contacts SET starred = 1 WHERE id = ?",
                &[Value::Blob(b.as_bytes().to_vec())],
            )
            .expect("external update");
    }
    let ((), changes) = day_model::record_changes(|| {
        assert!(c.check_external().expect("check"));
    });
    // Only the changed column of the changed row announced — key width changes nothing.
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].label, "starred");
    assert!(store.elem(b).starred().peek());
    assert!(!store.elem(a).starred().peek());

    let _ = std::fs::remove_file(&path);
}

#[test]
fn queries_speak_typed_ids_over_wide_keys() {
    let c = ModelContainer::open(Sqlite::memory(), schema![Contact]).expect("open");
    let store = c.store::<Contact>();
    let ids: Vec<Uuid> = (0..4).map(|_| Uuid::now_v7()).collect();
    for (i, id) in ids.iter().enumerate() {
        let mut row = contact(*id, &format!("contact {i}"));
        row.starred = i % 2 == 0;
        store.restructure("add", Op::Insert, *id, |v| v.push(row));
    }

    let starred = c
        .query::<Contact>()
        .filter(Contact::starred().eq(true))
        .sort(Contact::name().asc())
        .live();
    assert_eq!(starred.count(), 2);
    assert!(starred.contains(ids[0]));
    assert!(!starred.contains(ids[1]));

    // A query id addresses the store directly — the typed round trip.
    let first = starred.first().expect("has results");
    assert_eq!(store.elem(first).name().peek(), "contact 0");
    assert_eq!(first.key().as_uuid(), Some(ids[0]));

    // Membership follows a field write, same as integer keys.
    store.elem(ids[1]).starred().write(true);
    assert_eq!(starred.count(), 3);
}

#[test]
fn undo_writes_inverse_statements_with_blob_keys() {
    let (driver, _log) = Recorder::new();
    let c = ModelContainer::open(driver, schema![Contact]).expect("open");
    let undo = c.undo(10);
    let store = c.store::<Contact>();
    let id = Uuid::now_v7();

    store.restructure("add", Op::Insert, id, |v| v.push(contact(id, "Grace")));
    day_reactive::flush_sync();
    c.save().expect("flush insert");

    store.restructure("delete", Op::Delete, id, |v| {
        v.remove(ModelId::<Contact>::of(id).handle());
    });
    day_reactive::flush_sync();
    c.save().expect("flush delete");

    let sql = c
        .record_sql(|| {
            assert!(undo.undo(), "the delete is one unit");
        })
        .expect("undo flush");
    assert_eq!(sql.len(), 1);
    assert!(
        sql[0].starts_with("INSERT INTO contacts"),
        "undoing a delete is one INSERT: {sql:?}"
    );
    assert_eq!(store.elem(id).name().peek(), "Grace");
}

#[test]
fn fts_on_a_wide_key_is_refused_at_open() {
    #[derive(Model, Clone, Default, PartialEq, Debug)]
    #[model(table = "notes_fts_wide", fts("body"))]
    struct Note {
        #[model(id)]
        id: Uuid,
        body: String,
    }
    let err = ModelContainer::open(Sqlite::memory(), schema![Note])
        .err()
        .expect("wide-keyed fts must refuse");
    assert!(err.message.contains("ROWID"), "{}", err.message);
}

#[test]
fn three_key_shapes_coexist_in_one_container() {
    let path = temp_db("mixed");
    let ada = Uuid::now_v7();
    {
        let c = ModelContainer::open(Sqlite::at(&path), schema![Contact, WikiPage, Counter])
            .expect("open");
        c.store::<Contact>()
            .restructure("add", Op::Insert, ada, |v| v.push(contact(ada, "Ada")));
        c.store::<WikiPage>()
            .restructure("add", Op::Insert, "home", |v| {
                v.push(WikiPage {
                    slug: "home".into(),
                    body: "start".into(),
                });
            });
        c.store::<Counter>()
            .restructure("add", Op::Insert, 7, |v| v.push(Counter { id: 7, n: 1 }));
        c.save().expect("save");
    }
    {
        let c = ModelContainer::open(Sqlite::at(&path), schema![Contact, WikiPage, Counter])
            .expect("reopen");
        assert_eq!(c.store::<Contact>().elem(ada).name().peek(), "Ada");
        assert_eq!(c.store::<WikiPage>().elem("home").body().peek(), "start");
        assert_eq!(c.store::<Counter>().elem(7).n().peek(), 1);
    }
    let _ = std::fs::remove_file(&path);
}

#[test]
fn two_default_uuid_rows_coalesce_to_one_stored_row() {
    // The nil-uuid Default is the documented edge: two un-idified rows share a key, the
    // index resolves to the last, and the fold's upsert writes ONE row. Apps mint ids
    // (`Uuid::now_v7()`) before insert; this pins what happens when one forgets.
    let path = temp_db("nil");
    {
        let c = ModelContainer::open(Sqlite::at(&path), schema![Contact]).expect("open");
        let store = c.store::<Contact>();
        store.restructure("add", Op::Insert, Uuid::nil(), |v| {
            v.push(contact(Uuid::nil(), "first"))
        });
        store.restructure("add", Op::Insert, Uuid::nil(), |v| {
            v.push(contact(Uuid::nil(), "second"))
        });
        c.save().expect("save");
    }
    {
        let conn = rusqlite::Connection::open(&path).expect("raw open");
        let n: i64 = conn
            .query_row("SELECT count(*) FROM contacts", [], |r| r.get(0))
            .expect("count");
        assert_eq!(n, 1, "one key, one row — the upsert coalesced");
    }
    let _ = std::fs::remove_file(&path);
}
