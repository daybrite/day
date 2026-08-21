// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! `#[derive(Model)]`: the generated schema half, checked without a database — and one
//! container round-trip through the Recorder to show the derive and the runtime agree.

use day_macros::Model;
use day_model::Op;
use day_persistence::{
    ColumnValue, DbError, Model, ModelContainer, Recorder, SqlType, Value, ValueCodec, schema,
};
use day_reactive::Binding;

/// A codec under test: seconds stored as INTEGER minutes.
struct Minutes;
impl ValueCodec<u32> for Minutes {
    const SQL_TYPE: SqlType = SqlType::Integer;
    fn to_sqlite_value(v: &u32) -> Value {
        Value::Int(*v as i64 / 60)
    }
    fn from_sqlite_value(v: Value) -> Result<u32, DbError> {
        Ok(v.as_int()? as u32 * 60)
    }
}

#[derive(Model, Clone, Default, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
#[model(table = "trips", index("start_day", "done"))]
struct Trip {
    #[model(id)]
    id: u32,
    #[model(unique)]
    name: String,
    #[model(index)]
    start_day: i64,
    done: bool,
    rating: Option<f64>,
    #[model(column = "note_text")]
    notes: String,
    #[model(with = Minutes)]
    duration_s: u32,
    #[model(json)]
    tags: Vec<String>,
    #[model(transient)]
    is_selected: bool,
    #[obs(skip)]
    scratch: u8,
}

#[test]
fn the_derived_schema_matches_the_declaration() {
    assert_eq!(Trip::TABLE, "trips");
    assert_eq!(Trip::KEY, "id");
    let names: Vec<&str> = Trip::COLUMNS.iter().map(|c| c.name).collect();
    assert_eq!(
        names,
        [
            "id",
            "name",
            "start_day",
            "done",
            "rating",
            "note_text",
            "duration_s",
            "tags"
        ],
        "transient and skipped fields take no column; column = renames"
    );
    let by_name = |n: &str| Trip::COLUMNS.iter().find(|c| c.name == n).unwrap();
    assert_eq!(by_name("id").sql, SqlType::Integer);
    assert_eq!(by_name("name").sql, SqlType::Text);
    assert!(by_name("name").unique);
    assert!(by_name("start_day").indexed);
    assert_eq!(by_name("rating").sql, SqlType::Real);
    assert!(!by_name("rating").not_null, "Option drops NOT NULL");
    assert_eq!(
        by_name("duration_s").sql,
        SqlType::Integer,
        "the codec's type, not u32's"
    );
    assert_eq!(by_name("tags").sql, SqlType::Text, "json stores TEXT");
    assert_eq!(Trip::COMPOSITE_INDEXES, [["start_day", "done"]]);
}

#[test]
fn a_row_round_trips_through_the_mappers() {
    let trip = Trip {
        id: 7,
        name: "Kyoto".into(),
        start_day: 20_000,
        done: true,
        rating: None,
        notes: "shinkansen".into(),
        duration_s: 5 * 60,
        tags: vec!["rail".into(), "fall".into()],
        is_selected: true,
        scratch: 9,
    };
    let row = trip.to_row();
    assert_eq!(row[0], Value::Int(7));
    assert_eq!(row[4], Value::Null, "None encodes as NULL");
    assert_eq!(row[6], Value::Int(5), "through the Minutes codec");
    assert_eq!(
        row[7],
        Value::Text("[\"rail\",\"fall\"]".into()),
        "json codec"
    );

    let back = Trip::from_row(&row).expect("decode");
    assert_eq!(back.duration_s, 5 * 60);
    assert_eq!(back.tags, trip.tags);
    assert!(!back.is_selected, "transient reads as Default");
    assert_eq!(back.scratch, 0, "skipped reads as Default");
    assert_eq!(
        Trip {
            is_selected: false,
            scratch: 0,
            ..trip
        },
        back
    );
}

#[test]
fn nulls_decode_as_defaults_even_through_codecs() {
    let row: Vec<Value> = vec![Value::Null; Trip::COLUMNS.len()];
    let trip = Trip::from_row(&row).expect("decode");
    assert_eq!(trip, Trip::default());
}

#[test]
fn the_default_table_name_is_the_snake_cased_struct() {
    #[derive(Model, Clone, Default, PartialEq)]
    struct PackingItem {
        #[model(id)]
        id: u32,
        label: String,
    }
    assert_eq!(PackingItem::TABLE, "packing_item");
    assert_eq!(
        u32::from_sqlite_value(PackingItem::default_row()[0].clone()).unwrap(),
        0
    );
}

#[test]
fn the_derive_drives_the_container_end_to_end() {
    let (driver, log) = Recorder::new();
    let container = ModelContainer::open(driver, schema![Trip]).expect("open");
    let store = container.store::<Trip>();

    let ddl = log
        .sql()
        .into_iter()
        .find(|s| s.starts_with("CREATE TABLE trips"))
        .expect("DDL issued");
    assert!(ddl.contains("id INTEGER PRIMARY KEY"), "{ddl}");
    assert!(ddl.contains("name TEXT NOT NULL UNIQUE"), "{ddl}");
    assert!(ddl.contains("rating REAL"), "{ddl}");
    assert!(!ddl.contains("rating REAL NOT NULL"), "{ddl}");
    assert!(ddl.contains("note_text TEXT"), "{ddl}");
    assert!(
        log.sql()
            .iter()
            .any(|s| s.contains("day_idx_trips_start_day_done")),
        "composite index created: {:?}",
        log.sql()
    );

    let sql = container
        .record_sql(|| {
            store.restructure("add", Op::Insert, 1, |v| {
                v.push(Trip {
                    id: 1,
                    name: "Kyoto".into(),
                    ..Default::default()
                });
            });
            store.elem(1).notes().write("pack light".into());
            store.elem(1).is_selected().write(true);
        })
        .expect("save");
    assert_eq!(
        sql,
        [
            "INSERT INTO trips (id, name, start_day, done, rating, note_text, duration_s, tags) VALUES (?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET name = excluded.name, start_day = excluded.start_day, done = excluded.done, rating = excluded.rating, note_text = excluded.note_text, duration_s = excluded.duration_s, tags = excluded.tags"
        ]
    );

    let sql = container
        .record_sql(|| {
            store.elem(1).is_selected().write(false);
        })
        .expect("save");
    assert!(sql.is_empty(), "a transient edit reaches no SQL: {sql:?}");

    let sql = container
        .record_sql(|| {
            store.elem(1).notes().write("pack lighter".into());
        })
        .expect("save");
    assert_eq!(
        sql,
        ["UPDATE trips SET note_text = ? WHERE id = ?"],
        "the change label is the FIELD name; the fold maps it to the renamed column"
    );
}
