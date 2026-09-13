// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The bridged half of day-part-http (docs/bridge.md "Callbacks"): on Android the asynchronous
//! entry points run on OkHttp's own dispatcher and complete a `Done<Vec<u8>>` with the response
//! envelope, so no Rust thread is parked behind a request and a dropped future cancels the call
//! through its token. The blocking entry points keep the synchronous shim in
//! `platform/android/java` (`DayHttp.java`), whose client, header block and error mapping this
//! arm shares.

day_bridge::bridge! {
    #[day_bridge::declare]
    extern "day" {
        /// Start a request. `headers` is the envelope's header block (`k\nv\n…`), `body` the
        /// request body (empty for none). `done` completes with a response envelope in
        /// `day_android::envelope`'s layout — a negative status is a transport sentinel — or
        /// fails when the stack could not even start the call.
        fn fetch_native(
            method: &str,
            url: &str,
            headers: &str,
            body: &[u8],
            timeout_ms: i32,
            done: day_bridge::Done<Vec<u8>>,
        ) -> Result<(), day_bridge::Error>;
        /// Cancel the request started under `token` (its `Done`'s token). A miss — completed,
        /// or never started — is a no-op; the completion then arrives as the cancelled sentinel.
        fn cancel_native(token: i64);
    }

    // Android: `Call.enqueue` on the shared OkHttp client from DayHttp.java. The callback runs on
    // OkHttp's dispatcher thread and completes the token there (docs/bridge.md "Threads"); the
    // token is registered before the call goes out, so a cancel can never miss a live request.
    #[day_bridge::impl(java, platforms = [android])]
    java!(
        prelude = r#"
            import java.io.IOException;
            import java.util.concurrent.ConcurrentHashMap;
            import dev.daybrite.day.bridge.DayEnvelope;
            import dev.daybrite.day.http.DayHttp;
            import okhttp3.Call;
            import okhttp3.Callback;
            import okhttp3.MediaType;
            import okhttp3.RequestBody;
            import okhttp3.Response;
        "#,
        body = r#"
            private static final ConcurrentHashMap<Long, Call> CALLS = new ConcurrentHashMap<>();

            public static void fetch_native(String method, String url, String headers, byte[] body,
                                            int timeoutMs, final long done) {
                okhttp3.Request request;
                try {
                    okhttp3.Request.Builder b = new okhttp3.Request.Builder().url(url);
                    String[] lines = headers.split("\n", -1);
                    for (int i = 0; i + 1 < lines.length; i += 2) {
                        b.addHeader(lines[i], lines[i + 1]); // duplicates allowed, sent in order
                    }
                    // POST/PUT/PATCH require a RequestBody (an empty one is fine); GET/HEAD must
                    // pass null.
                    RequestBody rb = null;
                    if (body.length > 0 || "POST".equals(method) || "PUT".equals(method)
                            || "PATCH".equals(method)) {
                        rb = RequestBody.create(body, (MediaType) null);
                    }
                    request = b.method(method, rb).build();
                } catch (IllegalArgumentException e) {
                    // Request.Builder.url rejected it: bad url / scheme.
                    fetch_native_complete(done, DayEnvelope.error(-6, e.toString()));
                    return;
                }
                final Call call = DayHttp.client(timeoutMs).newCall(request);
                CALLS.put(done, call);
                call.enqueue(new Callback() {
                    @Override public void onFailure(Call c, IOException e) {
                        CALLS.remove(done);
                        fetch_native_complete(done, DayHttp.mapError(c, e));
                    }

                    @Override public void onResponse(Call c, Response resp) {
                        CALLS.remove(done);
                        byte[] envelope;
                        try (Response r = resp) {
                            // 4xx/5xx bodies arrive on the same body() — still a RESPONSE.
                            byte[] payload = r.body() == null ? new byte[0] : r.body().bytes();
                            envelope = DayEnvelope.pack(r.code(), DayHttp.headerBlock(r.headers()), payload);
                        } catch (Exception e) {
                            envelope = DayHttp.mapError(c, e);
                        }
                        fetch_native_complete(done, envelope);
                    }
                });
            }

            public static void cancel_native(long token) {
                Call c = CALLS.remove(token);
                if (c != null) c.cancel();
            }
        "#,
    );

    // Every other target realizes its asynchronous forms natively (Apple, the web) or on a
    // worker thread over its blocking arm (Windows, the Rust fallback), so the bridge answers
    // only on Android; elsewhere these are never reached.
    #[day_bridge::impl(rust, platforms = [other])]
    fn fetch_native(
        _method: &str,
        _url: &str,
        _headers: &str,
        _body: &[u8],
        _timeout_ms: i32,
        _done: day_bridge::Done<Vec<u8>>,
    ) -> Result<(), day_bridge::Error> {
        Err(day_bridge::Error::Unsupported)
    }

    #[day_bridge::impl(rust, platforms = [other])]
    fn cancel_native(_token: i64) {}
}

/// Cancel the request started under `token` — the generated arm is private to this module.
#[cfg(target_os = "android")]
pub(crate) fn cancel(token: u64) {
    cancel_native(token as i64);
}

/// The envelope's header block: `k\nv\n…`, the same layout `DayHttp.headerBlock` writes.
#[cfg(target_os = "android")]
pub(crate) fn header_block(headers: &[(String, String)]) -> String {
    let mut out = String::new();
    for (k, v) in headers {
        out.push_str(k);
        out.push('\n');
        out.push_str(v);
        out.push('\n');
    }
    out
}

/// A bridge failure as this crate's error: the stack could not start the call at all.
#[cfg(target_os = "android")]
pub(crate) fn bridge_error(e: day_bridge::Error) -> super::HttpError {
    match e {
        day_bridge::Error::Unsupported => super::HttpError::Unsupported,
        other => super::HttpError::Io(other.to_string()),
    }
}
