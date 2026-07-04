// ---------------------------------------------------------------------------
// Android: android.webkit.WebView. The Java factory (`dev.daybrite.day.piece.webview.DayWebView`)
// is bundled with THIS crate under `android/java` and pulled into the app's Gradle build via
// `[package.metadata.day.android]` — which ALSO contributes the INTERNET permission (the extension
// this piece motivates). The Java reports each finished URL back through DayBridge.nativeOnEvent's
// public TextChanged kind (1); the front-end handler maps it to the bound URL.
// ---------------------------------------------------------------------------

use super::*;
use day_android::jni::objects::JValue;
use day_android::{AHandle, Android, with_env};
use day_spec::{NodeId, Proposal, Renderer, Size};
use linkme::distributed_slice;

/// This piece's OWN Java class (in the crate's android/java, on the app classpath at build).
const WEBVIEW_CLASS: &str = "dev/daybrite/day/piece/webview/DayWebView";

fn make(_backend: &mut Android, props: &dyn std::any::Any, id: NodeId) -> AHandle {
    let p = props.downcast_ref::<WebProps>().unwrap();
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

fn update(_backend: &mut Android, h: &AHandle, patch: &dyn std::any::Any) {
    let Some(patch) = patch.downcast_ref::<WebPatch>() else {
        return;
    };
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

/// Fill the offered space. day-android's default measure (for `measure: None`) returns the view's
/// NATURAL size — which is ~0 for a WebView, so it would collapse. Returning the proposal makes the
/// grow leaf fill, exactly as the built-in `list` does (AppKit/Qt/UIKit already return the proposal
/// from their `measure: None` default, which is why they fill without this).
fn measure(_backend: &mut Android, _h: &AHandle, p: Proposal) -> Size {
    Size::new(p.width.unwrap_or(0.0), p.height.unwrap_or(0.0))
}

#[distributed_slice(day_android::RENDERERS)]
static WEBVIEW_ANDROID: fn() -> Renderer<Android> = || Renderer {
    kind: KIND,
    make,
    update,
    measure: Some(measure),
};
