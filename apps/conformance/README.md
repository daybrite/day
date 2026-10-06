# Day Conformance

The app `day test` runs Day's own piece tests in. Each `#[day::test]` case in
`crates/day-pieces/src/conformance.rs` is a route here, so a person can open any case on any
toolkit and look at it, and the engine can drive it. See
[docs/testing.md](../../docs/testing.md).

Only the app's own files are committed: `Cargo.toml`, `src/`, `dayscript/` and
`generate.sh`. The scaffold around them (`Day.toml`, `build.rs`, `resource/`, `platform/`) is
`day new app` output that `generate.sh` writes from this checkout's template, so it never
falls behind the template. Run it once after a clone, and again after a template change:

```sh
# From the day checkout, with the checkout's own CLI:
apps/conformance/generate.sh
cargo run -q -p day-cli -- --project apps/conformance test -p macos-appkit
cargo run -q -p day-cli -- --project apps/conformance test -p macos-appkit 'text-field-*'
cargo run -q -p day-cli -- --project apps/conformance test -p ios-uikit --list
```

The same cases run on the mock toolkit under `cargo test -p day-script --test conformance`.
