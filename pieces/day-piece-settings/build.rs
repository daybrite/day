// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Two jobs. First, bake the app identity `day build`/`day launch` export (`DAY_APP_ID`,
//! `DAY_APP_VERSION`, `DAY_APP_BUILD`, `DAY_APP_TITLE`, crates/day-cli/src/ops.rs) into this
//! crate, so `About::from_manifest()` knows the app without a runtime manifest read, and a changed
//! value recompiles the crate instead of reusing a stale one. Second, compile the native shims
//! when their feature is on, the day-piece-searchfield recipe: the Qt shim (src/lib-qt-shim.cpp,
//! the KDE-style preferences dialog) with `cc` + pkg-config, and the XAML shim
//! (src/lib-xaml-shim.cpp, the settings-card row) with `cc` (MSVC) + the Windows SDK cppwinrt
//! projection when the app targets XAML.
fn main() {
    for var in [
        "DAY_APP_ID",
        "DAY_APP_VERSION",
        "DAY_APP_BUILD",
        "DAY_APP_TITLE",
    ] {
        println!("cargo:rerun-if-env-changed={var}");
        if let Ok(value) = std::env::var(var) {
            println!("cargo:rustc-env=DAY_SETTINGS_{}={value}", &var[4..]);
        }
    }

    println!("cargo:rerun-if-changed=src/lib-qt-shim.cpp");
    println!("cargo:rerun-if-changed=src/lib-xaml-shim.cpp");
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_FEATURE_QT").is_ok() {
        build_qt();
    }
    // Windows-only, and only when the app targets XAML.
    if std::env::var("CARGO_FEATURE_XAML").is_ok() && std::env::var("CARGO_CFG_WINDOWS").is_ok() {
        build_xaml();
    }
}

fn build_qt() {
    let cflags = std::process::Command::new("pkg-config")
        .args(["--cflags", "Qt6Widgets"])
        .output()
        .expect("pkg-config Qt6Widgets");
    let mut build = cc::Build::new();
    build.cpp(true).std("c++17").file("src/lib-qt-shim.cpp");
    for tok in String::from_utf8_lossy(&cflags.stdout).split_whitespace() {
        build.flag(tok);
    }
    build.flag_if_supported("-Wno-unused-parameter");
    build.compile("daysettingsqtshim");
    // Qt libs themselves are already linked by day-qt-sys.
}

fn build_xaml() {
    // Same recipe as day-xaml-sys: the cppwinrt projection headers live under the SDK's
    // Include\<ver>\cppwinrt (not on the default INCLUDE path); C++20 + /bigobj + /EHsc.
    let cppwinrt = day_toolchain::cppwinrt_include_for_build_script().expect(
        "Windows 10/11 SDK cppwinrt headers not found. Install the Windows SDK \
         (Visual Studio 'Desktop development with C++'), or point DAY_CPPWINRT / \
         DAY_WINDOWS_KITS_ROOT at a relocated install (docs/environment.md).",
    );
    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std("c++20")
        .define("_SILENCE_EXPERIMENTAL_COROUTINE_DEPRECATION_WARNINGS", None)
        .file("src/lib-xaml-shim.cpp")
        .includes(day_toolchain::winappsdk::shim_includes(&cppwinrt))
        .flag("/EHsc")
        .flag("/bigobj")
        .flag_if_supported("/permissive-");
    // WinUI 3 (windows-winui): the same shim against Microsoft.UI.Xaml (docs/winui.md).
    if day_toolchain::winappsdk::shim_is_winui() {
        build.define("DAY_WINUI", None);
    }
    build.compile("daysettingsxamlshim");
    // WindowsApp.lib (WinRT umbrella) + the day_xaml_box/unbox functions are already linked by
    // day-xaml-sys; nothing extra to link here.
}
