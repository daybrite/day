// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Discovery must be validated before filtering or executing cases. WASM expectations come
//! from rustc's export arguments BEFORE archive extraction, not from the runtime inventory.

use super::TestFn;
use std::collections::BTreeSet;

#[cfg(target_arch = "wasm32")]
#[doc(hidden)]
pub struct Registration(pub &'static TestFn);

#[cfg(target_arch = "wasm32")]
inventory::collect!(Registration);

#[cfg(target_arch = "wasm32")]
thread_local! {
    static EXPECTED: std::cell::RefCell<Result<Vec<String>, String>> = const {
        std::cell::RefCell::new(Err(String::new()))
    };
}

pub(super) fn compiled_tests() -> Vec<TestFn> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        super::TESTS.to_vec()
    }
    #[cfg(target_arch = "wasm32")]
    {
        inventory::iter::<Registration>
            .into_iter()
            .map(|r| *r.0)
            .collect()
    }
}

/// Install the build's independent expectations and initialize WASM constructors once.
/// Custom hosts must supply the `day_test_expectations` section written by `day build`.
#[cfg(target_arch = "wasm32")]
pub fn install_web_registry(bytes: &[u8]) -> Result<(), String> {
    unsafe extern "C" {
        fn __wasm_call_ctors();
    }
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        // SAFETY: linker's constructors; inventory registers only static descriptors.
        unsafe {
            __wasm_call_ctors();
        }
    });
    let expected = parse_manifest(bytes);
    EXPECTED.with(|slot| *slot.borrow_mut() = expected);
    validate_registry()
}

/// Fail closed before listing or running compiled tests, independent of test filters.
pub fn validate_registry() -> Result<(), String> {
    let tests = compiled_tests();
    #[cfg(target_arch = "wasm32")]
    EXPECTED.with(|slot| {
        let expected = slot.borrow();
        let expected = expected.as_ref().map_err(|e| if e.is_empty() {
            "conformance discovery has no independent build manifest; build with `day build` and initialize the web host".to_owned()
        } else { e.clone() })?;
        compare(expected.iter().map(String::as_str), tests.iter().map(|t| t.ident))
    })?;
    let mut names = BTreeSet::new();
    for test in tests {
        if !names.insert(test.name()) {
            return Err(format!(
                "duplicate conformance test name {}; rename one before running",
                test.name()
            ));
        }
    }
    Ok(())
}

#[cfg(any(target_arch = "wasm32", test))]
fn parse_manifest(bytes: &[u8]) -> Result<Vec<String>, String> {
    let text =
        std::str::from_utf8(bytes).map_err(|e| format!("conformance manifest UTF-8: {e}"))?;
    let rest = text.strip_prefix("day-tests-v1\n").ok_or_else(|| {
        "missing or unsupported conformance build manifest; rebuild with `day build`".to_owned()
    })?;
    let mut names = BTreeSet::new();
    for name in rest.lines() {
        if name.is_empty() || !name.contains("::") || !names.insert(name.to_owned()) {
            return Err(format!(
                "invalid or duplicate conformance declaration {name:?}"
            ));
        }
    }
    Ok(names.into_iter().collect())
}

#[cfg(any(target_arch = "wasm32", test))]
fn compare<'a>(
    expected: impl Iterator<Item = &'a str>,
    actual: impl Iterator<Item = &'a str>,
) -> Result<(), String> {
    let expected: BTreeSet<_> = expected.collect();
    let mut found = BTreeSet::new();
    for name in actual {
        if !found.insert(name) {
            return Err(format!("duplicate conformance registration {name}"));
        }
    }
    let missing: Vec<_> = expected.difference(&found).copied().collect();
    let unexpected: Vec<_> = found.difference(&expected).copied().collect();
    if missing.is_empty() && unexpected.is_empty() {
        return Ok(());
    }
    Err(format!(
        "conformance discovery incomplete: expected {}, discovered {}; missing: [{}]; unexpected: [{}]. No tests executed",
        expected.len(),
        found.len(),
        missing.join(", "),
        unexpected.join(", ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_replaced_duplicate_and_uninitialized_are_errors() {
        assert!(parse_manifest(b"").is_err());
        assert!(parse_manifest(b"day-tests-v1\na::b\na::b\n").is_err());
        assert!(parse_manifest(b"day-tests-v1\n").unwrap().is_empty());
        let expected = ["a::one", "unused_module::two"];
        let error = compare(expected.into_iter(), ["a::one"].into_iter()).unwrap_err();
        assert!(error.contains("unused_module::two"));
        assert!(error.contains("No tests executed"));
        assert!(compare(expected.into_iter(), ["a::one", "other::three"].into_iter()).is_err());
        assert!(compare(expected.into_iter(), ["a::one", "a::one"].into_iter()).is_err());
        assert!(compare(expected.into_iter(), expected.into_iter().rev()).is_ok());
    }
}
