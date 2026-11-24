// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
//! Synthetic slow producers distinguish idle deadlines from total upload deadlines.
#![cfg(any(feature = "reqwest", target_os = "macos", target_os = "ios"))]
use day_part_http::{Client, HttpError, Provider, Request, Session, testing::Server};
use std::io::{Cursor, Read};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

fn client(provider: Provider, idle: Duration) -> Client {
    let session = Session::new();
    session.set_provider(provider);
    Client::builder()
        .session(session)
        .timeout_idle(idle)
        .build()
}
fn fetch(client: &Client, request: Request) -> Result<day_part_http::Response, HttpError> {
    let (tx, rx) = mpsc::channel();
    let _flight = client.fetch_async(request, move |r| {
        let _ = tx.send(r);
    });
    rx.recv_timeout(Duration::from_secs(10))
        .expect("bounded request")
}
struct SlowReader {
    bytes: Cursor<Vec<u8>>,
    delay: Duration,
}
impl Read for SlowReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        std::thread::sleep(self.delay);
        let count = out.len().min(16 * 1024);
        self.bytes.read(&mut out[..count])
    }
}
fn progressing_upload(provider: Provider) {
    let server = Server::start().unwrap();
    let client = client(provider, Duration::from_millis(500));
    let bytes = vec![90; 20 * 16 * 1024];
    let len = bytes.len();
    let progress = Arc::new(Mutex::new(Vec::new()));
    let observed = progress.clone();
    let start = Instant::now();
    let response = fetch(
        &client,
        Request::put(server.url("/upload"), Vec::new())
            .body_stream(
                SlowReader {
                    bytes: Cursor::new(bytes),
                    delay: Duration::from_millis(100),
                },
                Some(len as u64),
            )
            .upload_progress(move |sent, _| observed.lock().unwrap().push(sent))
            .timeout_total(Duration::from_secs(6)),
    );
    assert!(
        response.is_ok(),
        "active upload failed at {:?}, progress {:?}: {response:?}",
        start.elapsed(),
        progress.lock().unwrap()
    );
    let response = response.unwrap();
    assert_eq!(response.status, 200);
    assert!(response.text().starts_with(&format!("{len} ")));
    assert!(start.elapsed() >= Duration::from_secs(2));
    assert_eq!(progress.lock().unwrap().last(), Some(&(len as u64)));
}
#[cfg(any(target_os = "macos", target_os = "ios"))]
#[test]
fn apple_progressing_upload_outlives_idle_interval() {
    progressing_upload(Provider::Native);
}
#[cfg(feature = "reqwest")]
#[test]
fn reqwest_progressing_upload_outlives_idle_interval() {
    progressing_upload(Provider::Reqwest);
}
#[cfg(feature = "reqwest")]
#[test]
fn reqwest_request_can_extend_client_idle_timeout() {
    let server = Server::start().unwrap();
    let client = client(Provider::Reqwest, Duration::from_millis(100));
    let response = fetch(
        &client,
        Request::get(server.url("/delay/400"))
            .timeout(Duration::from_secs(2))
            .timeout_total(Duration::from_secs(5)),
    )
    .unwrap();
    assert_eq!(response.status, 200);
}

struct BrokenReader {
    delivered: bool,
}
impl Read for BrokenReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if self.delivered {
            return Err(std::io::Error::other("synthetic producer failure"));
        }
        self.delivered = true;
        out[..7].copy_from_slice(b"partial");
        Ok(7)
    }
}
fn producer_error(provider: Provider) {
    let server = Server::start().unwrap();
    let client = client(provider, Duration::from_secs(2));
    let response = fetch(
        &client,
        Request::put(server.url("/upload"), Vec::new())
            .body_stream(BrokenReader { delivered: false }, None)
            .timeout_total(Duration::from_secs(4)),
    );
    assert!(
        response.is_err(),
        "producer error was silently treated as complete: {response:?}"
    );
    assert_eq!(
        fetch(&client, Request::get(server.url("/")))
            .unwrap()
            .status,
        200
    );
}
#[cfg(any(target_os = "macos", target_os = "ios"))]
#[test]
fn apple_upload_producer_error_is_not_success() {
    producer_error(Provider::Native);
}
#[cfg(feature = "reqwest")]
#[test]
fn reqwest_upload_producer_error_is_not_success() {
    producer_error(Provider::Reqwest);
}

fn stalled_upload(provider: Provider) {
    let server = Server::start().unwrap();
    let client = client(provider, Duration::from_millis(300));
    let started = Instant::now();
    let response = fetch(
        &client,
        Request::put(server.url("/upload"), Vec::new())
            .body_stream(
                SlowReader {
                    bytes: Cursor::new(vec![90; 16 * 1024]),
                    delay: Duration::from_secs(2),
                },
                None,
            )
            .timeout_total(Duration::from_secs(5)),
    );
    assert_eq!(response.unwrap_err(), HttpError::Timeout);
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(
        fetch(&client, Request::get(server.url("/")))
            .unwrap()
            .status,
        200
    );
}
#[cfg(any(target_os = "macos", target_os = "ios"))]
#[test]
fn apple_stalled_upload_times_out_and_client_recovers() {
    stalled_upload(Provider::Native);
}
#[cfg(feature = "reqwest")]
#[test]
fn reqwest_stalled_upload_times_out_and_client_recovers() {
    stalled_upload(Provider::Reqwest);
}

fn total_upload_deadline(provider: Provider) {
    let server = Server::start().unwrap();
    let client = client(provider, Duration::from_secs(2));
    let response = fetch(
        &client,
        Request::put(server.url("/upload"), Vec::new())
            .body_stream(
                SlowReader {
                    bytes: Cursor::new(vec![90; 100 * 16 * 1024]),
                    delay: Duration::from_millis(100),
                },
                None,
            )
            .timeout_total(Duration::from_millis(500)),
    );
    assert_eq!(response.unwrap_err(), HttpError::Timeout);
    assert_eq!(
        fetch(&client, Request::get(server.url("/")))
            .unwrap()
            .status,
        200
    );
}
#[cfg(any(target_os = "macos", target_os = "ios"))]
#[test]
fn apple_total_deadline_stops_progressing_upload() {
    total_upload_deadline(Provider::Native);
}
#[cfg(feature = "reqwest")]
#[test]
fn reqwest_total_deadline_stops_progressing_upload() {
    total_upload_deadline(Provider::Reqwest);
}

#[cfg(feature = "reqwest")]
#[test]
fn reqwest_request_can_extend_body_idle_timeout() {
    let server = Server::start().unwrap();
    let client = client(Provider::Reqwest, Duration::from_millis(100));
    let response = fetch(
        &client,
        Request::get(server.url("/drip?chunks=4&size=100&delay_ms=250"))
            .timeout(Duration::from_secs(2))
            .timeout_total(Duration::from_secs(5)),
    )
    .unwrap();
    assert_eq!(response.body.len(), 400);
}

struct InterruptedReader {
    interrupted: bool,
    bytes: Cursor<Vec<u8>>,
}
impl Read for InterruptedReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if !self.interrupted {
            self.interrupted = true;
            return Err(std::io::Error::from(std::io::ErrorKind::Interrupted));
        }
        self.bytes.read(out)
    }
}
fn interrupted_producer(provider: Provider) {
    let server = Server::start().unwrap();
    let client = client(provider, Duration::from_secs(2));
    let response = fetch(
        &client,
        Request::put(server.url("/upload"), Vec::new())
            .body_stream(
                InterruptedReader {
                    interrupted: false,
                    bytes: Cursor::new(b"synthetic retry".to_vec()),
                },
                None,
            )
            .timeout_total(Duration::from_secs(4)),
    )
    .unwrap();
    assert_eq!(response.status, 200);
    assert!(response.text().starts_with("15 "));
}
#[cfg(any(target_os = "macos", target_os = "ios"))]
#[test]
fn apple_interrupted_producer_retries_read() {
    interrupted_producer(Provider::Native);
}
#[cfg(feature = "reqwest")]
#[test]
fn reqwest_interrupted_producer_retries_read() {
    interrupted_producer(Provider::Reqwest);
}

struct EndlessReader(Arc<std::sync::atomic::AtomicUsize>);
impl Read for EndlessReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(20));
        out.fill(90);
        Ok(out.len())
    }
}
fn cancel_producer(provider: Provider) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let server = Server::start().unwrap();
    let client = client(provider, Duration::from_secs(3));
    let reads = Arc::new(AtomicUsize::new(0));
    let (progress_tx, progress_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let flight = client.fetch_async(
        Request::put(server.url("/upload"), Vec::new())
            .body_stream(EndlessReader(reads.clone()), None)
            .upload_progress(move |_, _| {
                let _ = progress_tx.send(());
            })
            .timeout_total(Duration::from_secs(5)),
        move |r| {
            let _ = done_tx.send(r);
        },
    );
    progress_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    flight.cancel();
    assert!(matches!(
        done_rx.recv_timeout(Duration::from_secs(2)),
        Err(mpsc::RecvTimeoutError::Disconnected)
    ));
    std::thread::sleep(Duration::from_millis(200));
    let stopped = reads.load(Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(
        reads.load(Ordering::SeqCst),
        stopped,
        "cancelled upload kept reading"
    );
    assert_eq!(
        fetch(&client, Request::get(server.url("/")))
            .unwrap()
            .status,
        200
    );
}
#[cfg(any(target_os = "macos", target_os = "ios"))]
#[test]
fn apple_cancel_stops_upload_producer() {
    cancel_producer(Provider::Native);
}
#[cfg(feature = "reqwest")]
#[test]
fn reqwest_cancel_stops_upload_producer() {
    cancel_producer(Provider::Reqwest);
}

#[cfg(feature = "reqwest")]
#[test]
fn reqwest_unrepresentable_idle_deadline_does_not_panic() {
    let server = Server::start().unwrap();
    let client = client(Provider::Reqwest, Duration::from_secs(2));
    let response = fetch(
        &client,
        Request::get(server.url("/"))
            .timeout(Duration::MAX)
            .timeout_total(Duration::from_secs(3)),
    )
    .unwrap();
    assert_eq!(response.status, 200);
}
