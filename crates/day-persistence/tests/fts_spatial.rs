// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! FTS5 and R*Tree through the derive: generated shadow tables and triggers, typed
//! `matches`/`within`/`rank` predicates, index-follows-edit correctness, and the capability
//! refusal at open.

use day_macros::Model;
use day_model::Op;
use day_persistence::{
    Capabilities, DbError, DbErrorKind, GeoRect, ModelContainer, Sqlite, SqliteConnection,
    SqliteDriver, rank, schema,
};
use day_reactive::Binding;

#[derive(Model, Clone, Default, PartialEq, Debug)]
#[model(
    table = "posts",
    fts("title", "body"),
    spatial(lat = "lat", lon = "lon")
)]
struct Post {
    #[model(id)]
    id: u32,
    title: String,
    body: String,
    lat: f64,
    lon: f64,
    done: bool,
}

fn seeded() -> ModelContainer {
    let c = ModelContainer::open(Sqlite::memory(), schema![Post]).expect("open");
    c.cache::<Post>().update("seed", |k| {
        *k = day_model::Keyed::new(vec![
            Post {
                id: 1,
                title: "Autumn in Kyoto".into(),
                body: "temples and maple leaves".into(),
                lat: 35.0,
                lon: 135.7,
                done: false,
            },
            Post {
                id: 2,
                title: "Osaka street food".into(),
                body: "takoyaki near the river".into(),
                lat: 34.7,
                lon: 135.5,
                done: false,
            },
            Post {
                id: 3,
                title: "Hiking in Norway".into(),
                body: "fjords, rain, more fjords".into(),
                lat: 61.0,
                lon: 7.0,
                done: true,
            },
        ]);
    });
    c.save().expect("flush");
    c
}

#[test]
fn matches_answers_through_the_generated_index() {
    let c = seeded();
    let q = c
        .query::<Post>()
        .filter(Post::fts().matches("kyoto OR osaka"))
        .sort(Post::id().asc())
        .live();
    assert_eq!(q.ids(), [1, 2]);

    // Composable with ordinary predicates — the memory remainder narrows FTS candidates.
    let q2 = c
        .query::<Post>()
        .filter(Post::fts().matches("kyoto OR osaka"))
        .filter(Post::done().eq(false))
        .sort(Post::id().asc())
        .live();
    assert_eq!(q2.ids(), [1, 2]);
}

#[test]
fn the_index_follows_edits_through_the_triggers() {
    let c = seeded();
    let store = c.cache::<Post>();
    let q = c
        .query::<Post>()
        .filter(Post::fts().matches("fjords"))
        .live();
    assert_eq!(q.ids(), [3]);

    // Editing an INDEXED column re-queries after the flush lands (triggers run in it)…
    store.elem(1).body().write("chasing fjords someday".into());
    c.save().expect("flush");
    let mut ids = q.ids_untracked();
    ids.sort_unstable();
    assert_eq!(ids, [1, 3], "the trigger re-indexed the edited row");

    // …a delete leaves the index too…
    store.restructure("remove", Op::Delete, 3, |v| {
        v.remove(3);
    });
    c.save().expect("flush");
    assert_eq!(q.ids_untracked(), [1]);

    // …and a column OUTSIDE the indexed set never re-queries at all (deps-filtered).
    let before = q.ids_untracked();
    store.elem(1).lat().write(36.0);
    c.save().expect("flush");
    assert_eq!(q.ids_untracked(), before);
}

#[test]
fn rank_orders_by_relevance() {
    let c = ModelContainer::open(Sqlite::memory(), schema![Post]).expect("open");
    c.cache::<Post>().update("seed", |k| {
        *k = day_model::Keyed::new(vec![
            Post {
                id: 1,
                title: "rust".into(),
                body: "a page about gardening".into(),
                ..Default::default()
            },
            Post {
                id: 2,
                title: "rust rust rust".into(),
                body: "rust everywhere: rust".into(),
                ..Default::default()
            },
        ]);
    });
    c.save().expect("flush");
    let q = c
        .query::<Post>()
        .filter(Post::fts().matches("rust"))
        .sort(rank())
        .live();
    assert_eq!(q.ids(), [2, 1], "bm25: the denser document first");
}

#[test]
fn within_filters_and_follows_a_moved_pin() {
    let c = seeded();
    let store = c.cache::<Post>();
    let japan = GeoRect {
        min_lat: 30.0,
        max_lat: 40.0,
        min_lon: 130.0,
        max_lon: 140.0,
    };
    let q = c
        .query::<Post>()
        .filter(Post::geo().within(japan))
        .sort(Post::id().asc())
        .live();
    assert_eq!(q.ids(), [1, 2]);

    // A pin dragged out of the box leaves the set on the next read.
    store.elem(1).lat().write(52.0);
    assert_eq!(q.ids_untracked(), [2]);

    // The R*Tree shadow answers the same box directly — proving the triggers kept it true.
    c.save().expect("flush");
    let mut rtree_ids: Vec<i64> = Vec::new();
    c.with_connection(|conn| {
        conn.query(
            "SELECT id FROM posts_geo WHERE min_lat >= 30.0 AND max_lat <= 40.0 \
             AND min_lon >= 130.0 AND max_lon <= 140.0 ORDER BY id",
            &[],
            &mut |row| {
                if let Ok(id) = row.get(0).as_int() {
                    rtree_ids.push(id);
                }
            },
        )
        .expect("rtree query");
    });
    assert_eq!(rtree_ids, [2]);
}

#[test]
fn a_hand_written_row_reaches_the_indexes_too() {
    // The triggers run for ANY writer — the coexistence promise extended to the shadows.
    let c = seeded();
    c.with_connection(|conn| {
        conn.execute(
            "INSERT INTO posts (id, title, body, lat, lon, done) \
             VALUES (9, 'Sapporo snow festival', 'ice sculptures', 43.0, 141.3, 0)",
            &[],
        )
        .expect("raw insert");
    });
    c.rescan().expect("rescan");
    let q = c
        .query::<Post>()
        .filter(Post::fts().matches("sapporo"))
        .live();
    assert_eq!(q.ids(), [9]);
}

/// A driver that reports no FTS/R*Tree: open must refuse with the module named.
struct NoExtensions(Sqlite);

impl SqliteDriver for NoExtensions {
    type Connection = Box<dyn SqliteConnection>;
    fn open(self) -> Result<Self::Connection, DbError> {
        Ok(Box::new(self.0.open()?))
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            full_text_search: false,
            rtree: false,
            ..self.0.capabilities()
        }
    }
}

#[test]
fn a_missing_module_refuses_at_open_not_at_query_time() {
    let err = ModelContainer::open(NoExtensions(Sqlite::memory()), schema![Post])
        .err()
        .expect("refused");
    assert_eq!(err.kind, DbErrorKind::Unsupported);
    assert!(err.message.contains("FTS5"), "{}", err.message);
}
