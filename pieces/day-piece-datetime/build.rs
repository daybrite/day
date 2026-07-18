//! Compiles this piece's OWN native shims when their feature is on — an external Day Piece
//! carrying native C++ without touching Day's toolkit crates (DESIGN.md §15's tier-1+shim,
//! the day-piece-picker recipe). Qt uses `cc` + pkg-config; WinUI uses `cc` (MSVC) + the Windows
//! SDK cppwinrt projection; ArkUI uses the OpenHarmony NDK's clang against the sysroot headers
//! (day-arkui-sys already links the ArkUI libs — this object only ADDS picker-node calls).

use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=src/lib-qt-shim.cpp");
    println!("cargo:rerun-if-changed=src/lib-winui-shim.cpp");
    println!("cargo:rerun-if-changed=src/datetime-arkui.cpp");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=OHOS_NDK_HOME");

    if std::env::var("CARGO_FEATURE_QT").is_ok() {
        build_qt();
    }
    // Windows-only, and only when the app targets WinUI.
    if std::env::var("CARGO_FEATURE_WINUI").is_ok() && std::env::var("CARGO_CFG_WINDOWS").is_ok() {
        build_winui();
    }
    if std::env::var("CARGO_FEATURE_ARKUI").is_ok()
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("ohos")
    {
        build_arkui();
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
    build.compile("daydatetimeqtshim");
    // Qt libs themselves are already linked by day-qt-sys.
}

fn build_winui() {
    // Same recipe as day-winui-sys: the cppwinrt projection headers live under the SDK's
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
        .file("src/lib-winui-shim.cpp")
        .include(&cppwinrt)
        .flag("/EHsc")
        .flag("/bigobj")
        .flag_if_supported("/permissive-");
    build.compile("daydatetimewinuishim");
    // WindowsApp.lib (WinRT umbrella) + the day_winui_box/unbox seam are already linked by
    // day-winui-sys; nothing extra to link here.
}

fn build_arkui() {
    let ndk = std::env::var("OHOS_NDK_HOME")
        .expect("day-piece-datetime: set OHOS_NDK_HOME to the OpenHarmony NDK `native` dir");
    let ndk = PathBuf::from(ndk);
    let target = std::env::var("TARGET").unwrap();
    cc::Build::new()
        .cpp(true)
        .compiler(ndk.join("llvm/bin").join(format!("{target}-clang++")))
        .archiver(ndk.join("llvm/bin/llvm-ar"))
        .flag("-std=c++17")
        .flag("-fPIC")
        .include(ndk.join("sysroot/usr/include"))
        .file("src/datetime-arkui.cpp")
        .compile("daypiecedatetimearkui");
}
