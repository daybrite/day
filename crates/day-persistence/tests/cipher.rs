// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The SQLCipher build (`--features cipher`): encrypted at rest, wrong keys refused at open,
//! and the encrypt_to → open → decrypt_to lifecycle round-trips. Compiles empty otherwise.

#![cfg(feature = "cipher")]

use day_macros::Model;
use day_model::Op;
use day_persistence::{DbErrorKind, ModelContainer, Secret, Sqlite, schema};
use day_reactive::Binding;

#[derive(Model, Clone, Default, PartialEq, Debug)]
#[model(table = "notes")]
struct Note {
    #[model(id)]
    id: u32,
    title: String,
}

fn temp_db(name: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "day-persistence-cipher-{}-{}.sqlite",
        std::process::id(),
        name
    ));
    let _ = std::fs::remove_file(&path);
    path
}

fn add_note(container: &ModelContainer, id: u32, title: &str) {
    container
        .cache::<Note>()
        .restructure("add", Op::Insert, id as u64, |v| {
            v.push(Note {
                id,
                title: title.into(),
            });
        });
    container.save().expect("save");
}

#[test]
fn an_encrypted_file_opens_with_its_key_and_refuses_others() {
    let path = temp_db("keyed");
    {
        let driver = Sqlite::at(&path).key(Secret::new("correct horse"));
        let container = ModelContainer::open(driver, schema![Note]).expect("open encrypted");
        assert!(container.capabilities().encryption);
        add_note(&container, 1, "secret");
        container.checkpoint().expect("checkpoint");
    }
    // The bytes on disk are not a plaintext SQLite file.
    let head = std::fs::read(&path).expect("read file");
    assert!(
        !head.starts_with(b"SQLite format 3"),
        "the file is encrypted at rest"
    );
    // Wrong key: refused at open, with the kind an app can act on.
    let wrong = ModelContainer::open(Sqlite::at(&path).key(Secret::new("wrong")), schema![Note]);
    assert_eq!(wrong.err().expect("refused").kind, DbErrorKind::BadKey);
    // No key at all: same refusal (the driver probes before the schema layer touches it).
    let none = ModelContainer::open(Sqlite::at(&path), schema![Note]);
    assert_eq!(none.err().expect("refused").kind, DbErrorKind::BadKey);
    // The right key still works.
    let container = ModelContainer::open(
        Sqlite::at(&path).key(Secret::new("correct horse")),
        schema![Note],
    )
    .expect("reopen");
    assert_eq!(container.cache::<Note>().elem(1).title().peek(), "secret");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn encrypt_open_decrypt_round_trips() {
    let plain = temp_db("plain");
    let encrypted = temp_db("encrypted");
    let decrypted = temp_db("decrypted");

    // A plaintext database (cipher builds open those too — an empty key is plaintext)…
    {
        let container = ModelContainer::open(Sqlite::at(&plain), schema![Note]).expect("open");
        add_note(&container, 1, "travels");
        // …exported encrypted…
        container
            .encrypt_to(&encrypted, Secret::new("k1"))
            .expect("encrypt_to");
    }
    // …opens with the key…
    {
        let container =
            ModelContainer::open(Sqlite::at(&encrypted).key(Secret::new("k1")), schema![Note])
                .expect("open encrypted copy");
        assert_eq!(container.cache::<Note>().elem(1).title().peek(), "travels");
        // …and comes back out as plaintext.
        container.decrypt_to(&decrypted).expect("decrypt_to");
    }
    let head = std::fs::read(&decrypted).expect("read");
    assert!(head.starts_with(b"SQLite format 3"), "plaintext again");
    let container =
        ModelContainer::open(Sqlite::at(&decrypted), schema![Note]).expect("open decrypted");
    assert_eq!(container.cache::<Note>().elem(1).title().peek(), "travels");

    for p in [plain, encrypted, decrypted] {
        let _ = std::fs::remove_file(&p);
    }
}

#[test]
fn rekey_changes_the_key_in_place() {
    let path = temp_db("rekey");
    {
        let container =
            ModelContainer::open(Sqlite::at(&path).key(Secret::new("old key")), schema![Note])
                .expect("open");
        add_note(&container, 1, "kept");
        container.rekey(Secret::new("new key")).expect("rekey");
    }
    assert_eq!(
        ModelContainer::open(Sqlite::at(&path).key(Secret::new("old key")), schema![Note])
            .err()
            .expect("old key dead")
            .kind,
        DbErrorKind::BadKey
    );
    let container =
        ModelContainer::open(Sqlite::at(&path).key(Secret::new("new key")), schema![Note])
            .expect("new key works");
    assert_eq!(container.cache::<Note>().elem(1).title().peek(), "kept");
    let _ = std::fs::remove_file(&path);
}
