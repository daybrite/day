// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! End-to-end check of daybridge's C arm (docs/bridge.md): a crate source in, a compiled object
//! and generated Rust out.
//!
//! The unit tests in `bridge.rs` cover parsing and rendering; this one runs the part that needs a
//! real toolchain — `cc` compiling the generated translation unit — by claiming whichever platform
//! the test host happens to be, so it exercises the same path on every CI runner.

use std::path::Path;

/// The `platforms = [ … ]` name for the host running this test.
fn host_platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    }
}

fn crate_source(platform: &str) -> String {
    format!(
        r###"
day_bridge::bridge! {{
    #[day_bridge::declare]
    extern "day" {{
        fn add_native(a: i32, b: i32) -> Result<(), day_bridge::Error>;
        fn reset_native();
    }}

    #[day_bridge::prelude(c)]
    c!(r#"
        #include <stddef.h>
    "#);

    #[day_bridge::impl(c, platforms = [{platform}])]
    c!(r#"
        static int32_t total = 0;

        int32_t add_native(int32_t a, int32_t b) {{
            total += a + b;
            return 0;
        }}

        void reset_native(void) {{ total = 0; }}
    "#);

    #[day_bridge::impl(rust, platforms = [other])]
    fn add_native(_a: i32, _b: i32) -> Result<(), day_bridge::Error> {{
        Err(day_bridge::Error::Unsupported)
    }}

    #[day_bridge::impl(rust, platforms = [other])]
    fn reset_native() {{}}
}}
"###
    )
}

/// `cc` reads cargo's build-script environment; a test is not a build script, so supply it.
fn with_build_env(out: &Path, platform: &str, f: impl FnOnce()) {
    let vars = [
        ("OUT_DIR", out.display().to_string()),
        (
            "TARGET",
            std::env::var("TARGET").unwrap_or_else(|_| current_target()),
        ),
        ("HOST", current_target()),
        ("OPT_LEVEL", "0".into()),
        ("DEBUG", "false".into()),
        ("CARGO_CFG_TARGET_OS", platform.to_string()),
        ("CARGO_CFG_TARGET_ENV", String::new()),
        ("CARGO_CFG_TARGET_ARCH", std::env::consts::ARCH.to_string()),
    ];
    for (k, v) in &vars {
        unsafe { std::env::set_var(k, v) };
    }
    f();
    for (k, _) in &vars {
        unsafe { std::env::remove_var(k) };
    }
}

fn current_target() -> String {
    // Good enough for `cc`: it only needs a triple it can parse.
    format!(
        "{}-{}",
        std::env::consts::ARCH,
        if cfg!(target_os = "macos") {
            "apple-darwin"
        } else if cfg!(target_os = "windows") {
            "pc-windows-msvc"
        } else {
            "unknown-linux-gnu"
        }
    )
}

#[test]
fn c_arm_generates_and_compiles() {
    let tmp = std::env::temp_dir().join(format!("day-bridge-c-{}", std::process::id()));
    let src = tmp.join("src");
    std::fs::create_dir_all(&src).expect("temp crate");
    std::fs::write(src.join("lib.rs"), crate_source(host_platform())).expect("write source");
    let out = tmp.join("out");
    std::fs::create_dir_all(&out).expect("out dir");

    with_build_env(&out, host_platform(), || {
        day_build::bridge::generate_in(&tmp, &out, "day-part-demo").expect("bridge codegen");
    });

    // The generated translation unit carries the arm, the prelude, and a #line back to the source.
    let c_path = out
        .join("day-bridge")
        .join(format!("day-part-demo-{}.c", host_platform()));
    let c = std::fs::read_to_string(&c_path).expect("generated C");
    assert!(
        c.contains("#include <stddef.h>"),
        "prelude is hoisted:\n{c}"
    );
    assert!(
        c.contains("int32_t add_native(int32_t a, int32_t b)"),
        "{c}"
    );
    // The #line must name the source line the arm's first line of C actually sits on, so a
    // compiler diagnostic lands on code the author wrote. Compute it rather than hard-code it.
    let source = crate_source(host_platform());
    let want = source
        .lines()
        .position(|l| l.trim_start().starts_with("static int32_t total"))
        .expect("the arm's first line")
        + 1;
    assert!(
        c.contains(&format!("#line {want} \"src/lib.rs\"")),
        "expected #line {want}; a compile error must point back at the .rs:\n{c}"
    );
    // The adapter carries the prefixed symbol; the arm itself never mentions it.
    assert!(
        c.contains("int32_t day_bridge_day_part_demo_add_native(int32_t a, int32_t b) { return add_native(a, b); }"),
        "{c}"
    );
    assert!(
        c.contains("void day_bridge_day_part_demo_reset_native(void) { reset_native(); }"),
        "{c}"
    );

    // cc compiled it: a static library exists for the linker to consume.
    let lib = std::fs::read_dir(&out)
        .expect("out dir")
        .flatten()
        .any(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.starts_with("libday_bridge_day_part_demo") || name.ends_with(".lib")
        });
    assert!(lib, "cc produced no archive in {}", out.display());

    // And the Rust side declares the same symbols, cfg-gated to this platform.
    let rust = std::fs::read_to_string(out.join("day-bridge").join("mod.rs")).expect("mod.rs");
    assert!(
        rust.contains("fn day_bridge_day_part_demo_add_native(a: i32, b: i32) -> i32;"),
        "{rust}"
    );
    assert!(
        rust.contains("fn add_native(a: i32, b: i32) -> Result<(), day_bridge::Error>"),
        "{rust}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}
