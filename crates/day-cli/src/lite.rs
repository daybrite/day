//! `day lite test` (docs/lite.md §11): run a miniapp's headless tests through day-lite's
//! runner core — the miniapp's own `tests/*.test.ts` against the real `day.*` API (fresh
//! sqlite + fs sandbox, network never granted). Exit code 5 on failure, like dayscript.

use std::path::Path;

use anstream::eprintln;

use crate::term::{BOLD, ERROR, SUCCESS};

pub fn test(path: &str) -> i32 {
    let dir = match std::fs::canonicalize(Path::new(path)) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: {path}: {e}");
            return 5;
        }
    };
    let outcomes = match day_lite::run_tests(&dir) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("error: {e}");
            return 5;
        }
    };
    let mut failed = 0usize;
    let mut module = String::new();
    for o in &outcomes {
        if o.module != module {
            module = o.module.clone();
            eprintln!("  {BOLD}{module}{BOLD:#}");
        }
        if o.passed {
            eprintln!("    {SUCCESS}✓{SUCCESS:#} {}", o.name);
        } else {
            failed += 1;
            eprintln!("    {ERROR}✗{ERROR:#} {} — {}", o.name, o.detail);
        }
    }
    let total = outcomes.len();
    if failed == 0 {
        eprintln!("      {SUCCESS}{total}/{total} tests passed{SUCCESS:#}");
        0
    } else {
        eprintln!("      {ERROR}{failed}/{total} tests failed{ERROR:#}");
        5
    }
}
