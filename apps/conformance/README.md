# Day Conformance

The app `day test` runs Day's own piece tests in. Each `#[day::test]` case in
`crates/day-pieces/src/conformance.rs` is a route here, so a person can open any case on any
toolkit and look at it, and the engine can drive it. See
[docs/testing.md](../../docs/testing.md).

Only the app's own files are committed: `Cargo.toml`, `src/`, `dayscript/`, `scripts/` and
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

The separate ArkUI long-press fixture exercises real OS touches, including gesture
arbitration and disposal. Build the app, connect one unlocked HarmonyOS emulator/device,
and run (with `hdc` on `PATH`):

```sh
target/debug/day --project apps/conformance build -t harmony-arkui
python3 apps/conformance/scripts/harmony-long-press.py
```

See [native-input tests](../../docs/testing.md#native-arkui-long-press-tests) for scope and
limitations. The fixture is opt-in and does not replace the regular conformance root.


## Generated native projects

Use a Day CLI with automatic native project generation; Apple builds require **Xcode 27
or newer**. Xcode project documents, Android Gradle files, and HarmonyOS hvigor files are
created under `build/` by `day prepare`, `day build`, and `day launch`. Open the selected
project with `day open -t <target>`; edits to generated files are disposable. Apple Swift
sources, plists and `.xcconfig` files, and any Android resource overlays, remain app-owned
under `platform/`. For additional native configuration, use
`day project add-target <target> --persist-native-project` to create a persistent override.
