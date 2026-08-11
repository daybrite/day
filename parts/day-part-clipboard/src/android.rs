// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// Android, whole: the Java that drives ClipboardManager, the declaration that binds it, and the
// mapping into this crate's API. Nothing about this platform appears anywhere else in the crate.
//
// ClipboardManager needs a `Context` and has no C entry point, so it is this crate's only foreign
// arm (docs/bridge.md). Written in Java rather than Kotlin so it compiles in any Android project.
//
// Note: since Android 10, apps can only READ the clipboard while they hold input focus —
// `get_text`/`has_text` answer empty/false in the background. Writing is always allowed.

use day_bridge::Error;

pub fn set_text(text: &str) -> bool {
    set_text_native(text).unwrap_or(false)
}

pub fn get_text() -> Option<String> {
    // The arm answers with an empty string for "nothing on the clipboard", because `Option` does
    // not cross a bridge (docs/bridge.md "Types"). A clip holding the empty string is
    // indistinguishable from an empty clipboard, which no caller can act on differently anyway.
    match get_text_native() {
        Ok(text) if !text.is_empty() => Some(text),
        _ => None,
    }
}

pub fn has_text() -> bool {
    has_text_native().unwrap_or(false)
}

day_bridge::bridge! {
    #[day_bridge::declare]
    extern "day" {
        /// Whether the clip was placed.
        fn set_text_native(text: &str) -> Result<bool, day_bridge::Error>;
        /// The current clip coerced to text, or `""` when there is none to read.
        fn get_text_native() -> Result<String, day_bridge::Error>;
        /// Whether the clipboard holds a clip with a text (or coercible HTML) representation.
        fn has_text_native() -> Result<bool, day_bridge::Error>;
    }

    #[day_bridge::impl(java, platforms = [android])]
    java!(
        prelude = r#"
            import android.content.ClipData;
            import android.content.ClipDescription;
            import android.content.ClipboardManager;
            import android.content.Context;
            import dev.daybrite.day.bridge.DayBridge;
        "#,
        body = r#"
            private static ClipboardManager manager() {
                Context ctx = DayBridge.ctx;
                if (ctx == null) return null;
                return (ClipboardManager) ctx.getSystemService(Context.CLIPBOARD_SERVICE);
            }

            public static boolean set_text_native(String text) {
                ClipboardManager cm = manager();
                if (cm == null || text == null) return false;
                try {
                    cm.setPrimaryClip(ClipData.newPlainText("day", text));
                    return true;
                } catch (RuntimeException e) {
                    return false; // e.g. clipboard service unavailable
                }
            }

            public static String get_text_native() {
                ClipboardManager cm = manager();
                if (cm == null || !cm.hasPrimaryClip()) return "";
                ClipData clip = cm.getPrimaryClip();
                if (clip == null || clip.getItemCount() == 0) return "";
                CharSequence text = clip.getItemAt(0).coerceToText(DayBridge.ctx);
                return text != null ? text.toString() : "";
            }

            public static boolean has_text_native() {
                ClipboardManager cm = manager();
                if (cm == null || !cm.hasPrimaryClip()) return false;
                ClipDescription desc = cm.getPrimaryClipDescription();
                return desc != null
                        && (desc.hasMimeType(ClipDescription.MIMETYPE_TEXT_PLAIN)
                                || desc.hasMimeType(ClipDescription.MIMETYPE_TEXT_HTML));
            }
        "#,
    );

    // The fallback every bridge declares. This file is `#[cfg(target_os = "android")]`, so it is
    // never compiled — it satisfies the rule that a bridge always has an answer for an unclaimed
    // target.
    #[day_bridge::impl(rust, platforms = [other])]
    fn set_text_native(_text: &str) -> Result<bool, day_bridge::Error> {
        Err(Error::Unsupported)
    }

    #[day_bridge::impl(rust, platforms = [other])]
    fn get_text_native() -> Result<String, day_bridge::Error> {
        Err(Error::Unsupported)
    }

    #[day_bridge::impl(rust, platforms = [other])]
    fn has_text_native() -> Result<bool, day_bridge::Error> {
        Err(Error::Unsupported)
    }
}
