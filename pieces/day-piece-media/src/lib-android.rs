// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// Android: android.widget.VideoView + android.widget.MediaController for pictures, and a bare
// android.media.MediaPlayer behind an empty View for sound only — framework classes, so this piece
// adds ZERO Gradle dependencies (androidx.media3/ExoPlayer is the later upgrade). The Java factory
// (`dev.daybrite.day.piece.media.DayMedia`) is bundled with THIS crate under `android/java` and
// pulled into the app's Gradle build via `[package.metadata.day.android]` — which ALSO
// contributes the INTERNET permission for network sources. Playback state comes back through
// DayBridge.nativeOnEvent's open Custom-event kind (12), `num` = the piece's report code.
// ---------------------------------------------------------------------------

use super::*;
use day_android::DayEnv;
use day_android::jni::objects::JValue;
use day_android::{AHandle, Android, with_env};
use day_spec::{NodeId, Proposal, Size};

/// This piece's OWN Java class (in the crate's android/java, on the app classpath at build).
const MEDIA_CLASS: &str = "dev/daybrite/day/piece/media/DayMedia";

fn make(_backend: &mut Android, p: &MediaProps, id: NodeId) -> AHandle {
    with_env(|env| {
        let url = env.new_string(&p.url).expect("url");
        let view = env
            .dcall_static(
                MEDIA_CLASS,
                "makeMedia",
                "(JLjava/lang/String;ZZZZZD)Landroid/view/View;",
                &[
                    JValue::Long(id.0 as i64),
                    JValue::Object(&url),
                    JValue::Bool(p.autoplay),
                    JValue::Bool(p.looping),
                    JValue::Bool(p.muted),
                    JValue::Bool(p.controls),
                    JValue::Bool(p.audio_only),
                    JValue::Double(p.volume.clamp(0.0, 1.0)),
                ],
            )
            .expect("DayMedia.makeMedia")
            .l()
            .expect("View");
        AHandle(std::sync::Arc::new(
            env.new_global_ref(view).expect("global ref"),
        ))
    })
}

fn update(_backend: &mut Android, h: &AHandle, patch: &MediaPatch) {
    // Commands cross as (code, url, value): 0=load, 1=play, 2=pause, 3=stop, 4=volume.
    let (code, url, value) = match patch {
        MediaPatch::Load(u) => (0, u.as_str(), 0.0),
        MediaPatch::Play => (1, "", 0.0),
        MediaPatch::Pause => (2, "", 0.0),
        MediaPatch::Stop => (3, "", 0.0),
        MediaPatch::Volume(v) => (4, "", v.clamp(0.0, 1.0)),
    };
    with_env(|env| {
        let s = env.new_string(url).expect("cmd url");
        let _ = env.dcall_static(
            MEDIA_CLASS,
            "mediaCommand",
            "(Landroid/view/View;ILjava/lang/String;D)V",
            &[
                JValue::Object(h.0.as_obj()),
                JValue::Int(code),
                JValue::Object(&s),
                JValue::Double(value),
            ],
        );
    });
}

/// A sound-only player takes no room; a video fills what it is offered.
fn measure(backend: &mut Android, h: &AHandle, proposal: Proposal) -> Size {
    let audio_only = with_env(|env| {
        env.dcall_static(
            MEDIA_CLASS,
            "isAudioOnly",
            "(Landroid/view/View;)Z",
            &[JValue::Object(h.0.as_obj())],
        )
        .and_then(|v| v.z())
        .unwrap_or(false)
    });
    if audio_only {
        return Size::ZERO;
    }
    day_pieces::fill_measure(backend, h, proposal)
}

/// Release the MediaPlayer a sound-only view carries; a VideoView releases its own.
fn release(_backend: &mut Android, h: &AHandle) {
    with_env(|env| {
        let _ = env.dcall_static(
            MEDIA_CLASS,
            "releaseMedia",
            "(Landroid/view/View;)V",
            &[JValue::Object(h.0.as_obj())],
        );
    });
}

day_pieces::renderer!(day_android::RENDERERS, Android,
    kind: KIND, props: MediaProps, patch: MediaPatch,
    make: make, update: update, measure: measure, release: release);
