//! End-to-end tests against a local std::net server — no external network. On macOS CI hosts this
//! exercises the REAL NSURLSession half; on Linux the ureq fallback: two production halves under
//! one suite. (Android/Windows/HarmonyOS are covered by cross-compiles + the showcase device
//! smoke, the parts' established posture.)

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

use day_part_http::{HttpError, Request, fetch, fetch_async, fetch_to_file};

/// Serve `count` connections with a fixed HTTP/1.1 response; returns the bound port.
fn serve(count: usize, status_line: &'static str, body: Vec<u8>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for _ in 0..count {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            read_request(&mut stream);
            let head = format!(
                "{status_line}\r\nContent-Length: {}\r\nX-Day-Test: yes\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(&body);
        }
    });
    port
}

/// Read until the end of the request head (+ any Content-Length body) so clients that await
/// write completion don't race the response.
fn read_request(stream: &mut TcpStream) -> Vec<u8> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if let Some(head_end) = find_head_end(&buf) {
                    let head = String::from_utf8_lossy(&buf[..head_end]);
                    let content_len = head
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(str::trim)
                                .map(str::to_string)
                        })
                        .and_then(|v| v.parse::<usize>().ok())
                        .unwrap_or(0);
                    if buf.len() >= head_end + content_len {
                        break;
                    }
                }
            }
            Err(_) => break,
        }
    }
    buf
}

fn find_head_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4)
}

#[test]
fn ok_round_trip_with_headers() {
    let port = serve(1, "HTTP/1.1 200 OK", b"day-http-ok".to_vec());
    let resp = fetch(&Request::get(format!("http://127.0.0.1:{port}/"))).expect("fetch");
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body, b"day-http-ok");
    assert_eq!(
        resp.header("x-day-test"),
        Some("yes"),
        "case-insensitive header lookup"
    );
    assert_eq!(resp.text(), "day-http-ok");
}

#[test]
fn post_body_is_sent() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let received = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let req = read_request(&mut stream);
        let _ = stream.write_all(
            b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
        req
    });
    let resp = fetch(&Request::post(
        format!("http://127.0.0.1:{port}/submit"),
        b"hello-body".to_vec(),
    ))
    .expect("post");
    assert_eq!(resp.status, 204);
    let raw = received.join().unwrap();
    let raw = String::from_utf8_lossy(&raw);
    assert!(raw.starts_with("POST /submit"), "{raw}");
    assert!(raw.contains("hello-body"), "body reached the server");
}

#[test]
fn http_error_status_is_ok_not_err() {
    let port = serve(1, "HTTP/1.1 404 Not Found", b"nope".to_vec());
    let resp = fetch(&Request::get(format!("http://127.0.0.1:{port}/missing"))).expect("fetch");
    assert_eq!(resp.status, 404, "4xx is a Response, not an HttpError");
    assert_eq!(resp.body, b"nope");
}

#[test]
fn timeout_is_reported() {
    // A listener that accepts but never responds.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let _held = listener.accept();
        std::thread::sleep(Duration::from_secs(20));
    });
    let err = fetch(
        &Request::get(format!("http://127.0.0.1:{port}/")).timeout(Duration::from_millis(500)),
    )
    .expect_err("must time out");
    assert_eq!(err, HttpError::Timeout);
}

#[test]
fn connection_refused_maps_to_connect() {
    // Bind + drop to find a port that is (almost certainly) closed.
    let port = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let err = fetch(&Request::get(format!("http://127.0.0.1:{port}/"))).expect_err("refused");
    assert!(
        matches!(err, HttpError::Connect | HttpError::Io(_)),
        "expected Connect-class error, got {err:?}"
    );
}

#[test]
fn bad_url_is_rejected() {
    let err = fetch(&Request::get("not a url")).expect_err("bad url");
    assert!(matches!(err, HttpError::BadUrl(_)), "{err:?}");
}

#[test]
fn async_fetch_delivers_on_background_thread() {
    let port = serve(1, "HTTP/1.1 200 OK", b"async-ok".to_vec());
    let (tx, rx) = std::sync::mpsc::channel();
    fetch_async(
        Request::get(format!("http://127.0.0.1:{port}/")),
        move |result| {
            let _ = tx.send(result);
        },
    );
    let resp = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("completion")
        .expect("response");
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body, b"async-ok");
}

#[test]
fn download_streams_to_file() {
    // A multi-megabyte body: the point of fetch_to_file is that this never sits in a Vec.
    let body: Vec<u8> = (0..3_000_000u32).map(|i| (i % 251) as u8).collect();
    let expected = body.clone();
    let port = serve(1, "HTTP/1.1 200 OK", body);
    let dest = std::env::temp_dir().join(format!("day-http-dl-{}", std::process::id()));
    let dl = fetch_to_file(&Request::get(format!("http://127.0.0.1:{port}/big")), &dest)
        .expect("download");
    assert_eq!(dl.status, 200);
    assert_eq!(dl.bytes_written, expected.len() as u64);
    let on_disk = std::fs::read(&dest).expect("file");
    assert_eq!(on_disk, expected);
    let _ = std::fs::remove_file(&dest);
}

#[test]
fn tier_is_not_unavailable_on_hosts() {
    assert_ne!(day_part_http::tier(), day_part_http::Tier::Unavailable);
    assert!(!day_part_http::tier().label().is_empty());
}

#[test]
fn streamed_fetch_delivers_head_then_chunks() {
    let body: Vec<u8> = (0..1_000_000u32).map(|i| (i % 253) as u8).collect();
    let expected = body.clone();
    let port = serve(1, "HTTP/1.1 200 OK", body);

    struct Collect {
        status: u16,
        got: Vec<u8>,
        head_before_chunks: bool,
    }
    impl day_part_http::StreamSink for Collect {
        fn head(&mut self, status: u16, headers: &[(String, String)]) -> bool {
            self.status = status;
            self.head_before_chunks = self.got.is_empty();
            assert!(
                headers
                    .iter()
                    .any(|(k, _)| k.eq_ignore_ascii_case("x-day-test"))
            );
            true
        }
        fn chunk(&mut self, data: &[u8]) -> Result<(), HttpError> {
            self.got.extend_from_slice(data);
            Ok(())
        }
    }
    let mut sink = Collect {
        status: 0,
        got: Vec::new(),
        head_before_chunks: false,
    };
    let dl = day_part_http::fetch_streamed(
        &Request::get(format!("http://127.0.0.1:{port}/")),
        &mut sink,
    )
    .expect("stream");
    assert_eq!(dl.status, 200);
    assert_eq!(sink.status, 200);
    assert!(sink.head_before_chunks, "head arrived before any chunk");
    assert_eq!(dl.bytes_written, expected.len() as u64);
    assert_eq!(sink.got, expected, "chunks reassemble the body");
}

#[test]
fn streamed_fetch_cancels_mid_body() {
    let body: Vec<u8> = vec![7u8; 2_000_000];
    let port = serve(1, "HTTP/1.1 200 OK", body);

    struct CancelAfter(usize);
    impl day_part_http::StreamSink for CancelAfter {
        fn chunk(&mut self, data: &[u8]) -> Result<(), HttpError> {
            self.0 += data.len();
            if self.0 > 100_000 {
                Err(HttpError::Io("cancelled".into()))
            } else {
                Ok(())
            }
        }
    }
    let err = day_part_http::fetch_streamed(
        &Request::get(format!("http://127.0.0.1:{port}/")),
        &mut CancelAfter(0),
    )
    .expect_err("cancelled");
    assert_eq!(err, HttpError::Io("cancelled".into()));
}

#[test]
fn streamed_head_abort() {
    let port = serve(1, "HTTP/1.1 200 OK", b"body".to_vec());
    struct RejectHead;
    impl day_part_http::StreamSink for RejectHead {
        fn head(&mut self, _s: u16, _h: &[(String, String)]) -> bool {
            false
        }
        fn chunk(&mut self, _d: &[u8]) -> Result<(), HttpError> {
            panic!("no chunks after an aborted head");
        }
    }
    let err = day_part_http::fetch_streamed(
        &Request::get(format!("http://127.0.0.1:{port}/")),
        &mut RejectHead,
    )
    .expect_err("aborted");
    assert_eq!(err, HttpError::Io("aborted".into()));
}
