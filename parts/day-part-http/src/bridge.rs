// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The bridged transports of day-part-http (docs/bridge.md "Streams"): Android's Java arm over
//! OkHttp, HarmonyOS's ArkTS arm over the Network Kit, and the web's JavaScript arm over fetch and
//! WebSocket. Each exchange is one `Emit<Vec<u8>>` stream of tagged frames (a head, body chunks,
//! upload progress, trust questions, metrics, the end or a failure), and plain calls go the other
//! way: demand for more chunks, answers, body chunks for a streamed upload, cancellation.
//! WebSockets are a second stream of frames. Every frame starts with its tag and the stream's
//! token, so a frame that arrives before the call that started the stream has returned still
//! finds its exchange. `src/bridged.rs` reads the frames on every platform.

day_bridge::bridge! {
    #[day_bridge::declare]
    extern "day" {
        /// Android: an OkHttp client for one `Client`, sharing the process's connection pool.
        /// Zero means "not set" for the limits and the cache; an empty `identity` means none.
        fn client_native(
            idle_ms: i32,
            total_ms: i32,
            cache_bytes: i64,
            max_per_host: i32,
            ask_trust: bool,
            identity: &[u8],
            password: &str,
        ) -> Result<i64, day_bridge::Error>;
        /// Android: forget a client.
        fn client_release_native(client: i64);
        /// Android: empty the shared HTTP cache.
        fn cache_clear_native(client: i64);
        /// Android: start one exchange. `body_kind` is 0 none, 1 `body`, 2 the file at
        /// `body_path`, 3 a stream pulled through `need body` frames; `body_size` is −1 when
        /// unknown. `cache` is 0 the response's own rules, 1 reload, 2 prefer the cache.
        fn exchange_native(
            client: i64,
            method: &str,
            url: &str,
            headers: &str,
            body_kind: i32,
            body: &[u8],
            body_path: &str,
            body_size: i64,
            idle_ms: i32,
            cache: i32,
            emit: day_bridge::Emit<Vec<u8>>,
        ) -> Result<(), day_bridge::Error>;
        /// Android: allow `chunks` more body reads.
        fn demand_native(token: i64, chunks: i32);
        /// Android: answer question `question` with 0 default, 1 accept, 2 reject or 3 cancel.
        fn answer_native(token: i64, question: i32, answer: i32);
        /// Android: the next piece of a streamed request body; empty is the end.
        fn body_native(token: i64, chunk: &[u8]);
        /// Android: cancel an exchange; its stream then ends with the cancelled sentinel.
        fn exchange_cancel_native(token: i64);
        /// Android: open a WebSocket. `protocols` is comma-separated.
        fn ws_open_native(
            client: i64,
            url: &str,
            headers: &str,
            protocols: &str,
            emit: day_bridge::Emit<Vec<u8>>,
        ) -> Result<(), day_bridge::Error>;
        /// Android: queue a message; `false` when the socket is closed or its queue is full.
        fn ws_send_native(token: i64, binary: bool, data: &[u8]) -> Result<bool, day_bridge::Error>;
        /// Android: start the closing handshake.
        fn ws_close_native(token: i64, code: i32, reason: &str);
    }

    // Android: OkHttp on the platform's policy rails (system proxy, VPN routing, network security
    // config, the user CA store), with redirects and cookies left to the Rust client. A response
    // callback hands the body to a reader pool, so a slow reader never holds one of the
    // dispatcher's request slots; the reader waits for demand before each read.
    #[day_bridge::impl(java, platforms = [android])]
    java!(
        prelude = r#"
            import java.io.ByteArrayInputStream;
            import java.io.ByteArrayOutputStream;
            import java.io.DataOutputStream;
            import java.io.File;
            import java.io.FileInputStream;
            import java.io.IOException;
            import java.io.InterruptedIOException;
            import java.net.InetAddress;
            import java.net.InetSocketAddress;
            import java.net.Proxy;
            import java.net.Socket;
            import java.net.SocketTimeoutException;
            import java.nio.charset.StandardCharsets;
            import java.security.KeyStore;
            import java.security.cert.CertificateException;
            import java.security.cert.X509Certificate;
            import java.util.List;
            import java.util.concurrent.ArrayBlockingQueue;
            import java.util.concurrent.ConcurrentHashMap;
            import java.util.concurrent.ExecutorService;
            import java.util.concurrent.Executors;
            import java.util.concurrent.TimeUnit;
            import java.util.concurrent.atomic.AtomicInteger;
            import java.util.concurrent.atomic.AtomicLong;
            import javax.net.ssl.KeyManager;
            import javax.net.ssl.KeyManagerFactory;
            import javax.net.ssl.SSLContext;
            import javax.net.ssl.SSLEngine;
            import javax.net.ssl.TrustManager;
            import javax.net.ssl.X509ExtendedTrustManager;
            import javax.net.ssl.X509TrustManager;
            import dev.daybrite.day.bridge.DayBridge;
            import dev.daybrite.day.http.DayHttp;
            import okhttp3.Cache;
            import okhttp3.CacheControl;
            import okhttp3.Call;
            import okhttp3.Callback;
            import okhttp3.Connection;
            import okhttp3.CookieJar;
            import okhttp3.Dispatcher;
            import okhttp3.EventListener;
            import okhttp3.Handshake;
            import okhttp3.MediaType;
            import okhttp3.OkHttpClient;
            import okhttp3.Protocol;
            import okhttp3.Request;
            import okhttp3.RequestBody;
            import okhttp3.Response;
            import okhttp3.ResponseBody;
            import okhttp3.WebSocket;
            import okhttp3.WebSocketListener;
            import okio.BufferedSink;
            import okio.BufferedSource;
            import okio.ByteString;
        "#,
        body = r#"
            private static final int HEAD = 1, CHUNK = 2, SENT = 3, QUESTION = 4, METRICS = 5, END = 6,
                    FAILED = 7, NEED_BODY = 8;
            private static final int WS_OPEN = 16, WS_TEXT = 17, WS_BINARY = 18, WS_CLOSED = 19,
                    WS_FAILED = 20;
            private static final int ANSWER_DEFAULT = 0, ANSWER_ACCEPT = 1, ANSWER_REJECT = 2, ANSWER_CANCEL = 3;
            private static final int READ = 64 * 1024;

            private static final ConcurrentHashMap<Long, OkHttpClient> CLIENTS = new ConcurrentHashMap<>();
            private static final ConcurrentHashMap<Long, Exchange> EXCHANGES = new ConcurrentHashMap<>();
            private static final ConcurrentHashMap<Long, Ws> SOCKETS = new ConcurrentHashMap<>();
            private static final AtomicLong NEXT_CLIENT = new AtomicLong(1);
            private static final ExecutorService READERS = Executors.newCachedThreadPool();
            // The exchange whose TLS handshake runs on this thread, for the trust manager to ask on.
            private static final ThreadLocal<Exchange> HANDSHAKING = new ThreadLocal<>();
            private static Cache sharedCache;

            /** A frame: its tag, the stream's token, then its fields, big-endian. */
            private static final class Frame {
                private final ByteArrayOutputStream bytes = new ByteArrayOutputStream();
                private final DataOutputStream out = new DataOutputStream(bytes);

                Frame(int tag, long token) { u8(tag); i64(token); }

                Frame u8(int v) { try { out.writeByte(v); } catch (IOException ignored) {} return this; }
                Frame i32(int v) { try { out.writeInt(v); } catch (IOException ignored) {} return this; }
                Frame i64(long v) { try { out.writeLong(v); } catch (IOException ignored) {} return this; }
                Frame raw(byte[] b, int n) { try { out.write(b, 0, n); } catch (IOException ignored) {} return this; }
                Frame str(String s) {
                    byte[] b = (s == null ? "" : s).getBytes(StandardCharsets.UTF_8);
                    return i32(b.length).raw(b, b.length);
                }
                byte[] done() { return bytes.toByteArray(); }
            }

            private static Frame failure(long token, int sentinel, String message) {
                return new Frame(FAILED, token).i32(sentinel).str(message);
            }

            /** One exchange: its stream token, the call, demand for body reads, open questions. */
            private static final class Exchange {
                final long emit;
                final String host;
                final int idleMs;
                final Timing timing = new Timing(this);
                final ConcurrentHashMap<Integer, ArrayBlockingQueue<Integer>> questions = new ConcurrentHashMap<>();
                final AtomicInteger nextQuestion = new AtomicInteger();
                final ArrayBlockingQueue<byte[]> body = new ArrayBlockingQueue<>(4);
                volatile Call call;
                private int demand;
                private boolean cancelled;
                private boolean finished;

                Exchange(long emit, String host, int idleMs) {
                    this.emit = emit;
                    this.host = host;
                    this.idleMs = idleMs;
                }

                synchronized boolean isFinished() { return finished; }
                synchronized boolean isCancelled() { return cancelled; }

                void send(Frame frame) {
                    if (!isFinished()) exchange_native_emit(emit, frame.done());
                }

                void finish(Frame frame) {
                    synchronized (this) {
                        if (finished) return;
                        finished = true;
                        cancelled = true;
                        notifyAll();
                    }
                    EXCHANGES.remove(emit);
                    for (ArrayBlockingQueue<Integer> slot : questions.values()) slot.offer(ANSWER_CANCEL);
                    exchange_native_emit(emit, frame.done());
                    exchange_native_end(emit);
                }

                synchronized void addDemand(int chunks) {
                    demand += chunks;
                    notifyAll();
                }

                synchronized void takeDemand() throws IOException {
                    try {
                        while (demand == 0 && !cancelled) wait();
                    } catch (InterruptedException e) {
                        throw new InterruptedIOException("interrupted");
                    }
                    if (cancelled) throw new IOException("Canceled");
                    demand--;
                }

                void cancel() {
                    synchronized (this) {
                        cancelled = true;
                        notifyAll();
                    }
                    Call c = call;
                    if (c != null) c.cancel();
                    for (ArrayBlockingQueue<Integer> slot : questions.values()) slot.offer(ANSWER_CANCEL);
                    body.offer(new byte[0]);
                }

                /** Ask the client and wait on this thread for its answer. */
                int ask(Frame frame, int id) {
                    ArrayBlockingQueue<Integer> slot = new ArrayBlockingQueue<>(1);
                    questions.put(id, slot);
                    try {
                        if (isCancelled()) return ANSWER_CANCEL;
                        send(frame);
                        // The client answers every question within its own timeout; this bound only
                        // stops a lost answer from pinning a handshake forever.
                        Integer answer = slot.poll(10, TimeUnit.MINUTES);
                        return answer == null ? ANSWER_DEFAULT : answer;
                    } catch (InterruptedException e) {
                        return ANSWER_CANCEL;
                    } finally {
                        questions.remove(id);
                    }
                }

                /** The next piece of a streamed request body, asked for from Rust. */
                byte[] pull() throws IOException {
                    send(new Frame(NEED_BODY, emit).i32(READ));
                    try {
                        byte[] chunk = body.poll(idleMs, TimeUnit.MILLISECONDS);
                        if (chunk == null) throw new SocketTimeoutException("timeout");
                        if (isCancelled()) throw new IOException("Canceled");
                        return chunk;
                    } catch (InterruptedException e) {
                        throw new InterruptedIOException("interrupted");
                    }
                }
            }

            /** Timings and connection facts for one exchange, from OkHttp's events. */
            private static final class Timing extends EventListener {
                final Exchange exchange;
                long callStart, dnsStart, dnsEnd, connectStart, connectEnd, tlsStart, tlsEnd, responseStart;
                int reused = 2;
                String protocol = "", remote = "", tls = "";
                boolean fromCache;
                long sent = -1, received;

                Timing(Exchange exchange) { this.exchange = exchange; }

                @Override public void callStart(Call call) { callStart = System.nanoTime(); }
                @Override public void dnsStart(Call call, String domainName) { dnsStart = System.nanoTime(); }
                @Override public void dnsEnd(Call call, String domainName, List<InetAddress> addresses) { dnsEnd = System.nanoTime(); }
                @Override public void connectStart(Call call, InetSocketAddress address, Proxy proxy) {
                    connectStart = System.nanoTime();
                    reused = 0;
                }
                @Override public void secureConnectStart(Call call) {
                    tlsStart = System.nanoTime();
                    HANDSHAKING.set(exchange);
                }
                @Override public void secureConnectEnd(Call call, Handshake handshake) {
                    tlsEnd = System.nanoTime();
                    HANDSHAKING.remove();
                    if (handshake != null) tls = handshake.tlsVersion().javaName();
                }
                @Override public void connectEnd(Call call, InetSocketAddress address, Proxy proxy, Protocol p) { connectEnd = System.nanoTime(); }
                @Override public void connectFailed(Call call, InetSocketAddress address, Proxy proxy, Protocol p, IOException e) { HANDSHAKING.remove(); }
                @Override public void connectionAcquired(Call call, Connection connection) {
                    if (reused == 2) reused = 1;
                    protocol = connection.protocol().toString();
                    InetAddress address = connection.route().socketAddress().getAddress();
                    if (address != null) remote = address.getHostAddress();
                    Handshake handshake = connection.handshake();
                    if (handshake != null) tls = handshake.tlsVersion().javaName();
                }
                @Override public void cacheHit(Call call, Response response) { fromCache = true; }
                @Override public void responseHeadersStart(Call call) { responseStart = System.nanoTime(); }
                @Override public void requestBodyEnd(Call call, long byteCount) { sent = byteCount; }

                private static long micros(long start, long end) {
                    return start == 0 || end == 0 || end < start ? -1 : (end - start) / 1000;
                }

                Frame frame() {
                    long now = System.nanoTime();
                    return new Frame(METRICS, exchange.emit)
                            .i64(micros(dnsStart, dnsEnd))
                            .i64(micros(connectStart, connectEnd))
                            .i64(micros(tlsStart, tlsEnd))
                            .i64(micros(callStart, responseStart))
                            .i64(micros(callStart, now))
                            .str(protocol)
                            .u8(reused)
                            .str(remote)
                            .str(tls)
                            .u8(fromCache ? 1 : 0)
                            .i64(sent)
                            .i64(received);
                }
            }

            /** The platform's verdict first, then the client's answer. */
            private static final class AskingTrust extends X509ExtendedTrustManager {
                final X509TrustManager platform;

                AskingTrust(X509TrustManager platform) { this.platform = platform; }

                @Override public X509Certificate[] getAcceptedIssuers() { return platform.getAcceptedIssuers(); }
                @Override public void checkClientTrusted(X509Certificate[] chain, String authType) throws CertificateException { platform.checkClientTrusted(chain, authType); }
                @Override public void checkClientTrusted(X509Certificate[] chain, String authType, Socket socket) throws CertificateException { platform.checkClientTrusted(chain, authType); }
                @Override public void checkClientTrusted(X509Certificate[] chain, String authType, SSLEngine engine) throws CertificateException { platform.checkClientTrusted(chain, authType); }
                @Override public void checkServerTrusted(X509Certificate[] chain, String authType) throws CertificateException { decide(chain, authType, null, null); }
                @Override public void checkServerTrusted(X509Certificate[] chain, String authType, Socket socket) throws CertificateException { decide(chain, authType, socket, null); }
                @Override public void checkServerTrusted(X509Certificate[] chain, String authType, SSLEngine engine) throws CertificateException { decide(chain, authType, null, engine); }

                private void decide(X509Certificate[] chain, String authType, Socket socket, SSLEngine engine) throws CertificateException {
                    boolean trusted = true;
                    String error = "";
                    try {
                        // The hostname-aware checks keep a network security config's domain rules.
                        if (platform instanceof X509ExtendedTrustManager && socket != null) {
                            ((X509ExtendedTrustManager) platform).checkServerTrusted(chain, authType, socket);
                        } else if (platform instanceof X509ExtendedTrustManager && engine != null) {
                            ((X509ExtendedTrustManager) platform).checkServerTrusted(chain, authType, engine);
                        } else {
                            platform.checkServerTrusted(chain, authType);
                        }
                    } catch (CertificateException e) {
                        trusted = false;
                        error = String.valueOf(e.getMessage());
                    }
                    Exchange exchange = HANDSHAKING.get();
                    if (exchange == null) {
                        if (!trusted) throw new CertificateException(error);
                        return;
                    }
                    int id = exchange.nextQuestion.incrementAndGet();
                    Frame frame = new Frame(QUESTION, exchange.emit)
                            .i32(id).u8(1).str(exchange.host).u8(trusted ? 1 : 0).str(error).i32(chain.length);
                    for (X509Certificate certificate : chain) {
                        byte[] der = certificate.getEncoded();
                        frame.i32(der.length).raw(der, der.length);
                    }
                    int answer = exchange.ask(frame, id);
                    if (answer == ANSWER_ACCEPT) return;
                    if (answer == ANSWER_REJECT || answer == ANSWER_CANCEL) {
                        throw new CertificateException("the server is not trusted");
                    }
                    if (!trusted) throw new CertificateException(error);
                }
            }

            private static synchronized Cache sharedCache(long bytes) {
                if (sharedCache == null && DayBridge.ctx != null) {
                    sharedCache = new Cache(new File(DayBridge.ctx.getCacheDir(), "day-http"), bytes);
                }
                return sharedCache;
            }

            public static long client_native(int idleMs, int totalMs, long cacheBytes, int maxPerHost,
                                             boolean askTrust, byte[] identity, String password) throws Exception {
                OkHttpClient.Builder builder = DayHttp.client(idleMs).newBuilder()
                        .followRedirects(false)
                        .followSslRedirects(false)
                        .cookieJar(CookieJar.NO_COOKIES)
                        .eventListenerFactory(call -> {
                            Exchange exchange = call.request().tag(Exchange.class);
                            return exchange != null ? exchange.timing : EventListener.NONE;
                        });
                if (totalMs > 0) builder.callTimeout(totalMs, TimeUnit.MILLISECONDS);
                if (cacheBytes > 0) {
                    Cache cache = sharedCache(cacheBytes);
                    if (cache != null) builder.cache(cache);
                }
                if (maxPerHost > 0) {
                    Dispatcher dispatcher = new Dispatcher();
                    dispatcher.setMaxRequestsPerHost(maxPerHost);
                    builder.dispatcher(dispatcher);
                }
                if (askTrust || identity.length > 0) {
                    X509TrustManager platform = DayHttp.platformTrustManager();
                    X509TrustManager trust = askTrust ? new AskingTrust(platform) : platform;
                    KeyManager[] keys = null;
                    if (identity.length > 0) {
                        KeyStore store = KeyStore.getInstance("PKCS12");
                        store.load(new ByteArrayInputStream(identity), password.toCharArray());
                        KeyManagerFactory factory = KeyManagerFactory.getInstance(KeyManagerFactory.getDefaultAlgorithm());
                        factory.init(store, password.toCharArray());
                        keys = factory.getKeyManagers();
                    }
                    SSLContext context = SSLContext.getInstance("TLS");
                    context.init(keys, new TrustManager[] { trust }, null);
                    builder.sslSocketFactory(context.getSocketFactory(), trust);
                }
                long id = NEXT_CLIENT.getAndIncrement();
                CLIENTS.put(id, builder.build());
                return id;
            }

            public static void client_release_native(long client) {
                CLIENTS.remove(client);
            }

            public static void cache_clear_native(long client) {
                Cache cache = sharedCache;
                if (cache == null) return;
                try {
                    cache.evictAll();
                } catch (IOException ignored) {
                }
            }

            private static String hostOf(String url) {
                try {
                    okhttp3.HttpUrl parsed = okhttp3.HttpUrl.parse(url);
                    return parsed == null ? "" : parsed.host();
                } catch (RuntimeException e) {
                    return "";
                }
            }

            /** A request body that reports each piece it writes. */
            private abstract static class ProgressBody extends RequestBody {
                final Exchange exchange;
                final long length;
                final boolean oneShot;
                private long sent;

                ProgressBody(Exchange exchange, long length, boolean oneShot) {
                    this.exchange = exchange;
                    this.length = length;
                    this.oneShot = oneShot;
                }

                @Override public MediaType contentType() { return null; }
                @Override public long contentLength() { return length; }
                @Override public boolean isOneShot() { return oneShot; }

                @Override public void writeTo(BufferedSink sink) throws IOException {
                    sent = 0;
                    write(sink);
                }

                abstract void write(BufferedSink sink) throws IOException;

                void progress(int n) {
                    sent += n;
                    exchange.send(new Frame(SENT, exchange.emit).i64(sent).i64(length));
                }
            }

            private static RequestBody requestBody(final Exchange exchange, int kind, final byte[] bytes,
                                                   String path, long length) {
                switch (kind) {
                    case 1:
                        return new ProgressBody(exchange, bytes.length, false) {
                            @Override void write(BufferedSink sink) throws IOException {
                                for (int offset = 0; offset < bytes.length; ) {
                                    int n = Math.min(READ, bytes.length - offset);
                                    sink.write(bytes, offset, n);
                                    offset += n;
                                    progress(n);
                                }
                            }
                        };
                    case 2:
                        final File file = new File(path);
                        return new ProgressBody(exchange, file.length(), false) {
                            @Override void write(BufferedSink sink) throws IOException {
                                try (FileInputStream in = new FileInputStream(file)) {
                                    byte[] buffer = new byte[READ];
                                    int n;
                                    while ((n = in.read(buffer)) > 0) {
                                        sink.write(buffer, 0, n);
                                        progress(n);
                                    }
                                }
                            }
                        };
                    case 3:
                        return new ProgressBody(exchange, length, true) {
                            @Override void write(BufferedSink sink) throws IOException {
                                while (true) {
                                    byte[] chunk = exchange.pull();
                                    if (chunk.length == 0) return;
                                    sink.write(chunk);
                                    progress(chunk.length);
                                }
                            }
                        };
                    default:
                        return null;
                }
            }

            private static boolean requiresBody(String method) {
                return "POST".equals(method) || "PUT".equals(method) || "PATCH".equals(method);
            }

            public static void exchange_native(long client, String method, String url, String headers,
                                               int bodyKind, byte[] body, String bodyPath, long bodyLen,
                                               int idleMs, int cache, final long emit) {
                final Exchange exchange = new Exchange(emit, hostOf(url), idleMs);
                OkHttpClient okhttp = CLIENTS.get(client);
                if (okhttp == null) {
                    exchange.finish(failure(emit, -5, "the client was released"));
                    return;
                }
                EXCHANGES.put(emit, exchange);
                Request request;
                try {
                    Request.Builder builder = new Request.Builder().url(url).tag(Exchange.class, exchange);
                    String[] lines = headers.split("\n", -1);
                    for (int i = 0; i + 1 < lines.length; i += 2) {
                        builder.addHeader(lines[i], lines[i + 1]);
                    }
                    if (cache == 1) builder.cacheControl(CacheControl.FORCE_NETWORK);
                    if (cache == 2) builder.cacheControl(new CacheControl.Builder()
                            .maxStale(Integer.MAX_VALUE, TimeUnit.SECONDS).build());
                    RequestBody requestBody = requestBody(exchange, bodyKind, body, bodyPath, bodyLen);
                    if (requestBody == null && requiresBody(method)) {
                        requestBody = RequestBody.create(new byte[0], (MediaType) null);
                    }
                    request = builder.method(method, requestBody).build();
                } catch (IllegalArgumentException e) {
                    exchange.finish(failure(emit, -6, e.toString()));
                    return;
                }
                exchange.call = okhttp.newCall(request);
                if (exchange.isCancelled()) exchange.call.cancel();
                exchange.call.enqueue(new Callback() {
                    @Override public void onFailure(Call call, IOException e) {
                        exchange.finish(failure(emit, DayHttp.sentinel(call, e), String.valueOf(e)));
                    }

                    @Override public void onResponse(Call call, Response response) {
                        READERS.execute(() -> pump(exchange, call, response));
                    }
                });
            }

            /** Deliver the head, then read the body as demand allows. */
            private static void pump(Exchange exchange, Call call, Response response) {
                try (Response r = response) {
                    ResponseBody body = r.body();
                    long length = body == null ? -1 : body.contentLength();
                    exchange.send(new Frame(HEAD, exchange.emit)
                            .i32(r.code())
                            .str(r.request().url().toString())
                            .str(DayHttp.headerBlock(r.headers()))
                            .i64(length));
                    if (body != null) {
                        BufferedSource source = body.source();
                        byte[] buffer = new byte[READ];
                        while (true) {
                            exchange.takeDemand();
                            int n = source.read(buffer);
                            if (n == -1) break;
                            exchange.timing.received += n;
                            exchange.send(new Frame(CHUNK, exchange.emit).raw(buffer, n));
                        }
                    }
                    exchange.timing.fromCache |= r.cacheResponse() != null && r.networkResponse() == null;
                    exchange.send(exchange.timing.frame());
                    exchange.finish(new Frame(END, exchange.emit));
                } catch (Exception e) {
                    exchange.finish(failure(exchange.emit, DayHttp.sentinel(call, e), String.valueOf(e)));
                }
            }

            public static void demand_native(long token, int chunks) {
                Exchange exchange = EXCHANGES.get(token);
                if (exchange != null) exchange.addDemand(chunks);
            }

            public static void answer_native(long token, int question, int answer) {
                Exchange exchange = EXCHANGES.get(token);
                if (exchange == null) return;
                ArrayBlockingQueue<Integer> slot = exchange.questions.get(question);
                if (slot != null) slot.offer(answer);
            }

            public static void body_native(long token, byte[] chunk) {
                Exchange exchange = EXCHANGES.get(token);
                if (exchange != null) exchange.body.offer(chunk);
            }

            public static void exchange_cancel_native(long token) {
                Exchange exchange = EXCHANGES.get(token);
                if (exchange != null) exchange.cancel();
            }

            /** One WebSocket's stream. */
            private static final class Ws {
                final long emit;
                volatile WebSocket socket;
                private boolean finished;

                Ws(long emit) { this.emit = emit; }

                synchronized boolean isFinished() { return finished; }

                void send(Frame frame) {
                    if (!isFinished()) ws_open_native_emit(emit, frame.done());
                }

                void finish(Frame frame) {
                    synchronized (this) {
                        if (finished) return;
                        finished = true;
                    }
                    SOCKETS.remove(emit);
                    ws_open_native_emit(emit, frame.done());
                    ws_open_native_end(emit);
                }
            }

            public static void ws_open_native(long client, String url, String headers, String protocols, final long emit) {
                final Ws ws = new Ws(emit);
                OkHttpClient okhttp = CLIENTS.get(client);
                if (okhttp == null) {
                    ws.finish(new Frame(WS_FAILED, emit).i32(-5).str("the client was released"));
                    return;
                }
                SOCKETS.put(emit, ws);
                Request request;
                try {
                    Request.Builder builder = new Request.Builder().url(url);
                    String[] lines = headers.split("\n", -1);
                    for (int i = 0; i + 1 < lines.length; i += 2) {
                        builder.addHeader(lines[i], lines[i + 1]);
                    }
                    if (!protocols.isEmpty()) builder.header("Sec-WebSocket-Protocol", protocols);
                    request = builder.build();
                } catch (IllegalArgumentException e) {
                    ws.finish(new Frame(WS_FAILED, emit).i32(-6).str(e.toString()));
                    return;
                }
                ws.socket = okhttp.newWebSocket(request, new WebSocketListener() {
                    @Override public void onOpen(WebSocket socket, Response response) {
                        String protocol = response.header("Sec-WebSocket-Protocol");
                        ws.send(new Frame(WS_OPEN, emit).str(protocol == null ? "" : protocol));
                    }

                    @Override public void onMessage(WebSocket socket, String text) {
                        ws.send(new Frame(WS_TEXT, emit).str(text));
                    }

                    @Override public void onMessage(WebSocket socket, ByteString bytes) {
                        byte[] data = bytes.toByteArray();
                        ws.send(new Frame(WS_BINARY, emit).raw(data, data.length));
                    }

                    @Override public void onClosing(WebSocket socket, int code, String reason) {
                        socket.close(code, reason);
                        ws.finish(new Frame(WS_CLOSED, emit).i32(code).str(reason));
                    }

                    @Override public void onClosed(WebSocket socket, int code, String reason) {
                        ws.finish(new Frame(WS_CLOSED, emit).i32(code).str(reason));
                    }

                    @Override public void onFailure(WebSocket socket, Throwable t, Response response) {
                        int sentinel = t instanceof Exception ? DayHttp.sentinel(null, (Exception) t) : -5;
                        ws.finish(new Frame(WS_FAILED, emit).i32(sentinel).str(String.valueOf(t)));
                    }
                });
            }

            public static boolean ws_send_native(long token, boolean binary, byte[] data) {
                Ws ws = SOCKETS.get(token);
                if (ws == null || ws.socket == null) return false;
                return binary ? ws.socket.send(ByteString.of(data))
                        : ws.socket.send(new String(data, StandardCharsets.UTF_8));
            }

            public static void ws_close_native(long token, int code, String reason) {
                Ws ws = SOCKETS.get(token);
                if (ws == null || ws.socket == null) return;
                try {
                    ws.socket.close(code, reason);
                } catch (IllegalArgumentException e) {
                    ws.socket.cancel();
                }
            }
        "#,
    );

    // HarmonyOS: `@ohos.net.http` (the OpenHarmony SDK's own stack: system proxy, platform TLS)
    // and `@ohos.net.webSocket`. `requestInStream` reports the headers, each body buffer and the
    // end as events, and settles with the status; the arm holds body buffers until it has the
    // status for the head. The kit has no way to hold a body back, so demand is not consulted.
    #[day_bridge::impl(arkts, platforms = [ohos])]
    arkts!(
        prelude = r#"
            import { http, webSocket } from '@kit.NetworkKit';
            import { BusinessError } from '@kit.BasicServicesKit';
            import { util } from '@kit.ArkTS';
        "#,
        body = r#"
            const DAY_HEAD: number = 1;
            const DAY_CHUNK: number = 2;
            const DAY_SENT: number = 3;
            const DAY_END: number = 6;
            const DAY_FAILED: number = 7;
            const DAY_WS_OPEN: number = 16;
            const DAY_WS_TEXT: number = 17;
            const DAY_WS_BINARY: number = 18;
            const DAY_WS_CLOSED: number = 19;
            const DAY_WS_FAILED: number = 20;
            const DAY_HIGH: number = 4294967296;

            class DayFrame {
              private parts: Array<Uint8Array> = new Array<Uint8Array>();
              private size: number = 0;

              constructor(tag: number, token: number) {
                const b = new Uint8Array(9);
                const v = new DataView(b.buffer);
                v.setUint8(0, tag);
                v.setUint32(1, Math.floor(token / DAY_HIGH));
                v.setUint32(5, token % DAY_HIGH);
                this.push(b);
              }

              push(b: Uint8Array): DayFrame {
                this.parts.push(b);
                this.size += b.length;
                return this;
              }

              i32(n: number): DayFrame {
                const b = new Uint8Array(4);
                new DataView(b.buffer).setInt32(0, n);
                return this.push(b);
              }

              i64(n: number): DayFrame {
                const b = new Uint8Array(8);
                const v = new DataView(b.buffer);
                if (n < 0) {
                  v.setInt32(0, -1);
                  v.setInt32(4, n);
                } else {
                  v.setUint32(0, Math.floor(n / DAY_HIGH));
                  v.setUint32(4, n % DAY_HIGH);
                }
                return this.push(b);
              }

              str(s: string): DayFrame {
                const b = new util.TextEncoder().encodeInto(s);
                return this.i32(b.length).push(b);
              }

              done(): Uint8Array {
                const out = new Uint8Array(this.size);
                let at = 0;
                for (const p of this.parts) {
                  out.set(p, at);
                  at += p.length;
                }
                return out;
              }
            }

            interface DayClient {
              cache: boolean;
            }

            interface DayExchange {
              request: http.HttpRequest;
              url: string;
              finished: boolean;
              cancelled: boolean;
              headSent: boolean;
              ended: boolean;
              status: number;
              headers: string;
              pending: Array<Uint8Array>;
            }

            interface DaySocket {
              socket: webSocket.WebSocket;
              finished: boolean;
            }

            const dayClients: Map<number, DayClient> = new Map<number, DayClient>();
            const dayExchanges: Map<number, DayExchange> = new Map<number, DayExchange>();
            const daySockets: Map<number, DaySocket> = new Map<number, DaySocket>();
            let dayNextClient: number = 1;

            // Network Kit error codes (2300xxx) onto the crate's transport sentinels.
            function daySentinel(code: number): number {
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

            function dayMethod(method: string): http.RequestMethod {
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

            function dayHeaderBlock(header: Object): string {
              let out = '';
              const record = header as Record<string, string>;
              for (const key of Object.keys(record)) {
                out += key + '\n' + String(record[key]) + '\n';
              }
              return out;
            }

            function dayHeaderRecord(headers: string): Record<string, string> {
              const record: Record<string, string> = {};
              const lines = headers.split('\n');
              for (let i = 0; i + 1 < lines.length; i += 2) {
                record[lines[i]] = lines[i + 1];
              }
              return record;
            }

            function dayFinish(ex: DayExchange, emit: number, frame: DayFrame): void {
              if (ex.finished) {
                return;
              }
              ex.finished = true;
              dayExchanges.delete(emit);
              exchange_native_emit(emit, frame.done());
              exchange_native_end(emit);
              ex.request.destroy();
            }

            function dayHead(ex: DayExchange, emit: number): void {
              if (ex.headSent || ex.finished) {
                return;
              }
              ex.headSent = true;
              exchange_native_emit(emit, new DayFrame(DAY_HEAD, emit)
                .i32(ex.status).str(ex.url).str(ex.headers).i64(-1).done());
              for (const chunk of ex.pending) {
                exchange_native_emit(emit, new DayFrame(DAY_CHUNK, emit).push(chunk).done());
              }
              ex.pending = new Array<Uint8Array>();
              if (ex.ended) {
                dayFinish(ex, emit, new DayFrame(DAY_END, emit));
              }
            }

            export function client_native(idleMs: number, totalMs: number, cacheBytes: number,
                                          maxPerHost: number, askTrust: boolean, identity: Uint8Array,
                                          password: string): number {
              const id = dayNextClient;
              dayNextClient += 1;
              const client: DayClient = { cache: cacheBytes > 0 };
              dayClients.set(id, client);
              return id;
            }

            export function client_release_native(client: number): void {
              dayClients.delete(client);
            }

            export function cache_clear_native(client: number): void {
              http.createHttpResponseCache().delete().catch((err: BusinessError) => {});
            }

            export function exchange_native(client: number, method: string, url: string, headers: string,
                                            bodyKind: number, body: Uint8Array, bodyPath: string,
                                            bodyLen: number, idleMs: number, cache: number,
                                            emit: number): void {
              const request = http.createHttp();
              const ex: DayExchange = {
                request: request, url: url, finished: false, cancelled: false, headSent: false,
                ended: false, status: 0, headers: '', pending: new Array<Uint8Array>(),
              };
              dayExchanges.set(emit, ex);
              const settings = dayClients.get(client);
              const options: http.HttpRequestOptions = {
                method: dayMethod(method),
                header: dayHeaderRecord(headers),
                usingCache: settings !== undefined && settings.cache && cache !== 1,
                usingProxy: true,
              };
              if (idleMs > 0) {
                options.connectTimeout = idleMs;
                options.readTimeout = idleMs;
              }
              if (bodyKind === 1 && body.length > 0) {
                options.extraData = body.buffer.slice(body.byteOffset, body.byteOffset + body.byteLength);
              }
              request.on('headersReceive', (header: Object) => {
                ex.headers = dayHeaderBlock(header);
              });
              request.on('dataReceive', (data: ArrayBuffer) => {
                if (ex.finished) {
                  return;
                }
                const chunk = new Uint8Array(data);
                if (ex.headSent) {
                  exchange_native_emit(emit, new DayFrame(DAY_CHUNK, emit).push(chunk).done());
                } else {
                  ex.pending.push(chunk);
                }
              });
              request.on('dataSendProgress', (info: http.DataSendProgressInfo) => {
                if (!ex.finished) {
                  exchange_native_emit(emit, new DayFrame(DAY_SENT, emit)
                    .i64(info.sendSize).i64(info.totalSize).done());
                }
              });
              request.on('dataEnd', () => {
                ex.ended = true;
                if (ex.headSent) {
                  dayFinish(ex, emit, new DayFrame(DAY_END, emit));
                }
              });
              request.requestInStream(url, options).then((code: number) => {
                ex.status = code;
                dayHead(ex, emit);
              }).catch((err: BusinessError) => {
                const sentinel = ex.cancelled ? -7 : daySentinel(err.code);
                dayFinish(ex, emit, new DayFrame(DAY_FAILED, emit).i32(sentinel).str(`${err.code}: ${err.message}`));
              });
            }

            export function demand_native(token: number, chunks: number): void {}

            export function answer_native(token: number, question: number, answer: number): void {}

            export function body_native(token: number, chunk: Uint8Array): void {}

            export function exchange_cancel_native(token: number): void {
              const ex = dayExchanges.get(token);
              if (ex !== undefined) {
                ex.cancelled = true;
                dayFinish(ex, token, new DayFrame(DAY_FAILED, token).i32(-7).str('cancelled'));
              }
            }

            function dayWsFinish(s: DaySocket, emit: number, frame: DayFrame): void {
              if (s.finished) {
                return;
              }
              s.finished = true;
              daySockets.delete(emit);
              ws_open_native_emit(emit, frame.done());
              ws_open_native_end(emit);
            }

            export function ws_open_native(client: number, url: string, headers: string, protocols: string,
                                           emit: number): void {
              const socket = webSocket.createWebSocket();
              const s: DaySocket = { socket: socket, finished: false };
              daySockets.set(emit, s);
              socket.on('open', (err: BusinessError, value: Object) => {
                if (!s.finished) {
                  ws_open_native_emit(emit, new DayFrame(DAY_WS_OPEN, emit).str('').done());
                }
              });
              socket.on('message', (err: BusinessError, value: string | ArrayBuffer) => {
                if (s.finished) {
                  return;
                }
                if (typeof value === 'string') {
                  ws_open_native_emit(emit, new DayFrame(DAY_WS_TEXT, emit).str(value as string).done());
                } else {
                  ws_open_native_emit(emit, new DayFrame(DAY_WS_BINARY, emit)
                    .push(new Uint8Array(value as ArrayBuffer)).done());
                }
              });
              socket.on('close', (err: BusinessError, value: webSocket.CloseResult) => {
                dayWsFinish(s, emit, new DayFrame(DAY_WS_CLOSED, emit).i32(value.code).str(value.reason));
              });
              socket.on('error', (err: BusinessError) => {
                dayWsFinish(s, emit, new DayFrame(DAY_WS_FAILED, emit).i32(-5).str(`${err.code}: ${err.message}`));
              });
              const options: webSocket.WebSocketRequestOptions = { header: dayHeaderRecord(headers) };
              if (protocols.length > 0) {
                options.protocol = protocols;
              }
              socket.connect(url, options).catch((err: BusinessError) => {
                dayWsFinish(s, emit, new DayFrame(DAY_WS_FAILED, emit).i32(daySentinel(err.code)).str(`${err.code}: ${err.message}`));
              });
            }

            export function ws_send_native(token: number, binary: boolean, data: Uint8Array): boolean {
              const s = daySockets.get(token);
              if (s === undefined || s.finished) {
                return false;
              }
              if (binary) {
                s.socket.send(data.buffer.slice(data.byteOffset, data.byteOffset + data.byteLength));
              } else {
                s.socket.send(util.TextDecoder.create('utf-8').decodeToString(data));
              }
              return true;
            }

            export function ws_close_native(token: number, code: number, reason: string): void {
              const s = daySockets.get(token);
              if (s !== undefined) {
                const options: webSocket.WebSocketCloseOptions = { code: code, reason: reason };
                s.socket.close(options);
              }
            }
        "#,
    );

    // The web: the browser's own fetch and WebSocket. A response body is read from its stream
    // only as demand allows; a timer over the connection and the head realizes the idle bound,
    // and the body phase runs uncapped, like the other stacks' per-read bounds.
    #[day_bridge::impl(js, platforms = [web])]
    js!(r#"
        const DAY_HEAD = 1, DAY_CHUNK = 2, DAY_END = 6, DAY_FAILED = 7;
        const DAY_WS_OPEN = 16, DAY_WS_TEXT = 17, DAY_WS_BINARY = 18, DAY_WS_CLOSED = 19;
        const dayEncoder = new TextEncoder();
        const dayDecoder = new TextDecoder();
        const dayClients = new Map();
        const dayExchanges = new Map();
        const daySockets = new Map();
        let dayNextClient = 1;

        class DayFrame {
            constructor(tag, token) {
                this.parts = [];
                this.size = 0;
                const b = new Uint8Array(9);
                b[0] = tag;
                new DataView(b.buffer).setBigInt64(1, BigInt(token));
                this.push(b);
            }
            push(b) { this.parts.push(b); this.size += b.length; return this; }
            i32(n) { const b = new Uint8Array(4); new DataView(b.buffer).setInt32(0, n); return this.push(b); }
            i64(n) { const b = new Uint8Array(8); new DataView(b.buffer).setBigInt64(0, BigInt(Math.trunc(n))); return this.push(b); }
            str(s) { const b = dayEncoder.encode(s ?? ''); return this.i32(b.length).push(b); }
            done() {
                const out = new Uint8Array(this.size);
                let at = 0;
                for (const p of this.parts) { out.set(p, at); at += p.length; }
                return out;
            }
        }

        export function client_native(idleMs, totalMs, cacheBytes, maxPerHost, askTrust, identity, password) {
            const id = dayNextClient++;
            dayClients.set(id, { idleMs });
            return BigInt(id);
        }

        export function client_release_native(client) {
            dayClients.delete(Number(client));
        }

        // The browser owns its cache; a page cannot empty it.
        export function cache_clear_native(client) {}

        export function exchange_native(client, method, url, headers, bodyKind, body, bodyPath, bodyLen, idleMs, cache, emit) {
            const ex = { demand: 0, waiting: null, controller: new AbortController(), finished: false };
            dayExchanges.set(emit, ex);
            const send = (frame) => { if (!ex.finished) exchange_native_emit(emit, frame.done()); };
            const finish = (frame) => {
                if (ex.finished) return;
                send(frame);
                ex.finished = true;
                dayExchanges.delete(emit);
                exchange_native_end(emit);
            };
            let target;
            try {
                target = new URL(url, document.baseURI).toString();
            } catch (e) {
                finish(new DayFrame(DAY_FAILED, emit).i32(-6).str(String(e)));
                return;
            }
            const requestHeaders = new Headers();
            const lines = headers.split('\n');
            for (let i = 0; i + 1 < lines.length; i += 2) {
                try { requestHeaders.append(lines[i], lines[i + 1]); } catch {}
            }
            const init = {
                method,
                headers: requestHeaders,
                signal: ex.controller.signal,
                cache: cache === 1 ? 'reload' : cache === 2 ? 'force-cache' : 'default',
            };
            if (bodyKind === 1) init.body = body.slice();
            let timedOut = false;
            const timer = idleMs > 0 ? setTimeout(() => { timedOut = true; ex.controller.abort(); }, idleMs) : 0;
            (async () => {
                try {
                    const response = await fetch(target, init);
                    clearTimeout(timer);
                    let block = '';
                    for (const [k, v] of response.headers) block += k + '\n' + v + '\n';
                    const length = response.headers.get('content-length');
                    const encoded = response.headers.get('content-encoding');
                    send(new DayFrame(DAY_HEAD, emit)
                        .i32(response.status)
                        .str(response.url || target)
                        .str(block)
                        .i64(length && !encoded ? Number(length) : -1));
                    if (response.body) {
                        const reader = response.body.getReader();
                        for (;;) {
                            await dayDemand(ex);
                            const { done, value } = await reader.read();
                            if (done) break;
                            send(new DayFrame(DAY_CHUNK, emit).push(value));
                        }
                    }
                    finish(new DayFrame(DAY_END, emit));
                } catch (e) {
                    clearTimeout(timer);
                    const aborted = e && e.name === 'AbortError';
                    const sentinel = aborted ? (timedOut ? -1 : -7) : -5;
                    finish(new DayFrame(DAY_FAILED, emit).i32(sentinel).str(String((e && e.message) || e)));
                }
            })();
        }

        function dayDemand(ex) {
            if (ex.demand > 0) { ex.demand--; return Promise.resolve(); }
            return new Promise((resolve) => { ex.waiting = () => { ex.demand--; resolve(); }; });
        }

        export function demand_native(token, chunks) {
            const ex = dayExchanges.get(token);
            if (!ex) return;
            ex.demand += chunks;
            if (ex.waiting && ex.demand > 0) {
                const wake = ex.waiting;
                ex.waiting = null;
                wake();
            }
        }

        export function answer_native(token, question, answer) {}

        export function body_native(token, chunk) {}

        export function exchange_cancel_native(token) {
            const ex = dayExchanges.get(token);
            if (ex) {
                ex.controller.abort();
                if (ex.waiting) { const wake = ex.waiting; ex.waiting = null; wake(); }
            }
        }

        export function ws_open_native(client, url, headers, protocols, emit) {
            const state = { socket: null, finished: false };
            const send = (frame) => { if (!state.finished) ws_open_native_emit(emit, frame.done()); };
            const finish = (frame) => {
                if (state.finished) return;
                send(frame);
                state.finished = true;
                daySockets.delete(emit);
                ws_open_native_end(emit);
            };
            const offered = protocols ? protocols.split(',').map((p) => p.trim()).filter(Boolean) : [];
            try {
                const target = new URL(url, document.baseURI);
                if (target.protocol === 'http:') target.protocol = 'ws:';
                if (target.protocol === 'https:') target.protocol = 'wss:';
                state.socket = new WebSocket(target.toString(), offered);
            } catch (e) {
                finish(new DayFrame(20, emit).i32(-6).str(String(e)));
                return;
            }
            daySockets.set(emit, state);
            const socket = state.socket;
            socket.binaryType = 'arraybuffer';
            socket.onopen = () => send(new DayFrame(DAY_WS_OPEN, emit).str(socket.protocol));
            socket.onmessage = (event) => {
                if (typeof event.data === 'string') send(new DayFrame(DAY_WS_TEXT, emit).str(event.data));
                else send(new DayFrame(DAY_WS_BINARY, emit).push(new Uint8Array(event.data)));
            };
            // A failed handshake or a dropped connection reports through the close that follows.
            socket.onclose = (event) => finish(new DayFrame(DAY_WS_CLOSED, emit).i32(event.code).str(event.reason));
        }

        export function ws_send_native(token, binary, data) {
            const state = daySockets.get(token);
            if (!state || state.socket.readyState !== WebSocket.OPEN) return false;
            state.socket.send(binary ? data.slice() : dayDecoder.decode(data));
            return true;
        }

        export function ws_close_native(token, code, reason) {
            const state = daySockets.get(token);
            if (!state) return;
            try { state.socket.close(code, reason); } catch { state.socket.close(); }
        }
    "#);

    // Apple, Linux and Windows reach their stacks from Rust, so these answer only where an arm lives;
    // elsewhere they are never reached.
    #[day_bridge::impl(rust, platforms = [other])]
    fn client_native(
        _idle_ms: i32,
        _total_ms: i32,
        _cache_bytes: i64,
        _max_per_host: i32,
        _ask_trust: bool,
        _identity: &[u8],
        _password: &str,
    ) -> Result<i64, day_bridge::Error> {
        Err(day_bridge::Error::Unsupported)
    }

    #[day_bridge::impl(rust, platforms = [other])]
    fn client_release_native(_client: i64) {}

    #[day_bridge::impl(rust, platforms = [other])]
    fn cache_clear_native(_client: i64) {}

    #[day_bridge::impl(rust, platforms = [other])]
    fn exchange_native(
        _client: i64,
        _method: &str,
        _url: &str,
        _headers: &str,
        _body_kind: i32,
        _body: &[u8],
        _body_path: &str,
        _body_size: i64,
        _idle_ms: i32,
        _cache: i32,
        _emit: day_bridge::Emit<Vec<u8>>,
    ) -> Result<(), day_bridge::Error> {
        Err(day_bridge::Error::Unsupported)
    }

    #[day_bridge::impl(rust, platforms = [other])]
    fn demand_native(_token: i64, _chunks: i32) {}

    #[day_bridge::impl(rust, platforms = [other])]
    fn answer_native(_token: i64, _question: i32, _answer: i32) {}

    #[day_bridge::impl(rust, platforms = [other])]
    fn body_native(_token: i64, _chunk: &[u8]) {}

    #[day_bridge::impl(rust, platforms = [other])]
    fn exchange_cancel_native(_token: i64) {}

    #[day_bridge::impl(rust, platforms = [other])]
    fn ws_open_native(
        _client: i64,
        _url: &str,
        _headers: &str,
        _protocols: &str,
        _emit: day_bridge::Emit<Vec<u8>>,
    ) -> Result<(), day_bridge::Error> {
        Err(day_bridge::Error::Unsupported)
    }

    #[day_bridge::impl(rust, platforms = [other])]
    fn ws_send_native(_token: i64, _binary: bool, _data: &[u8]) -> Result<bool, day_bridge::Error> {
        Err(day_bridge::Error::Unsupported)
    }

    #[day_bridge::impl(rust, platforms = [other])]
    fn ws_close_native(_token: i64, _code: i32, _reason: &str) {}
}

// The arms' plain calls, for the transport in `src/bridged.rs`: the generated functions are private
// to this module.
#[cfg(any(
    target_os = "android",
    all(target_os = "linux", target_env = "ohos"),
    target_arch = "wasm32"
))]
pub(crate) fn client(
    idle_ms: i32,
    total_ms: i32,
    cache_bytes: i64,
    max_per_host: i32,
    ask_trust: bool,
    identity: &[u8],
    password: &str,
) -> Result<i64, day_bridge::Error> {
    client_native(
        idle_ms,
        total_ms,
        cache_bytes,
        max_per_host,
        ask_trust,
        identity,
        password,
    )
}

#[cfg(any(
    target_os = "android",
    all(target_os = "linux", target_env = "ohos"),
    target_arch = "wasm32"
))]
pub(crate) fn client_release(client: i64) {
    client_release_native(client)
}

#[cfg(any(
    target_os = "android",
    all(target_os = "linux", target_env = "ohos"),
    target_arch = "wasm32"
))]
pub(crate) fn cache_clear(client: i64) {
    cache_clear_native(client)
}

#[cfg(any(
    target_os = "android",
    all(target_os = "linux", target_env = "ohos"),
    target_arch = "wasm32"
))]
pub(crate) fn demand(token: i64, chunks: i32) {
    demand_native(token, chunks)
}

#[cfg(any(
    target_os = "android",
    all(target_os = "linux", target_env = "ohos"),
    target_arch = "wasm32"
))]
pub(crate) fn answer(token: i64, question: i32, answer: i32) {
    answer_native(token, question, answer)
}

#[cfg(any(
    target_os = "android",
    all(target_os = "linux", target_env = "ohos"),
    target_arch = "wasm32"
))]
pub(crate) fn body(token: i64, chunk: &[u8]) {
    body_native(token, chunk)
}

#[cfg(any(
    target_os = "android",
    all(target_os = "linux", target_env = "ohos"),
    target_arch = "wasm32"
))]
pub(crate) fn exchange_cancel(token: i64) {
    exchange_cancel_native(token)
}

#[cfg(any(
    target_os = "android",
    all(target_os = "linux", target_env = "ohos"),
    target_arch = "wasm32"
))]
pub(crate) fn ws_send(token: i64, binary: bool, data: &[u8]) -> Result<bool, day_bridge::Error> {
    ws_send_native(token, binary, data)
}

#[cfg(any(
    target_os = "android",
    all(target_os = "linux", target_env = "ohos"),
    target_arch = "wasm32"
))]
pub(crate) fn ws_close(token: i64, code: i32, reason: &str) {
    ws_close_native(token, code, reason)
}

/// The header block the frames carry: `k\nv\n…`, the layout every arm reads and writes.
#[cfg(any(
    target_os = "android",
    all(target_os = "linux", target_env = "ohos"),
    target_arch = "wasm32"
))]
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
#[cfg(any(
    target_os = "android",
    all(target_os = "linux", target_env = "ohos"),
    target_arch = "wasm32"
))]
pub(crate) fn bridge_error(e: day_bridge::Error) -> super::HttpError {
    match e {
        day_bridge::Error::Unsupported => super::HttpError::Unsupported,
        other => super::HttpError::Io(other.to_string()),
    }
}
