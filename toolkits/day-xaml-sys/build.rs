// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Compile the C++/WinRT XAML-Islands shim with `cc` (MSVC) and link the WinRT umbrella
//! library. The Windows SDK ships the cppwinrt projection headers under
//! `Include\<ver>\cppwinrt`, which is not on the default INCLUDE path — we locate the newest
//! one and add it. Everything else (um/shared/ucrt/winrt) comes from `cc`'s MSVC environment.

fn main() {
    // Windows-only shim: on other hosts this crate is an empty stub (see src/lib.rs).
    if std::env::var("CARGO_CFG_WINDOWS").is_err() {
        return;
    }

    let cppwinrt = day_toolchain::cppwinrt_include_for_build_script().expect(
        "Windows 10/11 SDK cppwinrt headers not found. Install the Windows SDK \
         (Visual Studio 'Desktop development with C++'), or point DAY_CPPWINRT / \
         DAY_WINDOWS_KITS_ROOT at a relocated install (docs/environment.md).",
    );

    let mut build = cc::Build::new();
    build
        .cpp(true)
        // C++20 so cppwinrt uses the standard <coroutine> header. Under /std:c++17 newer MSVC
        // STLs (VS 2022 17.x+ / SDK 26100) make cppwinrt's fallback include of
        // <experimental/coroutine> a hard error (STL1011).
        .std("c++20")
        .define("_SILENCE_EXPERIMENTAL_COROUTINE_DEPRECATION_WARNINGS", None)
        // One translation unit. The picker and textarea shims (moved in from their satellite
        // crates in 2026-07) were separate files until they were folded into shim.cpp: `cc`
        // recompiles every source whenever this script re-runs, so the split gave no incremental
        // win — editing the 177-line picker cost the same full rebuild as editing shim.cpp — while
        // each satellite still carried duplicate includes, namespace aliases and `hs`/`u8` copies.
        .file("src/shim.cpp");
    // WinUI 3 (the `windows-winui` target): the same shim compiled against Microsoft.UI.Xaml. The
    // generated Windows App SDK projection goes AHEAD of the SDK's cppwinrt directory (it shares
    // that directory's base.h), and the bootstrapper's path is compiled in as the fallback for a
    // development build that has no copy of it next to the exe.
    if std::env::var("CARGO_FEATURE_WINUI").is_ok() {
        let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
        let sdk = day_toolchain::winappsdk::resolve_for_build_script(&cppwinrt, &arch)
            .unwrap_or_else(|e| panic!("day-xaml-sys (winui): the Windows App SDK: {e}"));
        build.define("DAY_WINUI", None);
        build.define(
            "DAY_WINAPPSDK_MAJOR_MINOR",
            format!("0x{:08X}", day_toolchain::winappsdk::RELEASE_MAJOR_MINOR).as_str(),
        );
        // The oldest runtime accepted, as PACKAGE_VERSION's packed u64 (major.minor.build.rev,
        // 16 bits each). From 2.0 on one framework package serves the whole major version, so
        // without a floor a 2.0 runtime would be bound and fail on the first newer API.
        let min: u64 = day_toolchain::winappsdk::RUNTIME_VERSION
            .split('.')
            .chain(std::iter::repeat("0"))
            .take(4)
            .fold(0, |acc, p| (acc << 16) | p.parse::<u64>().unwrap_or(0));
        build.define(
            "DAY_WINAPPSDK_MIN_VERSION",
            format!("0x{min:016X}ull").as_str(),
        );
        let dll = sdk.bootstrap_dll.to_string_lossy().replace('\\', "\\\\");
        build.define(
            "DAY_WINAPPSDK_BOOTSTRAP_DLL",
            format!("L\"{dll}\"").as_str(),
        );
        build.include(&sdk.projection);
        for dir in &sdk.includes {
            build.include(dir);
        }
        println!("cargo:bootstrap_dll={}", sdk.bootstrap_dll.display());
    }
    build
        .include(&cppwinrt)
        .flag("/EHsc") // C++/WinRT uses exceptions
        .flag("/bigobj") // the XAML cppwinrt headers blow past the default section limit
        .flag_if_supported("/permissive-");
    build.compile("dayxamlshim");

    // WindowsApp.lib is the WinRT umbrella (RoInitialize, activation, XAML Islands).
    println!("cargo:rustc-link-lib=WindowsApp");
    println!("cargo:rustc-link-lib=user32");
    println!("cargo:rustc-link-lib=gdi32");
    println!("cargo:rustc-link-lib=gdiplus"); // window snapshot PNG encoding
    println!("cargo:rustc-link-lib=dwmapi"); // dark title bar opt-in (DwmSetWindowAttribute)
    println!("cargo:rustc-link-lib=dwrite"); // the system font collection (docs/fonts.md)
    // D3D11CreateDevice / CreateDirect3D11DeviceFromDXGIDevice: WinUI's window capture
    // (Windows.Graphics.Capture hands frames back as D3D11 textures).
    println!("cargo:rustc-link-lib=d3d11");
    // OleInitialize — the OLE layer cross-process drag and drop needs (docs/drag-and-drop.md).
    // Not covered by WindowsApp.lib, which carries the UWP surface only.
    println!("cargo:rustc-link-lib=ole32");
    // DragQueryFileW — reading CF_HDROP, the form Explorer offers dropped files in.
    println!("cargo:rustc-link-lib=shell32");
    println!("cargo:rerun-if-changed=src/shim.cpp");
    println!("cargo:rerun-if-changed=src/transfer.inc");
    println!("cargo:rerun-if-changed=src/transfer-host.inc");
    println!("cargo:rerun-if-changed=build.rs");
}
