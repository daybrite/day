// day-part-http's Android shim — java.net.HttpURLConnection, the platform HTTP stack: system
// ProxySelector (per-network proxy/PAC), VPN routing, network security config + the user CA
// store. BLOCKING by design: the Rust side calls this on the caller's (non-UI) thread via the
// attached JVM. Results cross JNI as ONE byte[] envelope (a single array copy each way):
//   [0..4)  status as i32 BE; NEGATIVE = transport error sentinel:
//           -1 timeout, -2 dns, -3 tls, -4 connect, -5 io, -6 bad url
//   [4..8)  header-block length as i32 BE
//   then    header block "k\nv\n..." UTF-8 (or the error message for sentinels)
//   then    body bytes (fetch) / an 8-byte BE bytes-written count (fetchToFile)
package dev.daybrite.day.http;

import java.io.ByteArrayOutputStream;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.ConnectException;
import java.net.HttpURLConnection;
import java.net.MalformedURLException;
import java.net.SocketTimeoutException;
import java.net.URL;
import java.net.UnknownHostException;
import java.nio.ByteBuffer;
import java.nio.charset.StandardCharsets;
import java.util.List;
import java.util.Map;

public final class DayHttp {
    public static byte[] fetch(String method, String url, String[] kv, byte[] body, int timeoutMs) {
        return run(method, url, kv, body, timeoutMs, null);
    }

    public static byte[] fetchToFile(String method, String url, String[] kv, byte[] body,
                                     int timeoutMs, String dest) {
        return run(method, url, kv, body, timeoutMs, dest);
    }

    private static byte[] run(String method, String url, String[] kv, byte[] body,
                              int timeoutMs, String dest) {
        HttpURLConnection conn = null;
        try {
            conn = (HttpURLConnection) new URL(url).openConnection();
            conn.setRequestMethod(method); // PATCH throws ProtocolException (classic Java gap) → io sentinel
            conn.setConnectTimeout(timeoutMs);
            conn.setReadTimeout(timeoutMs);
            conn.setInstanceFollowRedirects(true);
            for (int i = 0; i + 1 < kv.length; i += 2) {
                conn.addRequestProperty(kv[i], kv[i + 1]);
            }
            if (body != null && body.length > 0) {
                conn.setDoOutput(true);
                conn.setFixedLengthStreamingMode(body.length);
                OutputStream out = conn.getOutputStream();
                out.write(body);
                out.close();
            }

            int status = conn.getResponseCode();
            StringBuilder headers = new StringBuilder();
            Map<String, List<String>> fields = conn.getHeaderFields();
            for (Map.Entry<String, List<String>> e : fields.entrySet()) {
                if (e.getKey() == null) continue; // the status line rides the null key
                for (String v : e.getValue()) {
                    headers.append(e.getKey()).append('\n').append(v).append('\n');
                }
            }

            InputStream in;
            try {
                in = conn.getInputStream();
            } catch (IOException statusErr) {
                in = conn.getErrorStream(); // 4xx/5xx bodies arrive here — still a RESPONSE
            }

            byte[] payload;
            if (dest == null) {
                payload = readAll(in);
            } else {
                long written = copyToFile(in, dest);
                payload = ByteBuffer.allocate(8).putLong(written).array();
            }
            return envelope(status, headers.toString(), payload);
        } catch (MalformedURLException e) {
            return error(-6, e);
        } catch (SocketTimeoutException e) {
            return error(-1, e);
        } catch (UnknownHostException e) {
            return error(-2, e);
        } catch (javax.net.ssl.SSLException e) {
            return error(-3, e);
        } catch (ConnectException e) {
            return error(-4, e);
        } catch (Exception e) {
            return error(-5, e);
        } finally {
            if (conn != null) conn.disconnect();
        }
    }

    private static byte[] readAll(InputStream in) throws IOException {
        ByteArrayOutputStream buf = new ByteArrayOutputStream();
        if (in != null) {
            byte[] chunk = new byte[65536];
            int n;
            while ((n = in.read(chunk)) > 0) buf.write(chunk, 0, n);
            in.close();
        }
        return buf.toByteArray();
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
        byte[] hdr = headers.getBytes(StandardCharsets.UTF_8);
        ByteBuffer buf = ByteBuffer.allocate(8 + hdr.length + payload.length);
        buf.putInt(status).putInt(hdr.length).put(hdr).put(payload);
        return buf.array();
    }

    // --- Streaming (fetch_streamed): open → envelope(status/headers/4-byte handle), then the
    // Rust side PULLS 64 KiB chunks with streamRead until an empty array (EOF), and streamClose
    // releases the connection (also called on abort/cancel).
    private static final java.util.Map<Integer, Object[]> STREAMS = new java.util.HashMap<>();
    private static int nextStream = 1;

    public static byte[] streamOpen(String method, String url, String[] kv, byte[] body, int timeoutMs) {
        HttpURLConnection conn = null;
        try {
            conn = (HttpURLConnection) new URL(url).openConnection();
            conn.setRequestMethod(method);
            conn.setConnectTimeout(timeoutMs);
            conn.setReadTimeout(timeoutMs);
            conn.setInstanceFollowRedirects(true);
            for (int i = 0; i + 1 < kv.length; i += 2) {
                conn.addRequestProperty(kv[i], kv[i + 1]);
            }
            if (body != null && body.length > 0) {
                conn.setDoOutput(true);
                conn.setFixedLengthStreamingMode(body.length);
                OutputStream out = conn.getOutputStream();
                out.write(body);
                out.close();
            }
            int status = conn.getResponseCode();
            StringBuilder headers = new StringBuilder();
            for (Map.Entry<String, List<String>> e : conn.getHeaderFields().entrySet()) {
                if (e.getKey() == null) continue;
                for (String v : e.getValue()) {
                    headers.append(e.getKey()).append('\n').append(v).append('\n');
                }
            }
            InputStream in;
            try {
                in = conn.getInputStream();
            } catch (IOException statusErr) {
                in = conn.getErrorStream();
            }
            int handle;
            synchronized (STREAMS) {
                handle = nextStream++;
                STREAMS.put(handle, new Object[] { conn, in });
            }
            return envelope(status, headers.toString(), ByteBuffer.allocate(4).putInt(handle).array());
        } catch (MalformedURLException e) {
            if (conn != null) conn.disconnect();
            return error(-6, e);
        } catch (SocketTimeoutException e) {
            if (conn != null) conn.disconnect();
            return error(-1, e);
        } catch (UnknownHostException e) {
            if (conn != null) conn.disconnect();
            return error(-2, e);
        } catch (javax.net.ssl.SSLException e) {
            if (conn != null) conn.disconnect();
            return error(-3, e);
        } catch (ConnectException e) {
            if (conn != null) conn.disconnect();
            return error(-4, e);
        } catch (Exception e) {
            if (conn != null) conn.disconnect();
            return error(-5, e);
        }
    }

    /// One pulled chunk: empty array = EOF; null = read error (stream auto-closed).
    public static byte[] streamRead(int handle) {
        Object[] entry;
        synchronized (STREAMS) { entry = STREAMS.get(handle); }
        if (entry == null) return null;
        InputStream in = (InputStream) entry[1];
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
            if (entry[1] != null) ((InputStream) entry[1]).close();
        } catch (IOException ignored) {}
        ((HttpURLConnection) entry[0]).disconnect();
    }

    private static byte[] error(int sentinel, Exception e) {
        String msg = e.toString();
        byte[] m = msg.getBytes(StandardCharsets.UTF_8);
        ByteBuffer buf = ByteBuffer.allocate(8 + m.length);
        buf.putInt(sentinel).putInt(m.length).put(m);
        return buf.array();
    }
}
