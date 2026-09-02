// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! An attached, externally owned database beside the container's own: models over its tables
//! as they are, links by value across the two, and the swap a downloaded update makes.

use day_macros::Model;
use day_model::Op;
use day_persistence::{
    Linked, ModelContainer, Schema, Sqlite, SqliteConnection, SqliteDriver, Value, schema,
};
use day_reactive::Binding;

/// A row of the catalog SOMEONE ELSE built: keyed by a BLOB uuid, addressed here through its
/// implicit rowid, with a contentless FTS5 index that keeps the catalog's own shape.
#[derive(Model, Clone, Default, PartialEq, Debug)]
#[model(table = "stations", external = "catalog", fts("name"))]
struct Station {
    #[model(id, column = "rowid")]
    id: u64,
    uuid: Vec<u8>,
    name: String,
    country: String,
    votes: i64,
    #[model(link(target = Favorite, local = "uuid", remote = "station"))]
    favorites: Linked<Favorite>,
}

/// The listener's own row, in the container's own file, naming a catalog row by VALUE.
#[derive(Model, Clone, Default, PartialEq, Debug)]
#[model(table = "favorites")]
struct Favorite {
    #[model(id)]
    id: u64,
    station: Vec<u8>,
    note: String,
    #[model(link(target = Station, local = "station", remote = "uuid"))]
    link: Linked<Station>,
}

fn temp_path(name: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "day-persistence-attached-{}-{name}.sqlite",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    path
}

/// Build a catalog file the way an external tool would: plain DDL, no `_day_schema`, a
/// contentless FTS5 index the tool fills itself.
fn write_catalog(path: &std::path::Path, rows: &[(&[u8], &str, &str, i64)]) {
    let mut conn = Sqlite::at(path).open().expect("catalog file");
    conn.execute_batch(
        "CREATE TABLE stations (uuid BLOB NOT NULL PRIMARY KEY, name TEXT NOT NULL, \
         country TEXT, votes INTEGER); \
         CREATE VIRTUAL TABLE stations_fts USING fts5(name, content='');",
    )
    .expect("ddl");
    for (uuid, name, country, votes) in rows {
        conn.execute(
            "INSERT INTO stations (uuid, name, country, votes) VALUES (?, ?, ?, ?)",
            &[
                Value::Blob(uuid.to_vec()),
                Value::Text(name.to_string()),
                Value::Text(country.to_string()),
                Value::Int(*votes),
            ],
        )
        .expect("row");
        conn.execute(
            "INSERT INTO stations_fts (rowid, name) SELECT rowid, name FROM stations WHERE uuid = ?",
            &[Value::Blob(uuid.to_vec())],
        )
        .expect("fts row");
    }
}

const JAZZ: &[u8] = b"jazz-uuid-000001";
const NEWS: &[u8] = b"news-uuid-000002";
const ROCK: &[u8] = b"rock-uuid-000003";

fn catalog_schema() -> Schema {
    schema![Station]
}

#[test]
fn external_rows_read_through_the_container_and_links_cross_both_ways() {
    let catalog = temp_path("catalog");
    write_catalog(
        &catalog,
        &[
            (JAZZ, "Smooth Jazz", "US", 50),
            (NEWS, "World News", "GB", 10),
            (ROCK, "Rock", "DE", 30),
        ],
    );
    let container = ModelContainer::open(Sqlite::memory(), schema![Favorite]).expect("own store");
    container
        .attach_database(
            "catalog",
            catalog.to_str().expect("utf8 path"),
            catalog_schema(),
        )
        .expect("attach");

    // The catalog reads as any model does: a query over the table, rows faulting in by rowid.
    let all = container
        .query::<Station>()
        .sort(Station::votes().desc())
        .live();
    let ids = all.ids();
    assert_eq!(ids.len(), 3);
    let top = container.get::<Station>(ids[0]).expect("resident");
    assert_eq!(top.name().read(), "Smooth Jazz");
    assert_eq!(top.uuid().read(), JAZZ.to_vec());

    // Its FTS index is the catalog's own — no triggers of ours, but the same subquery.
    let jazz = container
        .query::<Station>()
        .filter(Station::fts().matches("jazz"))
        .live();
    assert_eq!(jazz.count(), 1);

    // A favorite names a station by value; the link crosses into the attached file.
    container.insert(Favorite {
        id: 1,
        station: JAZZ.to_vec(),
        note: "mornings".into(),
        link: Linked::default(),
    });
    container.insert(Favorite {
        id: 2,
        station: b"gone-uuid-000009".to_vec(),
        note: "a station the catalog dropped".into(),
        link: Linked::default(),
    });
    container.save().expect("save");

    let in_us = container
        .query::<Favorite>()
        .filter(Favorite::link().any(Station::country().eq("US".to_string())))
        .live();
    assert_eq!(in_us.ids().len(), 1);
    assert_eq!(in_us.ids()[0].handle(), 1);
    let dangling = container
        .query::<Favorite>()
        .filter(Favorite::link().is_empty())
        .live();
    assert_eq!(dangling.ids()[0].handle(), 2);

    // And from the catalog side: the stations the listener favorited, with a note.
    let favorited = container
        .query::<Station>()
        .filter(Station::favorites().any(Favorite::note().contains("mornings")))
        .live();
    assert_eq!(favorited.count(), 1);
    assert_eq!(favorited.ids()[0], ids[0]);

    // Repointing the favorite is a local column write the link watches: the query moves.
    let store = container.cache::<Favorite>();
    store.elem(1).station().write(ROCK.to_vec());
    container.save().expect("save");
    day_reactive::flush_sync();
    assert_eq!(in_us.ids().len(), 0, "the favorite left the US");

    let _ = std::fs::remove_file(&catalog);
}

#[test]
fn swapping_the_attached_file_reruns_queries_over_the_new_rows() {
    let first = temp_path("first");
    let second = temp_path("second");
    write_catalog(&first, &[(JAZZ, "Smooth Jazz", "US", 50)]);
    write_catalog(
        &second,
        &[
            (JAZZ, "Smooth Jazz", "US", 50),
            (NEWS, "World News", "GB", 10),
        ],
    );
    let container = ModelContainer::open(Sqlite::memory(), schema![Favorite]).expect("own store");
    container
        .attach_database("catalog", first.to_str().expect("utf8"), catalog_schema())
        .expect("attach");
    let all = container.query::<Station>().live();
    assert_eq!(all.count(), 1);
    let jazz_id = all.ids()[0];
    assert_eq!(
        container
            .get::<Station>(jazz_id)
            .expect("row")
            .name()
            .read(),
        "Smooth Jazz"
    );

    // The update landed: the same alias, the new file. Resident rows go, queries re-run.
    container
        .attach_database("catalog", second.to_str().expect("utf8"), catalog_schema())
        .expect("re-attach");
    day_reactive::flush_sync();
    assert_eq!(all.count(), 2);
    let news = container
        .query::<Station>()
        .filter(Station::name().starts_with("World"))
        .live();
    assert_eq!(news.count(), 1);

    // Detached, the models answer nothing rather than failing.
    container.detach_database("catalog").expect("detach");
    day_reactive::flush_sync();
    assert_eq!(all.count(), 0);

    let _ = std::fs::remove_file(&first);
    let _ = std::fs::remove_file(&second);
}

/// A write to an external model is refused at flush, not silently dropped.
#[test]
fn external_models_are_read_only() {
    let catalog = temp_path("readonly");
    write_catalog(&catalog, &[(JAZZ, "Smooth Jazz", "US", 50)]);
    let container = ModelContainer::open(Sqlite::memory(), schema![Favorite]).expect("own store");
    container
        .attach_database("catalog", catalog.to_str().expect("utf8"), catalog_schema())
        .expect("attach");
    let id = container.query::<Station>().live().ids()[0];
    let station = container.get::<Station>(id).expect("row");
    station.name().write("Renamed".into());
    assert!(container.save().is_err(), "the attached file is read-only");
    let _ = std::fs::remove_file(&catalog);
}

// Keep the `Op` import honest for the derive's Observable half, which some toolchains warn
// about when a test never restructures a store directly.
#[allow(dead_code)]
fn _uses_op(_: Op) {}
