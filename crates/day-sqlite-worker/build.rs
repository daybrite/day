// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// Compiles the vendored SQLite amalgamation plus the freestanding libc subset under
// `vendor/shim` (musl string/stdlib/math routines and a self-contained printf), with every
// libc symbol renamed through `vendor/shim/wasm-shim.h` so the objects are hermetic — no host
// libc anywhere. The recipe and the vendored tree come from sqlite-wasm-rs (MIT, see
// vendor/LICENSE); the Rust side of those renamed symbols lives in src/lib.rs.
//
// The same objects build for native hosts too: that is what lets `cargo test` exercise the
// real engine (over the in-memory OPFS fake) without a browser. On wasm the compiler must be
// a wasm-capable clang — `CC_wasm32_unknown_unknown` as usual (docs/web.md).

/// SQLite compile flags tuned for the single-threaded wasm environment: no threads, no
/// dlopen, FTS5 and R*Tree kept in (capability parity with the bundled native builds).
const SQLITE_FLAGS: [&str; 24] = [
    "-DSQLITE_OS_OTHER",
    // The native test build must stay on the musl shim headers: Apple's zone-malloc probe
    // would drag in host <sys/sysctl.h>, whose types clash with them. Harmless on wasm.
    "-DSQLITE_WITHOUT_ZONEMALLOC",
    "-DSQLITE_USE_URI",
    "-DSQLITE_THREADSAFE=0",
    "-DSQLITE_TEMP_STORE=2",
    "-DSQLITE_DEFAULT_CACHE_SIZE=-16384",
    "-DSQLITE_DEFAULT_PAGE_SIZE=8192",
    "-DSQLITE_OMIT_DEPRECATED",
    "-DSQLITE_OMIT_LOAD_EXTENSION",
    "-DSQLITE_OMIT_SHARED_CACHE",
    "-DSQLITE_ENABLE_UNLOCK_NOTIFY",
    "-DSQLITE_ENABLE_API_ARMOR",
    "-DSQLITE_ENABLE_BYTECODE_VTAB",
    "-DSQLITE_ENABLE_DBPAGE_VTAB",
    "-DSQLITE_ENABLE_DBSTAT_VTAB",
    "-DSQLITE_ENABLE_FTS5",
    "-DSQLITE_ENABLE_MATH_FUNCTIONS",
    "-DSQLITE_ENABLE_OFFSET_SQL_FUNC",
    "-DSQLITE_ENABLE_PREUPDATE_HOOK",
    "-DSQLITE_ENABLE_RTREE",
    "-DSQLITE_ENABLE_SESSION",
    "-DSQLITE_ENABLE_STMTVTAB",
    "-DSQLITE_ENABLE_UNKNOWN_SQL_FUNCTION",
    "-DSQLITE_ENABLE_COLUMN_METADATA",
];

/// The musl routines SQLite's amalgamation reaches for (renamed via wasm-shim.h).
const C_SOURCE: [&str; 36] = [
    "string/memchr.c",
    "string/memrchr.c",
    "string/stpcpy.c",
    "string/stpncpy.c",
    "string/strcat.c",
    "string/strchr.c",
    "string/strchrnul.c",
    "string/strcmp.c",
    "string/strcpy.c",
    "string/strcspn.c",
    "string/strlen.c",
    "string/strncat.c",
    "string/strncmp.c",
    "string/strncpy.c",
    "string/strrchr.c",
    "string/strspn.c",
    "stdlib/atoi.c",
    "stdlib/bsearch.c",
    "stdlib/qsort.c",
    "stdlib/qsort_nr.c",
    "stdlib/strtod.c",
    "stdlib/strtol.c",
    "math/__fpclassifyl.c",
    "math/acosh.c",
    "math/asinh.c",
    "math/atanh.c",
    "math/fmodl.c",
    "math/scalbn.c",
    "math/scalbnl.c",
    "math/sqrt.c",
    "math/trunc.c",
    "errno/__errno_location.c",
    "stdio/__toread.c",
    "stdio/__uflow.c",
    "internal/floatscan.c",
    "internal/shgetc.c",
];

fn main() {
    println!("cargo::rerun-if-changed=vendor");

    let mut cc = cc::Build::new();
    cc.warnings(false)
        .flag("-Wno-macro-redefined")
        .include("vendor/shim")
        .include("vendor/shim/musl/arch/generic")
        .include("vendor/shim/musl/include")
        .file("vendor/shim/printf/printf.c")
        .file("vendor/sqlite3/sqlite3.c")
        .files(C_SOURCE.map(|s| format!("vendor/shim/musl/{s}")))
        .flag("-DPRINTF_ALIAS_STANDARD_FUNCTION_NAMES_HARD")
        .flag("-include")
        .flag("vendor/shim/wasm-shim.h");

    for flag in SQLITE_FLAGS {
        cc.flag(flag);
    }

    cc.compile("day_wsqlite3");
}
