//! Compile the ArkUI/NAPI C++ shim (src/shim.cpp) with the OpenHarmony NDK's clang and link the
//! ArkUI native runtime libs. Only runs when building for a `*-linux-ohos` target; a no-op elsewhere
//! (so a host `cargo check` of the workspace passes). The NDK path comes from `OHOS_NDK_HOME`
//! (the SDK's `native` directory), which the `day` CLI sets when building the HarmonyOS target.

use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=src/shim.cpp");
    println!("cargo:rerun-if-env-changed=OHOS_NDK_HOME");

    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_env != "ohos" {
        // Not a HarmonyOS build — nothing to compile/link. Keeps host `cargo check` green.
        return;
    }

    let ndk = std::env::var("OHOS_NDK_HOME").unwrap_or_else(|_| {
        panic!(
            "day-arkui-sys: set OHOS_NDK_HOME to the OpenHarmony NDK `native` dir \
             (e.g. .../ohos-sdk/native) to build the HarmonyOS backend"
        )
    });
    let ndk = PathBuf::from(ndk);
    let target = std::env::var("TARGET").unwrap(); // e.g. aarch64-unknown-linux-ohos
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();

    let clangpp = ndk.join("llvm/bin").join(format!("{target}-clang++"));
    let ar = ndk.join("llvm/bin/llvm-ar");
    let include = ndk.join("sysroot/usr/include");

    cc::Build::new()
        .cpp(true)
        .compiler(&clangpp)
        .archiver(&ar)
        .flag("-std=c++17")
        .flag("-fPIC")
        .include(&include)
        .file("src/shim.cpp")
        .compile("day_arkui_shim");

    // ArkUI native (ace_ndk), NAPI (ace_napi), logging (hilog_ndk), and libuv (main-thread posting).
    let lib_arch = match arch.as_str() {
        "aarch64" => "aarch64-linux-ohos",
        "x86_64" => "x86_64-linux-ohos",
        "arm" => "arm-linux-ohos",
        other => panic!("day-arkui-sys: unsupported OHOS arch {other}"),
    };
    let libdir = ndk.join("sysroot/usr/lib").join(lib_arch);
    println!("cargo:rustc-link-search=native={}", libdir.display());
    for lib in ["ace_napi.z", "ace_ndk.z", "hilog_ndk.z", "uv"] {
        println!("cargo:rustc-link-lib=dylib={lib}");
    }
}
