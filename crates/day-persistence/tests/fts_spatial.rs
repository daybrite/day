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

// --- the tokenizer option and cross-model search -------------------------------------------

#[derive(Model, Clone, Default, PartialEq, Debug)]
#[model(
    table = "chapters",
    fts("heading", tokenize = "unicode61 remove_diacritics 2")
)]
struct Chapter {
    #[model(id)]
    id: u32,
    heading: String,
    book: One<Book>,
}

#[derive(Model, Clone, Default, PartialEq, Debug)]
#[model(table = "books")]
struct Book {
    #[model(id)]
    id: u32,
    title: String,
    #[model(relation(target = Chapter, inverse = "book", delete = "cascade"))]
    chapters: Many<Chapter>,
}

use day_persistence::{Many, One};

fn library() -> ModelContainer {
    let c = ModelContainer::open(Sqlite::memory(), schema![Book, Chapter]).expect("open");
    c.cache::<Book>().update("seed", |k| {
        *k = day_model::Keyed::new(vec![
            Book {
                id: 1,
                title: "Voyages".into(),
                ..Default::default()
            },
            Book {
                id: 2,
                title: "Essays".into(),
                ..Default::default()
            },
        ]);
    });
    c.cache::<Chapter>().update("seed", |k| {
        *k = day_model::Keyed::new(vec![
            Chapter {
                id: 10,
                heading: "École de la montagne".into(),
                book: One::to(1u32),
            },
            Chapter {
                id: 11,
                heading: "Harbor towns".into(),
                book: One::to(1u32),
            },
            Chapter {
                id: 12,
                heading: "On rivers".into(),
                book: One::to(2u32),
            },
        ]);
    });
    c.save().expect("seed");
    c
}

#[test]
fn a_declared_tokenizer_reaches_the_shadow_and_folds_diacritics() {
    let c = library();
    // remove_diacritics 2: `ecole` matches `École` through the index itself.
    let q = c
        .query::<Chapter>()
        .filter(Chapter::fts().matches("ecole"))
        .live();
    assert_eq!(q.ids(), [10]);

    // And the DDL carries the declaration — proven against the file.
    let mut ddl = String::new();
    c.with_connection(|conn| {
        conn.query(
            "SELECT sql FROM sqlite_master WHERE name = 'chapters_fts'",
            &[],
            &mut |row| {
                ddl = row.get(0).as_text().unwrap_or_default().to_string();
            },
        )
        .expect("read master");
    });
    assert!(
        ddl.contains("tokenize='unicode61 remove_diacritics 2'"),
        "{ddl}"
    );
}

#[test]
fn a_match_inside_a_relation_predicate_searches_the_target_table() {
    // The cross-model search shape: books whose chapters match — the FTS shadow lookup must
    // resolve the TARGET's table, not the EXISTS alias it travels under.
    let c = library();
    let books: Vec<u64> = c
        .query::<Book>()
        .filter(Book::chapters().any(Chapter::fts().matches("harbor")))
        .live()
        .ids()
        .iter()
        .map(|i| i.handle())
        .collect();
    assert_eq!(books, [1]);

    let none: Vec<u64> = c
        .query::<Book>()
        .filter(Book::chapters().any(Chapter::fts().matches("glacier")))
        .live()
        .ids()
        .iter()
        .map(|i| i.handle())
        .collect();
    assert!(none.is_empty());

    // Composed with a local predicate, as the reader's search box does.
    let either: Vec<u64> = c
        .query::<Book>()
        .filter(
            Book::title().contains("Essays")
                | Book::chapters().any(Chapter::fts().matches("ecole")),
        )
        .sort(Book::id().asc())
        .live()
        .ids()
        .iter()
        .map(|i| i.handle())
        .collect();
    assert_eq!(either, [1, 2]);
}
