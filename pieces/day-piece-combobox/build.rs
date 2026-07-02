//! Compiles this piece's OWN Qt shim when the `qt` feature is on — an external Day Piece
//! carrying native C++ with zero edits to day's toolkit crates (DESIGN.md §15's tier-1+shim).

fn main() {
    println!("cargo:rerun-if-changed=src/qt_shim.cpp");
    if std::env::var("CARGO_FEATURE_QT").is_err() {
        return;
    }
    let cflags = std::process::Command::new("pkg-config")
        .args(["--cflags", "Qt6Widgets"])
        .output()
        .expect("pkg-config Qt6Widgets");
    let mut build = cc::Build::new();
    build.cpp(true).std("c++17").file("src/qt_shim.cpp");
    for tok in String::from_utf8_lossy(&cflags.stdout).split_whitespace() {
        build.flag(tok);
    }
    build.flag_if_supported("-Wno-unused-parameter");
    build.compile("daycomboqtshim");
    // Qt libs themselves are already linked by day-qt-sys.
}
