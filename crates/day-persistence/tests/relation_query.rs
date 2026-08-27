// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Predicates that cross a relation — over a to-many, a to-one, a self-referential tree and a
//! many-to-many, from both sides.
//!
//! The assertions that matter are the counting ones. Answering correctly while evaluating
//! everything would miss the point: the claim is that a related column the predicate never
//! reads costs nothing, and that one the predicate does read moves exactly the rows it can.

use std::cell::RefCell;
use std::rc::Rc;

use day_macros::Model;
use day_model::{ModelId, Op, Uuid};
use day_persistence::{Many, ModelContainer, One, Sqlite, schema};
use day_reactive::Binding;

/// A container over a traced connection, so tests can count the requeries a change costs.
fn traced_travel() -> (ModelContainer, Rc<RefCell<Vec<String>>>) {
    let trace: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let sink = trace.clone();
    let driver = Sqlite::memory().trace_sql(move |sql| sink.borrow_mut().push(sql.to_string()));
    (seed_travel(driver), trace)
}

fn requeries(trace: &RefCell<Vec<String>>) -> usize {
    trace
        .borrow()
        .iter()
        .filter(|s| s.starts_with("SELECT trips.id FROM trips"))
        .count()
}

// --- a travel itinerary: one-to-many ---------------------------------------------------

#[derive(Model, Clone, Default, PartialEq, Debug)]
#[model(table = "trips")]
struct Trip {
    #[model(id)]
    id: u32,
    name: String,
    done: bool,
    #[model(relation(target = Lodging, inverse = "trip", delete = "cascade"))]
    lodging: Many<Lodging>,
}

#[derive(Model, Clone, Default, PartialEq, Debug)]
#[model(table = "lodging")]
struct Lodging {
    #[model(id)]
    id: u32,
    name: String,
    confirmed: bool,
    /// Read by no predicate in this file — the column whose edits must cost nothing.
    notes: String,
    trip: One<Trip>,
}

// --- note tagging: many-to-many, declared on both sides ---------------------------------

#[derive(Model, Clone, Default, PartialEq, Debug)]
#[model(table = "tags")]
struct Tag {
    #[model(id)]
    id: u32,
    name: String,
    #[model(relation(target = Note, join = "note_tags"))]
    notes: Many<Note>,
}

#[derive(Model, Clone, Default, PartialEq, Debug)]
#[model(table = "notes")]
struct Note {
    #[model(id)]
    id: Uuid,
    title: String,
    #[model(relation(target = Tag, join = "note_tags"))]
    tags: Many<Tag>,
}

// --- a drawing: the self-referential tree ------------------------------------------------

#[derive(Model, Clone, Default, PartialEq, Debug)]
#[model(table = "nodes")]
struct Node {
    #[model(id)]
    id: u32,
    name: String,
    parent: Option<One<Node>>,
    #[model(relation(target = Node, inverse = "parent", delete = "cascade"))]
    children: Many<Node>,
}

fn travel() -> ModelContainer {
    seed_travel(Sqlite::memory())
}

fn seed_travel(driver: Sqlite) -> ModelContainer {
    let c = ModelContainer::open(driver, schema![Trip, Lodging]).expect("open");
    let trips = c.cache::<Trip>();
    for (id, name, done) in [
        (1u32, "Kyoto", false),
        (2, "Oslo", false),
        (3, "Lima", true),
    ] {
        trips.restructure("add", Op::Insert, id, |v| {
            v.push(Trip {
                id,
                name: name.into(),
                done,
                ..Default::default()
            })
        });
    }
    let lodging = c.cache::<Lodging>();
    for (id, name, confirmed, trip) in [
        (10u32, "Ryokan", true, 1u32),
        (11, "Machiya", false, 1),
        (12, "Fjord cabin", true, 2),
    ] {
        lodging.restructure("add", Op::Insert, id, |v| {
            v.push(Lodging {
                id,
                name: name.into(),
                confirmed,
                notes: "quiet".into(),
                trip: One::to(trip),
            })
        });
    }
    c.save().expect("seed");
    c
}

fn trip_ids(c: &ModelContainer, pred: day_persistence::Pred) -> Vec<u64> {
    c.query::<Trip>()
        .filter(pred)
        .sort(Trip::id().asc())
        .live()
        .ids()
        .iter()
        .map(|i| i.handle())
        .collect()
}

// --- the quantifiers ---------------------------------------------------------------------

#[test]
fn any_asks_whether_some_related_row_matches() {
    let c = travel();
    assert_eq!(
        trip_ids(&c, Trip::lodging().any(Lodging::name().contains("Ryokan"))),
        [1]
    );
    assert_eq!(
        trip_ids(&c, Trip::lodging().any(Lodging::confirmed().eq(true))),
        [1, 2]
    );
    // Lima has no lodging at all: `any` over an empty relation is false.
    assert!(!trip_ids(&c, Trip::lodging().any(Lodging::confirmed().eq(true))).contains(&3));
}

#[test]
fn none_and_all_differ_exactly_over_the_empty_relation() {
    let c = travel();
    // Kyoto has an unconfirmed lodging; Oslo does not; Lima has none.
    assert_eq!(
        trip_ids(&c, Trip::lodging().none(Lodging::confirmed().eq(false))),
        [2, 3],
        "`none` is true for the trip with nothing booked"
    );
    assert_eq!(
        trip_ids(&c, Trip::lodging().all(Lodging::confirmed().eq(true))),
        [2, 3],
        "`all` is VACUOUSLY true for it — the documented surprise"
    );
    // …and they part company as soon as the empty one gains a failing row.
    let lodging = c.cache::<Lodging>();
    lodging.restructure("add", Op::Insert, 30u32, |v| {
        v.push(Lodging {
            id: 30,
            name: "Hostel".into(),
            confirmed: false,
            notes: String::new(),
            trip: One::to(3u32),
        })
    });
    assert_eq!(
        trip_ids(&c, Trip::lodging().all(Lodging::confirmed().eq(true))),
        [2]
    );
}

#[test]
fn membership_questions_read_no_related_row() {
    let c = travel();
    assert_eq!(trip_ids(&c, Trip::lodging().is_empty()), [3]);
    assert_eq!(trip_ids(&c, Trip::lodging().count_ge(2)), [1]);
    assert_eq!(trip_ids(&c, Trip::lodging().count_ge(1)), [1, 2]);
    assert_eq!(trip_ids(&c, Trip::lodging().count_ge(0)), [1, 2, 3]);
}

#[test]
fn a_relation_predicate_composes_like_any_other() {
    let c = travel();
    assert_eq!(
        trip_ids(
            &c,
            Trip::lodging().any(Lodging::confirmed().eq(true)) & Trip::done().eq(false),
        ),
        [1, 2]
    );
    assert_eq!(trip_ids(&c, !Trip::lodging().is_empty()), [1, 2]);
}

// --- the child side ------------------------------------------------------------------------

#[test]
fn a_to_one_reference_traverses_to_its_target() {
    let c = travel();
    let ids: Vec<u64> = c
        .query::<Lodging>()
        .filter(Lodging::trip().any(Trip::name().eq("Kyoto".to_string())))
        .sort(Lodging::id().asc())
        .live()
        .ids()
        .iter()
        .map(|i| i.handle())
        .collect();
    assert_eq!(ids, [10, 11]);

    let none: Vec<u64> = c
        .query::<Lodging>()
        .filter(Lodging::trip().none(Trip::name().eq("Kyoto".to_string())))
        .live()
        .ids()
        .iter()
        .map(|i| i.handle())
        .collect();
    assert_eq!(none, [12]);
}

// --- the counting claims --------------------------------------------------------------------

#[test]
fn a_related_column_the_predicate_never_reads_costs_nothing() {
    // THE tier this feature exists to stay in. `notes` is a column of the related table that
    // the predicate does not mention; editing it must re-run no SQL for this query.
    let (c, trace) = traced_travel();
    let q = c
        .query::<Trip>()
        .filter(Trip::lodging().any(Lodging::confirmed().eq(true)))
        .live();
    assert_eq!(q.count(), 2);
    let baseline = requeries(&trace);

    let lodging = c.cache::<Lodging>();
    for _ in 0..50 {
        lodging.elem(10).notes().write("edited".into());
        lodging.elem(11).notes().write("edited".into());
    }
    assert_eq!(q.count(), 2);
    assert_eq!(
        requeries(&trace),
        baseline,
        "a related column outside the dependency set must cost zero requeries"
    );
}

#[test]
fn a_related_predicate_column_moves_exactly_one_row() {
    let c = travel();
    let q = c
        .query::<Trip>()
        .filter(Trip::lodging().any(Lodging::confirmed().eq(true)))
        .sort(Trip::id().asc())
        .live();
    assert_eq!(
        q.ids().iter().map(|i| i.handle()).collect::<Vec<_>>(),
        [1, 2]
    );
    let _ = q.take_events();

    // Unconfirming Oslo's only lodging takes Oslo out — one requery, one precise delta.
    c.cache::<Lodging>().elem(12).confirmed().write(false);
    assert_eq!(q.ids().iter().map(|i| i.handle()).collect::<Vec<_>>(), [1]);
    assert!(
        matches!(
            q.take_events(),
            day_persistence::QueryEvents::Deltas(d) if d.len() == 1
        ),
        "and the consumer is told which row left, not to reload"
    );
}

#[test]
fn a_local_column_outside_the_fetch_still_costs_nothing() {
    // The original tier, unbroken by the relation machinery.
    let (c, trace) = traced_travel();
    let q = c
        .query::<Trip>()
        .filter(Trip::lodging().any(Lodging::confirmed().eq(true)))
        .live();
    assert_eq!(q.count(), 2);
    let baseline = requeries(&trace);
    c.cache::<Trip>().elem(1).name().write("Kyōto".into());
    assert_eq!(q.count(), 2);
    assert_eq!(requeries(&trace), baseline);
}

// --- membership changes -----------------------------------------------------------------

#[test]
fn reparenting_a_child_moves_both_parents() {
    let c = travel();
    let q = c
        .query::<Trip>()
        .filter(Trip::lodging().any(Lodging::name().contains("Fjord")))
        .sort(Trip::id().asc())
        .live();
    assert_eq!(q.ids().iter().map(|i| i.handle()).collect::<Vec<_>>(), [2]);

    // The cabin moves to Kyoto: Oslo leaves the set, Kyoto enters it.
    c.cache::<Lodging>().elem(12).trip().write(One::to(1u32));
    assert_eq!(q.ids().iter().map(|i| i.handle()).collect::<Vec<_>>(), [1]);
}

#[test]
fn inserting_and_deleting_a_related_row_moves_its_parent() {
    let c = travel();
    let q = c
        .query::<Trip>()
        .filter(Trip::lodging().is_empty())
        .sort(Trip::id().asc())
        .live();
    assert_eq!(q.ids().iter().map(|i| i.handle()).collect::<Vec<_>>(), [3]);

    // Lima gains a lodging and leaves the "nothing booked" set.
    let lodging = c.cache::<Lodging>();
    lodging.restructure("add", Op::Insert, 31u32, |v| {
        v.push(Lodging {
            id: 31,
            name: "Hostal".into(),
            confirmed: false,
            notes: String::new(),
            trip: One::to(3u32),
        })
    });
    assert!(q.ids().is_empty());

    // …and comes back when it loses it. The deletion has to resolve to its parent BEFORE the
    // relation index forgets the row, which is the ordering hazard this pins.
    lodging.restructure("remove", Op::Delete, 31u32, |v| {
        v.remove(31);
    });
    assert_eq!(q.ids().iter().map(|i| i.handle()).collect::<Vec<_>>(), [3]);
}

#[test]
fn a_cascade_leaves_no_stale_ids_in_a_relation_filtered_set() {
    let c = travel();
    let q = c
        .query::<Trip>()
        .filter(Trip::lodging().count_ge(1))
        .sort(Trip::id().asc())
        .live();
    assert_eq!(
        q.ids().iter().map(|i| i.handle()).collect::<Vec<_>>(),
        [1, 2]
    );

    // Deleting Kyoto cascades its two lodgings away; the set must lose Kyoto and keep Oslo.
    c.cache::<Trip>()
        .restructure("delete", Op::Delete, 1u32, |v| {
            v.remove(1);
        });
    assert_eq!(q.ids().iter().map(|i| i.handle()).collect::<Vec<_>>(), [2]);
}

// --- many-to-many, both directions --------------------------------------------------------

struct Notes {
    c: ModelContainer,
    ids: Vec<Uuid>,
}

fn tagged() -> Notes {
    let c = ModelContainer::open(Sqlite::memory(), schema![Tag, Note]).expect("open");
    let tags = c.cache::<Tag>();
    for (id, name) in [(1u32, "rust"), (2, "design"), (3, "archive")] {
        tags.restructure("add", Op::Insert, id, |v| {
            v.push(Tag {
                id,
                name: name.into(),
                ..Default::default()
            })
        });
    }
    let notes = c.cache::<Note>();
    let ids: Vec<Uuid> = (0..3).map(|_| Uuid::now_v7()).collect();
    for (i, id) in ids.iter().enumerate() {
        notes.restructure("add", Op::Insert, *id, |v| {
            v.push(Note {
                id: *id,
                title: if i == 0 {
                    "Draft one".into()
                } else {
                    format!("Note {i}")
                },
                ..Default::default()
            })
        });
    }
    tags.elem(1).notes().add(ids[0]);
    tags.elem(1).notes().add(ids[1]);
    tags.elem(2).notes().add(ids[0]);
    c.save().expect("seed");
    Notes { c, ids }
}

#[test]
fn a_join_traverses_from_either_side() {
    let app = tagged();
    // Notes carrying a given tag.
    let notes: Vec<u64> = app
        .c
        .query::<Note>()
        .filter(Note::tags().any(Tag::name().eq("design".to_string())))
        .live()
        .ids()
        .iter()
        .map(|i| i.handle())
        .collect();
    assert_eq!(notes, [ModelId::<Note>::of(app.ids[0]).handle()]);

    // Tags on a matching note — the same memberships, read the other way.
    let tags: Vec<u64> = app
        .c
        .query::<Tag>()
        .filter(Tag::notes().any(Note::title().starts_with("Draft")))
        .sort(Tag::id().asc())
        .live()
        .ids()
        .iter()
        .map(|i| i.handle())
        .collect();
    assert_eq!(tags, [1, 2]);
}

#[test]
fn a_join_reports_emptiness_from_either_side() {
    let app = tagged();
    let untagged: Vec<u64> = app
        .c
        .query::<Note>()
        .filter(Note::tags().is_empty())
        .live()
        .ids()
        .iter()
        .map(|i| i.handle())
        .collect();
    assert_eq!(untagged, [ModelId::<Note>::of(app.ids[2]).handle()]);

    let unused: Vec<u64> = app
        .c
        .query::<Tag>()
        .filter(Tag::notes().is_empty())
        .live()
        .ids()
        .iter()
        .map(|i| i.handle())
        .collect();
    assert_eq!(unused, [3]);
}

#[test]
fn linking_and_unlinking_move_the_set() {
    let app = tagged();
    let q = app
        .c
        .query::<Tag>()
        .filter(Tag::notes().any(Note::title().starts_with("Draft")))
        .sort(Tag::id().asc())
        .live();
    assert_eq!(
        q.ids().iter().map(|i| i.handle()).collect::<Vec<_>>(),
        [1, 2]
    );

    app.c.cache::<Tag>().elem(3).notes().add(app.ids[0]);
    assert_eq!(
        q.ids().iter().map(|i| i.handle()).collect::<Vec<_>>(),
        [1, 2, 3]
    );

    app.c.cache::<Tag>().elem(1).notes().remove(app.ids[0]);
    assert_eq!(
        q.ids().iter().map(|i| i.handle()).collect::<Vec<_>>(),
        [2, 3]
    );
}

#[test]
fn a_join_edit_outside_the_result_leaves_it_unchanged() {
    let app = tagged();
    let q = app
        .c
        .query::<Tag>()
        .filter(Tag::notes().any(Note::title().starts_with("Draft")))
        .sort(Tag::id().asc())
        .live();
    assert_eq!(
        q.ids().iter().map(|i| i.handle()).collect::<Vec<_>>(),
        [1, 2]
    );
    // Editing a dependency column of an untagged note re-answers (one indexed EXISTS) and
    // changes nothing.
    app.c
        .cache::<Note>()
        .elem(app.ids[2])
        .title()
        .write("Untagged, still".into());
    assert_eq!(
        q.ids().iter().map(|i| i.handle()).collect::<Vec<_>>(),
        [1, 2]
    );
}

// --- the tree ------------------------------------------------------------------------------

#[test]
fn a_self_referential_relation_traverses_both_ways() {
    let c = ModelContainer::open(Sqlite::memory(), schema![Node]).expect("open");
    let nodes = c.cache::<Node>();
    for (id, name, parent) in [
        (1u32, "root", None),
        (2, "group", Some(1u32)),
        (3, "leaf", Some(2u32)),
        (4, "orphan", None),
    ] {
        nodes.restructure("add", Op::Insert, id, |v| {
            v.push(Node {
                id,
                name: name.into(),
                parent: parent.map(One::to),
                ..Default::default()
            })
        });
    }
    c.save().expect("seed");

    let ids = |p: day_persistence::Pred| -> Vec<u64> {
        c.query::<Node>()
            .filter(p)
            .sort(Node::id().asc())
            .live()
            .ids()
            .iter()
            .map(|i| i.handle())
            .collect()
    };
    // Parents of a named child.
    assert_eq!(
        ids(Node::children().any(Node::name().eq("leaf".to_string()))),
        [2]
    );
    // Children of a named parent — the same relation, the other direction.
    assert_eq!(
        ids(Node::parent().any(Node::name().eq("root".to_string()))),
        [2]
    );
    // Leaves.
    assert_eq!(ids(Node::children().is_empty()), [3, 4]);
}

// --- the honest limits -----------------------------------------------------------------------

#[test]
fn nesting_compiles_to_nested_exists_and_stays_live() {
    let c = ModelContainer::open(Sqlite::memory(), schema![Node]).expect("open");
    let nodes = c.cache::<Node>();
    for (id, name, parent) in [
        (1u32, "root", None),
        (2, "group", Some(1u32)),
        (3, "leaf", Some(2u32)),
    ] {
        nodes.restructure("add", Op::Insert, id, |v| {
            v.push(Node {
                id,
                name: name.into(),
                parent: parent.map(One::to),
                ..Default::default()
            })
        });
    }
    c.save().expect("seed");

    // Two hops: "nodes whose parent's parent is named root".
    let q = c
        .query::<Node>()
        .filter(Node::parent().any(Node::parent().any(Node::name().eq("root".to_string()))))
        .live();
    assert_eq!(
        q.ids().iter().map(|i| i.handle()).collect::<Vec<_>>(),
        [3],
        "evaluation handles any depth"
    );

    // …and the fetch names EVERY level it crosses, so a change at any depth marks it stale.
    let deps = day_persistence::Fetch::new()
        .filter(Node::parent().any(Node::parent().any(Node::name().eq("root".to_string()))))
        .dependencies();
    assert_eq!(deps.related.len(), 2, "both hops are dependencies");
    assert_eq!(deps.related_tables(), ["nodes"]);

    // The result stays correct across a change either way.
    c.cache::<Node>().elem(1).name().write("trunk".into());
    assert!(q.ids().is_empty());
}

#[test]
fn a_relation_predicate_compiles_to_a_correlated_exists() {
    let (driver, log) = day_persistence::Recorder::new();
    let c = ModelContainer::open(driver, schema![Trip, Lodging]).expect("open");
    log.clear();
    let _q = c
        .query::<Trip>()
        .filter(Trip::lodging().any(Lodging::confirmed().eq(true)))
        .live();
    let sql = log
        .sql()
        .into_iter()
        .rev()
        .find(|s| s.starts_with("SELECT trips.id FROM trips"))
        .expect("compiled");
    assert!(sql.contains("EXISTS (SELECT 1 FROM lodging AS"), "{sql}");
    assert!(
        sql.contains(".trip = trips.id"),
        "correlated on the fk: {sql}"
    );
}

#[test]
fn the_dependency_set_names_the_related_table_and_its_columns() {
    let deps = day_persistence::Fetch::new()
        .filter(Trip::lodging().any(Lodging::confirmed().eq(true)))
        .sort(Trip::id().asc())
        .dependencies();

    assert_eq!(deps.related_tables(), ["lodging"]);
    assert!(deps.touches_related("lodging", "confirmed"));
    assert!(
        !deps.touches_related("lodging", "notes"),
        "a related column the predicate never reads is not a dependency"
    );
    assert!(deps.touches_local("id"), "the sort key is still local");
    assert!(
        !deps.touches_local("confirmed"),
        "that column belongs to the OTHER table"
    );
}
