// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// Android: a LottieAnimationView from com.airbnb.android:lottie, created by this crate's OWN Java
// (android/java/…/DayLottie.java) — folded into the app's Gradle build via
// [package.metadata.day.android] (which also declares the Gradle dependency), without touching
// day-android. Rust calls its own class through the re-exported `jni`.
// ---------------------------------------------------------------------------

use super::*;
use day_android::DayEnv;
use day_android::jni::objects::JValue;
use day_android::{AHandle, Android, with_env};
use day_spec::NodeId;

/// This piece's OWN Java class (in the crate's android/java, on the app classpath at build).
const LOTTIE_CLASS: &str = "dev/daybrite/day/piece/lottie/DayLottie";

fn make(_backend: &mut Android, p: &LottieProps, _id: NodeId) -> AHandle {
    with_env(|env| {
        // A Java throw (e.g. the Lottie dependency missing from the app build) must not
        // panic inside realize — the panic unwinds the JNI up-call and aborts. Placeholder.
        let made = env.new_string(&p.name).ok().and_then(|name| {
            day_android::try_make_view_on(
                env,
                LOTTIE_CLASS,
                "makeLottie",
                "(Ljava/lang/String;ZZF)Landroid/view/View;",
                &[
                    JValue::Object(&name),
                    JValue::Bool(p.looping),
                    JValue::Bool(p.autoplay),
                    JValue::Float(p.speed as f32),
                ],
            )
            .ok()
        });
        AHandle(made.unwrap_or_else(|| {
            eprintln!("day-piece-lottie: DayLottie.makeLottie failed; substituting a placeholder");
            day_android::placeholder_view(env, "lottie")
        }))
    })
}

fn update(_backend: &mut Android, h: &AHandle, patch: &LottiePatch) {
    match patch {
        LottiePatch::Speed(s) => with_env(|env| {
            let _ = env.dcall_static(
                LOTTIE_CLASS,
                "setSpeed",
                "(Landroid/view/View;F)V",
                &[JValue::Object(h.0.as_obj()), JValue::Float(*s as f32)],
            );
        }),
    }
}

// name/looping/autoplay are set once at build; only `speed` patches. `fill_measure` gives the uniform
// growing-leaf sizing (which day-android's `measure: None` default would otherwise collapse).
day_pieces::renderer!(day_android::RENDERERS, Android,
    kind: KIND, props: LottieProps, patch: LottiePatch, make: make, update: update,
    measure: day_pieces::fill_measure);
