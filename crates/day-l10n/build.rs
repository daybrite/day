// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Generate the core catalog's symbolic names from `catalog/en.ftl` (docs/localization.md).
//!
//! Code names a core string through what this writes, never through its key as a string literal:
//! `key::day_ok` for the one-shot `t()` the framework's own crates use, and the typed
//! `day::res::str::day_ok()` (expanded in day-fluent from `__core_strings!`) for pieces and apps.
//! So every key must be a Rust identifier, and a key a translation defines that English does not
//! is a build error rather than a string nothing can reach.
//!
//! The catalog is plain Fluent with no attributes and no multi-line values, so a line reader is
//! enough; day-build's full parser would make this crate, low in the graph, build a dependency
//! tree it otherwise never needs.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::Path;

/// One message: its key, its English text, and its `$variables`, each flagged numeric when a
/// message selects on it (`{ $n -> … }`), which is what makes it an `IntoNumberFArg`.
struct Message {
    key: String,
    text: String,
    params: Vec<(String, bool)>,
}

fn messages(src: &str) -> Vec<Message> {
    let mut out: Vec<Message> = Vec::new();
    for line in src.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        if line.starts_with(char::is_whitespace) {
            // A continuation line (a select arm): its variables belong to the message above.
            if let Some(m) = out.last_mut() {
                collect_params(line, &mut m.params);
            }
            continue;
        }
        let Some((key, text)) = line.split_once('=') else {
            continue;
        };
        let mut m = Message {
            key: key.trim().to_string(),
            text: text.trim().to_string(),
            params: Vec::new(),
        };
        collect_params(text, &mut m.params);
        out.push(m);
    }
    out
}

fn collect_params(text: &str, params: &mut Vec<(String, bool)>) {
    let mut rest = text;
    while let Some(i) = rest.find('$') {
        rest = &rest[i + 1..];
        let name: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
            .collect();
        let numeric = rest[name.len()..].trim_start().starts_with("->");
        match params.iter_mut().find(|(n, _)| *n == name) {
            Some(p) => p.1 |= numeric,
            None if !name.is_empty() => params.push((name, numeric)),
            None => {}
        }
    }
}

fn is_ident(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase() || c == '_')
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

fn main() {
    let dir = Path::new("catalog");
    println!("cargo::rerun-if-changed=catalog");
    let read = |file: &str| {
        std::fs::read_to_string(dir.join(file))
            .unwrap_or_else(|e| panic!("day-l10n: reading catalog/{file}: {e}"))
    };
    let english = messages(&read("en.ftl"));
    let mut errors = Vec::new();
    for m in &english {
        if !is_ident(&m.key) {
            errors.push(format!(
                "catalog/en.ftl: key {:?} is not a Rust identifier; name it {:?}",
                m.key,
                m.key.replace('-', "_")
            ));
        }
        for (p, _) in &m.params {
            if !is_ident(p) {
                errors.push(format!(
                    "catalog/en.ftl: {} takes ${p}, which is not a Rust identifier",
                    m.key
                ));
            }
        }
    }
    let known: BTreeSet<&str> = english.iter().map(|m| m.key.as_str()).collect();
    let mut locales: Vec<_> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("day-l10n: listing catalog/: {e}"))
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|f| f.ends_with(".ftl") && f != "en.ftl")
        .collect();
    locales.sort();
    for file in &locales {
        for m in messages(&read(file)) {
            if !known.contains(m.key.as_str()) {
                errors.push(format!(
                    "catalog/{file}: {:?} is not in catalog/en.ftl, so no accessor names it",
                    m.key
                ));
            }
        }
    }
    if !errors.is_empty() {
        panic!(
            "day-l10n: the core catalog is invalid:\n  {}",
            errors.join("\n  ")
        );
    }

    let mut keys = String::new();
    let mut fns = String::new();
    for m in &english {
        let doc = format!("`{}` — `{}`", m.key, m.text);
        let _ = writeln!(
            keys,
            "    /// {doc}\n    pub const {k}: &str = {k:?};",
            k = m.key
        );
        let generics: Vec<String> = (0..m.params.len()).map(|i| format!("M{i}")).collect();
        let args: Vec<String> = m
            .params
            .iter()
            .enumerate()
            .map(|(i, (p, numeric))| {
                let ty = if *numeric {
                    "IntoNumberFArg"
                } else {
                    "IntoFArg"
                };
                format!("{p}: impl $crate::{ty}<M{i}>")
            })
            .collect();
        let mut body = format!("tr($crate::key::{})", m.key);
        for (p, _) in &m.params {
            let _ = write!(body, ".arg({p:?}, {p})");
        }
        let generics = if generics.is_empty() {
            String::new()
        } else {
            format!("<{}>", generics.join(", "))
        };
        let _ = writeln!(
            fns,
            "        /// {doc}\n        pub fn {}{generics}({}) -> LocalizedText {{ {body} }}",
            m.key,
            args.join(", ")
        );
    }
    let code = format!(
        "/// The core catalog's keys, one constant per message, named as the key itself\n\
         /// (`key::day_ok` is `\"day_ok\"`): what the framework's own crates pass to [`t`].\n\
         /// Generated from `catalog/en.ftl` by this crate's build script.\n\
         #[allow(non_upper_case_globals)]\n\
         pub mod key {{\n{keys}}}\n\n\
         /// The typed accessors for the core catalog, one `pub fn` per message with its arguments\n\
         /// in the signature. day-fluent expands this as `res::str`, in a module that has\n\
         /// `LocalizedText` and `tr` in scope.\n\
         #[doc(hidden)]\n\
         #[macro_export]\n\
         macro_rules! __core_strings {{\n    () => {{\n{fns}    }};\n}}\n"
    );
    let out =
        Path::new(&std::env::var("OUT_DIR").expect("cargo sets OUT_DIR")).join("core_strings.rs");
    std::fs::write(&out, code)
        .unwrap_or_else(|e| panic!("day-l10n: writing {}: {e}", out.display()));
}
