// ---------------------------------------------------------------------------
// Android: a LottieAnimationView from com.airbnb.android:lottie, created by this crate's OWN Java
// (android/java/…/DayLottie.java) — folded into the app's Gradle build via
// [package.metadata.day.android] (which also declares the Gradle dependency), with ZERO edits to
// day-android. Rust calls its own class through the re-exported `jni`.
// ---------------------------------------------------------------------------

use super::*;
use day_android::jni::objects::JValue;
use day_android::{AHandle, Android, with_env};
use day_spec::{NodeId, Proposal, Renderer, Size};
use linkme::distributed_slice;

/// This piece's OWN Java class (in the crate's android/java, on the app classpath at build).
const LOTTIE_CLASS: &str = "dev/daybrite/day/piece/lottie/DayLottie";

fn make(_backend: &mut Android, props: &dyn std::any::Any, _id: NodeId) -> AHandle {
    let p = props.downcast_ref::<LottieProps>().unwrap();
    with_env(|env| {
        let name = env.new_string(&p.name).expect("name");
        let view = env
            .call_static_method(
                LOTTIE_CLASS,
                "makeLottie",
                "(Ljava/lang/String;ZZ)Landroid/view/View;",
                &[
                    JValue::Object(&name),
                    JValue::Bool(p.looping as u8),
                    JValue::Bool(p.autoplay as u8),
                ],
            )
            .expect("DayLottie.makeLottie")
            .l()
            .expect("View");
        AHandle(env.new_global_ref(view).expect("global ref"))
    })
}

fn update(_backend: &mut Android, _h: &AHandle, _patch: &dyn std::any::Any) {
    // No patches: the animation is configured once at build (name/looping/autoplay).
}

/// Fill the offered space (day-android's `measure: None` default returns the view's natural size).
fn measure(_backend: &mut Android, _h: &AHandle, p: Proposal) -> Size {
    Size::new(p.width.unwrap_or(0.0), p.height.unwrap_or(0.0))
}

#[distributed_slice(day_android::RENDERERS)]
static LOTTIE_ANDROID: fn() -> Renderer<Android> = || Renderer {
    kind: KIND,
    make,
    update,
    measure: Some(measure),
};
