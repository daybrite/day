//! Compiles this piece's OWN native shims per feature — a standalone Day Piece carrying native C++
//! without touching Day's toolkit crates (like day-piece-picker). Qt uses `cc` + pkg-config, and
//! (unlike the picker) links Qt6WebEngineWidgets, which day-qt-sys does NOT link. WinUI uses `cc`
//! (MSVC) + the Windows SDK cppwinrt projection, mirroring day-winui-sys.

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
    // NOTE: the iOS WKWebView (lib-uikit.rs) needs WebKit.framework linked. That's declared in
    // Cargo.toml's `[package.metadata.day.ios].frameworks = ["WebKit"]` and linked by the generated
    // DayPieces SwiftPM package — not from this build script (a `cargo:rustc-link-lib` never reaches
    // xcodebuild, which performs the app's final link).
}

fn build_qt() {
    // QWebEngineView lives in Qt6WebEngineWidgets, which not every Qt host ships: MSYS2/MINGW64
    // (windows-qt) has no Qt6 WebEngine at all (Chromium won't build with MinGW GCC). When it's
    // absent the shim degrades to a QtWidgets URL label (see lib-qt-shim.cpp's #else), so build
    // against Qt6Widgets — which day-qt-sys already links — instead of failing the build.
    let has_webengine = pkg_config_exists("Qt6WebEngineWidgets");
    let cflags_pkg = if has_webengine {
        "Qt6WebEngineWidgets" // --cflags pull in Qt6Core/Gui/Widgets too
    } else {
        println!(
            "cargo:warning=Qt6WebEngineWidgets not found; day-piece-webview degrades to a URL \
             label on qt (no native browser)."
        );
        "Qt6Widgets"
    };
    let cflags = pkg_config(&["--cflags", cflags_pkg]);
    let mut build = cc::Build::new();
    build.cpp(true).std("c++17").file("src/lib-qt-shim.cpp");
    if has_webengine {
        build.define("DAY_WEBVIEW_QT_ENGINE", None);
    }
    for tok in cflags.split_whitespace() {
        build.flag(tok);
    }
    build.flag_if_supported("-Wno-unused-parameter");
    build.compile("daywebviewqtshim");

    // day-qt-sys already links Qt6Core/Qt6Widgets, but NOT the WebEngine modules — emit those.
    // Duplicates with day-qt-sys's flags are harmless (the linker dedups). The label fallback needs
    // nothing beyond Qt6Widgets (already linked), so emit no extra libs there.
    if has_webengine {
        let libs = pkg_config(&["--libs", "Qt6WebEngineWidgets"]);
        emit_link_flags(&libs);
    }
}

fn build_winui() {
    // Same recipe as day-winui-sys / the picker's WinUI shim: the cppwinrt projection headers live
    // under the SDK's Include\<ver>\cppwinrt (not on the default INCLUDE path); C++20 + /bigobj + /EHsc.
    let cppwinrt = day_toolchain::cppwinrt_include_for_build_script().expect(
        "Windows 10/11 SDK cppwinrt headers not found. Install the Windows SDK \
         (Visual Studio 'Desktop development with C++'), or point DAY_CPPWINRT / \
         DAY_WINDOWS_KITS_ROOT at a relocated install (docs/environment.md).",
    );
    // The system-XAML WebView (EdgeHTML) is unsupported in Day's Win32 XAML-Islands host — it renders
    // blank and crashes on navigation. The supported engine is WebView2, hosted as a child window over
    // the XAML content. WebView2.h + the loader ship in the Microsoft.Web.WebView2 NuGet package (NOT
    // the base SDK); we statically link WebView2LoaderStatic.lib so there is no DLL to bundle (the
    // WebView2 Runtime itself is a system-wide install, present on Win11 and the CI runners).
    let webview2 = webview2_sdk_root();
    let arch = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("x86_64") => "x64",
        Ok("aarch64") => "arm64",
        Ok("x86") => "x86",
        other => panic!("day-piece-webview: unsupported WebView2 target arch {other:?}"),
    };
    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std("c++20")
        .define("_SILENCE_EXPERIMENTAL_COROUTINE_DEPRECATION_WARNINGS", None)
        .file("src/lib-winui-shim.cpp")
        .include(&cppwinrt)
        .include(webview2.join("build/native/include"))
        .flag("/EHsc")
        .flag("/bigobj")
        .flag_if_supported("/permissive-");
    build.compile("daywebviewwinuishim");
    // WindowsApp.lib (WinRT umbrella) + the day_winui_box/unbox seam are already linked by
    // day-winui-sys. Add the statically-linked WebView2 loader (pulls in the runtime at first use).
    println!(
        "cargo:rustc-link-search=native={}",
        webview2.join(format!("build/native/{arch}")).display()
    );
    println!("cargo:rustc-link-lib=static=WebView2LoaderStatic");
}

/// Locate the Microsoft.Web.WebView2 SDK (headers + static loader), returning its package root.
/// Resolution order: `DAY_WEBVIEW2_SDK` override → the NuGet global cache → download+extract the
/// pinned `.nupkg` (a zip) into `OUT_DIR`. The runner-friendly middle path avoids any network on
/// dev machines that already have the package restored.
fn webview2_sdk_root() -> std::path::PathBuf {
    use std::path::PathBuf;
    const VERSION: &str = "1.0.3179.45";
    let has_header = |root: &std::path::Path| root.join("build/native/include/WebView2.h").exists();

    println!("cargo:rerun-if-env-changed=DAY_WEBVIEW2_SDK");
    if let Ok(p) = std::env::var("DAY_WEBVIEW2_SDK") {
        let root = PathBuf::from(&p);
        assert!(
            has_header(&root),
            "DAY_WEBVIEW2_SDK={p} has no build/native/include/WebView2.h"
        );
        return root;
    }
    if let Some(home) = std::env::var_os("USERPROFILE") {
        let cached = PathBuf::from(home)
            .join(".nuget/packages/microsoft.web.webview2")
            .join(VERSION);
        if has_header(&cached) {
            return cached;
        }
    }
    // Last resort: fetch the pinned package. `curl`/`tar` ship in-box on Windows 10+ and the CI
    // runners; a .nupkg is an OPC zip that bsdtar extracts. Cache the extraction in OUT_DIR.
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let sdk = out.join(format!("webview2-{VERSION}"));
    if has_header(&sdk) {
        return sdk;
    }
    std::fs::create_dir_all(&sdk).expect("create webview2 sdk dir");
    let nupkg = out.join(format!("webview2-{VERSION}.nupkg"));
    let url = format!("https://www.nuget.org/api/v2/package/Microsoft.Web.WebView2/{VERSION}");
    run("curl", &["-sSL", "-o", nupkg.to_str().unwrap(), &url]);
    run(
        "tar",
        &["-xf", nupkg.to_str().unwrap(), "-C", sdk.to_str().unwrap()],
    );
    assert!(
        has_header(&sdk),
        "failed to obtain WebView2 SDK {VERSION} (set DAY_WEBVIEW2_SDK to a restored package root)"
    );
    sdk
}

/// Run a build helper command, panicking with its output on failure.
fn run(cmd: &str, args: &[&str]) {
    let out = std::process::Command::new(cmd)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("day-piece-webview: spawning `{cmd}`: {e}"));
    assert!(
        out.status.success(),
        "day-piece-webview: `{cmd} {}` failed:\n{}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
}
/// True if pkg-config knows the module (used to pick the real QWebEngineView shim vs. the label
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
            "pkg-config {args:?} failed — is Qt6WebEngineWidgets installed?\n{}",
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
