// ---------------------------------------------------------------------------
// Android: android.widget.VideoView + android.widget.MediaController — framework widgets, so this
// piece adds ZERO Gradle dependencies (androidx.media3/ExoPlayer is the later upgrade). The Java
// factory (`dev.daybrite.day.piece.media.DayMedia`) is bundled with THIS crate under `android/java`
// and pulled into the app's Gradle build via `[package.metadata.day.android]` — which ALSO
// contributes the INTERNET permission for network sources.
// ---------------------------------------------------------------------------

use super::*;
use day_android::jni::objects::JValue;
use day_android::{AHandle, Android, with_env};
use day_spec::NodeId;

/// This piece's OWN Java class (in the crate's android/java, on the app classpath at build).
const MEDIA_CLASS: &str = "dev/daybrite/day/piece/media/DayMedia";

fn make(_backend: &mut Android, p: &MediaProps, _id: NodeId) -> AHandle {
    with_env(|env| {
        let url = env.new_string(&p.url).expect("url");
        let view = env
            .call_static_method(
                MEDIA_CLASS,
                "makeMedia",
                "(Ljava/lang/String;ZZZZ)Landroid/view/View;",
                &[
                    JValue::Object(&url),
                    JValue::Bool(p.autoplay as u8),
                    JValue::Bool(p.looping as u8),
                    JValue::Bool(p.muted as u8),
                    JValue::Bool(p.controls as u8),
                ],
            )
            .expect("DayMedia.makeMedia")
            .l()
            .expect("View");
        AHandle(env.new_global_ref(view).expect("global ref"))
    })
}

fn update(_backend: &mut Android, h: &AHandle, patch: &MediaPatch) {
    // Commands cross as (code, url): 0=load, 1=play, 2=pause.
    let (code, url) = match patch {
        MediaPatch::Load(u) => (0, u.as_str()),
        MediaPatch::Play => (1, ""),
        MediaPatch::Pause => (2, ""),
    };
    with_env(|env| {
        let s = env.new_string(url).expect("cmd url");
        let _ = env.call_static_method(
            MEDIA_CLASS,
            "mediaCommand",
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
    kind: KIND, props: MediaProps, patch: MediaPatch,
    make: make, update: update, measure: day_pieces::fill_measure);
