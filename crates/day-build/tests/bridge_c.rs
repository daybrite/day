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

    #[day_bridge::impl(c, platforms = [{platform}])]
    c!(
        prelude = r#"
        #include <stddef.h>
    "#,
        body = r#"
        static int32_t total = 0;

        int32_t add_native(int32_t a, int32_t b) {{
            total += a + b;
            return 0;
        }}

        void reset_native(void) {{ total = 0; }}
    "#,
    );

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

/// The build-script environment is process-wide, and the tests here run on separate threads:
/// one at a time through the fake environment, or one test's `remove_var` lands mid-way
/// through another's `cc` run.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// `cc` reads cargo's build-script environment; a test is not a build script, so supply it.
fn with_build_env(out: &Path, platform: &str, f: impl FnOnce()) {
    let _serial = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
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
        ("CARGO_CFG_TARGET_ENV", target_env().to_string()),
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

/// The host's `target_env`. `cc` picks the compiler FAMILY from cargo's cfg environment, not from
/// the triple, so leaving this empty on a `windows-msvc` host reads as GNU and sends it looking
/// for `gcc.exe` — which is not what builds this workspace, and is not installed on the runner.
fn target_env() -> &'static str {
    if cfg!(target_env = "msvc") {
        "msvc"
    } else if cfg!(target_env = "gnu") {
        "gnu"
    } else if cfg!(target_env = "musl") {
        "musl"
    } else {
        "" // apple targets report no env
    }
}

fn current_target() -> String {
    // Good enough for `cc`: it only needs a triple it can parse. Built from the same cfgs
    // `target_env` reads, so the triple and the cfg environment can never disagree.
    let sys = if cfg!(target_os = "macos") {
        "apple-darwin".to_string()
    } else if cfg!(target_os = "windows") {
        format!("pc-windows-{}", target_env())
    } else {
        format!(
            "unknown-linux-{}",
            if target_env().is_empty() {
                "gnu"
            } else {
                target_env()
            }
        )
    };
    format!("{}-{sys}", std::env::consts::ARCH)
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

/// An asynchronous arm (docs/bridge.md "Callbacks"): the generated translation unit declares the
/// completion symbol Rust exports and the `<fn>_complete` / `<fn>_fail` helpers the arm calls,
/// and it has to compile as plain C with nothing but the arm's own code.
#[test]
fn c_arm_with_a_completion_compiles() {
    let platform = host_platform();
    let source = format!(
        r###"
day_bridge::bridge! {{
    #[day_bridge::declare]
    extern "day" {{
        fn lookup_native(key: &str, done: day_bridge::Done<String>) -> Result<(), day_bridge::Error>;
        fn ping_native(done: day_bridge::Done<()>) -> Result<(), day_bridge::Error>;
    }}

    #[day_bridge::impl(c, platforms = [{platform}])]
    c!(r#"
        int32_t lookup_native(const char* key, uint64_t done) {{
            if (key[0] == 0) {{
                lookup_native_fail(done);
                return 0;
            }}
            lookup_native_complete(done, key);
            return 0;
        }}

        int32_t ping_native(uint64_t done) {{ ping_native_complete(done); return 0; }}
    "#);

    #[day_bridge::impl(rust, platforms = [other])]
    fn lookup_native(_key: &str, done: day_bridge::Done<String>) -> Result<(), day_bridge::Error> {{
        done.complete(Err(day_bridge::Error::Unsupported));
        Ok(())
    }}

    #[day_bridge::impl(rust, platforms = [other])]
    fn ping_native(done: day_bridge::Done<()>) -> Result<(), day_bridge::Error> {{
        done.complete(Err(day_bridge::Error::Unsupported));
        Ok(())
    }}
}}
"###
    );
    let tmp = std::env::temp_dir().join(format!("day-bridge-c-done-{}", std::process::id()));
    let src = tmp.join("src");
    std::fs::create_dir_all(&src).expect("temp crate");
    std::fs::write(src.join("lib.rs"), &source).expect("write source");
    let out = tmp.join("out");
    std::fs::create_dir_all(&out).expect("out dir");

    with_build_env(&out, platform, || {
        day_build::bridge::generate_in(&tmp, &out, "day-part-async").expect("bridge codegen");
    });

    let c = std::fs::read_to_string(
        out.join("day-bridge")
            .join(format!("day-part-async-{platform}.c")),
    )
    .expect("generated C");
    assert!(
        c.contains("extern void day_bridge_complete_day_part_async_lookup_native(uint64_t done, int32_t status, const char* value);"),
        "{c}"
    );
    assert!(
        c.contains("int32_t day_bridge_day_part_async_lookup_native(const char* key, uint64_t done) { return lookup_native(key, done); }"),
        "{c}"
    );
    let compiled = std::fs::read_dir(&out)
        .expect("out dir")
        .flatten()
        .any(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.starts_with("libday_bridge_day_part_async") || name.ends_with(".lib")
        });
    assert!(compiled, "cc produced no archive in {}", out.display());

    let rust = std::fs::read_to_string(out.join("day-bridge").join("mod.rs")).expect("mod.rs");
    assert!(
        rust.contains("pub extern \"C\" fn day_bridge_complete_day_part_async_lookup_native(done: u64, status: i32, value: *const std::ffi::c_char)"),
        "{rust}"
    );
    assert!(
        rust.contains(
            "pub(crate) fn lookup_native_future(key: &str) -> day_bridge::Completion<String>"
        ),
        "{rust}"
    );
    assert!(
        rust.contains("pub(crate) fn ping_native_async(on_done: impl FnOnce(Result<(), day_bridge::Error>) + Send + 'static) -> Result<u64, day_bridge::Error>"),
        "{rust}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}
