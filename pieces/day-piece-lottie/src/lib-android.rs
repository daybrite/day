// ---------------------------------------------------------------------------
// Android: a LottieAnimationView from com.airbnb.android:lottie, created by this crate's OWN Java
// (android/java/…/DayLottie.java) — folded into the app's Gradle build via
// [package.metadata.day.android] (which also declares the Gradle dependency), with ZERO edits to
// day-android. Rust calls its own class through the re-exported `jni`.
// ---------------------------------------------------------------------------

use super::*;
use day_android::jni::objects::JValue;
use day_android::{AHandle, Android, with_env};
use day_spec::NodeId;

/// This piece's OWN Java class (in the crate's android/java, on the app classpath at build).
const LOTTIE_CLASS: &str = "dev/daybrite/day/piece/lottie/DayLottie";

fn make(_backend: &mut Android, p: &LottieProps, _id: NodeId) -> AHandle {
    with_env(|env| {
        let name = env.new_string(&p.name).expect("name");
        let view = env
            .call_static_method(
                LOTTIE_CLASS,
                "makeLottie",
                "(Ljava/lang/String;ZZF)Landroid/view/View;",
                &[
                    JValue::Object(&name),
                    JValue::Bool(p.looping as u8),
                    JValue::Bool(p.autoplay as u8),
                    JValue::Float(p.speed as f32),
                ],
            )
            .expect("DayLottie.makeLottie")
            .l()
            .expect("View");
        AHandle(env.new_global_ref(view).expect("global ref"))
    })
}

fn update(_backend: &mut Android, h: &AHandle, patch: &LottiePatch) {
    match patch {
        LottiePatch::Speed(s) => with_env(|env| {
            let _ = env.call_static_method(
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
