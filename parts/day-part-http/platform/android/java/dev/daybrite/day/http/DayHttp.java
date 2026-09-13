// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// day-part-http's Android helpers, shared by the crate's Java arm (src/bridge.rs): one OkHttp
// engine for the process, whose dispatcher and connection pool (keep-alive, HTTP/2 multiplexing)
// every client shares; the header block the frames carry; the transport sentinels; and the
// platform's trust manager, which a client that asks trust questions wraps. OkHttp rides the
// platform's policy rails: the system ProxySelector (per-network proxy and PAC), VPN routing, the
// network security config and the user CA store all still apply.
package dev.daybrite.day.http;

import java.io.IOException;
import java.io.InterruptedIOException;
import java.net.ConnectException;
import java.net.SocketTimeoutException;
import java.net.UnknownHostException;
import java.security.KeyStore;
import java.util.concurrent.TimeUnit;

import javax.net.ssl.TrustManager;
import javax.net.ssl.TrustManagerFactory;
import javax.net.ssl.X509TrustManager;

import okhttp3.Call;
import okhttp3.Headers;
import okhttp3.OkHttpClient;

public final class DayHttp {
    private DayHttp() {}

    private static OkHttpClient base;

    /**
     * The shared engine with idle bounds of {@code timeoutMs}: connect, read and write are
     * per-phase bounds, so a long transfer that keeps moving is never cut off. The variants
     * newBuilder() makes share the dispatcher and the pool, an OkHttp-documented cheap clone.
     */
    public static synchronized OkHttpClient client(int timeoutMs) {
        if (base == null) base = new OkHttpClient();
        return base.newBuilder()
                .connectTimeout(timeoutMs, TimeUnit.MILLISECONDS)
                .readTimeout(timeoutMs, TimeUnit.MILLISECONDS)
                .writeTimeout(timeoutMs, TimeUnit.MILLISECONDS)
                .build();
    }

    /**
     * The transport sentinel for a failure: -1 timeout, -2 dns, -3 tls, -4 connect, -5 anything
     * else, -7 cancelled. Cancellation comes first: a cancel mid-read surfaces as an ordinary
     * IOException, and isCanceled() is the truth.
     */
    public static int sentinel(Call call, Exception e) {
        if (call != null && call.isCanceled()) return -7;
        if (e instanceof SocketTimeoutException) return -1;
        if (e instanceof InterruptedIOException && "timeout".equals(e.getMessage())) return -1;
        if (e instanceof UnknownHostException) return -2;
        if (e instanceof javax.net.ssl.SSLException) return -3;
        if (e instanceof ConnectException) return -4;
        if (e instanceof IOException && "Canceled".equals(e.getMessage())) return -7;
        return -5;
    }

    /** The header block the frames carry: "k\nv\n…", arrival order, duplicates kept. */
    public static String headerBlock(Headers h) {
        StringBuilder sb = new StringBuilder();
        for (int i = 0; i < h.size(); i++) {
            sb.append(h.name(i)).append('\n').append(h.value(i)).append('\n');
        }
        return sb.toString();
    }

    /** The platform's default trust manager: the system and user CA stores, with the app's
     *  network security config applied. */
    public static X509TrustManager platformTrustManager() throws Exception {
        TrustManagerFactory factory = TrustManagerFactory.getInstance(TrustManagerFactory.getDefaultAlgorithm());
        factory.init((KeyStore) null);
        for (TrustManager manager : factory.getTrustManagers()) {
            if (manager instanceof X509TrustManager) return (X509TrustManager) manager;
        }
        throw new IllegalStateException("no X509TrustManager");
    }
}
