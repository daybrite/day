//! day — the umbrella crate apps depend on. One backend feature per binary (§3.2).

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

pub use day_core::{AnyPiece, BuildCx, Piece, PieceSeq};
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

/// Android glue (§17.4): the app cdylib's JNI exports forward here.
#[cfg(all(feature = "widget", target_os = "android"))]
pub mod android {
    pub use day_android::jni;
    pub use day_android::{dispatch_event, run_posted};

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
