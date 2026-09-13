// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The bridged half of day-part-http (docs/bridge.md "Callbacks"): on Android the asynchronous
//! entry points run on OkHttp's own dispatcher, and on HarmonyOS every entry point runs the
//! ArkTS Network Kit on the JS thread; both complete a `Done<Vec<u8>>` with the response
//! envelope, so no Rust thread is parked behind a request and a dropped future cancels the call
//! through its token. Android's blocking entry points keep the synchronous shim in
//! `platform/android/java` (`DayHttp.java`), whose client, header block and error mapping the
//! Java arm shares.

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

    // HarmonyOS: `@ohos.net.http` (the OpenHarmony SDK's own stack: system proxy, platform TLS).
    // The whole request is a promise on the JS thread; its settlement packs the same envelope
    // the Java arm does and completes the token. `destroy()` is the cancellation, and a
    // request cancelled that way answers with the cancelled sentinel rather than the kit's
    // own error.
    #[day_bridge::impl(arkts, platforms = [ohos])]
    arkts!(
        prelude = r#"
            import { http } from '@kit.NetworkKit';
            import { BusinessError } from '@kit.BasicServicesKit';
            import { util } from '@kit.ArkTS';
        "#,
        body = r#"
            const dayCalls: Map<number, http.HttpRequest> = new Map<number, http.HttpRequest>();
            const dayCancelled: Set<number> = new Set<number>();

            function packEnvelope(status: number, meta: string, payload: Uint8Array): Uint8Array {
              const metaBytes = new util.TextEncoder().encodeInto(meta);
              const out = new Uint8Array(8 + metaBytes.length + payload.length);
              const view = new DataView(out.buffer);
              view.setInt32(0, status, false);
              view.setInt32(4, metaBytes.length, false);
              out.set(metaBytes, 8);
              out.set(payload, 8 + metaBytes.length);
              return out;
            }

            function errorEnvelope(sentinel: number, message: string): Uint8Array {
              return packEnvelope(sentinel, message, new Uint8Array(0));
            }

            // Network Kit error codes (2300xxx) onto the crate's transport sentinels.
            function sentinelOf(code: number): number {
              switch (code) {
                case 2300003: return -6;
                case 2300006: return -2;
                case 2300007: return -4;
                case 2300028: return -1;
                case 2300035:
                case 2300051:
                case 2300060: return -3;
                default: return -5;
              }
            }

            function methodOf(method: string): http.RequestMethod {
              switch (method) {
                case 'POST': return http.RequestMethod.POST;
                case 'PUT': return http.RequestMethod.PUT;
                case 'DELETE': return http.RequestMethod.DELETE;
                case 'HEAD': return http.RequestMethod.HEAD;
                case 'OPTIONS': return http.RequestMethod.OPTIONS;
                case 'TRACE': return http.RequestMethod.TRACE;
                case 'CONNECT': return http.RequestMethod.CONNECT;
                default: return http.RequestMethod.GET;
              }
            }

            function headerBlock(header: Object): string {
              let out = '';
              const record = header as Record<string, string>;
              for (const key of Object.keys(record)) {
                out += key + '\n' + String(record[key]) + '\n';
              }
              return out;
            }

            export function fetch_native(method: string, url: string, headers: string,
                                         body: Uint8Array, timeoutMs: number, done: number): void {
              const request = http.createHttp();
              dayCalls.set(done, request);
              const header: Record<string, string> = {};
              const lines = headers.split('\n');
              for (let i = 0; i + 1 < lines.length; i += 2) {
                header[lines[i]] = lines[i + 1];
              }
              const options: http.HttpRequestOptions = {
                method: methodOf(method),
                header: header,
                connectTimeout: timeoutMs,
                readTimeout: timeoutMs,
                expectDataType: http.HttpDataType.ARRAY_BUFFER,
                usingProxy: true,
              };
              if (body.length > 0) {
                options.extraData = body.buffer.slice(body.byteOffset, body.byteOffset + body.byteLength);
              }
              request.request(url, options).then((response: http.HttpResponse) => {
                dayCalls.delete(done);
                let payload = new Uint8Array(0);
                if (response.result instanceof ArrayBuffer) {
                  payload = new Uint8Array(response.result);
                } else if (typeof response.result === 'string') {
                  payload = new util.TextEncoder().encodeInto(response.result);
                }
                fetch_native_complete(done, packEnvelope(response.responseCode, headerBlock(response.header), payload));
                request.destroy();
              }).catch((err: BusinessError) => {
                dayCalls.delete(done);
                const cancelled = dayCancelled.delete(done);
                fetch_native_complete(done, errorEnvelope(cancelled ? -7 : sentinelOf(err.code), `${err.code}: ${err.message}`));
                request.destroy();
              });
            }

            export function cancel_native(token: number): void {
              const request = dayCalls.get(token);
              if (request) {
                dayCancelled.add(token);
                request.destroy();
              }
            }
        "#,
    );

    // Every other target realizes its asynchronous forms natively (Apple, Windows, the web) or
    // on a worker thread over its blocking arm (the Rust fallback), so the bridge answers only on
    // Android and HarmonyOS; elsewhere these are never reached.
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
#[cfg(any(target_os = "android", all(target_os = "linux", target_env = "ohos")))]
pub(crate) fn cancel(token: u64) {
    cancel_native(token as i64);
}

/// The envelope's header block: `k\nv\n…`, the same layout `DayHttp.headerBlock` writes.
#[cfg(any(target_os = "android", all(target_os = "linux", target_env = "ohos")))]
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
#[cfg(any(target_os = "android", all(target_os = "linux", target_env = "ohos")))]
pub(crate) fn bridge_error(e: day_bridge::Error) -> super::HttpError {
    match e {
        day_bridge::Error::Unsupported => super::HttpError::Unsupported,
        other => super::HttpError::Io(other.to_string()),
    }
}
