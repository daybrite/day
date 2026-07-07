// day-part-clipboard's OWN Android backend — a headless capability shim (no UI). It is bundled with
// this crate and folded into the app's Gradle build via [package.metadata.day.android], with ZERO
// edits to day-android; it registers no view. It talks to ClipboardManager using day-android's
// public Context (DayBridge.ctx); no manifest permission is needed. It is the Android twin of
// parts/day-part-clipboard/src/*.rs's other per-OS impls.
//
// Android 10+ restricts clipboard READS to the focused app (or the current IME): getText()/hasText()
// return null/false while the app is in the background. Writes are unrestricted.
package dev.daybrite.day.clipboard;

import android.content.ClipData;
import android.content.ClipDescription;
import android.content.ClipboardManager;
import android.content.Context;

import dev.daybrite.day.bridge.DayBridge;

public final class DayClipboard {
    private DayClipboard() {}

    private static ClipboardManager manager() {
        Context ctx = DayBridge.ctx;
        if (ctx == null) return null;
        return (ClipboardManager) ctx.getSystemService(Context.CLIPBOARD_SERVICE);
    }

    /** Places {@code text} on the clipboard as a plain-text clip. Returns success. */
    public static boolean setText(String text) {
        ClipboardManager cm = manager();
        if (cm == null || text == null) return false;
        try {
            cm.setPrimaryClip(ClipData.newPlainText("day", text));
            return true;
        } catch (RuntimeException e) {
            return false; // e.g. clipboard service unavailable
        }
    }

    /**
     * The current clip coerced to text, or null (empty clipboard, non-text clip, or the read was
     * denied because the app is not focused).
     */
    public static String getText() {
        ClipboardManager cm = manager();
        if (cm == null || !cm.hasPrimaryClip()) return null;
        ClipData clip = cm.getPrimaryClip();
        if (clip == null || clip.getItemCount() == 0) return null;
        CharSequence text = clip.getItemAt(0).coerceToText(DayBridge.ctx);
        return text != null ? text.toString() : null;
    }

    /** Whether the clipboard holds a clip with a text (or coercible HTML) representation. */
    public static boolean hasText() {
        ClipboardManager cm = manager();
        if (cm == null || !cm.hasPrimaryClip()) return false;
        ClipDescription desc = cm.getPrimaryClipDescription();
        return desc != null
                && (desc.hasMimeType(ClipDescription.MIMETYPE_TEXT_PLAIN)
                        || desc.hasMimeType(ClipDescription.MIMETYPE_TEXT_HTML));
    }
}
