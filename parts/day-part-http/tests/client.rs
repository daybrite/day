// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The client over this platform's own transport, against the crate's loopback test server.
//! Each test checks what [`Client::capabilities`] claims: a capability the platform reports is
//! exercised end to end, and one it does not report must fail with `Unsupported` rather than
//! quietly doing something else.

use std::future::Future;
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Wake, Waker};
use std::time::Duration;

use day_part_http::testing::{Server, pattern_sha256};
use day_part_http::{
    Cache, CachePolicy, Client, Cookies, Form, HttpError, Message, Redirects, Request, Scheme,
};
use sha2::{Digest, Sha256};

fn block_on<F: Future>(future: F) -> F::Output {
    struct Unpark(std::thread::Thread);
    impl Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        if let Poll::Ready(value) = future.as_mut().poll(&mut cx) {
            return value;
        }
        std::thread::park_timeout(Duration::from_millis(50));
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn jar_client() -> Client {
    Client::builder()
        .cookies(Cookies::jar())
        .cache(Cache::Off)
        .build()
}

#[test]
fn fetches_through_the_platform_stack() {
    let server = Server::start().expect("server");
    let resp = block_on(jar_client().fetch_future(Request::get(server.url("/")))).expect("fetch");
    assert_eq!((resp.status, resp.text().as_ref()), (200, "day-http-ok"));
    let patch = Request::patch(server.url("/"), Vec::new());
    let resp = block_on(jar_client().fetch_future(patch)).expect("patch");
    assert_eq!(resp.text(), "day-http-ok:PATCH");
}

#[test]
fn a_redirect_chain_is_followed() {
    let server = Server::start().expect("server");
    let client = jar_client();
    let resp =
        block_on(client.fetch_future(Request::get(server.url("/redirect/3")))).expect("fetch");
    assert_eq!((resp.status, resp.text().as_ref()), (200, "redirected"));
    if client.capabilities().manual_redirects {
        assert!(resp.url.ends_with("/redirect/0"), "{}", resp.url);
    }
}

#[test]
fn a_redirect_handler_sees_each_hop_where_the_platform_allows() {
    let server = Server::start().expect("server");
    let hops = Arc::new(AtomicUsize::new(0));
    let seen = hops.clone();
    let client = Client::builder()
        .cookies(Cookies::jar())
        .on_redirect(move |_, reply| {
            seen.fetch_add(1, Ordering::SeqCst);
            reply.follow();
        })
        .build();
    let result = block_on(client.fetch_future(Request::get(server.url("/redirect/2"))));
    if client.capabilities().manual_redirects {
        assert_eq!(result.expect("fetch").text(), "redirected");
        assert_eq!(hops.load(Ordering::SeqCst), 2);
    } else {
        assert_eq!(result.unwrap_err(), HttpError::Unsupported);
    }
    let never = Client::builder()
        .redirects(Redirects::Never)
        .cookies(Cookies::jar())
        .build();
    let result = block_on(never.fetch_future(Request::get(server.url("/redirect/1"))));
    if never.capabilities().manual_redirects {
        assert_eq!(result.expect("fetch").status, 302);
    }
}

#[test]
fn basic_digest_and_bearer_challenges_are_answered() {
    let server = Server::start().expect("server");
    // A client remembers credentials that worked for an origin and realm, and all three routes
    // share both, so each path gets a client of its own to be asked afresh.
    let schemes = Arc::new(Mutex::new(Vec::new()));
    let client = || {
        let log = schemes.clone();
        Client::builder()
            .cookies(Cookies::jar())
            .cache(Cache::Off)
            .on_challenge(move |challenge, reply| {
                log.lock().unwrap().push(challenge.scheme.clone());
                match challenge.scheme {
                    Scheme::Bearer => reply.bearer("day-token"),
                    _ => reply.credential("day", "sunrise"),
                }
            })
            .build()
    };
    for path in ["/basic-auth/day/sunrise", "/digest-auth/day/sunrise"] {
        let resp = block_on(client().fetch_future(Request::get(server.url(path)))).expect("fetch");
        assert_eq!(
            (resp.status, resp.text().as_ref()),
            (200, "authenticated day"),
            "{path}"
        );
    }
    let resp = block_on(client().fetch_future(Request::get(server.url("/bearer/day-token"))))
        .expect("fetch");
    assert_eq!((resp.status, resp.text().as_ref()), (200, "authenticated"));
    let asked = schemes.lock().unwrap().clone();
    assert!(asked.contains(&Scheme::Basic), "{asked:?}");
    assert!(asked.contains(&Scheme::Digest), "{asked:?}");
    assert!(asked.contains(&Scheme::Bearer), "{asked:?}");
}

#[test]
fn without_a_challenge_handler_the_401_is_the_response() {
    let server = Server::start().expect("server");
    let resp = block_on(jar_client().fetch_future(Request::get(server.url("/basic-auth/a/b"))))
        .expect("fetch");
    assert_eq!(resp.status, 401);
}

#[test]
fn a_jar_keeps_cookies_across_a_redirect() {
    let server = Server::start().expect("server");
    let client = jar_client();
    let resp = block_on(client.fetch_future(Request::get(server.url("/cookies/set?flavor=oat"))))
        .expect("set");
    assert_eq!(resp.text(), "flavor=oat");
    assert!(client.cookies().iter().any(|c| c.name == "flavor"));
    let resp = block_on(client.fetch_future(Request::get(server.url("/cookies/delete?flavor"))))
        .expect("delete");
    assert_eq!(resp.text(), "none");
    assert!(client.cookies().is_empty());
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
#[test]
fn the_platform_store_keeps_cookies_on_apple() {
    let server = Server::start().expect("server");
    let client = Client::builder().cookies(Cookies::Platform).build();
    if !client.capabilities().platform_cookies {
        return;
    }
    client.clear_cookies();
    let resp = block_on(client.fetch_future(Request::get(server.url("/cookies/set?shared=1"))))
        .expect("set");
    assert_eq!(resp.text(), "shared=1");
    assert!(client.cookies().iter().any(|c| c.name == "shared"));
    client.clear_cookies();
    assert!(!client.cookies().iter().any(|c| c.name == "shared"));
}

#[test]
fn a_large_body_streams_in_chunks_and_hashes_correctly() {
    const LEN: u64 = 6_000_000;
    let server = Server::start().expect("server");
    let client = jar_client();
    let streaming =
        block_on(client.send_future(Request::get(server.url(&format!("/bytes/{LEN}")))))
            .expect("head");
    assert_eq!(streaming.status(), 200);
    assert_eq!(streaming.expected_length(), Some(LEN));
    let mut body = streaming.into_body();
    let mut hasher = Sha256::new();
    let mut chunks = 0;
    let mut total = 0u64;
    while let Some(chunk) = block_on(body.next()) {
        let chunk = chunk.expect("chunk");
        total += chunk.len() as u64;
        hasher.update(&chunk);
        chunks += 1;
    }
    assert_eq!(total, LEN);
    assert_eq!(hex(hasher.finalize().as_slice()), pattern_sha256(LEN));
    if client.capabilities().streaming {
        assert!(chunks > 1, "one chunk for {LEN} bytes");
    }
}

#[test]
fn a_validated_range_resumes_and_a_stale_one_restarts() {
    let server = Server::start().expect("server");
    let client = jar_client();
    let url = server.url("/bytes/5000");
    let resume = Request::get(url.clone())
        .header("Range", "bytes=1000-")
        .header("If-Range", "\"day-bytes-5000\"");
    let resp = block_on(client.fetch_future(resume)).expect("resume");
    assert_eq!((resp.status, resp.body.len()), (206, 4000));
    assert_eq!(resp.header("content-range"), Some("bytes 1000-4999/5000"));
    let stale = Request::get(url)
        .header("Range", "bytes=1000-")
        .header("If-Range", "\"something-else\"");
    let resp = block_on(client.fetch_future(stale)).expect("restart");
    assert_eq!((resp.status, resp.body.len()), (200, 5000));
}

#[test]
fn uploads_report_the_bytes_the_server_received() {
    let server = Server::start().expect("server");
    let client = jar_client();
    let caps = client.capabilities();
    let payload: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    let digest = hex(Sha256::digest(&payload).as_slice());
    let expected = format!("{} {digest}", payload.len());

    let sent = Arc::new(Mutex::new(Vec::new()));
    let log = sent.clone();
    let bytes = Request::post(server.url("/upload"), payload.clone())
        .upload_progress(move |done, total| log.lock().unwrap().push((done, total)));
    let resp = block_on(client.fetch_future(bytes)).expect("bytes");
    assert_eq!(resp.text(), expected);
    if caps.upload_progress {
        let sent = sent.lock().unwrap();
        assert_eq!(
            sent.last().map(|s| s.0),
            Some(payload.len() as u64),
            "{sent:?}"
        );
    }

    let dir = std::env::temp_dir().join(format!("day-http-upload-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let path = dir.join("payload.bin");
    std::fs::File::create(&path)
        .and_then(|mut f| f.write_all(&payload))
        .expect("write");
    let file = Request::new(day_part_http::Method::Put, server.url("/upload")).body_file(&path);
    let stream = Request::post(server.url("/upload"), Vec::new()).body_stream(
        std::io::Cursor::new(payload.clone()),
        Some(payload.len() as u64),
    );
    for (name, request) in [("file", file), ("stream", stream)] {
        let result = block_on(client.fetch_future(request));
        if caps.upload_streaming {
            assert_eq!(result.expect(name).text(), expected, "{name}");
        } else {
            assert_eq!(result.unwrap_err(), HttpError::Unsupported, "{name}");
        }
    }
    let form =
        Form::new()
            .text("title", "sunrise")
            .file("photo", &path, "application/octet-stream");
    let result =
        block_on(client.fetch_future(Request::post(server.url("/upload"), Vec::new()).form(form)));
    if caps.upload_streaming {
        let text = result.expect("form").text().into_owned();
        let len: usize = text.split(' ').next().unwrap_or("0").parse().unwrap_or(0);
        assert!(len > payload.len(), "{text}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_total_limit_ends_a_slow_request() {
    let server = Server::start().expect("server");
    let client = Client::builder()
        .cookies(Cookies::jar())
        .timeout_total(Duration::from_millis(300))
        .build();
    let err = block_on(client.fetch_future(Request::get(server.url("/delay/3000")))).unwrap_err();
    assert_eq!(err, HttpError::Timeout);
}

#[test]
fn a_platform_cache_answers_a_repeat_request() {
    let server = Server::start().expect("server");
    let client = Client::builder()
        .cookies(Cookies::jar())
        .cache(Cache::platform())
        .build();
    let url = server.url("/cache/60");
    let first = block_on(client.fetch_future(Request::get(url.clone()))).expect("first");
    let second = block_on(client.fetch_future(Request::get(url.clone()))).expect("second");
    if client.capabilities().platform_cache {
        assert_eq!(first.text(), second.text());
        let reload = Request::get(url).cache(CachePolicy::Reload);
        let third = block_on(client.fetch_future(reload)).expect("reload");
        assert_ne!(third.text(), first.text());
        client.clear_cache();
    } else {
        assert_ne!(first.text(), second.text());
    }
}

#[test]
fn a_websocket_echoes_and_closes_where_the_platform_has_them() {
    let server = Server::start().expect("server");
    let client = jar_client();
    let opened = block_on(
        client.websocket_future(Request::get(server.ws_url("/ws/echo")).protocols(["day"])),
    );
    if !client.capabilities().websockets {
        assert_eq!(opened.unwrap_err(), HttpError::Unsupported);
        return;
    }
    let mut socket = opened.expect("open");
    assert_eq!(socket.protocol().as_deref(), Some("day"));
    block_on(socket.send_future(Message::Text("hello".into())))
        .expect("delivered")
        .expect("sent");
    assert_eq!(
        block_on(socket.next()),
        Some(Ok(Message::Text("hello".into())))
    );
    block_on(socket.send_future(Message::Binary(vec![1, 2, 3])))
        .expect("delivered")
        .expect("sent");
    assert_eq!(
        block_on(socket.next()),
        Some(Ok(Message::Binary(vec![1, 2, 3])))
    );
    if client.capabilities().websocket_ping {
        block_on(socket.sender().ping_future())
            .expect("delivered")
            .expect("pong");
    }
    socket.close(4000, "done");
    let closing = block_on(socket.next());
    assert!(
        matches!(closing, Some(Ok(Message::Close { code: 4000, .. }))),
        "{closing:?}"
    );
    assert_eq!(block_on(socket.next()), None);
}
