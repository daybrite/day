// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Android-only glue: the persistent files dir (for the store root) and the uncaught-exception
//! handler install. The handler itself lives in Java (`android/java/.../crash/DayBreak.java`) so
//! that no JNI transition happens during a crash — it writes a `java-<sid>.kv` artifact directly,
//! in the same kv format [`crate::store`] reconciles.

use day_android::jni::objects::JValue;
use day_android::{DayEnv, as_jstring, with_env};

const CRASH_CLASS: &str = "dev/daybrite/day/crash/DayBreak";

/// `Context.getFilesDir().getAbsolutePath()` via day-android's bridge (resolved on the main
/// thread at init).
pub(crate) fn files_dir() -> Option<std::path::PathBuf> {
    with_env(|env| {
        let obj = env
            .dcall_static(
                "dev/daybrite/day/bridge/DayBridge",
                "filesDirPath",
                "()Ljava/lang/String;",
                &[],
            )
            .ok()?
            .l()
            .ok()?;
        if obj.is_null() {
            return None;
        }
        let path = env.dstr(&as_jstring(obj)).ok()?;
        if path.is_empty() {
            None
        } else {
            Some(std::path::PathBuf::from(path))
        }
    })
}

/// Install the default uncaught-exception handler (Java side), pointing it at our store dir and
/// session id so its report artifact lands beside the native ones. Best-effort.
pub(crate) fn install(dir: &std::path::Path, sid: &str) {
    let dir = dir.to_string_lossy().to_string();
    let sid = sid.to_string();
    with_env(|env| {
        let d = env.new_string(&dir).ok()?;
        let s = env.new_string(&sid).ok()?;
        env.dcall_static(
            CRASH_CLASS,
            "install",
            "(Ljava/lang/String;Ljava/lang/String;)V",
            &[JValue::Object(&d), JValue::Object(&s)],
        )
        .ok()?;
        // Don't return the borrowed JValueOwned out of the closure (it borrows `env`).
        Some(())
    });
}
