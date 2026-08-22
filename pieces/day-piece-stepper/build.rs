// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Compiles this piece's OWN Qt shim when the feature is on — an external Day Piece carrying
//! native C++ without touching Day's toolkit crates (DESIGN.md §15's tier-1+shim).

fn main() {
    println!("cargo:rerun-if-changed=src/lib-qt-shim.cpp");
    println!("cargo:rerun-if-changed=build.rs");

    if std::env::var("CARGO_FEATURE_QT").is_ok() {
        build_qt();
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
    build.compile("daystepperqtshim");
    // Qt libs themselves are already linked by day-qt-sys.
}
