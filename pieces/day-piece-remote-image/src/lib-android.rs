// ---------------------------------------------------------------------------
// Android: an ImageView decoding the pushed bytes via BitmapFactory. The Java factory
// (`dev.daybrite.day.piece.remoteimage.DayRemoteImage`) is bundled with THIS crate under
// `android/java` and pulled into the app's Gradle build automatically via
// `[package.metadata.day.android]` — so the piece carries its own backend Java without touching
// day-android. The circle / rounded clip is a ViewOutlineProvider + clipToOutline (resize-correct),
// and the placeholder is the view's background color. Bytes cross the JNI as a `byte[]`; a SetBytes
// patch re-decodes (or clears on `None`). It is a growing leaf: `measure` fills the proposed frame.
// ---------------------------------------------------------------------------

use super::*;
use day_android::DayEnv;
use day_android::jni::Env;
use day_android::jni::objects::{JObject, JValue};
use day_android::{AHandle, Android, with_env};
use day_spec::NodeId;

/// This piece's OWN Java class (in the crate's android/java, on the app classpath at build).
const IMAGE_CLASS: &str = "dev/daybrite/day/piece/remoteimage/DayRemoteImage";
const SET_BYTES_SIG: &str = "(Landroid/view/View;[B)V";

/// (clip discriminant, radius) for the Java factory: 0 none, 1 circle, 2 rounded.
fn clip_args(clip: Clip) -> (i32, f64) {
    match clip {
        Clip::None => (0, 0.0),
        Clip::Circle => (1, 0.0),
        Clip::Rounded(r) => (2, r),
    }
}

/// Content mode for the Java factory: 1 fill (CENTER_CROP), 0 fit (FIT_CENTER).
fn mode_code(mode: ContentMode) -> i32 {
    match mode {
        ContentMode::Fill => 1,
        ContentMode::Fit => 0,
    }
}

/// Pack a `Color` (0..1 components) into an Android ARGB int.
fn argb(c: Color) -> i32 {
    let ch = |v: f64| ((v.clamp(0.0, 1.0) * 255.0).round() as u32) & 0xff;
    ((ch(c.a) << 24) | (ch(c.r) << 16) | (ch(c.g) << 8) | ch(c.b)) as i32
}

/// Push bytes (or clear on `None`) into `view` via `DayRemoteImage.setBytes(View, byte[])`.
fn push_bytes(env: &mut Env, view: &JObject, bytes: &Option<std::sync::Arc<Vec<u8>>>) {
    match bytes {
        Some(b) => {
            let arr = env.byte_array_from_slice(b).expect("byte[]");
            let _ = env.dcall_static(
                IMAGE_CLASS,
                "setBytes",
                SET_BYTES_SIG,
                &[JValue::Object(view), JValue::Object(&arr)],
            );
        }
        None => {
            let null = JObject::null();
            let _ = env.dcall_static(
                IMAGE_CLASS,
                "setBytes",
                SET_BYTES_SIG,
                &[JValue::Object(view), JValue::Object(&null)],
            );
        }
    }
}

fn make(_backend: &mut Android, p: &RemoteImageProps, id: NodeId) -> AHandle {
    let (clip, radius) = clip_args(p.clip);
    with_env(|env| {
        let view = env
            .dcall_static(
                IMAGE_CLASS,
                "makeImage",
                "(JIIDI)Landroid/view/View;",
                &[
                    JValue::Long(id.0 as i64),
                    JValue::Int(mode_code(p.mode)),
                    JValue::Int(clip),
                    JValue::Double(radius),
                    JValue::Int(argb(p.placeholder)),
                ],
            )
            .expect("DayRemoteImage.makeImage")
            .l()
            .expect("View");
        push_bytes(env, &view, &p.bytes);
        AHandle(std::sync::Arc::new(
            env.new_global_ref(&view).expect("global ref"),
        ))
    })
}

fn update(_backend: &mut Android, h: &AHandle, patch: &RemoteImagePatch) {
    let RemoteImagePatch::SetBytes(bytes) = patch;
    with_env(|env| push_bytes(env, h.0.as_obj(), bytes));
}

day_pieces::renderer!(day_android::RENDERERS, Android,
    kind: KIND, props: RemoteImageProps, patch: RemoteImagePatch,
    make: make, update: update, measure: day_pieces::fill_measure);
