// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// day-break's OWN Android crash layer — a headless shim (no UI), bundled with the crate and folded
// into the app's Gradle build via [package.metadata.day.android], with ZERO edits to day-android.
// It installs a default uncaught-exception handler that writes a `java-<sid>.kv` artifact directly
// to day-break's report dir — the SAME line-oriented key=value format the Rust reconciler reads
// (crates/day-break/src/report.rs) — then chains to the previous handler so the system still shows
// its dialog and kills the process. Writing the file from Java (not through JNI) keeps the crash
// path off a second failure surface: the JVM is wounded but a file write is simple and local.
//
// NOTE the package is `.crash`, not `.break` — `break` is a Java keyword.
package dev.daybrite.day.crash;

import java.io.File;
import java.io.FileOutputStream;
import java.io.PrintWriter;
import java.io.StringWriter;
import java.nio.charset.StandardCharsets;

public final class DayBreak {
    private DayBreak() {}

    private static volatile String dir;
    private static volatile String sid;
    private static Thread.UncaughtExceptionHandler prev;

    /** Called once from Rust at init (day_break::java_android::install) on the main thread. */
    public static void install(String reportDir, String sessionId) {
        dir = reportDir;
        sid = sessionId;
        prev = Thread.getDefaultUncaughtExceptionHandler();
        Thread.setDefaultUncaughtExceptionHandler((thread, ex) -> {
            try {
                write(thread, ex);
            } catch (Throwable ignored) {
                // Never let the reporter's own failure mask the crash.
            }
            if (prev != null) {
                prev.uncaughtException(thread, ex); // system dialog + process death
            } else {
                android.os.Process.killProcess(android.os.Process.myPid());
            }
        });
    }

    private static void write(Thread thread, Throwable ex) throws Exception {
        String d = dir;
        String s = sid;
        if (d == null || s == null) {
            return;
        }
        StringWriter trace = new StringWriter();
        ex.printStackTrace(new PrintWriter(trace));

        String message = ex.getClass().getName();
        String detail = ex.getMessage();
        if (detail != null) {
            message = message + ": " + detail;
        }

        StringBuilder kv = new StringBuilder();
        kvLine(kv, "message", message);
        kvLine(kv, "thread", thread == null ? "" : thread.getName());
        kvLine(kv, "main", thread != null && "main".equals(thread.getName()) ? "1" : "0");
        kvLine(kv, "backtrace", trace.toString());

        File out = new File(d, "java-" + s + ".kv");
        try (FileOutputStream fos = new FileOutputStream(out)) {
            fos.write(kv.toString().getBytes(StandardCharsets.UTF_8));
            fos.getFD().sync();
        }
    }

    /** One `key=value` line, escaping the value the same way the Rust kv codec does. */
    private static void kvLine(StringBuilder sb, String key, String value) {
        sb.append(key).append('=');
        if (value != null) {
            for (int i = 0; i < value.length(); i++) {
                char c = value.charAt(i);
                switch (c) {
                    case '\\': sb.append("\\\\"); break;
                    case '\n': sb.append("\\n"); break;
                    case '\r': sb.append("\\r"); break;
                    default: sb.append(c);
                }
            }
        }
        sb.append('\n');
    }
}
