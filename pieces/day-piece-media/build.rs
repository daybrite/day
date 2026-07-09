//! Compiles this piece's OWN native shim per feature — a standalone Day Piece carrying native C++
//! without touching Day's toolkit crates (like day-piece-webview). Qt uses `cc` + pkg-config and
//! (unlike day-qt-sys) links Qt6MultimediaWidgets when the host ships it. WinUI uses `cc` (MSVC) +
//! the Windows SDK cppwinrt projection, mirroring day-winui-sys / the webview's WinUI shim.

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
    // NOTE: the iOS AVPlayerViewController (lib-uikit.rs) needs AVKit.framework linked. That's
    // declared in Cargo.toml's `[package.metadata.day.ios].frameworks = ["AVKit", "AVFoundation"]`
    // and linked by the generated DayPieces SwiftPM package — not from this build script (a
    // `cargo:rustc-link-lib` never reaches xcodebuild, which performs the app's final link).
}

fn build_winui() {
    // Same recipe as day-winui-sys / the webview's WinUI shim: the cppwinrt projection headers live
    // under the SDK's Include\<ver>\cppwinrt (not on the default INCLUDE path); C++20 + /bigobj + /EHsc.
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
    build.compile("daymediawinuishim");
    // WindowsApp.lib (WinRT umbrella) + the day_winui_box/unbox seam are already linked by
    // day-winui-sys; nothing extra to link here.
}
fn build_qt() {
    // QMediaPlayer/QVideoWidget live in Qt6Multimedia(Widgets), which not every Qt host ships.
    // When absent the shim degrades to a QtWidgets URL label (see lib-qt-shim.cpp's #else), so
    // build against Qt6Widgets — which day-qt-sys already links — instead of failing the build.
    let has_multimedia = pkg_config_exists("Qt6MultimediaWidgets");
    let cflags_pkg = if has_multimedia {
        "Qt6MultimediaWidgets" // --cflags pull in Qt6Core/Gui/Widgets/Multimedia too
    } else {
        println!(
            "cargo:warning=Qt6MultimediaWidgets not found; day-piece-media degrades to a URL \
             label on qt (no native playback)."
        );
        "Qt6Widgets"
    };
    let cflags = pkg_config(&["--cflags", cflags_pkg]);
    let mut build = cc::Build::new();
    build.cpp(true).std("c++17").file("src/lib-qt-shim.cpp");
    if has_multimedia {
        build.define("DAY_MEDIA_QT_MM", None);
    }
    for tok in cflags.split_whitespace() {
        build.flag(tok);
    }
    build.flag_if_supported("-Wno-unused-parameter");
    build.compile("daymediaqtshim");

    // day-qt-sys already links Qt6Core/Qt6Widgets, but NOT the Multimedia modules — emit those.
    // Duplicates with day-qt-sys's flags are harmless (the linker dedups). The label fallback needs
    // nothing beyond Qt6Widgets (already linked), so emit no extra libs there.
    if has_multimedia {
        let libs = pkg_config(&["--libs", "Qt6MultimediaWidgets"]);
        emit_link_flags(&libs);
    }
}

/// True if pkg-config knows the module (used to pick the real QMediaPlayer shim vs. the label
/// fallback). `--exists` is the standard availability probe; a missing pkg-config counts as absent.
fn pkg_config_exists(pkg: &str) -> bool {
    std::process::Command::new("pkg-config")
        .args(["--exists", pkg])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn pkg_config(args: &[&str]) -> String {
    let out = std::process::Command::new("pkg-config")
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("pkg-config {args:?}: {e}"));
    if !out.status.success() {
        panic!(
            "pkg-config {args:?} failed — is Qt6MultimediaWidgets installed?\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Translate `pkg-config --libs` tokens into Cargo link directives (frameworks on macOS, plain
/// libs/search-paths elsewhere).
fn emit_link_flags(libs: &str) {
    let mut toks = libs.split_whitespace().peekable();
    while let Some(tok) = toks.next() {
        if let Some(path) = tok.strip_prefix("-F") {
            let path = take_value(path, &mut toks);
            println!("cargo:rustc-link-search=framework={path}");
        } else if tok == "-framework" {
            if let Some(fw) = toks.next() {
                println!("cargo:rustc-link-lib=framework={fw}");
            }
        } else if let Some(path) = tok.strip_prefix("-L") {
            let path = take_value(path, &mut toks);
            println!("cargo:rustc-link-search=native={path}");
        } else if let Some(name) = tok.strip_prefix("-l") {
            let name = take_value(name, &mut toks);
            println!("cargo:rustc-link-lib={name}");
        }
        // Ignore other tokens (e.g. -Wl,... rpath hints handled by the Qt toolkit crate).
    }
}

/// Some pkg-config outputs separate the flag from its value (`-F /path`); handle both forms.
fn take_value(inline: &str, toks: &mut std::iter::Peekable<std::str::SplitWhitespace>) -> String {
    if !inline.is_empty() {
        return inline.to_string();
    }
    toks.next().unwrap_or("").to_string()
}
