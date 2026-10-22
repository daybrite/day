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

The built-in cases also run on the mock toolkit under `cargo test -p day-script --test conformance`.

The app additionally registers `vector-*` pixel cases in `src/vector_icons.rs`, using the SVG
fixtures in `crates/day-vector/fixtures/conformance/`. `generate.sh` copies these into the app's
generated resources, so they exercise the same staging and loading paths as application icons.
Run them with:

```sh
target/debug/day test --project apps/conformance -t windows-winui 'vector-*' --shots always
cargo test -p day-vector --test conformance_fixtures
```

These check tint, strokes and rounded caps, even-odd versus winding fills, three display sizes,
and a native button's icon slot (the WinUI `PathIcon` path used by navigation and toolbars).
The fine-detail case rejects enlargement of the 256px raster cache; its unit-test negative
control proves that the cached image cannot meet the pixel tolerance. It is a rendering
regression check, not a universal detector of rasterization at arbitrary resolutions.

CI runs these cases on all twelve active platform-toolkit targets, with no platform skip list.
They require screenshot support; the mock cannot verify pixels. GTK and Qt retain SVG
geometry and render at the requested size. Additional cases exercise resizing an existing
view, partial-opacity and stroke tint changes, non-square fit/fill/stretch, source replacement between
SVG and bitmap, and native button foreground updates. Every target uses the same assertions;
a local pass on one target does not establish the others' results.

On Linux, run the GTK cases under X11/Xvfb as CI does as well as the desktop session.
Client-side decoration can change the allocated content size on X11; the fine-detail
samples also catch disagreement between that allocation and Day's snapshot coordinates.

The separate ArkUI long-press fixture exercises real OS touches, including gesture
arbitration and disposal. Build the app, connect one unlocked HarmonyOS emulator/device,
and run (with `hdc` on `PATH`):

```sh
target/debug/day --project apps/conformance build -t harmony-arkui
python3 apps/conformance/scripts/harmony-long-press.py
```

See [native-input tests](../../docs/testing.md#native-arkui-long-press-tests) for scope and
limitations. The fixture is opt-in and does not replace the regular conformance root.

Native alerts, prompts, and action sheets have a separate real-input suite using the same
build and device setup:

```sh
python3 apps/conformance/scripts/harmony-dialogs.py
```

It checks native choices/text editing, Back/outside dismissal, sheet placement, queuing,
and programmatic close races. See [dialog tests](../../docs/testing.md#native-arkui-dialog-tests).


## Generated native projects

Use a Day CLI with automatic native project generation; Apple builds require **Xcode 27
or newer**. Xcode project documents, Android Gradle files, and HarmonyOS hvigor files are
created under `build/` by `day prepare`, `day build`, and `day launch`. Open the selected
project with `day open -t <target>`; edits to generated files are disposable. Apple Swift
sources, plists and `.xcconfig` files, and any Android resource overlays, remain app-owned
under `platform/`. For additional native configuration, use
`day project add-target <target> --persist-native-project` to create a persistent override.

## Native WinUI canvas input

After building conformance for `windows-winui`, run this on an unlocked Windows desktop:

```powershell
python apps/conformance/scripts/windows-canvas.py
```

The fixture continuously repaints a Win2D canvas. The script uses Windows `SendInput`
for clicks held across redraws and for a drag, checking tap counts, focus and drag completion.
UI Automation only locates the surface; Day's driver only asserts results. This complements
`canvas-*` conformance cases, whose pixel checks cover PNG export, scale, transparency,
geometric clipping, transforms and image opacity. It requires an interactive desktop and
is separate from headless unit tests.
