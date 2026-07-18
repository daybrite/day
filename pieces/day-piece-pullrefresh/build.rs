//! Compiles this piece's OWN ArkUI native shim when the `arkui` feature is on — the
//! bring-your-own-native recipe (docs/extending.md), mirroring day-tweak-slider-tickmarks'
//! build.rs: the OpenHarmony NDK's clang against the sysroot headers. day-arkui-sys already links
//! the ArkUI libs; this object only ADDS calls (creating an `ARKUI_NODE_REFRESH` node).

use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=src/refresh-arkui.cpp");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=OHOS_NDK_HOME");

    if std::env::var("CARGO_FEATURE_ARKUI").is_ok()
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("ohos")
    {
        build_arkui();
    }
}

fn build_arkui() {
    let ndk = std::env::var("OHOS_NDK_HOME")
        .expect("day-piece-pullrefresh: set OHOS_NDK_HOME to the OpenHarmony NDK `native` dir");
    let ndk = PathBuf::from(ndk);
    let target = std::env::var("TARGET").unwrap();
    cc::Build::new()
        .cpp(true)
        .compiler(ndk.join("llvm/bin").join(format!("{target}-clang++")))
        .archiver(ndk.join("llvm/bin/llvm-ar"))
        .flag("-std=c++17")
        .flag("-fPIC")
        .include(ndk.join("sysroot/usr/include"))
        .file("src/refresh-arkui.cpp")
        .compile("daypiecepullrefresharkui");
}
