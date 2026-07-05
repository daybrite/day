// ---------------------------------------------------------------------------
// Android: android.webkit.WebView. The Java factory (`dev.daybrite.day.piece.webview.DayWebView`)
// is bundled with THIS crate under `android/java` and pulled into the app's Gradle build via
// `[package.metadata.day.android]` — which ALSO contributes the INTERNET permission (the extension
// this piece motivates). The Java reports each finished URL back through DayBridge.nativeOnEvent's
// open Custom-event kind (12) — §8.2's piece-defined event channel; the front-end handler maps the
// text payload to the bound URL.
// ---------------------------------------------------------------------------

use super::*;
use day_android::jni::objects::JValue;
use day_android::{AHandle, Android, with_env};
use day_spec::NodeId;

/// This piece's OWN Java class (in the crate's android/java, on the app classpath at build).
const WEBVIEW_CLASS: &str = "dev/daybrite/day/piece/webview/DayWebView";

fn make(_backend: &mut Android, p: &WebProps, id: NodeId) -> AHandle {
    with_env(|env| {
        let url = env.new_string(&p.url).expect("url");
        let view = env
            .call_static_method(
                WEBVIEW_CLASS,
                "makeWebView",
                "(JLjava/lang/String;)Landroid/view/View;",
                &[JValue::Long(id.0 as i64), JValue::Object(&url)],
            )
            .expect("DayWebView.makeWebView")
            .l()
            .expect("View");
        AHandle(env.new_global_ref(view).expect("global ref"))
    })
}

fn update(_backend: &mut Android, h: &AHandle, patch: &WebPatch) {
    // Commands cross as (code, url): 0=load, 1=back, 2=forward, 3=stop, 4=reload.
    let (code, url) = match patch {
        WebPatch::Load(u) => (0, u.as_str()),
        WebPatch::Back => (1, ""),
        WebPatch::Forward => (2, ""),
        WebPatch::Stop => (3, ""),
        WebPatch::Reload => (4, ""),
    };
    with_env(|env| {
        let s = env.new_string(url).expect("cmd url");
        let _ = env.call_static_method(
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
