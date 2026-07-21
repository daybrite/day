//! Compile the Qt C++ shim and link Qt 6. On macOS Qt ships as frameworks, so pkg-config's
//! `--cflags` feeds `cc` directly and `--libs` translates into framework link directives plus
//! an rpath (pane's proven build recipe).

use std::process::Command;

fn pkg_config(args: &[&str]) -> String {
    let out = Command::new("pkg-config")
        .args(args)
        .output()
        .expect("pkg-config not found — install Qt6 (brew install qt) and pkg-config");
    assert!(
        out.status.success(),
        "pkg-config {:?} failed: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn main() {
    let cflags = pkg_config(&["--cflags", "Qt6Widgets"]);

    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std("c++17")
        .file("src/shim.cpp")
        // Built-in leaf shims moved in from their satellite crates (2026-07).
        .file("src/shim-picker.cpp")
        .file("src/shim-textarea.cpp");
    for tok in cflags.split_whitespace() {
        build.flag(tok);
    }
    build.flag_if_supported("-Wno-unused-parameter");
    build.compile("dayqtshim");

    let libs = pkg_config(&["--libs", "Qt6Widgets"]);
    let toks: Vec<&str> = libs.split_whitespace().collect();
    let mut i = 0;
    let mut framework_dir = String::new();
    while i < toks.len() {
        let t = toks[i];
        if let Some(d) = t.strip_prefix("-F") {
            println!("cargo:rustc-link-search=framework={d}");
            framework_dir = d.to_string();
        } else if t == "-framework" {
            i += 1;
            println!("cargo:rustc-link-lib=framework={}", toks[i]);
        } else if let Some(d) = t.strip_prefix("-L") {
            println!("cargo:rustc-link-search=native={d}");
        } else if let Some(l) = t.strip_prefix("-l") {
            println!("cargo:rustc-link-lib={l}");
        }
        i += 1;
    }
    if !framework_dir.is_empty() {
        println!("cargo:rustc-link-arg=-Wl,-rpath,{framework_dir}");
    }

    // `pkg-config --libs` omits `-L` when the libdir is the host's default (e.g. MSYS2's
    // /mingw64/lib), but a mismatched linker won't search it — emit it explicitly.
    let libdir = pkg_config(&["--variable=libdir", "Qt6Widgets"]);
    let libdir = libdir.trim();
    if !libdir.is_empty() {
        println!("cargo:rustc-link-search=native={libdir}");
    }

    println!("cargo:rerun-if-changed=src/shim.cpp");
    println!("cargo:rerun-if-changed=src/shim-picker.cpp");
    println!("cargo:rerun-if-changed=src/shim-textarea.cpp");
}
