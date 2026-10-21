// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! WASM conformance discovery. The CLI acts as rustc's final linker driver: capture the
//! compiler's required exports BEFORE linking, delegate unchanged to rust-lld, then append
//! those expectations to the module. No archive/source scanning, target-cfg approximation,
//! second compilation, or handwritten test list. LTO uses the same export contract.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

const PREFIX: &str = "__day_test_v1__";
const SECTION: &str = "day_test_expectations";
const LINKER_ENV: &str = "DAY_WEB_TEST_LINKER";

/// Called before normal CLI parsing. Only cargo's final WASM link enters this path.
pub fn linker_entry() -> Option<i32> {
    let linker = std::env::var_os(LINKER_ENV)?;
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let expanded = match expand_responses(&args) {
        Ok(expanded) => expanded,
        Err(error) => {
            eprintln!("day: {error}");
            return Some(1);
        }
    };
    if expanded.first().is_none_or(|s| s != "-flavor")
        || expanded.get(1).is_none_or(|s| s != "wasm")
    {
        return None;
    }
    Some(match link(&linker, &args, &expanded) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("day: {error}");
            1
        }
    })
}

fn link(linker: &std::ffi::OsStr, args: &[OsString], expanded: &[String]) -> Result<(), String> {
    let (names, output) = expectations(expanded)?;
    let providers: Vec<String> = serde_json::from_str(
        &std::env::var("DAY_WEB_TEST_PROVIDERS").unwrap_or_else(|_| "[]".into()),
    )
    .map_err(|e| format!("conformance provider configuration: {e}"))?;
    check_providers(&names, &providers)?;
    // Keep response files and every original argument unchanged for the actual linker.
    let status = Command::new(linker)
        .args(args)
        .status()
        .map_err(|e| format!("rust-lld: {e}"))?;
    if !status.success() {
        return Err(format!("WASM link failed: {status}"));
    }
    let mut bytes = std::fs::read(&output).map_err(|e| format!("{}: {e}", output.display()))?;
    if !bytes.starts_with(b"\0asm\x01\0\0\0") {
        return Err("linker did not produce a WASM module".into());
    }
    append_manifest(&mut bytes, &names);
    std::fs::write(&output, bytes).map_err(|e| format!("{}: {e}", output.display()))
}

fn expand_responses(args: &[OsString]) -> Result<Vec<String>, String> {
    let mut expanded = Vec::new();
    for arg in args {
        let arg = arg.to_str().ok_or("non-UTF-8 WASM linker argument")?;
        if let Some(path) = arg.strip_prefix('@') {
            let text = std::fs::read_to_string(path)
                .map_err(|e| format!("linker response file {path}: {e}"))?;
            // rustc's LLD response format quotes arguments with GNU shell escaping.
            expanded
                .extend(shlex::split(&text).ok_or("invalid WASM linker response file quoting")?);
        } else {
            expanded.push(arg.to_owned());
        }
    }
    Ok(expanded)
}

fn expectations(args: &[String]) -> Result<(BTreeSet<String>, PathBuf), String> {
    let mut names = BTreeSet::new();
    let mut output = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let export = if arg == "--export" {
            args.next().map(String::as_str)
        } else {
            arg.strip_prefix("--export=")
        };
        if let Some(name) = export.and_then(|s| s.strip_prefix(PREFIX)) {
            names.insert(name.to_owned());
        }
        if arg == "-o" || arg == "--output" {
            output = args.next().map(PathBuf::from);
        } else if let Some(path) = arg.strip_prefix("--output=") {
            output = Some(path.into());
        }
    }
    Ok((
        names,
        output.ok_or("WASM linker invocation has no output path")?,
    ))
}

fn check_providers(names: &BTreeSet<String>, providers: &[String]) -> Result<(), String> {
    for provider in providers {
        if !names
            .iter()
            .any(|n| n.split_once('@').is_some_and(|(p, _)| p == provider))
        {
            return Err(format!(
                "required conformance provider {provider} declares no tests in this build; enable its test feature and link it (use {0} as _)",
                provider.replace('-', "_")
            ));
        }
    }
    Ok(())
}

fn leb(mut n: usize, bytes: &mut Vec<u8>) {
    loop {
        let low = (n & 127) as u8;
        n >>= 7;
        bytes.push(low | if n == 0 { 0 } else { 128 });
        if n == 0 {
            break;
        }
    }
}

fn append_manifest(wasm: &mut Vec<u8>, names: &BTreeSet<String>) {
    let mut payload = Vec::new();
    leb(SECTION.len(), &mut payload);
    payload.extend_from_slice(SECTION.as_bytes());
    payload.extend_from_slice(b"day-tests-v1\n");
    for name in names {
        payload.extend_from_slice(name.as_bytes());
        payload.push(b'\n');
    }
    wasm.push(0); // custom section, independent of the registration objects
    leb(payload.len(), wasm);
    wasm.extend(payload);
}

/// Configure only the final application's link, not dependency builds or build scripts.
pub fn configure(cmd: &mut Command, root: &Path) -> Result<(), String> {
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let query = |args: &[&str]| -> Result<String, String> {
        let out = Command::new(&rustc)
            .args(args)
            .output()
            .map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).into_owned());
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    };
    let sysroot = query(&["--print", "sysroot"])?;
    let version = query(&["-vV"])?;
    let host = version
        .lines()
        .find_map(|l| l.strip_prefix("host: "))
        .ok_or("rustc did not report its host")?;
    let linker = Path::new(&sysroot)
        .join("lib/rustlib")
        .join(host)
        .join("bin")
        .join(if cfg!(windows) {
            "rust-lld.exe"
        } else {
            "rust-lld"
        });
    if !linker.is_file() {
        return Err(format!("WASM linker missing: {}", linker.display()));
    }
    let manifest: toml::Value = toml::from_str(
        &std::fs::read_to_string(root.join("Cargo.toml")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("Cargo.toml: {e}"))?;
    let providers = manifest
        .get("package")
        .and_then(|v| v.get("metadata"))
        .and_then(|v| v.get("day"))
        .and_then(|v| v.get("conformance"))
        .and_then(|v| v.get("required-providers"));
    let providers: Vec<String> = providers
        .cloned()
        .map(|v| v.try_into())
        .transpose()
        .map_err(|e| format!("required-providers: {e}"))?
        .unwrap_or_default();
    // Cargo does not fingerprint arbitrary linker environment variables. Put the provider
    // contract in rustc's arguments too, so changing only that contract forces a fresh link.
    use sha2::Digest;
    let providers_json = serde_json::to_string(&providers).map_err(|e| e.to_string())?;
    let contract_hash: String = sha2::Sha256::digest(providers_json.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    cmd.arg(format!("-Cmetadata=day-web-tests-v1-{contract_hash}"));
    cmd.env(LINKER_ENV, linker)
        .env("DAY_WEB_TEST_PROVIDERS", providers_json)
        .arg(format!(
            "-Clinker={}",
            std::env::current_exe()
                .map_err(|e| e.to_string())?
                .display()
        ))
        .arg("-Clinker-flavor=wasm-ld");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_files_keep_quoted_paths_and_export_names() {
        let path = std::env::temp_dir().join(format!("day-wasm-link-{}.rsp", std::process::id()));
        std::fs::write(
            &path,
            r#"-flavor
wasm
--export="__day_test_v1__widgets@1::hidden::case"
-o
"C:\\Program Files\\day\\app.wasm"
"#,
        )
        .unwrap();
        let expanded = expand_responses(&[format!("@{}", path.display()).into()]);
        std::fs::remove_file(path).unwrap();
        let (names, output) = expectations(&expanded.unwrap()).unwrap();
        assert_eq!(output, Path::new(r"C:\Program Files\day\app.wasm"));
        assert!(names.contains("widgets@1::hidden::case"));
    }

    #[test]
    fn expectations_precede_linking_and_provider_loss_is_fatal() {
        let args = [
            "--export",
            "__day_test_v1__widgets@1::hidden::case",
            "--export=__day_test_v1__app@1::root::case",
            "--export",
            "day_dom_main",
            "-o",
            "app.wasm",
        ]
        .map(str::to_owned);
        let (names, output) = expectations(&args).unwrap();
        assert_eq!(names.len(), 2);
        assert_eq!(output, Path::new("app.wasm"));
        assert!(check_providers(&names, &["widgets".into()]).is_ok());
        assert!(
            check_providers(&names, &["missing".into()])
                .unwrap_err()
                .contains("missing")
        );
        let mut wasm = b"\0asm\x01\0\0\0".to_vec();
        append_manifest(&mut wasm, &names);
        assert!(wasm.windows(SECTION.len()).any(|w| w == SECTION.as_bytes()));
        assert!(wasm.ends_with(b"widgets@1::hidden::case\n"));
    }
}
