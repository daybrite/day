// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Relations: maintained inverses over one source of truth (the child's foreign key), delete
//! rules that flow through the normal pipeline, ordered to-many over a visible order field,
//! and the SQL-level clauses that keep another process honest about the same rules.

use day_macros::Model;
use day_model::{ModelId, Op, Uuid};
use day_persistence::{
    DbErrorKind, DeleteRule, Many, Model, ModelContainer, One, Sqlite, SqliteConnection,
    SqliteDriver, schema,
};
use day_reactive::Binding;

// --- the travel app: one-to-many, cascade --------------------------------------------------

#[derive(Model, Clone, Default, PartialEq, Debug)]
#[model(table = "trips")]
struct Trip {
    #[model(id)]
    id: u32,
    name: String,
    #[model(relation(target = Lodging, inverse = "trip", delete = "cascade"))]
    lodging: Many<Lodging>,
}

#[derive(Model, Clone, Default, PartialEq, Debug)]
#[model(table = "lodging")]
struct Lodging {
    #[model(id)]
    id: u32,
    name: String,
    trip: One<Trip>,
}

// --- the library: nullify over an optional reference ---------------------------------------

#[derive(Model, Clone, Default, PartialEq, Debug)]
#[model(table = "authors")]
struct Author {
    #[model(id)]
    id: u32,
    name: String,
    #[model(relation(target = Book, inverse = "author", delete = "nullify"))]
    books: Many<Book>,
}

#[derive(Model, Clone, Default, PartialEq, Debug)]
#[model(table = "books")]
struct Book {
    #[model(id)]
    id: u32,
    title: String,
    author: Option<One<Author>>,
}

// --- the drawing: a self-referential tree, uuid-keyed, cascade ------------------------------

#[derive(Model, Clone, Default, PartialEq, Debug)]
#[model(table = "nodes")]
struct Node {
    #[model(id)]
    id: Uuid,
    name: String,
    parent: Option<One<Node>>,
    #[model(relation(target = Node, inverse = "parent", delete = "cascade"))]
    children: Many<Node>,
}

// --- the playlist: ordered to-many over a visible REAL field --------------------------------

#[derive(Model, Clone, Default, PartialEq, Debug)]
#[model(table = "playlists")]
struct Playlist {
    #[model(id)]
    id: u32,
    title: String,
    #[model(relation(target = Track, inverse = "playlist", delete = "cascade", ordered = "position"))]
    tracks: Many<Track>,
}

#[derive(Model, Clone, Default, PartialEq, Debug)]
#[model(table = "tracks")]
struct Track {
    #[model(id)]
    id: u32,
    title: String,
    playlist: One<Playlist>,
    position: f64,
}

// --- the ledger: deny -----------------------------------------------------------------------

#[derive(Model, Clone, Default, PartialEq, Debug)]
#[model(table = "invoices")]
struct Invoice {
    #[model(id)]
    id: u32,
    number: String,
    #[model(relation(target = LineItem, inverse = "invoice", delete = "deny"))]
    items: Many<LineItem>,
}

#[derive(Model, Clone, Default, PartialEq, Debug)]
#[model(table = "line_items")]
struct LineItem {
    #[model(id)]
    id: u32,
    what: String,
    invoice: One<Invoice>,
}

// --------------------------------------------------------------------------------------------

fn temp_db(name: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "day-persistence-rel-{}-{}.sqlite",
        std::process::id(),
        name
    ));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
    let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    path
}

fn travel() -> ModelContainer {
    let c = ModelContainer::open(Sqlite::memory(), schema![Trip, Lodging]).expect("open");
    let trips = c.store::<Trip>();
    trips.restructure("add", Op::Insert, 1, |v| {
        v.push(Trip {
            id: 1,
            name: "Kyoto".into(),
            ..Default::default()
        })
    });
    trips.restructure("add", Op::Insert, 2, |v| {
        v.push(Trip {
            id: 2,
            name: "Oslo".into(),
            ..Default::default()
        })
    });
    let lodging = c.store::<Lodging>();
    for (id, name, trip) in [
        (10, "Ryokan", 1),
        (11, "Machiya", 1),
        (12, "Fjord cabin", 2),
    ] {
        lodging.restructure("add", Op::Insert, id, |v| {
            v.push(Lodging {
                id: id as u32,
                name: name.into(),
                trip: One::to(trip as u32),
            })
        });
    }
    // Land the setup, so a `record_sql` below sees only the operation under test.
    c.save().expect("seed");
    c
}

#[test]
fn the_index_answers_from_loaded_foreign_keys() {
    let c = travel();
    let trips = c.store::<Trip>();
    assert_eq!(trips.elem(1).lodging().ids(), [10, 11]);
    assert_eq!(trips.elem(2).lodging().ids(), [12]);
    assert_eq!(trips.elem(1).lodging().count(), 2);
    assert!(trips.elem(1).lodging().contains(11));
    assert!(!trips.elem(2).lodging().contains(11));
}

#[test]
fn writing_a_child_foreign_key_wakes_the_parents_many() {
    let c = travel();
    let trips = c.store::<Trip>();
    let lodging = c.store::<Lodging>();

    // Direction one of the maintained inverse: the child's One is the truth, and setting it
    // announces the affected parents' Many fields.
    let ((), changes) = day_model::record_changes(|| {
        lodging.elem(12).trip().write(One::to(1u32));
    });
    assert_eq!(trips.elem(1).lodging().ids(), [10, 11, 12]);
    assert!(trips.elem(2).lodging().is_empty());
    let labels: Vec<_> = changes.iter().map(|c| c.label).collect();
    assert!(labels.contains(&"trip"), "the FK write itself: {labels:?}");
    assert_eq!(
        labels.iter().filter(|l| **l == "lodging").count(),
        2,
        "both the old and the new parent announced: {labels:?}"
    );
}

#[test]
fn adding_through_the_many_writes_the_foreign_key() {
    let c = travel();
    let trips = c.store::<Trip>();
    let lodging = c.store::<Lodging>();

    // Direction two: the Many side is sugar over the same truth.
    let sql = c
        .record_sql(|| {
            assert!(trips.elem(2).lodging().add(10));
        })
        .expect("flush");
    assert_eq!(sql, ["UPDATE lodging SET trip = ? WHERE id = ?"]);
    assert_eq!(lodging.elem(10).trip().peek().id(), Some(ModelId::of(2u32)));
    assert_eq!(trips.elem(1).lodging().ids(), [11]);
    assert_eq!(trips.elem(2).lodging().ids(), [10, 12], "ascending by id");

    // remove() clears the optional-free reference — a required One surfaces at flush, so
    // reparent instead; here we only assert the membership left.
    assert!(trips.elem(2).lodging().remove(10));
    assert_eq!(trips.elem(2).lodging().ids(), [12]);
}

#[test]
fn cascade_deletes_children_with_the_parent_in_one_flush() {
    let c = travel();
    let trips = c.store::<Trip>();
    let lodging = c.store::<Lodging>();

    let sql = c
        .record_sql(|| {
            trips.restructure("delete", Op::Delete, 1, |v| {
                v.remove(1);
            });
        })
        .expect("flush");
    assert!(lodging.with_untracked(|k| k.get(10).is_none()), "cascaded");
    assert!(lodging.with_untracked(|k| k.get(11).is_none()), "cascaded");
    assert!(
        lodging.with_untracked(|k| k.get(12).is_some()),
        "other trip's"
    );
    assert_eq!(sql.len(), 3, "one DELETE per row: {sql:?}");
    assert!(sql.iter().all(|s| s.starts_with("DELETE FROM")));
}

#[test]
fn cascade_recurses_through_a_self_referential_tree() {
    let c = ModelContainer::open(Sqlite::memory(), schema![Node]).expect("open");
    let nodes = c.store::<Node>();
    let (root, group, leaf_a, leaf_b, other) = (
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
    );
    let add = |id: Uuid, name: &str, parent: Option<Uuid>| {
        let node = Node {
            id,
            name: name.into(),
            parent: parent.map(One::to),
            ..Default::default()
        };
        nodes.restructure("add", Op::Insert, id, |v| v.push(node));
    };
    add(root, "root", None);
    add(group, "group", Some(root));
    add(leaf_a, "leaf a", Some(group));
    add(leaf_b, "leaf b", Some(group));
    add(other, "elsewhere", None);

    assert_eq!(nodes.elem(root).children().count(), 1);
    assert_eq!(nodes.elem(group).children().count(), 2);

    nodes.restructure("delete", Op::Delete, root, |v| {
        v.remove(ModelId::<Node>::of(root).handle());
    });
    // Three generations went; the unrelated root stayed.
    assert_eq!(nodes.keys().len(), 1);
    assert!(nodes.with_untracked(|k| k.get(ModelId::<Node>::of(other).handle()).is_some()));
}

#[test]
fn a_cascade_is_one_undo_unit_that_restores_the_subtree() {
    let c = travel();
    let undo = c.undo(10);
    let trips = c.store::<Trip>();
    let lodging = c.store::<Lodging>();

    trips.restructure("delete", Op::Delete, 1, |v| {
        v.remove(1);
    });
    day_reactive::flush_sync();
    assert!(lodging.with_untracked(|k| k.get(10).is_none()));

    assert!(undo.undo(), "parent and cascaded children seal as one turn");
    assert_eq!(trips.elem(1).name().peek(), "Kyoto");
    assert_eq!(lodging.elem(10).name().peek(), "Ryokan");
    assert_eq!(lodging.elem(11).name().peek(), "Machiya");
    assert_eq!(
        trips.elem(1).lodging().ids(),
        [10, 11],
        "the index followed the replayed inserts"
    );
}

#[test]
fn nullify_clears_optional_references_and_the_children_survive() {
    let c = ModelContainer::open(Sqlite::memory(), schema![Author, Book]).expect("open");
    let authors = c.store::<Author>();
    let books = c.store::<Book>();
    authors.restructure("add", Op::Insert, 1, |v| {
        v.push(Author {
            id: 1,
            name: "Le Guin".into(),
            ..Default::default()
        })
    });
    for (id, title) in [(10, "Dispossessed"), (11, "Left Hand")] {
        books.restructure("add", Op::Insert, id, |v| {
            v.push(Book {
                id: id as u32,
                title: title.into(),
                author: Some(One::to(1u32)),
            })
        });
    }
    assert_eq!(authors.elem(1).books().count(), 2);
    c.save().expect("seed");

    let sql = c
        .record_sql(|| {
            authors.restructure("delete", Op::Delete, 1, |v| {
                v.remove(1);
            });
        })
        .expect("flush");
    assert!(books.with_untracked(|k| k.get(10).is_some()), "survived");
    assert_eq!(books.elem(10).author().peek(), None, "reference cleared");
    assert!(
        sql.iter()
            .any(|s| s.starts_with("UPDATE books SET author = ?")),
        "the nullify folded to UPDATE statements: {sql:?}"
    );
}

#[test]
fn deny_refuses_through_the_checked_door() {
    let c = ModelContainer::open(Sqlite::memory(), schema![Invoice, LineItem]).expect("open");
    let invoices = c.store::<Invoice>();
    let items = c.store::<LineItem>();
    invoices.restructure("add", Op::Insert, 1, |v| {
        v.push(Invoice {
            id: 1,
            number: "INV-0001".into(),
            ..Default::default()
        })
    });
    items.restructure("add", Op::Insert, 10, |v| {
        v.push(LineItem {
            id: 10,
            what: "consulting".into(),
            invoice: One::to(1u32),
        })
    });

    let err = c.delete::<Invoice>(1u32).expect_err("still referenced");
    assert_eq!(err.kind, DbErrorKind::Deny);
    assert!(invoices.with_untracked(|k| k.get(1).is_some()), "refused");

    items.restructure("delete", Op::Delete, 10, |v| {
        v.remove(10);
    });
    c.delete::<Invoice>(1u32).expect("no children left");
    assert!(invoices.with_untracked(|k| k.get(1).is_none()));
}

#[test]
fn ordered_children_read_in_order_and_a_move_writes_one_row() {
    let c = ModelContainer::open(Sqlite::memory(), schema![Playlist, Track]).expect("open");
    let lists = c.store::<Playlist>();
    let tracks = c.store::<Track>();
    lists.restructure("add", Op::Insert, 1, |v| {
        v.push(Playlist {
            id: 1,
            title: "Road trip".into(),
            ..Default::default()
        })
    });
    for (id, title) in [(10, "Opening"), (11, "Middle"), (12, "Closer")] {
        tracks.restructure("add", Op::Insert, id, |v| {
            v.push(Track {
                id: id as u32,
                title: title.into(),
                playlist: One::to(1u32),
                position: 0.0,
            })
        });
        // Append through the relation, so positions assign.
        lists.elem(1).tracks().add(id as u32);
    }
    assert_eq!(lists.elem(1).tracks().ids(), [10, 11, 12]);
    c.save().expect("seed");

    // Moving the closer to the front is ONE update of ONE row's position.
    let sql = c
        .record_sql(|| {
            assert!(lists.elem(1).tracks().move_to(12u32, 0));
        })
        .expect("flush");
    assert_eq!(lists.elem(1).tracks().ids(), [12, 10, 11]);
    assert_eq!(sql.len(), 1, "{sql:?}");
    assert!(sql[0].starts_with("UPDATE tracks SET position = ?"));
}

#[test]
fn a_spent_gap_rebalances_and_order_stays_true() {
    let c = ModelContainer::open(Sqlite::memory(), schema![Playlist, Track]).expect("open");
    let lists = c.store::<Playlist>();
    let tracks = c.store::<Track>();
    lists.restructure("add", Op::Insert, 1, |v| {
        v.push(Playlist {
            id: 1,
            title: "Bisection".into(),
            ..Default::default()
        })
    });
    for id in 10..14u32 {
        tracks.restructure("add", Op::Insert, id, |v| {
            v.push(Track {
                id,
                title: format!("t{id}"),
                playlist: One::to(1u32),
                position: 0.0,
            })
        });
        lists.elem(1).tracks().add(id);
    }
    // Hammer the same slot until the fractional gap between two neighbors is spent — the
    // rebalance path must keep the order exact throughout.
    for _ in 0..70 {
        assert!(lists.elem(1).tracks().move_to(13u32, 1));
        assert!(lists.elem(1).tracks().move_to(12u32, 1));
    }
    assert_eq!(lists.elem(1).tracks().ids(), [10, 12, 13, 11]);
}

#[test]
fn fk_columns_carry_references_clauses_for_external_writers() {
    let path = temp_db("ddl");
    {
        let _c = ModelContainer::open(Sqlite::at(&path), schema![Trip, Lodging]).expect("open");
    }
    let conn = rusqlite::Connection::open(&path).expect("raw open");
    let (table, from, to, on_delete): (String, String, String, String) = conn
        .query_row(
            "SELECT \"table\", \"from\", \"to\", on_delete FROM pragma_foreign_key_list('lodging')",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .expect("the clause exists");
    assert_eq!(
        (table.as_str(), from.as_str(), to.as_str()),
        ("trips", "trip", "id")
    );
    assert_eq!(on_delete, "CASCADE");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn an_external_cascade_merges_cleanly() {
    let path = temp_db("external");
    let c = ModelContainer::open(Sqlite::at(&path), schema![Trip, Lodging]).expect("open");
    let trips = c.store::<Trip>();
    let lodging = c.store::<Lodging>();
    trips.restructure("add", Op::Insert, 1, |v| {
        v.push(Trip {
            id: 1,
            name: "Kyoto".into(),
            ..Default::default()
        })
    });
    lodging.restructure("add", Op::Insert, 10, |v| {
        v.push(Lodging {
            id: 10,
            name: "Ryokan".into(),
            trip: One::to(1u32),
        })
    });
    c.save().expect("save");

    {
        // Another process deletes the trip; ITS engine runs the ON DELETE CASCADE clause.
        let mut other = Sqlite::at(&path).open().expect("second connection");
        other
            .execute("DELETE FROM trips WHERE id = 1", &[])
            .expect("external delete");
    }
    assert!(c.check_external().expect("check"));
    assert!(
        trips.with_untracked(|k| k.get(1).is_none()),
        "parent merged away"
    );
    assert!(
        lodging.with_untracked(|k| k.get(10).is_none()),
        "the file's cascade merged too"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn relations_rebuild_from_a_reopened_file() {
    let path = temp_db("reopen");
    {
        let c = travel_at(&path);
        c.save().expect("save");
    }
    {
        let c = ModelContainer::open(Sqlite::at(&path), schema![Trip, Lodging]).expect("reopen");
        assert_eq!(c.store::<Trip>().elem(1).lodging().ids(), [10, 11]);
        assert_eq!(c.store::<Trip>().elem(2).lodging().ids(), [12]);
    }
    let _ = std::fs::remove_file(&path);
}

fn travel_at(path: &std::path::Path) -> ModelContainer {
    let c = ModelContainer::open(Sqlite::at(path), schema![Trip, Lodging]).expect("open");
    let trips = c.store::<Trip>();
    trips.restructure("add", Op::Insert, 1, |v| {
        v.push(Trip {
            id: 1,
            name: "Kyoto".into(),
            ..Default::default()
        })
    });
    trips.restructure("add", Op::Insert, 2, |v| {
        v.push(Trip {
            id: 2,
            name: "Oslo".into(),
            ..Default::default()
        })
    });
    let lodging = c.store::<Lodging>();
    for (id, name, trip) in [
        (10, "Ryokan", 1),
        (11, "Machiya", 1),
        (12, "Fjord cabin", 2),
    ] {
        lodging.restructure("add", Op::Insert, id, |v| {
            v.push(Lodging {
                id: id as u32,
                name: name.into(),
                trip: One::to(trip as u32),
            })
        });
    }
    c
}

#[test]
fn wiring_validations_name_the_problem() {
    // The relation's target is not in the schema.
    let err = ModelContainer::open(Sqlite::memory(), schema![Trip])
        .err()
        .expect("target missing");
    assert!(err.message.contains("lodging"), "{}", err.message);

    // Nullify needs an Option<One<…>> — a required reference cannot hold nothing.
    #[derive(Model, Clone, Default, PartialEq, Debug)]
    #[model(table = "shelves")]
    struct Shelf {
        #[model(id)]
        id: u32,
        #[model(relation(target = Jar, inverse = "shelf", delete = "nullify"))]
        jars: Many<Jar>,
    }
    #[derive(Model, Clone, Default, PartialEq, Debug)]
    #[model(table = "jars")]
    struct Jar {
        #[model(id)]
        id: u32,
        shelf: One<Shelf>,
    }
    let err = ModelContainer::open(Sqlite::memory(), schema![Shelf, Jar])
        .err()
        .expect("nullify over required must refuse");
    assert!(err.message.contains("Option"), "{}", err.message);

    // Ordered must name a REAL field of the child.
    #[derive(Model, Clone, Default, PartialEq, Debug)]
    #[model(table = "racks")]
    struct Rack {
        #[model(id)]
        id: u32,
        #[model(relation(target = Disc, inverse = "rack", delete = "cascade", ordered = "title"))]
        discs: Many<Disc>,
    }
    #[derive(Model, Clone, Default, PartialEq, Debug)]
    #[model(table = "discs")]
    struct Disc {
        #[model(id)]
        id: u32,
        title: String,
        rack: One<Rack>,
    }
    let err = ModelContainer::open(Sqlite::memory(), schema![Rack, Disc])
        .err()
        .expect("ordered over TEXT must refuse");
    assert!(err.message.contains("REAL"), "{}", err.message);
}

#[test]
fn queries_filter_by_the_fk_column() {
    let c = travel();
    let q = c
        .query::<Lodging>()
        .filter(Lodging::trip().eq(One::to(1u32)))
        .sort(Lodging::name().asc())
        .live();
    assert_eq!(q.ids(), [11, 10], "Machiya, Ryokan");

    // Reparenting moves the row between result sets incrementally.
    c.store::<Lodging>().elem(12).trip().write(One::to(1u32));
    assert_eq!(q.count(), 3);
    let evals = q.evaluations();
    c.store::<Lodging>().elem(12).name().write("Renamed".into());
    assert!(
        q.evaluations() > evals,
        "name is the sort key, so the edit re-evaluated one row"
    );
}

#[test]
fn a_dangling_reference_reads_as_a_missing_parent() {
    let c = travel();
    let trips = c.store::<Trip>();
    let lodging = c.store::<Lodging>();

    // Bypass the rules on purpose: a raw delete of the parent with cascade runs the rule…
    trips.restructure("delete", Op::Delete, 2, |v| {
        v.remove(2);
    });
    // …so nothing dangles. The DEGRADE case is a reference to a key that never loaded:
    lodging.restructure("add", Op::Insert, 99, |v| {
        v.push(Lodging {
            id: 99,
            name: "Orphan".into(),
            trip: One::to(777u32),
        })
    });
    let parent = lodging.elem(99).trip().peek().id().expect("set");
    assert!(!trips.elem(parent).exists(), "deleted-row semantics apply");
}

#[test]
fn delete_rules_are_declared_data() {
    // The derive's const table is API: a tool (or a test) can read the rules.
    assert_eq!(Trip::RELATIONS.len(), 1);
    assert_eq!(Trip::RELATIONS[0].field, "lodging");
    assert_eq!(Trip::RELATIONS[0].inverse, "trip");
    assert_eq!(Trip::RELATIONS[0].delete, DeleteRule::Cascade);
    assert_eq!(Playlist::RELATIONS[0].ordered, Some("position"));
    assert_eq!(Author::RELATIONS[0].delete, DeleteRule::Nullify);
}

#[test]
fn a_renamed_foreign_key_column_still_wires_and_folds() {
    // `#[model(column = …)]` renames the COLUMN; the relation names the FIELD. Both halves
    // have to meet — the DDL clause on the renamed column, the change log's field label in
    // the fold — which is what the derive's per-column `field` entry is for.
    #[derive(Model, Clone, Default, PartialEq, Debug)]
    #[model(table = "crates_")]
    struct Crate {
        #[model(id)]
        id: u32,
        #[model(relation(target = Bottle, inverse = "holder", delete = "cascade"))]
        bottles: Many<Bottle>,
    }
    #[derive(Model, Clone, Default, PartialEq, Debug)]
    #[model(table = "bottles")]
    struct Bottle {
        #[model(id)]
        id: u32,
        label: String,
        #[model(column = "crate_id")]
        holder: One<Crate>,
    }

    let path = temp_db("renamed");
    {
        let c = ModelContainer::open(Sqlite::at(&path), schema![Crate, Bottle]).expect("open");
        let crates = c.store::<Crate>();
        for id in [1u32, 2] {
            crates.restructure("add", Op::Insert, id, |v| {
                v.push(Crate {
                    id,
                    ..Default::default()
                })
            });
        }
        c.store::<Bottle>().restructure("add", Op::Insert, 10, |v| {
            v.push(Bottle {
                id: 10,
                label: "Riesling".into(),
                holder: One::to(1u32),
            })
        });
        c.save().expect("seed");
        assert_eq!(crates.elem(1).bottles().ids(), [10]);

        // The fold writes the RENAMED column. (Reparent rather than remove: the inverse is a
        // required `One<…>`, so clearing it is the constraint violation `remove` documents.)
        let sql = c
            .record_sql(|| {
                assert!(crates.elem(2).bottles().add(10));
            })
            .expect("flush");
        assert_eq!(sql, ["UPDATE bottles SET crate_id = ? WHERE id = ?"]);
        assert_eq!(crates.elem(2).bottles().ids(), [10]);
        assert!(crates.elem(1).bottles().is_empty());
    }
    // …and the DDL hung its clause there too.
    let conn = rusqlite::Connection::open(&path).expect("raw open");
    let from: String = conn
        .query_row(
            "SELECT \"from\" FROM pragma_foreign_key_list('bottles')",
            [],
            |r| r.get(0),
        )
        .expect("the clause exists");
    assert_eq!(from, "crate_id");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn ordered_edges_place_correctly() {
    let c = ModelContainer::open(Sqlite::memory(), schema![Playlist, Track]).expect("open");
    let lists = c.store::<Playlist>();
    let tracks = c.store::<Track>();
    lists.restructure("add", Op::Insert, 1, |v| {
        v.push(Playlist {
            id: 1,
            title: "Edges".into(),
            ..Default::default()
        })
    });
    let add = |id: u32| {
        tracks.restructure("add", Op::Insert, id, |v| {
            v.push(Track {
                id,
                title: format!("t{id}"),
                playlist: One::to(1u32),
                position: 0.0,
            })
        });
    };

    // insert_at into an EMPTY relation, then before the first, then past the end.
    add(10);
    assert!(lists.elem(1).tracks().insert_at(10u32, 0));
    assert_eq!(lists.elem(1).tracks().ids(), [10]);

    add(11);
    assert!(lists.elem(1).tracks().insert_at(11u32, 0));
    assert_eq!(lists.elem(1).tracks().ids(), [11, 10]);

    add(12);
    assert!(
        lists.elem(1).tracks().insert_at(12u32, 99),
        "an index past the end clamps to last"
    );
    assert_eq!(lists.elem(1).tracks().ids(), [11, 10, 12]);

    // Moving a row to its own current index is a no-op in ORDER, not a corruption.
    assert!(lists.elem(1).tracks().move_to(10u32, 1));
    assert_eq!(lists.elem(1).tracks().ids(), [11, 10, 12]);
}
