// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Compiles this tweak's OWN native shims when their feature is on — the bring-your-own-native
//! recipe for tweaks (docs/tweaks.md), mirroring day-piece-picker's build.rs exactly:
//! Qt via `cc` + pkg-config; XAML via `cc` (MSVC) + the Windows SDK cppwinrt projection. The
//! toolkits' own libs are already linked by day-qt-sys / day-xaml-sys — these objects only ADD
//! calls. ArkUI needs no shim: lib.rs sets the slider's step attributes through day-arkui's
//! `node` module.

fn main() {
    println!("cargo:rerun-if-changed=src/ticks-qt.cpp");
    println!("cargo:rerun-if-changed=src/ticks-xaml.cpp");
    println!("cargo:rerun-if-changed=build.rs");

    if std::env::var("CARGO_FEATURE_QT").is_ok() {
        build_qt();
    }
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
    build.cpp(true).std("c++17").file("src/ticks-qt.cpp");
    for tok in String::from_utf8_lossy(&cflags.stdout).split_whitespace() {
        build.flag(tok);
    }
    build.flag_if_supported("-Wno-unused-parameter");
    build.compile("daytweaktickqt");
}

fn build_xaml() {
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
        .file("src/ticks-xaml.cpp")
        .include(&cppwinrt)
        .flag("/EHsc")
        .flag("/bigobj")
        .flag_if_supported("/permissive-");
    build.compile("daytweaktickxaml");
}
