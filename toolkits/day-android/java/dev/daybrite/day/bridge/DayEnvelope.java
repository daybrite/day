// The part↔Java payload convention (docs/extending.md, "The Android bridging contract"): ONE
// byte[] crosses JNI per call, laid out as
//   [0..4)  status i32 BE (NEGATIVE = the part's transport-error sentinel)
//   [4..8)  meta-block length i32 BE
//   then    meta "k\nv\n..." UTF-8 (for sentinels: the error message instead)
//   then    payload bytes
// day_android::envelope is the Rust twin; the two encode identically and Rust-side unit tests
// pin the format. Parts build responses with these helpers instead of hand-packing buffers.
package dev.daybrite.day.bridge;

import java.nio.ByteBuffer;
import java.nio.charset.StandardCharsets;

public final class DayEnvelope {
    private DayEnvelope() {}

    /** A success envelope: status, "k\nv\n..." meta block, payload bytes. */
    public static byte[] pack(int status, String meta, byte[] payload) {
        byte[] m = meta.getBytes(StandardCharsets.UTF_8);
        ByteBuffer buf = ByteBuffer.allocate(8 + m.length + payload.length);
        buf.putInt(status).putInt(m.length).put(m).put(payload);
        return buf.array();
    }

    /** An error envelope: negative sentinel status, the message riding the meta block. */
    public static byte[] error(int sentinel, String message) {
        byte[] m = message.getBytes(StandardCharsets.UTF_8);
        ByteBuffer buf = ByteBuffer.allocate(8 + m.length);
        buf.putInt(sentinel).putInt(m.length).put(m);
        return buf.array();
    }
}
