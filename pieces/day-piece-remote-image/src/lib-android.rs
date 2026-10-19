// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// Android: an ImageView decoding the pushed bytes via BitmapFactory. The Java factory
// (`dev.daybrite.day.piece.remoteimage.DayRemoteImage`) is bundled with this crate in
// `src/DayRemoteImage.java` and pulled into the app's Gradle build automatically via
// `[package.metadata.day.android]`, so the piece carries its own backend Java without touching
// day-android. The circle / rounded clip uses a resize-correct ViewOutlineProvider on screen
// and a matching Canvas path for software snapshots. Both also clip the placeholder background.
// Bytes cross the JNI as a `byte[]`; a SetBytes
// patch re-decodes (or clears on `None`). It is a growing leaf: `measure` fills the proposed frame.
// ---------------------------------------------------------------------------

use super::*;
use day_android::jni::Env;
use day_android::jni::objects::{JByteArray, JObject};
use day_android::{AHandle, Android, java};
use day_spec::NodeId;

// This piece's Java class (src/DayRemoteImage.java, on the app classpath at build).
day_android::java_class! {
    DayRemoteImage => dev.daybrite.day.piece.remoteimage.DayRemoteImage,
    methods {
        static fn makeImage(id: jlong, mode: jint, clip: jint, radius: jdouble, placeholder: jint)
            -> android.view.View,
        static fn setBytes(view: android.view.View, bytes: jbyte[]),
    },
}

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

/// Push bytes (or clear on `None`, a null `byte[]`) into `view`.
fn push_bytes(
    env: &mut Env,
    view: &JObject,
    bytes: &Option<std::sync::Arc<Vec<u8>>>,
) -> day_android::jni::errors::Result<()> {
    match bytes {
        Some(b) => {
            let arr = env.byte_array_from_slice(b)?;
            DayRemoteImage::setBytes(env, view, &arr)
        }
        None => DayRemoteImage::setBytes(env, view, JByteArray::default()),
    }
}

fn make(_backend: &mut Android, p: &RemoteImageProps, id: NodeId) -> AHandle {
    let (clip, radius) = clip_args(p.clip);
    day_android::realize("remote_image", |env| {
        let view = DayRemoteImage::makeImage(
            env,
            id.0 as i64,
            mode_code(p.mode),
            clip,
            radius,
            argb(p.placeholder),
        )?;
        push_bytes(env, &view, &p.bytes)?;
        Ok(view)
    })
}

fn update(_backend: &mut Android, h: &AHandle, patch: &RemoteImagePatch) {
    let RemoteImagePatch::SetBytes(bytes) = patch;
    java(|env| push_bytes(env, h.0.as_obj(), bytes));
}

day_pieces::renderer!(day_android::RENDERERS, Android,
    kind: KIND, props: RemoteImageProps, patch: RemoteImagePatch,
    make: make, update: update, measure: day_pieces::fill_measure);
