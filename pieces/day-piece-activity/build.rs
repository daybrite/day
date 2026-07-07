//! Compiles this piece's OWN native shim per feature — a standalone Day Piece carrying native C++
//! with zero edits to Day's toolkit crates (like day-piece-media). The Qt shim needs only
//! Qt6Widgets (already linked by day-qt-sys), so we pull its --cflags and emit no extra link flags.
//! The WinUI shim uses `cc` (MSVC) + the Windows SDK cppwinrt projection, mirroring day-winui-sys.

use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=src/lib-qt-shim.cpp");
    println!("cargo:rerun-if-changed=src/lib-winui-shim.cpp");
    println!("cargo:rerun-if-changed=build.rs");

    if std::env::var("CARGO_FEATURE_QT").is_ok() {
        build_qt();
    }
    // Windows-only, and only when the app targets WinUI.
    if std::env::var("CARGO_FEATURE_WINUI").is_ok() && std::env::var("CARGO_CFG_WINDOWS").is_ok() {
        build_winui();
    }
}

fn build_qt() {
    // QProgressBar lives in Qt6Widgets, which day-qt-sys already links — so we only need its include
    // flags to compile, and emit NO extra link directives (the linker already has Qt6Widgets).
    let cflags = pkg_config(&["--cflags", "Qt6Widgets"]);
    let mut build = cc::Build::new();
    build.cpp(true).std("c++17").file("src/lib-qt-shim.cpp");
    for tok in cflags.split_whitespace() {
        build.flag(tok);
    }
    build.flag_if_supported("-Wno-unused-parameter");
    build.compile("dayactivityqtshim");
}

fn pkg_config(args: &[&str]) -> String {
    let out = std::process::Command::new("pkg-config")
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("pkg-config {args:?}: {e}"));
    if !out.status.success() {
        panic!(
            "pkg-config {args:?} failed — is Qt6Widgets installed?\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn build_winui() {
    // Same recipe as day-winui-sys / the media WinUI shim: the cppwinrt projection headers live
    // under the SDK's Include\<ver>\cppwinrt (not on the default INCLUDE path); C++20 + /bigobj + /EHsc.
    let cppwinrt = find_cppwinrt().expect(
        "Windows 10/11 SDK cppwinrt headers not found. Install the Windows SDK \
         (Visual Studio 'Desktop development with C++').",
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
    build.compile("dayactivitywinuishim");
    // WindowsApp.lib (WinRT umbrella) + the day_winui_box/unbox seam are already linked by
    // day-winui-sys; nothing extra to link here.
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
