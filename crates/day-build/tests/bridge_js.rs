// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The generated web arm has to be a valid ES module, and it has to carry no cargo directives —
//! `parse_crate` runs inside `day build` as well as inside a build script, and a stray `cargo:`
//! line once ended up inside the module itself (docs/bridge.md).

use std::path::Path;

const CRATE: &str = r###"
day_bridge::bridge! {
    #[day_bridge::declare]
    extern "day" {
        fn speak_native(text: &str) -> Result<(), day_bridge::Error>;
        fn stop_native();
    }

    #[day_bridge::impl(js, platforms = [web])]
    js!(r#"
        export function speak_native(text) {
            speechSynthesis.speak(new SpeechSynthesisUtterance(text));
        }

        export function stop_native() { speechSynthesis.cancel(); }
    "#);

    #[day_bridge::impl(rust, platforms = [other])]
    fn speak_native(_text: &str) -> Result<(), day_bridge::Error> {
        Err(day_bridge::Error::Unsupported)
    }

    #[day_bridge::impl(rust, platforms = [other])]
    fn stop_native() {}
}
"###;

#[test]
fn js_arm_is_a_valid_es_module() {
    let tmp = std::env::temp_dir().join(format!("day-bridge-js-{}", std::process::id()));
    std::fs::create_dir_all(tmp.join("src")).unwrap();
    std::fs::write(tmp.join("src/lib.rs"), CRATE).unwrap();

    let bridge = day_build::bridge::parse_crate(&tmp).expect("parse");
    let arm = bridge
        .arms
        .iter()
        .find(|a| a.lang == day_build::bridge::Lang::Js)
        .expect("js arm");
    let js = day_build::bridge::js_adapter(&bridge, arm, "day-part-demo");

    // A `&str` crosses as (ptr, len) and is decoded by the shim's helper — wasm has no C strings.
    assert!(
        js.contains("day_bridge_day_part_demo_speak_native(text_ptr, text_len)"),
        "{js}"
    );
    assert!(js.contains("rt.str(text_ptr, text_len)"), "{js}");
    assert!(js.contains("export function register(rt)"), "{js}");
    assert!(
        !js.contains("cargo:"),
        "no build-script chatter in a module:\n{js}"
    );

    // node parses it as a module, which is the only real check of the generated syntax.
    let module = tmp.join("bridge.mjs");
    std::fs::write(&module, &js).unwrap();
    match std::process::Command::new("node")
        .arg("--check")
        .arg(&module)
        .output()
    {
        Ok(out) => assert!(
            out.status.success(),
            "node rejected the generated module:\n{}\n{js}",
            String::from_utf8_lossy(&out.stderr)
        ),
        // A machine without node still runs every assertion above.
        Err(_) => eprintln!("node not installed; skipped the parse check"),
    }
    let _ = std::fs::remove_dir_all(Path::new(&tmp));
}
