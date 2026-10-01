// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
#![cfg(not(target_arch = "wasm32"))]
use day_macros::Model;
use day_persistence::{
    DatabaseWorker, DbError, DbErrorKind, ModelContainer, Sqlite, WorkerOptions, schema,
};
use day_reactive::Binding;
use std::{
    future::Future,
    sync::{Arc, Barrier, mpsc},
    task::{Context, Poll, Wake, Waker},
    time::{Duration, Instant},
};

#[derive(Model, Clone, Default, PartialEq, Debug)]
#[model(table = "notes", fts("title"))]
struct Note {
    #[model(id)]
    id: u64,
    #[model(unique)]
    title: String,
    read: bool,
}
fn block_on<F: Future>(f: F) -> F::Output {
    struct Unpark(std::thread::Thread);
    impl Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let w = Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut cx = Context::from_waker(&w);
    let mut f = std::pin::pin!(f);
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
        assert!(Instant::now() < deadline, "worker future never completed");
        std::thread::park_timeout(Duration::from_millis(20));
    }
}
fn open() -> DatabaseWorker {
    block_on(DatabaseWorker::open(|| {
        ModelContainer::open(Sqlite::memory(), schema![Note])
    }))
    .unwrap()
}
fn note(id: u64) -> Note {
    Note {
        id,
        title: format!("Synthetic note {id}"),
        read: false,
    }
}
fn count(w: &DatabaseWorker) -> u64 {
    block_on(w.read(|db| db.table_count::<Note>())).unwrap()
}

#[test]
fn open_queries_mutations_and_drop_stay_on_the_owning_thread() {
    let ui = std::thread::current().id();
    let w = block_on(DatabaseWorker::open(move || {
        assert_ne!(ui, std::thread::current().id());
        ModelContainer::open(Sqlite::memory(), schema![Note])
    }))
    .unwrap();
    let thread = block_on(w.write(|db| {
        db.insert(note(1));
        Ok(std::thread::current().id())
    }))
    .unwrap();
    assert_ne!(ui, thread);
    assert_eq!(
        thread,
        block_on(w.read(|_| Ok(std::thread::current().id()))).unwrap()
    );
    assert_eq!(count(&w), 1);
    block_on(w.close()).unwrap();
}
#[test]
fn fifo_and_read_your_writes_work_without_awaiting_each_submission() {
    let w = open();
    let mut pending = vec![];
    for id in 0..100 {
        pending.push(w.write(move |db| {
            db.insert(note(id));
            Ok(())
        }));
    }
    let counted = w.read(|db| db.table_count::<Note>());
    for r in pending {
        block_on(r).unwrap();
    }
    assert_eq!(block_on(counted).unwrap(), 100);
    block_on(w.close()).unwrap();
}
#[test]
fn failed_closure_rolls_back_even_its_explicit_saves_and_restores_cache() {
    let w = open();
    block_on(w.write(|db| {
        db.insert(note(1));
        Ok(())
    }))
    .unwrap();
    let e = block_on(w.write::<()>(|db| {
        db.get::<Note>(1u64)
            .unwrap()
            .title()
            .write("changed".into());
        db.insert(note(2));
        db.save()?;
        Err(DbError::driver("synthetic rejection"))
    }))
    .unwrap_err();
    assert_eq!(e.kind, DbErrorKind::Driver);
    assert_eq!(
        block_on(w.read(|db| Ok(db.get::<Note>(1u64).unwrap().title().read()))).unwrap(),
        "Synthetic note 1"
    );
    assert_eq!(count(&w), 1);
    block_on(w.write(|db| {
        db.insert(note(2));
        Ok(())
    }))
    .unwrap();
    assert_eq!(count(&w), 2);
    block_on(w.close()).unwrap();
}
#[test]
fn constraint_failure_rolls_back_every_row_then_allows_retry() {
    let w = open();
    let e = block_on(w.write(|db| {
        db.insert(note(1));
        db.insert(Note { id: 2, ..note(1) });
        Ok(())
    }))
    .unwrap_err();
    assert_eq!(e.kind, DbErrorKind::Driver);
    assert_eq!(count(&w), 0);
    block_on(w.write(|db| {
        db.insert(note(1));
        Ok(())
    }))
    .unwrap();
    assert_eq!(count(&w), 1);
    block_on(w.close()).unwrap();
}
#[test]
fn accidental_model_mutations_in_reads_are_rejected_and_undone() {
    let w = open();
    block_on(w.write(|db| {
        db.insert(note(1));
        Ok(())
    }))
    .unwrap();
    assert!(
        block_on(w.read(|db| {
            db.get::<Note>(1u64).unwrap().read().write(true);
            Ok(())
        }))
        .is_err()
    );
    assert!(!block_on(w.read(|db| Ok(db.get::<Note>(1u64).unwrap().read().read()))).unwrap());
    block_on(w.close()).unwrap();
}
#[test]
fn accidental_sql_writes_in_reads_are_rejected() {
    let w = open();
    assert!(
        block_on(w.read(|db| db.try_with_connection(|c| c.execute("DELETE FROM notes", &[]))))
            .is_err()
    );
    block_on(w.write(|db| {
        db.insert(note(1));
        Ok(())
    }))
    .unwrap();
    assert_eq!(count(&w), 1);
    block_on(w.close()).unwrap();
}
#[test]
fn dropping_accepted_write_futures_preserves_commits() {
    let w = open();
    for id in 0..100 {
        drop(w.write(move |db| {
            db.insert(note(id));
            Ok(())
        }));
    }
    assert_eq!(count(&w), 100);
    block_on(w.close()).unwrap();
}
#[test]
fn dropped_queued_reads_do_not_execute() {
    let w = open();
    let gate = Arc::new(Barrier::new(2));
    let gate2 = gate.clone();
    let (tx, rx) = mpsc::channel();
    let first = w.read(move |_| {
        tx.send(()).unwrap();
        gate2.wait();
        Ok(())
    });
    rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let ran = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let copy = ran.clone();
    drop(w.read(move |_| {
        copy.store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }));
    gate.wait();
    block_on(first).unwrap();
    block_on(w.close()).unwrap();
    assert!(!ran.load(std::sync::atomic::Ordering::SeqCst));
}
#[test]
fn bounded_admission_never_blocks_or_silently_drops_writes() {
    let w = block_on(DatabaseWorker::open_with(
        WorkerOptions {
            capacity: 1,
            ..Default::default()
        },
        || ModelContainer::open(Sqlite::memory(), schema![Note]),
    ))
    .unwrap();
    let gate = Arc::new(Barrier::new(2));
    let gate2 = gate.clone();
    let (tx, rx) = mpsc::channel();
    let first = w.read(move |_| {
        tx.send(()).unwrap();
        gate2.wait();
        Ok(())
    });
    rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let accepted = w.write(|db| {
        db.insert(note(1));
        Ok(())
    });
    assert_eq!(
        block_on(w.write(|db| {
            db.insert(note(2));
            Ok(())
        }))
        .unwrap_err()
        .kind,
        DbErrorKind::Busy
    );
    gate.wait();
    block_on(first).unwrap();
    block_on(accepted).unwrap();
    assert_eq!(count(&w), 1);
    block_on(w.close()).unwrap();
}
#[test]
fn close_drains_accepted_writes_and_rejects_later_requests_on_all_clones() {
    let w = open();
    let clone = w.clone();
    for id in 0..100 {
        drop(w.write(move |db| {
            db.insert(note(id));
            Ok(())
        }));
    }
    block_on(w.close()).unwrap();
    block_on(clone.close()).unwrap();
    assert_eq!(
        block_on(clone.read(|_| Ok(()))).unwrap_err().kind,
        DbErrorKind::Closed
    );
}
#[test]
fn panic_closes_worker_and_completes_queued_requests() {
    let w = open();
    let bad = w.write::<()>(|db| {
        db.insert(note(1));
        panic!("synthetic worker failure")
    });
    let after = w.read(|db| db.table_count::<Note>());
    assert_eq!(block_on(bad).unwrap_err().kind, DbErrorKind::Panicked);
    assert!(block_on(after).is_err());
    assert!(block_on(w.close()).is_err());
}
#[test]
fn factory_failure_and_factory_panic_complete_open() {
    assert!(
        block_on(DatabaseWorker::open(|| Err(DbError::driver(
            "synthetic open failure"
        ))))
        .is_err()
    );
    assert!(block_on(DatabaseWorker::open(|| panic!("synthetic factory panic"))).is_err());
}
#[test]
fn recursive_queue_access_is_rejected_instead_of_deadlocking() {
    let w = open();
    let clone = w.clone();
    let kind =
        block_on(w.read(move |_| Ok(block_on(clone.read(|_| Ok(()))).unwrap_err().kind))).unwrap();
    assert_eq!(kind, DbErrorKind::Driver);
    let clone = w.clone();
    assert!(block_on(w.read(move |_| block_on(clone.close()))).is_err());
    block_on(w.close()).unwrap();
}
#[test]
fn clones_serialize_concurrent_updates_without_lost_writes() {
    let w = open();
    block_on(w.write(|db| {
        db.insert(note(1));
        Ok(())
    }))
    .unwrap();
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let w = w.clone();
            std::thread::spawn(move || {
                for _ in 0..25 {
                    block_on(w.write(|db| {
                        let n = db.get::<Note>(1u64).unwrap();
                        let value = n.read().read();
                        n.read().write(!value);
                        Ok(())
                    }))
                    .unwrap();
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    assert!(!block_on(w.read(|db| Ok(db.get::<Note>(1u64).unwrap().read().read()))).unwrap());
    block_on(w.close()).unwrap();
}

#[test]
fn observations_deliver_committed_values_and_monotonic_revisions() {
    let w = open();
    let mut subscription = block_on(w.observe(|db| db.table_count::<Note>())).unwrap();
    let first = block_on(subscription.next()).unwrap().unwrap();
    assert_eq!(first.value, 0);
    block_on(w.write(|db| {
        db.insert(note(1));
        Ok(())
    }))
    .unwrap();
    let next = block_on(subscription.next()).unwrap().unwrap();
    assert_eq!(next.value, 1);
    assert!(next.revision > first.revision);
    block_on(w.close()).unwrap();
    assert!(block_on(subscription.next()).is_none());
}
#[test]
fn observations_coalesce_a_slow_consumer_and_do_not_hold_worker_alive() {
    let w = open();
    let mut sub = block_on(w.observe(|db| db.table_count::<Note>())).unwrap();
    assert_eq!(block_on(sub.next()).unwrap().unwrap().value, 0);
    for id in 0..100 {
        block_on(w.write(move |db| {
            db.insert(note(id));
            Ok(())
        }))
        .unwrap();
    }
    block_on(w.close()).unwrap();
    assert_eq!(block_on(sub.next()).unwrap().unwrap().value, 100);
    assert!(block_on(sub.next()).is_none());
}
#[test]
fn failed_transactions_never_publish_speculative_values() {
    let w = open();
    let mut sub = block_on(w.observe(|db| db.table_count::<Note>())).unwrap();
    assert_eq!(block_on(sub.next()).unwrap().unwrap().value, 0);
    assert!(
        block_on(w.write::<()>(|db| {
            db.insert(note(1));
            db.save()?;
            Err(DbError::driver("synthetic rollback"))
        }))
        .is_err()
    );
    block_on(w.close()).unwrap();
    assert!(block_on(sub.next()).is_none());
}
#[test]
fn unchanged_projections_do_not_wake_the_consumer() {
    let w = open();
    let mut sub = block_on(w.observe(|db| db.table_count::<Note>())).unwrap();
    block_on(sub.next()).unwrap().unwrap();
    for _ in 0..20 {
        block_on(w.write(|_| Ok(()))).unwrap();
    }
    block_on(w.close()).unwrap();
    assert!(block_on(sub.next()).is_none());
}
#[test]
fn dropping_observation_stops_evaluation() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let w = open();
    let calls = Arc::new(AtomicUsize::new(0));
    let called = calls.clone();
    let mut sub = block_on(w.observe(move |db| {
        called.fetch_add(1, Ordering::SeqCst);
        db.table_count::<Note>()
    }))
    .unwrap();
    block_on(sub.next()).unwrap().unwrap();
    drop(sub);
    let count = calls.load(Ordering::SeqCst);
    for _ in 0..5 {
        block_on(w.write(|_| Ok(()))).unwrap();
    }
    block_on(w.close()).unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), count);
}
#[test]
fn projection_errors_are_reported_and_recover_on_next_commit() {
    let w = open();
    let mut sub = block_on(w.observe(|db| {
        let count = db.table_count::<Note>()?;
        if count == 0 {
            Err(DbError::driver("synthetic projection failure"))
        } else {
            Ok(count)
        }
    }))
    .unwrap();
    assert!(block_on(sub.next()).unwrap().is_err());
    block_on(w.write(|db| {
        db.insert(note(1));
        Ok(())
    }))
    .unwrap();
    assert_eq!(block_on(sub.next()).unwrap().unwrap().value, 1);
    block_on(w.close()).unwrap();
}
#[test]
fn projection_panic_closes_and_wakes_pending_subscription() {
    let w = open();
    let mut sub = block_on(w.observe::<u64>(|_| panic!("synthetic projection panic"))).unwrap();
    assert!(block_on(sub.next()).is_none());
    assert_eq!(block_on(w.close()).unwrap_err().kind, DbErrorKind::Panicked);
}
#[test]
fn undo_redo_are_committed_and_background_authors_are_excluded() {
    let w = open();
    block_on(w.enable_undo(10)).unwrap();
    block_on(w.write(|db| {
        db.insert(note(1));
        Ok(())
    }))
    .unwrap();
    assert_eq!(
        block_on(w.undo_status()).unwrap(),
        (true, false),
        "before background import"
    );
    block_on(w.write(|db| {
        day_model::with_author("synthetic-import", || {
            db.insert(note(2));
            Ok(())
        })
    }))
    .unwrap();
    assert_eq!(block_on(w.undo_status()).unwrap(), (true, false));
    assert!(block_on(w.undo(false)).unwrap());
    assert_eq!(count(&w), 1);
    assert_eq!(block_on(w.undo_status()).unwrap(), (false, true));
    assert!(block_on(w.undo(true)).unwrap());
    assert_eq!(count(&w), 2);
    block_on(w.close()).unwrap();
}
#[test]
fn failed_edit_preserves_a_redo_branch() {
    let w = open();
    block_on(w.enable_undo(10)).unwrap();
    block_on(w.write(|db| {
        db.insert(note(1));
        Ok(())
    }))
    .unwrap();
    block_on(w.undo(false)).unwrap();
    assert!(
        block_on(w.write::<()>(|db| {
            db.insert(note(2));
            db.save()?;
            Err(DbError::driver("synthetic failure"))
        }))
        .is_err()
    );
    assert_eq!(block_on(w.undo_status()).unwrap(), (false, true));
    assert!(block_on(w.undo(true)).unwrap());
    assert_eq!(count(&w), 1);
    assert!(block_on(w.read(|db| Ok(db.get::<Note>(1u64).is_some()))).unwrap());
    block_on(w.close()).unwrap();
}
#[test]
fn fts_and_cache_restore_after_failed_delete_and_update() {
    let w = open();
    block_on(w.write(|db| {
        db.insert(note(1));
        db.insert(note(2));
        Ok(())
    }))
    .unwrap();
    assert!(
        block_on(w.write::<()>(|db| {
            db.delete::<Note>(1u64)?;
            db.get::<Note>(2u64)
                .unwrap()
                .title()
                .write("rollbackword".into());
            db.save()?;
            Err(DbError::driver("synthetic failure"))
        }))
        .is_err()
    );
    let hits = block_on(w.read(|db| {
        db.query::<Note>()
            .filter(Note::fts().matches("Synthetic"))
            .live()
            .try_collect()
    }))
    .unwrap();
    assert_eq!(hits.len(), 2);
    assert!(
        block_on(w.read(|db| {
            db.query::<Note>()
                .filter(Note::fts().matches("rollbackword"))
                .live()
                .try_collect()
        }))
        .unwrap()
        .is_empty()
    );
    block_on(w.close()).unwrap();
}
fn temporary_db(test: &str) -> std::path::PathBuf {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "day-worker-{test}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("test.sqlite")
}
#[test]
fn acknowledged_commits_survive_reopen_and_panics_do_not_leave_partial_writes() {
    let file = temporary_db("durability");
    let path = file.clone();
    let w = block_on(DatabaseWorker::open(move || {
        ModelContainer::open(Sqlite::at(path), schema![Note])
    }))
    .unwrap();
    block_on(w.write(|db| {
        db.insert(note(1));
        Ok(())
    }))
    .unwrap();
    assert_eq!(
        block_on(w.write::<()>(|db| {
            db.insert(note(2));
            db.save()?;
            panic!("synthetic panic after flush")
        }))
        .unwrap_err()
        .kind,
        DbErrorKind::Panicked
    );
    assert!(block_on(w.close()).is_err());
    let path = file.clone();
    let reopened = block_on(DatabaseWorker::open(move || {
        ModelContainer::open(Sqlite::at(path), schema![Note])
    }))
    .unwrap();
    assert_eq!(count(&reopened), 1);
    block_on(reopened.close()).unwrap();
    std::fs::remove_dir_all(file.parent().unwrap()).unwrap();
}
#[test]
fn dropping_last_handle_drains_accepted_writes_and_releases_subscription() {
    let file = temporary_db("drop");
    let path = file.clone();
    let w = block_on(DatabaseWorker::open(move || {
        ModelContainer::open(Sqlite::at(path), schema![Note])
    }))
    .unwrap();
    let mut sub = block_on(w.observe(|db| db.table_count::<Note>())).unwrap();
    for id in 0..100 {
        drop(w.write(move |db| {
            db.insert(note(id));
            Ok(())
        }));
    }
    drop(w);
    while block_on(sub.next()).is_some() {}
    let db = ModelContainer::open(Sqlite::at(&file), schema![Note]).unwrap();
    assert_eq!(db.table_count::<Note>().unwrap(), 100);
    drop(db);
    std::fs::remove_dir_all(file.parent().unwrap()).unwrap();
}
#[test]
fn external_commits_are_merged_on_worker_and_publish_new_revision() {
    let file = temporary_db("external");
    let path = file.clone();
    let w = block_on(DatabaseWorker::open(move || {
        ModelContainer::open(Sqlite::at(path), schema![Note])
    }))
    .unwrap();
    let mut sub = block_on(w.observe(|db| db.table_count::<Note>())).unwrap();
    block_on(sub.next()).unwrap().unwrap();
    let db = ModelContainer::open(Sqlite::at(&file), schema![Note]).unwrap();
    db.insert(note(1));
    db.save().unwrap();
    assert!(block_on(w.check_external()).unwrap());
    assert_eq!(block_on(sub.next()).unwrap().unwrap().value, 1);
    assert!(!block_on(w.check_external()).unwrap());
    block_on(w.close()).unwrap();
    drop(db);
    std::fs::remove_dir_all(file.parent().unwrap()).unwrap();
}
#[test]
fn multi_statement_read_has_a_consistent_sqlite_snapshot() {
    let file = temporary_db("snapshot");
    let path = file.clone();
    let w = block_on(DatabaseWorker::open(move || {
        ModelContainer::open(Sqlite::at(path), schema![Note])
    }))
    .unwrap();
    let (tx, rx) = mpsc::channel();
    let (go, wait) = mpsc::channel();
    let read = w.read(move |db| {
        let before = db.table_count::<Note>()?;
        tx.send(()).unwrap();
        wait.recv_timeout(Duration::from_secs(5)).unwrap();
        Ok((before, db.table_count::<Note>()?))
    });
    rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let db = ModelContainer::open(Sqlite::at(&file), schema![Note]).unwrap();
    db.insert(note(1));
    db.save().unwrap();
    go.send(()).unwrap();
    assert_eq!(block_on(read).unwrap(), (0, 0));
    assert_eq!(count(&w), 1);
    block_on(w.close()).unwrap();
    drop(db);
    std::fs::remove_dir_all(file.parent().unwrap()).unwrap();
}
#[test]
fn rejected_closure_captures_drop_outside_admission_lock() {
    struct Reenter(DatabaseWorker, mpsc::Sender<()>);
    impl Drop for Reenter {
        fn drop(&mut self) {
            drop(self.0.read(|_| Ok(())));
            self.1.send(()).unwrap();
        }
    }
    let w = block_on(DatabaseWorker::open_with(
        WorkerOptions {
            capacity: 1,
            ..Default::default()
        },
        || ModelContainer::open(Sqlite::memory(), schema![Note]),
    ))
    .unwrap();
    let gate = Arc::new(Barrier::new(2));
    let g = gate.clone();
    let (tx, rx) = mpsc::channel();
    let first = w.read(move |_| {
        tx.send(()).unwrap();
        g.wait();
        Ok(())
    });
    rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let pending = w.write(|db| {
        db.insert(note(1));
        Ok(())
    });
    let (tx, rx) = mpsc::channel();
    let capture = Reenter(w.clone(), tx);
    let e = block_on(w.read(move |_| {
        let _keep = capture;
        Ok(())
    }))
    .unwrap_err();
    assert_eq!(e.kind, DbErrorKind::Busy);
    rx.recv_timeout(Duration::from_secs(5)).unwrap();
    gate.wait();
    block_on(first).unwrap();
    block_on(pending).unwrap();
    block_on(w.close()).unwrap();
}
#[test]
fn many_concurrent_closers_finish_while_writers_race_admission() {
    let w = open();
    let accepted = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut threads = vec![];
    for lane in 0..8 {
        let w = w.clone();
        let accepted = accepted.clone();
        threads.push(std::thread::spawn(move || {
            for index in 0..100 {
                match block_on(w.write(move |db| {
                    db.insert(note(lane * 100 + index));
                    Ok(())
                })) {
                    Ok(()) => {
                        accepted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    }
                    Err(e) => assert!(matches!(e.kind, DbErrorKind::Closed | DbErrorKind::Busy)),
                }
            }
        }));
    }
    let mut closers = vec![];
    for _ in 0..8 {
        let w = w.clone();
        closers.push(std::thread::spawn(move || block_on(w.close()).unwrap()));
    }
    for t in threads {
        t.join().unwrap();
    }
    for t in closers {
        t.join().unwrap();
    }
    assert_eq!(
        block_on(w.read(|_| Ok(()))).unwrap_err().kind,
        DbErrorKind::Closed
    );
}

#[test]
fn rejected_read_mutation_does_not_pollute_undo_or_destroy_redo() {
    let w = open();
    block_on(w.enable_undo(10)).unwrap();
    block_on(w.write(|db| {
        db.insert(note(1));
        Ok(())
    }))
    .unwrap();
    block_on(w.undo(false)).unwrap();
    assert_eq!(block_on(w.undo_status()).unwrap(), (false, true));
    assert!(
        block_on(w.read(|db| {
            db.insert(note(2));
            Ok(())
        }))
        .is_err()
    );
    assert_eq!(block_on(w.undo_status()).unwrap(), (false, true));
    assert_eq!(count(&w), 0);
    assert!(block_on(w.undo(true)).unwrap());
    assert_eq!(count(&w), 1);
    w.close_blocking().unwrap();
    w.close_blocking().unwrap();
}
#[test]
fn deferred_constraint_commit_failure_rolls_back_and_worker_remains_usable() {
    let w = block_on(DatabaseWorker::open(|| {
        let db = ModelContainer::open(Sqlite::memory(), schema![Note])?;
        db.try_with_connection(|c| c.execute_batch(
            "CREATE TABLE parents(id INTEGER PRIMARY KEY); CREATE TABLE children(parent INTEGER REFERENCES parents(id) DEFERRABLE INITIALLY DEFERRED);"
        ))?;
        Ok(db)
    })).unwrap();
    assert!(
        block_on(w.write(|db| {
            db.insert(note(1));
            db.try_with_connection(|c| c.execute("INSERT INTO children VALUES(99)", &[]))
        }))
        .is_err()
    );
    assert_eq!(count(&w), 0);
    block_on(w.write(|db| {
        db.insert(note(1));
        db.try_with_connection(|c| {
            c.execute_batch("INSERT INTO parents VALUES(99); INSERT INTO children VALUES(99);")
        })
    }))
    .unwrap();
    assert_eq!(count(&w), 1);
    w.close_blocking().unwrap();
}
#[test]
fn cancellation_and_failed_transactions_stress_preserve_exact_committed_state() {
    for round in 0..10 {
        let w = open();
        let mut requests = Vec::new();
        for i in 0..100 {
            requests.push(w.write(move |db| {
                db.insert(note(i));
                if (i + round) % 3 == 0 {
                    db.save()?;
                    Err(DbError::driver("synthetic rollback"))
                } else {
                    Ok(())
                }
            }));
            drop(w.read(|db| db.table_count::<Note>()));
        }
        let committed = requests
            .into_iter()
            .map(block_on)
            .filter(Result::is_ok)
            .count();
        assert_eq!(count(&w), committed as u64);
        w.close_blocking().unwrap();
    }
}
#[test]
fn backup_on_read_queue_is_a_reopenable_committed_snapshot() {
    let file = temporary_db("backup");
    let w = open();
    block_on(w.write(|db| {
        db.insert(note(1));
        Ok(())
    }))
    .unwrap();
    let destination = file.clone();
    block_on(w.backup_to(destination)).unwrap();
    block_on(w.write(|db| {
        db.insert(note(2));
        Ok(())
    }))
    .unwrap();
    let backup = ModelContainer::open(Sqlite::at(&file), schema![Note]).unwrap();
    assert_eq!(backup.table_count::<Note>().unwrap(), 1);
    w.close_blocking().unwrap();
    drop(backup);
    std::fs::remove_dir_all(file.parent().unwrap()).unwrap();
}

struct FaultDriver {
    fail: Arc<std::sync::atomic::AtomicU8>,
    dropped: mpsc::Sender<std::thread::ThreadId>,
}
struct FaultConnection {
    inner: Box<dyn day_persistence::SqliteConnection>,
    fail: Arc<std::sync::atomic::AtomicU8>,
    dropped: mpsc::Sender<std::thread::ThreadId>,
    owner: std::thread::ThreadId,
}
impl Drop for FaultConnection {
    fn drop(&mut self) {
        assert_eq!(self.owner, std::thread::current().id());
        let _ = self.dropped.send(self.owner);
    }
}
impl day_persistence::SqliteDriver for FaultDriver {
    type Connection = FaultConnection;
    fn capabilities(&self) -> day_persistence::Capabilities {
        Sqlite::memory().capabilities()
    }
    fn open(self) -> Result<Self::Connection, DbError> {
        Ok(FaultConnection {
            inner: Box::new(Sqlite::memory().open()?),
            fail: self.fail,
            dropped: self.dropped,
            owner: std::thread::current().id(),
        })
    }
}
impl day_persistence::SqliteConnection for FaultConnection {
    fn execute(&mut self, sql: &str, params: &[day_persistence::Value]) -> Result<u64, DbError> {
        assert_eq!(self.owner, std::thread::current().id());
        if self.fail.load(std::sync::atomic::Ordering::SeqCst) == 1 && sql == "ROLLBACK" {
            return Err(DbError::driver("synthetic rollback failure"));
        }
        self.inner.execute(sql, params)
    }
    fn query(
        &mut self,
        sql: &str,
        params: &[day_persistence::Value],
        row: &mut dyn FnMut(&dyn day_persistence::Row),
    ) -> Result<(), DbError> {
        assert_eq!(self.owner, std::thread::current().id());
        self.inner.query(sql, params, row)
    }
    fn execute_batch(&mut self, sql: &str) -> Result<(), DbError> {
        assert_eq!(self.owner, std::thread::current().id());
        if self.fail.load(std::sync::atomic::Ordering::SeqCst) == 2
            && sql == "PRAGMA query_only = OFF"
        {
            return Err(DbError::driver("synthetic read-mode recovery failure"));
        }
        self.inner.execute_batch(sql)
    }
}
fn fault_worker() -> (
    DatabaseWorker,
    Arc<std::sync::atomic::AtomicU8>,
    mpsc::Receiver<std::thread::ThreadId>,
) {
    let fail = Arc::new(std::sync::atomic::AtomicU8::new(0));
    let flag = fail.clone();
    let (tx, rx) = mpsc::channel();
    let w = block_on(DatabaseWorker::open(move || {
        ModelContainer::open(
            FaultDriver {
                fail: flag,
                dropped: tx,
            },
            schema![Note],
        )
    }))
    .unwrap();
    (w, fail, rx)
}
#[test]
fn failed_rollback_closes_connection_on_owner_thread_and_rejects_further_work() {
    let (w, fail, dropped) = fault_worker();
    fail.store(1, std::sync::atomic::Ordering::SeqCst);
    let error = block_on(w.write::<()>(|db| {
        db.insert(note(1));
        Err(DbError::driver("synthetic failed edit"))
    }))
    .unwrap_err();
    assert_eq!(error.kind, DbErrorKind::Closed);
    assert_eq!(block_on(w.close()).unwrap_err().kind, DbErrorKind::Closed);
    assert_ne!(
        dropped.recv_timeout(Duration::from_secs(5)).unwrap(),
        std::thread::current().id()
    );
    assert!(block_on(w.write(|_| Ok(()))).is_err());
}
#[test]
fn projection_recovery_failure_reports_error_then_ends_subscription() {
    let (w, fail, dropped) = fault_worker();
    let mut sub = block_on(w.observe(|db| db.table_count::<Note>())).unwrap();
    assert_eq!(block_on(sub.next()).unwrap().unwrap().value, 0);
    fail.store(2, std::sync::atomic::Ordering::SeqCst);
    block_on(w.write(|db| {
        db.insert(note(1));
        Ok(())
    }))
    .unwrap();
    assert_eq!(
        block_on(sub.next()).unwrap().unwrap_err().kind,
        DbErrorKind::Closed
    );
    assert!(block_on(sub.next()).is_none());
    assert!(block_on(w.close()).is_err());
    dropped.recv_timeout(Duration::from_secs(5)).unwrap();
}
#[test]
fn cancelled_open_still_closes_its_eventual_connection_on_worker() {
    let (started, begin) = mpsc::channel();
    let (release, gate) = mpsc::channel();
    let (dropped, finished) = mpsc::channel();
    let mut opening = Box::pin(DatabaseWorker::open(move || {
        started.send(()).unwrap();
        gate.recv_timeout(Duration::from_secs(5)).unwrap();
        ModelContainer::open(
            FaultDriver {
                fail: Arc::new(std::sync::atomic::AtomicU8::new(0)),
                dropped,
            },
            schema![Note],
        )
    }));
    assert!(
        opening
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    begin.recv_timeout(Duration::from_secs(5)).unwrap();
    drop(opening);
    release.send(()).unwrap();
    assert_ne!(
        finished.recv_timeout(Duration::from_secs(5)).unwrap(),
        std::thread::current().id()
    );
}
#[test]
fn normal_close_and_panic_both_drop_connections_on_worker() {
    for panic in [false, true] {
        let (w, _, dropped) = fault_worker();
        if panic {
            assert!(block_on(w.write::<()>(|_| panic!("synthetic operation failure"))).is_err());
        }
        let result = w.close_blocking();
        assert_eq!(result.is_err(), panic);
        assert_ne!(
            dropped.recv_timeout(Duration::from_secs(5)).unwrap(),
            std::thread::current().id()
        );
    }
}
