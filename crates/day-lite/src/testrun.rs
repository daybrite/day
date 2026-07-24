//! The `day lite test` core (docs/lite.md §11): run a miniapp's `tests/*.test.ts` headlessly.
//! Tests get the full `day.*` API against a throwaway store (fresh sqlite + fs sandbox per
//! run); the network bridge is installed UNGRANTED, so `day.net.fetch` always rejects —
//! tests never touch the live network. v1 tests are synchronous (an async test is reported
//! as a failure with a clear message rather than silently passing).

use std::path::Path;

use crate::store::Store;
use crate::{Bridge, PermissionSet};

#[derive(Clone, Debug)]
pub struct TestOutcome {
    pub module: String,
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

/// Additional JS installed for test modules: `test()` registration + `expect()` matchers.
const TEST_BOOTSTRAP: &str = r#"
globalThis.__day_tests = [];
globalThis.test = (name, fn) => { __day_tests.push({ name, fn }); };
globalThis.beforeEach = (fn) => { __day_tests.beforeEach = fn; };
globalThis.__day_run = () => {
  const out = [];
  for (const t of __day_tests) {
    try {
      if (__day_tests.beforeEach) __day_tests.beforeEach();
      const r = t.fn();
      if (r && typeof r.then === "function") {
        out.push({ name: t.name,
                   error: "async tests are not supported yet - keep tests synchronous" });
        continue;
      }
      out.push({ name: t.name, error: null });
    } catch (e) {
      out.push({ name: t.name, error: String(e) });
    }
  }
  return out;
};
class ExpectError extends Error {}
const fmt = (v) => { try { return JSON.stringify(v); } catch (_) { return String(v); } };
globalThis.expect = (actual) => ({
  toBe: (want) => { if (actual !== want)
    throw new ExpectError(`expected ${fmt(want)}, got ${fmt(actual)}`); },
  toEqual: (want) => { if (fmt(actual) !== fmt(want))
    throw new ExpectError(`expected ${fmt(want)}, got ${fmt(actual)}`); },
  toContain: (want) => { if (!actual || !actual.includes(want))
    throw new ExpectError(`expected ${fmt(actual)} to contain ${fmt(want)}`); },
  toBeTruthy: () => { if (!actual)
    throw new ExpectError(`expected truthy, got ${fmt(actual)}`); },
  toThrow: () => {
    let threw = false;
    try { actual(); } catch (_) { threw = true; }
    if (!threw) throw new ExpectError("expected the function to throw");
  },
});
"#;

/// Install `dir` (a miniapp working tree) into a throwaway store and run every
/// `tests/*.test.ts` / `.test.js` module.
pub fn run_tests(dir: &Path) -> Result<Vec<TestOutcome>, String> {
    let tests_dir = dir.join("tests");
    let mut modules: Vec<String> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&tests_dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.ends_with(".test.ts") || name.ends_with(".test.js") {
                modules.push(format!("tests/{name}"));
            }
        }
    }
    modules.sort();
    if modules.is_empty() {
        return Err(format!("no tests/*.test.ts under {}", dir.display()));
    }

    // A fresh store per run: sqlite + fs state never leaks between runs.
    let tmp = std::env::temp_dir().join(format!("day-lite-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let store = Store::at(&tmp);
    let plan = store
        .install(&dir.display().to_string())
        .map_err(|e| e.to_string())?;
    let declared: Vec<String> = plan
        .manifest
        .req_permissions
        .iter()
        .map(|p| p.name.clone())
        .collect();
    let app_id = plan.manifest.app_id.clone();
    plan.confirm(&declared).map_err(|e| e.to_string())?;

    // Grant everything declared EXCEPT network (never live in tests) + the storage set.
    let mut granted: Vec<String> = declared
        .into_iter()
        .filter(|p| p != crate::permission::NETWORK)
        .collect();
    for p in [
        crate::permission::STORAGE,
        crate::permission::PREFS,
        crate::permission::FS,
    ] {
        if !granted.iter().any(|g| g == p) {
            granted.push(p.to_string());
        }
    }
    let bridges: Vec<Bridge> = vec![crate::bridges::net()];

    let app = crate::LiteApp::boot(store, &app_id, &bridges, PermissionSet::new(granted))?;
    let raw = app.run_test_modules(&modules, TEST_BOOTSTRAP);
    drop(app);
    let _ = std::fs::remove_dir_all(&tmp);
    Ok(raw?
        .into_iter()
        .map(|(module, name, error)| TestOutcome {
            module,
            name,
            passed: error.is_none(),
            detail: error.unwrap_or_default(),
        })
        .collect())
}
