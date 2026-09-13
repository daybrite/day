// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The manager against day-part-http's loopback test server, over the platform's own transport.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use day_part_downloads::{Download, Downloads, Progress, State};
use day_part_http::Request;
use day_part_http::testing::{Server, pattern_sha256};

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!(
            "day-downloads-{name}-{}-{}",
            std::process::id(),
            Instant::now().elapsed().as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        Scratch(dir)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn wait_for(
    downloads: &Downloads,
    id: day_part_downloads::DownloadId,
    what: &str,
    done: impl Fn(&Progress) -> bool,
) -> Progress {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let progress = downloads.progress(id).expect("known download");
        if done(&progress) {
            return progress;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {what}: {progress:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn sha_of(path: &Path) -> String {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(path).expect("read");
    Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[test]
fn a_download_finishes_verified_and_moves_into_place() {
    const LEN: u64 = 400_000;
    let server = Server::start().expect("server");
    let scratch = Scratch::new("finish");
    let downloads = Downloads::open(scratch.path("manager")).expect("open");
    let dest = scratch.path("out/file.bin");
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    let _watch = downloads.watch(move |p| log.lock().unwrap().push(p.state));
    let id = downloads
        .enqueue(
            Download::new(Request::get(server.url(&format!("/bytes/{LEN}"))), &dest)
                .expect_len(LEN)
                .expect_sha256(&pattern_sha256(LEN)),
        )
        .expect("enqueue");
    let done = wait_for(&downloads, id, "done", |p| p.state.is_settled());
    assert_eq!(done.state, State::Done, "{done:?}");
    assert_eq!(done.received, LEN);
    assert_eq!(done.sha256.as_deref(), Some(pattern_sha256(LEN).as_str()));
    assert_eq!(sha_of(&dest), pattern_sha256(LEN));
    assert!(!scratch.path(&format!("manager/{id}.part")).exists());
    let states = seen.lock().unwrap().clone();
    assert!(states.contains(&State::Running), "{states:?}");
    assert!(states.contains(&State::Verifying), "{states:?}");
}

#[test]
fn pause_then_resume_continues_with_a_validated_range() {
    const LEN: u64 = 3_000_000;
    let server = Server::start().expect("server");
    let scratch = Scratch::new("resume");
    let downloads = Downloads::open(scratch.path("manager")).expect("open");
    let dest = scratch.path("file.bin");
    let url = server.url(&format!("/bytes/{LEN}?rate=1500000"));
    let id = downloads
        .enqueue(Download::new(Request::get(url), &dest).expect_sha256(&pattern_sha256(LEN)))
        .expect("enqueue");
    wait_for(&downloads, id, "some bytes", |p| p.received >= 400_000);
    downloads.pause(id).expect("pause");
    let paused = wait_for(&downloads, id, "paused", |p| p.state == State::Paused);
    let part = scratch.path(&format!("manager/{id}.part"));
    let kept = std::fs::metadata(&part).expect("partial file").len();
    assert!(
        (400_000..LEN).contains(&kept),
        "{kept} of {LEN}, {paused:?}"
    );
    downloads.resume(id).expect("resume");
    let done = wait_for(&downloads, id, "done", |p| p.state.is_settled());
    assert_eq!(done.state, State::Done, "{done:?}");
    assert!(done.resumed_from >= kept, "{done:?}");
    assert_eq!(sha_of(&dest), pattern_sha256(LEN));
}

#[test]
fn cancel_removes_the_partial_file() {
    let server = Server::start().expect("server");
    let scratch = Scratch::new("cancel");
    let downloads = Downloads::open(scratch.path("manager")).expect("open");
    let dest = scratch.path("file.bin");
    let id = downloads
        .enqueue(Download::new(
            Request::get(server.url("/bytes/2000000?rate=500000")),
            &dest,
        ))
        .expect("enqueue");
    wait_for(&downloads, id, "some bytes", |p| p.received > 0);
    downloads.cancel(id).expect("cancel");
    let cancelled = wait_for(&downloads, id, "cancelled", |p| p.state == State::Cancelled);
    assert_eq!(cancelled.received, 0);
    assert!(!scratch.path(&format!("manager/{id}.part")).exists());
    assert!(!dest.exists());
}

#[test]
fn a_wrong_digest_fails_and_leaves_no_file() {
    let server = Server::start().expect("server");
    let scratch = Scratch::new("digest");
    let downloads = Downloads::open(scratch.path("manager")).expect("open");
    let dest = scratch.path("file.bin");
    let id = downloads
        .enqueue(
            Download::new(Request::get(server.url("/bytes/5000")), &dest)
                .expect_sha256(&"0".repeat(64)),
        )
        .expect("enqueue");
    let failed = wait_for(&downloads, id, "failed", |p| p.state.is_settled());
    assert_eq!(failed.state, State::Failed, "{failed:?}");
    assert!(
        failed.error.as_deref().unwrap_or("").contains("SHA-256"),
        "{failed:?}"
    );
    assert!(!dest.exists());
}

#[test]
fn server_errors_retry_then_fail() {
    let server = Server::start().expect("server");
    let scratch = Scratch::new("retry");
    let downloads = Downloads::open(scratch.path("manager")).expect("open");
    let id = downloads
        .enqueue(
            Download::new(Request::get(server.url("/status/503")), scratch.path("x"))
                .retries(2)
                .retry_delay(Duration::from_millis(20)),
        )
        .expect("enqueue");
    let failed = wait_for(&downloads, id, "failed", |p| p.state == State::Failed);
    assert_eq!(failed.attempt, 3, "{failed:?}");
    assert_eq!(server.hits("/status/503"), 3);
}

#[test]
fn a_relaunch_restores_a_paused_download_from_the_journal() {
    const LEN: u64 = 2_000_000;
    let server = Server::start().expect("server");
    let scratch = Scratch::new("relaunch");
    let dest = scratch.path("file.bin");
    let url = server.url(&format!("/bytes/{LEN}?rate=1000000"));
    let id = {
        let downloads = Downloads::open(scratch.path("manager")).expect("open");
        let id = downloads
            .enqueue(Download::new(Request::get(url), &dest).expect_sha256(&pattern_sha256(LEN)))
            .expect("enqueue");
        wait_for(&downloads, id, "some bytes", |p| p.received >= 200_000);
        downloads.pause(id).expect("pause");
        wait_for(&downloads, id, "paused", |p| p.state == State::Paused);
        id
    };
    let downloads = Downloads::open(scratch.path("manager")).expect("reopen");
    let restored = downloads.progress(id).expect("restored");
    assert_eq!(restored.state, State::Paused);
    assert!(restored.received >= 200_000, "{restored:?}");
    downloads.resume(id).expect("resume");
    let done = wait_for(&downloads, id, "done", |p| p.state.is_settled());
    assert_eq!(done.state, State::Done, "{done:?}");
    assert_eq!(sha_of(&dest), pattern_sha256(LEN));
}

#[test]
fn the_limit_holds_extra_downloads_in_the_queue() {
    let server = Server::start().expect("server");
    let scratch = Scratch::new("limit");
    let downloads = Downloads::open(scratch.path("manager")).expect("open");
    downloads.limits(1, 1);
    let most = Arc::new(Mutex::new(0usize));
    let seen = most.clone();
    let lister = downloads.clone();
    let _watch = downloads.watch(move |_| {
        let running = lister
            .list()
            .iter()
            .filter(|p| p.state == State::Running)
            .count();
        let mut most = seen.lock().unwrap();
        *most = (*most).max(running);
    });
    let ids: Vec<_> = (0..3)
        .map(|i| {
            downloads
                .enqueue(Download::new(
                    Request::get(server.url("/bytes/100000?rate=400000")),
                    scratch.path(&format!("file{i}.bin")),
                ))
                .expect("enqueue")
        })
        .collect();
    for id in ids {
        let done = wait_for(&downloads, id, "done", |p| p.state.is_settled());
        assert_eq!(done.state, State::Done, "{done:?}");
    }
    assert_eq!(*most.lock().unwrap(), 1);
}
