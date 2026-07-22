// day-part-http's Android shim — OkHttp riding the platform's policy rails: the system
// ProxySelector (per-network proxy/PAC), VPN routing, network security config + the user CA
// store all still apply (OkHttp uses the platform TrustManager and NetworkSecurityPolicy).
// The engine swap (from java.net.HttpURLConnection, 2026-07) adds HTTP/2, real PATCH, and
// per-call cancellation — AOSP's HttpURLConnection has been a frozen OkHttp fork since 4.4,
// so this is the same lineage, current. BLOCKING by design: the Rust side calls this on the
// caller's (non-UI) thread via the attached JVM. Results cross JNI as ONE byte[] envelope
// (a single array copy each way):
//   [0..4)  status as i32 BE; NEGATIVE = transport error sentinel:
//           -1 timeout, -2 dns, -3 tls, -4 connect, -5 io, -6 bad url, -7 cancelled
//   [4..8)  header-block length as i32 BE
//   then    header block "k\nv\n..." UTF-8 (or the error message for sentinels)
//   then    body bytes (fetch) / an 8-byte BE bytes-written count (fetchToFile)
package dev.daybrite.day.http;

import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.net.ConnectException;
import java.net.SocketTimeoutException;
import java.net.UnknownHostException;
import java.nio.ByteBuffer;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.TimeUnit;

import okhttp3.Call;
import okhttp3.Headers;
import okhttp3.MediaType;
import okhttp3.OkHttpClient;
import okhttp3.RequestBody;
import okhttp3.Response;

public final class DayHttp {
    // One engine: the dispatcher + connection pool (keep-alive, HTTP/2 multiplexing) are shared.
    // Per-call timeout variants via newBuilder() reuse them — an OkHttp-documented cheap clone.
    private static OkHttpClient base;

    private static synchronized OkHttpClient client(int timeoutMs) {
        if (base == null) base = new OkHttpClient();
        // connect/read/write are PER-PHASE idle-style bounds (no callTimeout), preserving the
        // crate's "timeout bounds progress, not the transfer" contract for long downloads.
        return base.newBuilder()
                .connectTimeout(timeoutMs, TimeUnit.MILLISECONDS)
                .readTimeout(timeoutMs, TimeUnit.MILLISECONDS)
                .writeTimeout(timeoutMs, TimeUnit.MILLISECONDS)
                .build();
    }

    // In-flight calls by Rust-side cancel token. The put-before-execute / remove-in-finally
    // pairing keeps the map leak-free (every put has a matching remove on the same thread);
    // cancel() is remove-then-cancel with NO tombstone, so a cancel racing registration
    // degrades to discard-only: the request runs out under its timeout with nobody reading
    // the result (the Rust future is already gone).
    private static final ConcurrentHashMap<Long, Call> CALLS = new ConcurrentHashMap<>();

    // Cancel the in-flight call registered under token — safe from any thread (Call.cancel is).
    public static void cancel(long token) {
        Call c = CALLS.remove(token);
        if (c != null) c.cancel();
    }

    public static byte[] fetch(String method, String url, String[] kv, byte[] body, int timeoutMs,
                               long cancelToken) {
        return run(method, url, kv, body, timeoutMs, null, cancelToken);
    }

    public static byte[] fetchToFile(String method, String url, String[] kv, byte[] body,
                                     int timeoutMs, String dest) {
        return run(method, url, kv, body, timeoutMs, dest, 0);
    }

    private static okhttp3.Request build(String method, String url, String[] kv, byte[] body) {
        okhttp3.Request.Builder b = new okhttp3.Request.Builder().url(url);
        for (int i = 0; i + 1 < kv.length; i += 2) {
            b.addHeader(kv[i], kv[i + 1]); // duplicates allowed, sent in order
        }
        // POST/PUT/PATCH require a RequestBody (an empty one is fine); GET/HEAD must pass null.
        RequestBody rb = null;
        if ((body != null && body.length > 0)
                || "POST".equals(method) || "PUT".equals(method) || "PATCH".equals(method)) {
            rb = RequestBody.create(body == null ? new byte[0] : body, (MediaType) null);
        }
        return b.method(method, rb).build();
    }

    private static byte[] run(String method, String url, String[] kv, byte[] body,
                              int timeoutMs, String dest, long token) {
        Call call = null;
        try {
            call = client(timeoutMs).newCall(build(method, url, kv, body));
            if (token != 0) CALLS.put(token, call);
            try (Response resp = call.execute()) {
                int status = resp.code();
                String headers = headerBlock(resp.headers());
                byte[] payload;
                if (dest == null) {
                    // 4xx/5xx bodies arrive on the same body() — still a RESPONSE (no
                    // getErrorStream split as under HttpURLConnection).
                    payload = resp.body() == null ? new byte[0] : resp.body().bytes();
                } else {
                    InputStream in = resp.body() == null ? null : resp.body().byteStream();
                    long written = copyToFile(in, dest);
                    payload = ByteBuffer.allocate(8).putLong(written).array();
                }
                return envelope(status, headers, payload);
            }
        } catch (IllegalArgumentException e) {
            return error(-6, e); // Request.Builder.url rejected it (bad url / scheme)
        } catch (Exception e) {
            return mapError(call, e);
        } finally {
            if (token != 0) CALLS.remove(token);
        }
    }

    private static byte[] mapError(Call call, Exception e) {
        // Cancellation FIRST: a cancel mid-read surfaces as SocketException("Socket closed") or
        // IOException("Canceled"), not a distinct exception type — isCanceled() is the truth.
        if (call != null && call.isCanceled()) return error(-7, e);
        if (e instanceof SocketTimeoutException) return error(-1, e);
        if (e instanceof UnknownHostException) return error(-2, e);
        if (e instanceof javax.net.ssl.SSLException) return error(-3, e);
        if (e instanceof ConnectException) return error(-4, e);
        return error(-5, e);
    }

    private static String headerBlock(Headers h) {
        // Indexed iteration: arrival order, duplicates preserved.
        StringBuilder sb = new StringBuilder();
        for (int i = 0; i < h.size(); i++) {
            sb.append(h.name(i)).append('\n').append(h.value(i)).append('\n');
        }
        return sb.toString();
    }

    private static long copyToFile(InputStream in, String dest) throws IOException {
        long total = 0;
        try (FileOutputStream out = new FileOutputStream(dest)) {
            if (in != null) {
                byte[] chunk = new byte[65536];
                int n;
                while ((n = in.read(chunk)) > 0) {
                    out.write(chunk, 0, n);
                    total += n;
                }
                in.close();
            }
        }
        return total;
    }

    private static byte[] envelope(int status, String headers, byte[] payload) {
        return dev.daybrite.day.bridge.DayEnvelope.pack(status, headers, payload);
    }

    // --- Streaming (fetch_streamed): open → envelope(status/headers/4-byte handle), then the
    // Rust side PULLS 64 KiB chunks with streamRead until an empty array (EOF), and streamClose
    // releases the connection (also called on abort/cancel).
    private static final java.util.Map<Integer, Object[]> STREAMS = new java.util.HashMap<>();
    private static int nextStream = 1;

    public static byte[] streamOpen(String method, String url, String[] kv, byte[] body, int timeoutMs) {
        Call call = null;
        try {
            call = client(timeoutMs).newCall(build(method, url, kv, body));
            Response resp = call.execute();
            int status = resp.code();
            String headers = headerBlock(resp.headers());
            InputStream in = resp.body() == null ? null : resp.body().byteStream();
            int handle;
            synchronized (STREAMS) {
                handle = nextStream++;
                STREAMS.put(handle, new Object[] { call, resp, in });
            }
            return envelope(status, headers, ByteBuffer.allocate(4).putInt(handle).array());
        } catch (IllegalArgumentException e) {
            return error(-6, e);
        } catch (Exception e) {
            return mapError(call, e);
        }
    }

    // One pulled chunk: empty array = EOF; null = read error (stream auto-closed).
    public static byte[] streamRead(int handle) {
        Object[] entry;
        synchronized (STREAMS) { entry = STREAMS.get(handle); }
        if (entry == null) return null;
        InputStream in = (InputStream) entry[2];
        try {
            if (in == null) return new byte[0];
            byte[] chunk = new byte[65536];
            int n = in.read(chunk);
            if (n <= 0) return new byte[0];
            return java.util.Arrays.copyOf(chunk, n);
        } catch (IOException e) {
            streamClose(handle);
            return null;
        }
    }

    public static void streamClose(int handle) {
        Object[] entry;
        synchronized (STREAMS) { entry = STREAMS.remove(handle); }
        if (entry == null) return;
        try {
            ((Response) entry[1]).close();
        } catch (Exception ignored) {}
        // Frees the connection immediately on a mid-body abort; a no-op after normal EOF.
        ((Call) entry[0]).cancel();
    }

    private static byte[] error(int sentinel, Exception e) {
        return dev.daybrite.day.bridge.DayEnvelope.error(sentinel, e.toString());
    }
}
