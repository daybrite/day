// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// Android: android.webkit.WebView. The Java factory (`dev.daybrite.day.piece.webview.DayWebView`)
// is bundled with THIS crate under `android/java` and pulled into the app's Gradle build via
// `[package.metadata.day.android]` — which ALSO contributes the INTERNET permission (the extension
// this piece motivates). The Java reports each finished URL back through DayBridge.nativeOnEvent's
// open Custom-event kind (12) — §8.2's piece-defined event channel; the front-end handler maps the
// text payload to the bound URL.
// ---------------------------------------------------------------------------

use super::*;
use day_android::DayEnv;
use day_android::jni::objects::JValue;
use day_android::{AHandle, Android, with_env};
use day_spec::NodeId;

/// This piece's OWN Java class (in the crate's android/java, on the app classpath at build).
const WEBVIEW_CLASS: &str = "dev/daybrite/day/piece/webview/DayWebView";

fn make(_backend: &mut Android, p: &WebProps, id: NodeId) -> AHandle {
    // Inline mode (docs/webview.md): the assets tree IS the APK `assets/` root, and WebView
    // browses it through `file:///android_asset/` (exempt from the API-30 file-access
    // default). The URL is composed here; the Java side polices navigations against the
    // prefix and reports external ones back (LINK_REPORT).
    let (url, prefix) = if p.inline_root.is_empty() {
        (p.url.clone(), String::new())
    } else {
        (
            format!(
                "file:///android_asset/{}/{}",
                p.inline_root, p.inline_start
            ),
            format!("file:///android_asset/{}/", p.inline_root),
        )
    };
    with_env(|env| {
        // A Java throw (e.g. the staged DayWebView class missing) must not panic inside
        // realize: the panic unwinds the JNI up-call and aborts. Placeholder instead.
        let made = env.new_string(&url).ok().and_then(|url| {
            let prefix = env.new_string(&prefix).ok()?;
            day_android::try_make_view_on(
                env,
                WEBVIEW_CLASS,
                "makeWebView",
                "(JLjava/lang/String;Ljava/lang/String;)Landroid/view/View;",
                &[
                    JValue::Long(id.0 as i64),
                    JValue::Object(&url),
                    JValue::Object(&prefix),
                ],
            )
            .ok()
        });
        AHandle(made.unwrap_or_else(|| {
            log::warn!("day-piece-webview: DayWebView.makeWebView failed; substituting a placeholder");
            day_android::placeholder_view(env, "web_view")
        }))
    })
}

fn update(_backend: &mut Android, h: &AHandle, patch: &WebPatch) {
    // Evaluation (docs/webview-eval.md): `evaluateJavascript` plus the outer-JSON unquote and
    // the null/empty→engine-error mapping live in DayWebView.evalJs; the reply rides the
    // kind-12 channel keyed by `req`, like every other backend.
    if let WebPatch::Eval { req, script } = patch {
        with_env(|env| {
            let Ok(s) = env.new_string(script) else { return };
            let _ = env.dcall_static(
                WEBVIEW_CLASS,
                "evalJs",
                "(Landroid/view/View;DLjava/lang/String;)V",
                &[
                    JValue::Object(h.0.as_obj()),
                    JValue::Double(*req as f64),
                    JValue::Object(&s),
                ],
            );
        });
        return;
    }
    // Commands cross as (code, url): 0=load, 1=back, 2=forward, 3=stop, 4=reload.
    let (code, url) = match patch {
        WebPatch::Load(u) => (0, u.as_str()),
        WebPatch::Back => (1, ""),
        WebPatch::Forward => (2, ""),
        WebPatch::Stop => (3, ""),
        WebPatch::Reload => (4, ""),
        WebPatch::Eval { .. } => return, // handled above; keeps the match exhaustive
    };
    with_env(|env| {
        let Ok(s) = env.new_string(url) else { return };
        let _ = env.dcall_static(
            WEBVIEW_CLASS,
            "webCommand",
            "(Landroid/view/View;ILjava/lang/String;)V",
            &[
                JValue::Object(h.0.as_obj()),
                JValue::Int(code),
                JValue::Object(&s),
            ],
        );
    });
}

day_pieces::renderer!(day_android::RENDERERS, Android,
    kind: KIND, props: WebProps, patch: WebPatch,
    make: make, update: update, measure: day_pieces::fill_measure);
