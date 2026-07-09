//! Compiles this tweak's OWN native shims when their feature is on — the bring-your-own-native
//! recipe for tweaks (docs/tweaks.md), mirroring day-piece-picker's build.rs exactly:
//! Qt via `cc` + pkg-config; WinUI via `cc` (MSVC) + the Windows SDK cppwinrt projection;
//! ArkUI via the OpenHarmony NDK's clang (like day-arkui-sys). The toolkits' own libs are already
//! linked by day-qt-sys / day-winui-sys / day-arkui-sys — these objects only ADD calls.

use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=src/ticks-qt.cpp");
    println!("cargo:rerun-if-changed=src/ticks-winui.cpp");
    println!("cargo:rerun-if-changed=src/ticks-arkui.cpp");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=OHOS_NDK_HOME");

    if std::env::var("CARGO_FEATURE_QT").is_ok() {
        build_qt();
    }
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
    build.cpp(true).std("c++17").file("src/ticks-qt.cpp");
    for tok in String::from_utf8_lossy(&cflags.stdout).split_whitespace() {
        build.flag(tok);
    }
    build.flag_if_supported("-Wno-unused-parameter");
    build.compile("daytweaktickqt");
}

fn build_winui() {
    let cppwinrt = find_cppwinrt().expect(
        "Windows 10/11 SDK cppwinrt headers not found. Install the Windows SDK \
         (Visual Studio 'Desktop development with C++').",
    );
    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std("c++20")
        .define("_SILENCE_EXPERIMENTAL_COROUTINE_DEPRECATION_WARNINGS", None)
        .file("src/ticks-winui.cpp")
        .include(&cppwinrt)
        .flag("/EHsc")
        .flag("/bigobj")
        .flag_if_supported("/permissive-");
    build.compile("daytweaktickwinui");
}

fn build_arkui() {
    let ndk = std::env::var("OHOS_NDK_HOME").expect(
        "day-tweak-slider-tickmarks: set OHOS_NDK_HOME to the OpenHarmony NDK `native` dir",
    );
    let ndk = PathBuf::from(ndk);
    let target = std::env::var("TARGET").unwrap();
    cc::Build::new()
        .cpp(true)
        .compiler(ndk.join("llvm/bin").join(format!("{target}-clang++")))
        .archiver(ndk.join("llvm/bin/llvm-ar"))
        .flag("-std=c++17")
        .flag("-fPIC")
        .include(ndk.join("sysroot/usr/include"))
        .file("src/ticks-arkui.cpp")
        .compile("daytweaktickarkui");
}

/// Newest `Windows Kits\10\Include\<ver>\cppwinrt` on the machine (mirrors day-winui-sys).
fn find_cppwinrt() -> Option<PathBuf> {
    let mut bases: Vec<PathBuf> = Vec::new();
    if let Ok(sdk) = std::env::var("WindowsSdkDir") {
        bases.push(PathBuf::from(sdk).join("Include"));
    }
    bases.push(PathBuf::from(
        r"C:\Program Files (x86)\Windows Kits\10\Include",
    ));
    bases.push(PathBuf::from(r"C:\Program Files\Windows Kits\10\Include"));

    let mut found: Vec<PathBuf> = Vec::new();
    for base in bases {
        let Ok(rd) = std::fs::read_dir(&base) else {
            continue;
        };
        for entry in rd.flatten() {
            let cppwinrt = entry.path().join("cppwinrt");
            if cppwinrt.join("winrt").join("base.h").exists() {
                found.push(cppwinrt);
            }
        }
    }
    found.sort();
    found.pop()
}
