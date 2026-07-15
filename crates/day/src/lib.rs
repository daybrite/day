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
pub use day_core::{AssetName, FontFamily, ImageName, Resource, resource};
pub use day_core::{lifecycle_supported, on_lifecycle};
// Tweaks (docs/tweaks.md): the realized-node id, the size-invalidation hook for native
// mutations Day can't see, and the retained ref live in the prelude via day-pieces.
pub use day_core::{RNode, invalidate_size};
pub use day_pieces::NativeRef;
// Typed routes (docs/navigation.md): `day::routes! { enum Section { Home => "home", … } }`.
pub use day_pieces::routes;
pub use day_spec::{Lifecycle, WindowOptions};

/// The display name of the toolkit compiled into THIS binary — `"AppKit"`, `"GTK"`, `"Qt"`,
/// `"UIKit"`, `"Android"`, `"WinUI"` (or `"Mock"`). Handy for a window title that names its backend.
pub const fn toolkit_name() -> &'static str {
    #[cfg(feature = "appkit")]
    {
        return "AppKit";
    }
    #[cfg(feature = "gtk")]
    {
        return "GTK";
    }
    #[cfg(feature = "qt")]
    {
        return "Qt";
    }
    #[cfg(feature = "uikit")]
    {
        return "UIKit";
    }
    #[cfg(feature = "widget")]
    {
        return "Android";
    }
    #[cfg(feature = "winui")]
    {
        return "WinUI";
    }
    #[cfg(feature = "arkui")]
    {
        return "ArkUI";
    }
    #[allow(unreachable_code)]
    {
        "Mock"
    }
}

pub mod prelude {
    pub use day_fluent::{LocalizedText, install as install_locales, set_locale, tr};
    pub use day_pieces::prelude::*;
    pub use day_spec::{Lifecycle, Size, WindowOptions};
    pub use {super::lifecycle_supported, super::on_lifecycle};
    // Bundled-resource random-access API (§18.3): `resource("name")` -> `Resource`.
    pub use day_core::{AssetName, FontFamily, ImageName, Resource, resource};
    // Toolkit capability probe (docs): lets app/piece content adapt to the backend, e.g. skip a
    // title the native nav already shows (`Cap::NavHeader`). `capability(cap) -> Support`.
    pub use day_core::capability;
    pub use day_spec::{Cap, Support};
    // Layout direction (docs/localization): `is_rtl()` lets a piece mirror its own drawing under a
    // right-to-left locale — the layout engine mirrors placement, but a `canvas` owns its coordinates.
    pub use day_core::{is_rtl, layout_direction};
    pub use day_spec::LayoutDirection;
}

/// App-lifecycle support for the backend compiled into THIS binary (docs/lifecycle.md).
///
/// Register handlers with [`on_lifecycle`]; guard phases a platform may not deliver either at runtime
/// (`if day::lifecycle::supported(p) { … }`) or at compile time with [`require_lifecycle!`].
pub mod lifecycle {
    pub use day_core::{lifecycle_supported, on_lifecycle};
    pub use day_spec::Lifecycle;

    /// Does the backend compiled into this binary deliver `phase`? A `const fn`, so it drives both a
    /// runtime guard and the compile-time [`crate::require_lifecycle!`] assertion. Agrees with the
    /// runtime [`day_core::lifecycle_supported`] once the app is running.
    pub const fn supported(phase: Lifecycle) -> bool {
        #[cfg(feature = "appkit")]
        {
            return day_appkit::lifecycle_supported(phase);
        }
        #[cfg(feature = "gtk")]
        {
            return day_gtk::lifecycle_supported(phase);
        }
        #[cfg(feature = "qt")]
        {
            return day_qt::lifecycle_supported(phase);
        }
        #[cfg(all(feature = "uikit", target_os = "ios"))]
        {
            return day_uikit::lifecycle_supported(phase);
        }
        #[cfg(all(feature = "widget", target_os = "android"))]
        {
            return day_android::lifecycle_supported(phase);
        }
        #[cfg(all(feature = "winui", windows))]
        {
            return day_winui::lifecycle_supported(phase);
        }
        // No concrete backend (mock, or a mobile backend compiled for the host to check): the
        // universal phases are always deliverable.
        #[allow(unreachable_code)]
        {
            phase.is_universal()
        }
    }
}

/// Compile-time assert that the backend in this binary delivers `$phase`, else a build error. Use it
/// to make a hard dependency on a platform-specific phase explicit:
/// `day::require_lifecycle!(day::Lifecycle::DidEnterBackground);` fails to compile on desktop.
/// For soft handling, guard with [`lifecycle::supported`] / [`lifecycle_supported`] instead.
#[macro_export]
macro_rules! require_lifecycle {
    ($phase:expr) => {
        const {
            ::core::assert!(
                $crate::lifecycle::supported($phase),
                "this Day backend does not deliver that lifecycle phase (see docs/lifecycle.md)",
            )
        }
    };
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
        // jni 0.22 native methods receive the FFI-safe `EnvUnowned`; `with_env` upgrades it to the
        // real `Env` (sharing the frame's `'local`, so the object args pass straight in) and wraps
        // the body in a `catch_unwind` so a panic never unwinds across the JNI boundary.
        #[cfg(target_os = "android")]
        #[unsafe(no_mangle)]
        pub extern "system" fn Java_dev_daybrite_day_bridge_DayBridge_nativeStart<'local>(
            mut env: $crate::android::jni::EnvUnowned<'local>,
            _class: $crate::android::jni::objects::JClass<'local>,
            root: $crate::android::jni::objects::JObject<'local>,
            density: $crate::android::jni::sys::jfloat,
            w: $crate::android::jni::sys::jint,
            h: $crate::android::jni::sys::jint,
            autodrive: $crate::android::jni::objects::JString<'local>,
            locale: $crate::android::jni::objects::JString<'local>,
            env_blob: $crate::android::jni::objects::JString<'local>,
        ) {
            let _ = env
                .with_env(|env| {
                    let a = $crate::android::read_jstring(env, &autodrive);
                    let l = $crate::android::read_jstring(env, &locale);
                    let e = $crate::android::read_jstring(env, &env_blob);
                    $crate::android::start(env, root, density, w, h, a, l, e, $root);
                    ::core::result::Result::Ok::<(), $crate::android::jni::errors::Error>(())
                })
                .into_outcome();
        }

        #[cfg(target_os = "android")]
        #[unsafe(no_mangle)]
        pub extern "system" fn Java_dev_daybrite_day_bridge_DayBridge_nativeOnEvent<'local>(
            mut env: $crate::android::jni::EnvUnowned<'local>,
            _class: $crate::android::jni::objects::JClass<'local>,
            id: $crate::android::jni::sys::jlong,
            kind: $crate::android::jni::sys::jint,
            num: $crate::android::jni::sys::jdouble,
            s: $crate::android::jni::objects::JString<'local>,
        ) {
            let _ = env
                .with_env(|env| {
                    $crate::android::dispatch_event(env, id, kind, num, &s);
                    ::core::result::Result::Ok::<(), $crate::android::jni::errors::Error>(())
                })
                .into_outcome();
        }

        #[cfg(target_os = "android")]
        #[unsafe(no_mangle)]
        pub extern "system" fn Java_dev_daybrite_day_bridge_DayBridge_nativeRunPosted(
            _env: $crate::android::jni::EnvUnowned,
            _class: $crate::android::jni::objects::JClass,
            token: $crate::android::jni::sys::jlong,
        ) {
            $crate::android::run_posted(token);
        }

        #[cfg(target_os = "android")]
        #[unsafe(no_mangle)]
        pub extern "system" fn Java_dev_daybrite_day_bridge_DayBridge_nativeListLen(
            _env: $crate::android::jni::EnvUnowned,
            _class: $crate::android::jni::objects::JClass,
            host_id: $crate::android::jni::sys::jlong,
        ) -> $crate::android::jni::sys::jint {
            $crate::android::list_len(host_id) as $crate::android::jni::sys::jint
        }

        #[cfg(target_os = "android")]
        #[unsafe(no_mangle)]
        pub extern "system" fn Java_dev_daybrite_day_bridge_DayBridge_nativeListBind<'local>(
            mut env: $crate::android::jni::EnvUnowned<'local>,
            _class: $crate::android::jni::objects::JClass<'local>,
            host_id: $crate::android::jni::sys::jlong,
            position: $crate::android::jni::sys::jint,
            cell: $crate::android::jni::objects::JObject<'local>,
        ) {
            let _ = env
                .with_env(|env| {
                    $crate::android::list_bind(env, host_id, position, cell);
                    ::core::result::Result::Ok::<(), $crate::android::jni::errors::Error>(())
                })
                .into_outcome();
        }
    };
}

/// Android glue (§17.4): the app cdylib's JNI exports forward here.
#[cfg(all(feature = "widget", target_os = "android"))]
pub mod android {
    pub use day_android::jni;
    pub use day_android::{dispatch_event, list_bind, list_len, read_jstring, run_posted};

    #[allow(clippy::too_many_arguments)]
    pub fn start(
        env: &mut jni::Env,
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

/// Expands to the `day_arkui_start` C export the HarmonyOS ArkUI shim's `start(...)` NAPI wrapper
/// calls (from ArkTS: `import native from 'libday_arkui.so'; native.start(nodeContent, w, h, density)`).
/// It mounts the app's `root` piece into the ArkTS `NodeContent` and runs the loop.
///
/// ```ignore
/// day::arkui_main!(root);
/// ```
#[macro_export]
macro_rules! arkui_main {
    ($root:expr) => {
        /// HarmonyOS entry: the ArkUI shim's NAPI `start` calls this from the app cdylib (§17.4).
        #[cfg(target_env = "ohos")]
        #[unsafe(no_mangle)]
        pub extern "C" fn day_arkui_start(
            content: *mut ::core::ffi::c_void,
            w: f64,
            h: f64,
            density: f64,
        ) {
            $crate::arkui::start(content, w, h, density, $root);
        }
    };
}

/// HarmonyOS ArkUI glue (§17.4): the app cdylib's `day_arkui_start` export forwards here.
#[cfg(all(feature = "arkui", target_env = "ohos"))]
pub mod arkui {
    use core::ffi::c_void;

    /// Mount `root` into the ArkTS `NodeContent` and run the loop. `w_vp`/`h_vp` are the content
    /// size in vp; `density` is px-per-vp (both passed by the ArkTS host).
    pub fn start(
        content: *mut c_void,
        w_vp: f64,
        h_vp: f64,
        density: f64,
        root: impl FnOnce() -> crate::AnyPiece + 'static,
    ) {
        day_arkui::init(content, w_vp, h_vp, density);
        day_script::init();
        day_core::launch_with(
            day_arkui::ArkUi::new(),
            crate::WindowOptions::default(),
            root,
        );
    }
}
