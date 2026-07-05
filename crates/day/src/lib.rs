//! Day — the umbrella crate apps depend on. One backend feature per binary (§3.2).

#[cfg(any(
    all(feature = "appkit", feature = "gtk"),
    all(feature = "appkit", feature = "mock"),
    all(feature = "gtk", feature = "mock"),
    all(feature = "qt", feature = "appkit"),
    all(feature = "qt", feature = "gtk"),
    all(feature = "qt", feature = "mock"),
    all(feature = "winui", feature = "appkit"),
    all(feature = "winui", feature = "gtk"),
    all(feature = "winui", feature = "qt"),
    all(feature = "winui", feature = "mock"),
))]
compile_error!("day: enable exactly one backend feature");

pub use day_core::{AnyPiece, BuildCx, Piece, PieceSeq, task};
pub use day_spec::WindowOptions;

pub mod prelude {
    pub use day_fluent::{LocalizedText, install as install_locales, set_locale, tr};
    pub use day_pieces::prelude::*;
    pub use day_spec::{Size, WindowOptions};
}

/// Launch the app on the selected backend (blocks; owns the native main loop).
#[cfg(feature = "appkit")]
pub fn launch(options: WindowOptions, root: impl FnOnce() -> AnyPiece + 'static) {
    day_script::init();
    day_core::launch_with(day_appkit::AppKit::new(), options, root);
}

#[cfg(feature = "gtk")]
pub fn launch(options: WindowOptions, root: impl FnOnce() -> AnyPiece + 'static) {
    day_script::init();
    day_core::launch_with(day_gtk::Gtk::new(), options, root);
}

#[cfg(feature = "qt")]
pub fn launch(options: WindowOptions, root: impl FnOnce() -> AnyPiece + 'static) {
    day_script::init();
    day_core::launch_with(day_qt::Qt::new(), options, root);
}

#[cfg(all(feature = "uikit", target_os = "ios"))]
pub fn launch(options: WindowOptions, root: impl FnOnce() -> AnyPiece + 'static) {
    day_script::init();
    day_core::launch_with(day_uikit::Uikit::new(), options, root);
}

#[cfg(all(feature = "winui", windows))]
pub fn launch(options: WindowOptions, root: impl FnOnce() -> AnyPiece + 'static) {
    day_script::init();
    day_core::launch_with(day_winui::WinUi::new(), options, root);
}

#[cfg(feature = "mock")]
pub fn launch(options: WindowOptions, root: impl FnOnce() -> AnyPiece + 'static) {
    let (mock, _probe) = day_mock::MockToolkit::new();
    day_core::launch_with(mock, options, root);
}

// ---------------------------------------------------------------------------
// App entry macros (§17.4): the mobile shells bind fixed exported symbols
// (Runner/main.swift → `day_main`; dev.daybrite.day.bridge.DayBridge → `Java_…` natives).
// These expand to that glue so an app's lib.rs carries one line per platform.
// Both emit nothing off their target OS, so apps invoke them unconditionally.
// ---------------------------------------------------------------------------

/// Expands to the `day_main` C export the iOS Runner's `main.swift` calls
/// (`@_silgen_name("day_main")`). The optional title is currently unused on
/// iOS (the window fills the screen bounds); accepted for future window-scene use.
///
/// ```ignore
/// day::ios_main!(root);              // or: day::ios_main!("My App", root);
/// ```
#[macro_export]
macro_rules! ios_main {
    ($root:expr) => {
        $crate::ios_main!("", $root);
    };
    ($title:expr, $root:expr) => {
        /// iOS entry: the Runner's main.swift calls this from the app staticlib (§17.4).
        #[cfg(target_os = "ios")]
        #[unsafe(no_mangle)]
        pub extern "C" fn day_main() {
            $crate::launch(
                $crate::WindowOptions {
                    title: ($title).into(),
                    ..::core::default::Default::default()
                },
                $root,
            );
        }
    };
}

/// Expands to the three JNI exports `dev.daybrite.day.bridge.DayBridge`'s natives resolve
/// against in the app cdylib (`nativeStart`/`nativeOnEvent`/`nativeRunPosted`),
/// wired to the given root piece.
///
/// ```ignore
/// day::android_main!(root);
/// ```
#[macro_export]
macro_rules! android_main {
    ($root:expr) => {
        #[cfg(target_os = "android")]
        #[unsafe(no_mangle)]
        pub extern "system" fn Java_dev_daybrite_day_bridge_DayBridge_nativeStart(
            mut env: $crate::android::jni::JNIEnv,
            _class: $crate::android::jni::objects::JClass,
            root: $crate::android::jni::objects::JObject,
            density: $crate::android::jni::sys::jfloat,
            w: $crate::android::jni::sys::jint,
            h: $crate::android::jni::sys::jint,
            autodrive: $crate::android::jni::objects::JString,
            locale: $crate::android::jni::objects::JString,
            env_blob: $crate::android::jni::objects::JString,
        ) {
            fn opt_string(
                env: &mut $crate::android::jni::JNIEnv,
                s: &$crate::android::jni::objects::JString,
            ) -> ::core::option::Option<::std::string::String> {
                if s.is_null() {
                    ::core::option::Option::None
                } else {
                    env.get_string(s).ok().map(|v| v.into())
                }
            }
            let a = opt_string(&mut env, &autodrive);
            let l = opt_string(&mut env, &locale);
            let e = opt_string(&mut env, &env_blob);
            $crate::android::start(&mut env, root, density, w, h, a, l, e, $root);
        }

        #[cfg(target_os = "android")]
        #[unsafe(no_mangle)]
        pub extern "system" fn Java_dev_daybrite_day_bridge_DayBridge_nativeOnEvent(
            mut env: $crate::android::jni::JNIEnv,
            _class: $crate::android::jni::objects::JClass,
            id: $crate::android::jni::sys::jlong,
            kind: $crate::android::jni::sys::jint,
            num: $crate::android::jni::sys::jdouble,
            s: $crate::android::jni::objects::JString,
        ) {
            $crate::android::dispatch_event(&mut env, id, kind, num, &s);
        }

        #[cfg(target_os = "android")]
        #[unsafe(no_mangle)]
        pub extern "system" fn Java_dev_daybrite_day_bridge_DayBridge_nativeRunPosted(
            _env: $crate::android::jni::JNIEnv,
            _class: $crate::android::jni::objects::JClass,
            token: $crate::android::jni::sys::jlong,
        ) {
            $crate::android::run_posted(token);
        }

        #[cfg(target_os = "android")]
        #[unsafe(no_mangle)]
        pub extern "system" fn Java_dev_daybrite_day_bridge_DayBridge_nativeListLen(
            _env: $crate::android::jni::JNIEnv,
            _class: $crate::android::jni::objects::JClass,
            host_id: $crate::android::jni::sys::jlong,
        ) -> $crate::android::jni::sys::jint {
            $crate::android::list_len(host_id) as $crate::android::jni::sys::jint
        }

        #[cfg(target_os = "android")]
        #[unsafe(no_mangle)]
        pub extern "system" fn Java_dev_daybrite_day_bridge_DayBridge_nativeListBind<'a>(
            mut env: $crate::android::jni::JNIEnv<'a>,
            _class: $crate::android::jni::objects::JClass<'a>,
            host_id: $crate::android::jni::sys::jlong,
            position: $crate::android::jni::sys::jint,
            cell: $crate::android::jni::objects::JObject<'a>,
        ) {
            $crate::android::list_bind(&mut env, host_id, position, cell);
        }
    };
}

/// Android glue (§17.4): the app cdylib's JNI exports forward here.
#[cfg(all(feature = "widget", target_os = "android"))]
pub mod android {
    pub use day_android::jni;
    pub use day_android::{dispatch_event, list_bind, list_len, run_posted};

    #[allow(clippy::too_many_arguments)]
    pub fn start(
        env: &mut jni::JNIEnv,
        root: jni::objects::JObject,
        density: f32,
        w: i32,
        h: i32,
        autodrive: Option<String>,
        locale: Option<String>,
        env_blob: Option<String>,
        root_piece: impl FnOnce() -> crate::AnyPiece + 'static,
    ) {
        // Before any println!: send stdout/stderr to logcat (Android drops them otherwise).
        day_android::redirect_stdio_to_logcat();
        if let Some(a) = autodrive {
            unsafe { std::env::set_var("DAY_AUTODRIVE", a) };
        }
        if let Some(l) = locale {
            unsafe { std::env::set_var("DAY_LOCALE", l) };
        }
        if let Some(blob) = env_blob {
            for line in blob.lines() {
                if let Some((k, v)) = line.split_once('=') {
                    unsafe { std::env::set_var(k, v) };
                }
            }
        }
        day_android::init(env, root, density, w, h);
        day_script::init();
        day_core::launch_with(
            day_android::Android::new(),
            crate::WindowOptions::default(),
            root_piece,
        );
    }
}
