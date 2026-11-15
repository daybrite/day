// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The per-toolkit glue-module declaration for a piece crate (docs/extending.md §2). The
//! renderer registration macros are day-spec's (`day_spec::renderer!`), beside the `Renderer`
//! type they fill in.

/// Declare a satellite piece's per-toolkit glue modules: the `#[cfg]`/`#[path]` block every
/// piece otherwise hand-writes (docs/extending.md §2). Each named toolkit expands to the
/// house-convention module gate binding `lib-<toolkit>.rs` next to the invoking lib.rs:
///
/// ```ignore
/// day_pieces::glue_modules!(appkit, gtk, qt, uikit, material, xaml);
/// day_pieces::glue_modules!(uikit, material, arkui);   // a piece with partial coverage
/// ```
///
/// Adding a toolkit to Day means one new arm here instead of an edit in every piece.
#[macro_export]
macro_rules! glue_modules {
    ($($tk:ident),+ $(,)?) => { $($crate::__glue_module!($tk);)+ };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __glue_module {
    (appkit) => {
        #[cfg(all(feature = "appkit", target_os = "macos"))]
        #[path = "lib-appkit.rs"]
        mod appkit_impl;
    };
    (gtk) => {
        #[cfg(feature = "gtk")]
        #[path = "lib-gtk.rs"]
        mod gtk_impl;
    };
    (qt) => {
        #[cfg(feature = "qt")]
        #[path = "lib-qt.rs"]
        mod qt_impl;
    };
    (uikit) => {
        #[cfg(all(feature = "uikit", target_os = "ios"))]
        #[path = "lib-uikit.rs"]
        mod uikit_impl;
    };
    (material) => {
        #[cfg(all(feature = "material", target_os = "android"))]
        #[path = "lib-android.rs"]
        mod android_impl;
    };
    (xaml) => {
        #[cfg(all(feature = "xaml", windows))]
        #[path = "lib-xaml.rs"]
        mod xaml_impl;
    };
    (arkui) => {
        #[cfg(all(feature = "arkui", target_env = "ohos"))]
        #[path = "lib-arkui.rs"]
        mod arkui_impl;
    };
    // `pub(crate)` unlike its siblings: web-dom registers at runtime, so the piece's own
    // constructor calls `dom_impl::register()` (see `dom_renderer!`), so the module has a caller.
    (dom) => {
        #[cfg(all(feature = "dom", target_arch = "wasm32"))]
        #[path = "lib-dom.rs"]
        pub(crate) mod dom_impl;
    };
}
