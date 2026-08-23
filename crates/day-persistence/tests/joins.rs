// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Many-to-many: a generated join table whose rows are keyed by the PAIR, so memberships
//! fold to SQL, undo, merge and animate through the same machinery every other row uses.

use day_macros::Model;
use day_model::{Op, Uuid};
use day_persistence::{
    DbErrorKind, Many, ModelContainer, Sqlite, SqliteConnection, SqliteDriver, Value, schema,
};

// A note-taking app: notes carry tags, tags carry notes, and neither owns the other.
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

// A course catalog, ordered: a syllabus lists its readings in a chosen order, and the same
// reading sits at different positions in different courses.
#[derive(Model, Clone, Default, PartialEq, Debug)]
#[model(table = "courses")]
struct Course {
    #[model(id)]
    id: u32,
    code: String,
    #[model(relation(target = Reading, join = "course_readings", ordered))]
    readings: Many<Reading>,
}

#[derive(Model, Clone, Default, PartialEq, Debug)]
#[model(table = "readings")]
struct Reading {
    #[model(id)]
    id: u32,
    title: String,
}

fn temp_db(name: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "day-persistence-join-{}-{}.sqlite",
        std::process::id(),
        name
    ));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
    let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    path
}

struct Notes {
    c: ModelContainer,
    ids: Vec<Uuid>,
}

fn notes_app(path: Option<&std::path::Path>) -> Notes {
    let driver = match path {
        Some(p) => Sqlite::at(p),
        None => Sqlite::memory(),
    };
    let c = ModelContainer::open(driver, schema![Tag, Note]).expect("open");
    let tags = c.store::<Tag>();
    for (id, name) in [(1, "rust"), (2, "design"), (3, "archive")] {
        tags.restructure("add", Op::Insert, id, |v| {
            v.push(Tag {
                id: id as u32,
                name: name.into(),
                ..Default::default()
            })
        });
    }
    let notes = c.store::<Note>();
    let ids: Vec<Uuid> = (0..3).map(|_| Uuid::now_v7()).collect();
    for (i, id) in ids.iter().enumerate() {
        notes.restructure("add", Op::Insert, *id, |v| {
            v.push(Note {
                id: *id,
                title: format!("note {i}"),
                ..Default::default()
            })
        });
    }
    c.save().expect("seed");
    Notes { c, ids }
}

#[test]
fn membership_reads_from_both_sides() {
    let app = notes_app(None);
    let tags = app.c.store::<Tag>();
    let notes = app.c.store::<Note>();

    assert!(tags.elem(1).notes().add(app.ids[0]));
    assert!(tags.elem(1).notes().add(app.ids[1]));
    assert!(notes.elem(app.ids[0]).tags().add(2u32));

    // Both directions answer from the same memberships.
    assert_eq!(tags.elem(1).notes().count(), 2);
    assert_eq!(notes.elem(app.ids[0]).tags().count(), 2, "rust + design");
    assert_eq!(notes.elem(app.ids[1]).tags().count(), 1);
    assert!(notes.elem(app.ids[0]).tags().contains(1u32));
    assert!(tags.elem(2).notes().contains(app.ids[0]));
    assert!(tags.elem(3).notes().is_empty());
}

#[test]
fn membership_is_a_set_and_unlinking_is_precise() {
    let app = notes_app(None);
    let tags = app.c.store::<Tag>();

    assert!(tags.elem(1).notes().add(app.ids[0]));
    assert!(
        !tags.elem(1).notes().add(app.ids[0]),
        "adding twice is not a second membership"
    );
    assert_eq!(tags.elem(1).notes().count(), 1);

    assert!(tags.elem(1).notes().remove(app.ids[0]));
    assert!(!tags.elem(1).notes().remove(app.ids[0]), "already gone");
    assert!(tags.elem(1).notes().is_empty());
}

#[test]
fn linking_folds_to_one_insert_and_unlinking_to_one_delete() {
    let app = notes_app(None);
    let tags = app.c.store::<Tag>();

    let sql = app
        .c
        .record_sql(|| {
            tags.elem(1).notes().add(app.ids[0]);
        })
        .expect("flush");
    assert_eq!(sql.len(), 1, "{sql:?}");
    assert!(
        sql[0].starts_with("INSERT INTO note_tags (tag_id, note_id)"),
        "{sql:?}"
    );

    let sql = app
        .c
        .record_sql(|| {
            tags.elem(1).notes().remove(app.ids[0]);
        })
        .expect("flush");
    assert_eq!(
        sql,
        ["DELETE FROM note_tags WHERE tag_id = ? AND note_id = ?"]
    );
}

#[test]
fn memberships_survive_a_reopen() {
    let path = temp_db("reopen");
    let ids = {
        let app = notes_app(Some(&path));
        let tags = app.c.store::<Tag>();
        tags.elem(1).notes().add(app.ids[0]);
        tags.elem(1).notes().add(app.ids[2]);
        tags.elem(2).notes().add(app.ids[0]);
        app.c.save().expect("save");
        app.ids
    };
    {
        let c = ModelContainer::open(Sqlite::at(&path), schema![Tag, Note]).expect("reopen");
        assert_eq!(c.store::<Tag>().elem(1).notes().count(), 2);
        assert!(c.store::<Note>().elem(ids[0]).tags().contains(2u32));
        assert!(c.store::<Note>().elem(ids[1]).tags().is_empty());
    }
    let _ = std::fs::remove_file(&path);
}

#[test]
fn the_join_table_is_readable_and_indexed() {
    let path = temp_db("ddl");
    {
        let app = notes_app(Some(&path));
        app.c.store::<Tag>().elem(1).notes().add(app.ids[0]);
        app.c.save().expect("save");
    }
    let conn = rusqlite::Connection::open(&path).expect("raw open");
    // The membership is an ordinary row any tool can read: the tag's INTEGER, the note's BLOB.
    let (tag, kind): (i64, String) = conn
        .query_row("SELECT tag_id, typeof(note_id) FROM note_tags", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .expect("the membership row");
    assert_eq!((tag, kind.as_str()), (1, "blob"));

    // Both directions are indexed: the pair is the primary key, the reverse side its own index.
    let idx: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type = 'index' AND tbl_name = 'note_tags'",
            [],
            |r| r.get(0),
        )
        .expect("index count");
    assert!(idx >= 1, "the reverse index exists");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn deleting_either_side_drops_its_memberships() {
    let app = notes_app(None);
    let tags = app.c.store::<Tag>();
    let notes = app.c.store::<Note>();
    tags.elem(1).notes().add(app.ids[0]);
    tags.elem(1).notes().add(app.ids[1]);
    tags.elem(2).notes().add(app.ids[0]);

    // Delete the note: it leaves both tags, and the tags' own rows survive.
    notes.restructure("delete", Op::Delete, app.ids[0], |k| {
        k.remove(day_model::ModelId::<Note>::of(app.ids[0]).handle());
    });
    assert_eq!(tags.elem(1).notes().ids().len(), 1);
    assert!(tags.elem(2).notes().is_empty());
    assert!(
        tags.with_untracked(|k| k.get(2).is_some()),
        "the tag stayed"
    );

    // Delete the tag: the remaining note survives, now untagged.
    tags.restructure("delete", Op::Delete, 1, |k| {
        k.remove(1);
    });
    assert!(notes.with_untracked(|k| k.len() == 2));
    assert!(notes.elem(app.ids[1]).tags().is_empty());
}

#[test]
fn a_membership_is_undoable_like_any_other_row() {
    let app = notes_app(None);
    let undo = app.c.undo(10);
    let tags = app.c.store::<Tag>();

    tags.elem(1).notes().add(app.ids[0]);
    day_reactive::flush_sync();
    assert_eq!(tags.elem(1).notes().count(), 1);

    assert!(undo.undo(), "linking sealed a unit");
    assert!(tags.elem(1).notes().is_empty(), "the membership came out");
    assert!(undo.redo(), "and goes back in");
    assert_eq!(tags.elem(1).notes().count(), 1);
}

#[test]
fn another_connections_membership_merges() {
    let path = temp_db("external");
    let app = notes_app(Some(&path));
    let tags = app.c.store::<Tag>();
    tags.elem(1).notes().add(app.ids[0]);
    app.c.save().expect("save");

    {
        let mut other = Sqlite::at(&path).open().expect("second connection");
        other
            .execute(
                "INSERT INTO note_tags (tag_id, note_id) VALUES (?, ?)",
                &[Value::Int(2), Value::Blob(app.ids[1].as_bytes().to_vec())],
            )
            .expect("external link");
        other
            .execute("DELETE FROM note_tags WHERE tag_id = 1", &[])
            .expect("external unlink");
    }
    assert!(app.c.check_external().expect("check"));
    assert!(
        tags.elem(1).notes().is_empty(),
        "the external unlink merged"
    );
    assert!(
        tags.elem(2).notes().contains(app.ids[1]),
        "the external link merged"
    );
    let _ = std::fs::remove_file(&path);
}

// --- ordered many-to-many ------------------------------------------------------------------

fn catalog() -> ModelContainer {
    let c = ModelContainer::open(Sqlite::memory(), schema![Course, Reading]).expect("open");
    let courses = c.store::<Course>();
    for (id, code) in [(1, "CS101"), (2, "CS201")] {
        courses.restructure("add", Op::Insert, id, |v| {
            v.push(Course {
                id: id as u32,
                code: code.into(),
                ..Default::default()
            })
        });
    }
    let readings = c.store::<Reading>();
    for (id, title) in [(10, "Structure"), (11, "Interpretation"), (12, "Programs")] {
        readings.restructure("add", Op::Insert, id, |v| {
            v.push(Reading {
                id: id as u32,
                title: title.into(),
            })
        });
    }
    c.save().expect("seed");
    c
}

#[test]
fn an_ordered_join_keeps_per_parent_order() {
    let c = catalog();
    let courses = c.store::<Course>();

    for r in [10u32, 11, 12] {
        courses.elem(1).readings().add(r);
    }
    // The same readings, a different order, under another course.
    for r in [12u32, 10, 11] {
        courses.elem(2).readings().add(r);
    }
    assert_eq!(courses.elem(1).readings().ids(), [10, 11, 12]);
    assert_eq!(courses.elem(2).readings().ids(), [12, 10, 11]);

    // Reordering one course leaves the other alone — the position is the membership's.
    c.save().expect("seed");
    let sql = c
        .record_sql(|| {
            assert!(courses.elem(1).readings().move_to(12u32, 0));
        })
        .expect("flush");
    assert_eq!(courses.elem(1).readings().ids(), [12, 10, 11]);
    assert_eq!(courses.elem(2).readings().ids(), [12, 10, 11], "untouched");
    assert_eq!(sql.len(), 1, "one membership moved: {sql:?}");
    assert!(sql[0].starts_with("UPDATE course_readings SET position = ?"));
}

#[test]
fn an_ordered_join_survives_a_reopen_in_order() {
    let path = temp_db("ordered");
    {
        let c = ModelContainer::open(Sqlite::at(&path), schema![Course, Reading]).expect("open");
        let courses = c.store::<Course>();
        courses.restructure("add", Op::Insert, 1, |v| {
            v.push(Course {
                id: 1,
                code: "CS101".into(),
                ..Default::default()
            })
        });
        let readings = c.store::<Reading>();
        for id in [10u32, 11, 12] {
            readings.restructure("add", Op::Insert, id, |v| {
                v.push(Reading {
                    id,
                    title: format!("r{id}"),
                })
            });
            courses.elem(1).readings().add(id);
        }
        courses.elem(1).readings().move_to(12u32, 0);
        c.save().expect("save");
    }
    {
        let c = ModelContainer::open(Sqlite::at(&path), schema![Course, Reading]).expect("reopen");
        assert_eq!(c.store::<Course>().elem(1).readings().ids(), [12, 10, 11]);
    }
    let _ = std::fs::remove_file(&path);
}

#[test]
fn insert_at_places_a_new_member_directly() {
    let c = catalog();
    let courses = c.store::<Course>();
    courses.elem(1).readings().add(10u32);
    courses.elem(1).readings().add(11u32);

    // A reading that is not yet a member joins AT the index in one call.
    assert!(courses.elem(1).readings().insert_at(12u32, 1));
    assert_eq!(courses.elem(1).readings().ids(), [10, 12, 11]);
}

#[test]
fn an_unordered_join_refuses_a_move() {
    let app = notes_app(None);
    let tags = app.c.store::<Tag>();
    tags.elem(1).notes().add(app.ids[0]);
    tags.elem(1).notes().add(app.ids[1]);
    assert!(
        !tags.elem(1).notes().move_to(app.ids[1], 0),
        "an unordered relation has no order to write"
    );
}

#[test]
fn clear_unlinks_every_membership_of_one_parent() {
    let app = notes_app(None);
    let tags = app.c.store::<Tag>();
    let notes = app.c.store::<Note>();
    for id in &app.ids {
        tags.elem(1).notes().add(*id);
        tags.elem(2).notes().add(*id);
    }
    tags.elem(1).notes().clear();
    assert!(tags.elem(1).notes().is_empty());
    assert_eq!(
        tags.elem(2).notes().count(),
        3,
        "the other tag is untouched"
    );
    assert!(notes.elem(app.ids[0]).tags().contains(2u32));
}

#[test]
fn a_join_deny_refuses_while_memberships_remain() {
    #[derive(Model, Clone, Default, PartialEq, Debug)]
    #[model(table = "shelves")]
    struct Shelf {
        #[model(id)]
        id: u32,
        #[model(relation(target = Title, join = "shelf_titles", delete = "deny"))]
        titles: Many<Title>,
    }
    #[derive(Model, Clone, Default, PartialEq, Debug)]
    #[model(table = "titles")]
    struct Title {
        #[model(id)]
        id: u32,
        name: String,
    }

    let c = ModelContainer::open(Sqlite::memory(), schema![Shelf, Title]).expect("open");
    c.store::<Shelf>()
        .restructure("add", Op::Insert, 1, |v| v.push(Shelf::default()));
    c.store::<Title>().restructure("add", Op::Insert, 5, |v| {
        v.push(Title {
            id: 5,
            name: "Dune".into(),
        })
    });
    c.store::<Shelf>().elem(1).titles().add(5u32);

    let err = c
        .delete::<Shelf>(1u32)
        .expect_err("still holds memberships");
    assert_eq!(err.kind, DbErrorKind::Deny);

    c.store::<Shelf>().elem(1).titles().clear();
    c.delete::<Shelf>(1u32).expect("empty now");
}

#[test]
fn a_join_cascade_deletes_only_the_children_nobody_else_holds() {
    #[derive(Model, Clone, Default, PartialEq, Debug)]
    #[model(table = "albums")]
    struct Album {
        #[model(id)]
        id: u32,
        #[model(relation(target = Photo, join = "album_photos", delete = "cascade"))]
        photos: Many<Photo>,
    }
    #[derive(Model, Clone, Default, PartialEq, Debug)]
    #[model(table = "photos")]
    struct Photo {
        #[model(id)]
        id: u32,
        caption: String,
    }

    let c = ModelContainer::open(Sqlite::memory(), schema![Album, Photo]).expect("open");
    let albums = c.store::<Album>();
    let photos = c.store::<Photo>();
    for id in [1u32, 2] {
        albums.restructure("add", Op::Insert, id, |v| {
            v.push(Album {
                id,
                ..Default::default()
            })
        });
    }
    for id in [10u32, 11] {
        photos.restructure("add", Op::Insert, id, |v| {
            v.push(Photo {
                id,
                caption: format!("p{id}"),
            })
        });
    }
    albums.elem(1).photos().add(10u32);
    albums.elem(1).photos().add(11u32);
    albums.elem(2).photos().add(11u32); // shared with the second album

    albums.restructure("delete", Op::Delete, 1, |k| {
        k.remove(1);
    });
    assert!(
        photos.with_untracked(|k| k.get(10).is_none()),
        "held only by the deleted album"
    );
    assert!(
        photos.with_untracked(|k| k.get(11).is_some()),
        "still in album 2 — a shared photo is not collateral"
    );
    assert_eq!(albums.elem(2).photos().ids(), [11]);
}

#[test]
fn join_relations_are_declared_data() {
    use day_persistence::Model as _;
    assert_eq!(Tag::RELATIONS[0].join, Some("note_tags"));
    assert_eq!(Tag::RELATIONS[0].ordered, None);
    assert_eq!(Course::RELATIONS[0].ordered, Some("position"));
    assert_eq!(Note::RELATIONS[0].join, Some("note_tags"));
}
