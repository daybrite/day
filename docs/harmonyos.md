# HarmonyOS Next: the ArkUI backend (§9)

HarmonyOS Next dropped the AOSP layer; its UI framework is **ArkUI**. Day targets it with a native
backend (`day-arkui`) built on the **ArkUI Native NodeAPI**, using the same "real native widgets, Day
owns layout" model as every other backend, adapted to HarmonyOS's ArkTS-hosted world.

## Architecture

It mirrors the Android backend: a managed UI runtime (ArkTS) hosts the window, native Rust builds the
widget tree over a thin bridge, and Day owns absolute layout.

```
ArkTS host (Index.ets)           libentry.so (Rust cdylib)
  NodeContent slot  ──start()──▶  day-arkui-sys  (C++ shim over the ArkUI NodeAPI + NAPI)
  ContentSlot(content)            day-arkui      (Toolkit/Platform: Stack/Text/Button/…)
        ▲                         day-core / day-pieces / day-fluent / day-script
        └── OH_ArkUI_NodeContent_AddNode ── the native tree Day builds is mounted here
```

- **`day-arkui-sys`**: a C++ shim (like `day-qt-sys`/`day-winui-sys`) exposing a flat C ABI over
  `arkui/native_node.h` (`createNode`/`setAttribute`/`addChild`/`registerNodeEvent`/`measureNode`),
  `arkui/native_node_napi.h` (`NodeContent`), and `napi/native_api.h`. It also registers the NAPI
  module (`entry`) whose `start(nodeContent, widthVp, heightVp, density)` kicks off Day, wires the
  global node-event receiver back to Rust, and posts to the main (JS) thread via libuv `uv_async`.
- **`day-arkui`**: the `Toolkit`/`Platform` impl. Pieces map to ArkUI node types
  (`ARKUI_NODE_STACK` for containers, `TEXT`, `BUTTON`, `TEXT_INPUT`, `TOGGLE`, `SLIDER`), children
  get an explicit position + size in **vp** (≈ Day points), and events (click / text / toggle /
  slider) come back through `day_arkui_on_event`.
- **`day::arkui_main!(root)`**: exports `day_arkui_start`, the symbol the shim's NAPI `start` calls;
  it mounts the app's root piece and runs the loop (`day::arkui::start` → `launch_with`).

## Local development environment

1. **OpenHarmony SDK / NDK.** Easiest: install the **command-line-tools** (see Build & run), one
   bundle carrying the NDK + hvigor + ohpm + node + signing material, and point `OHOS_NDK_HOME` at
   its `sdk/default/openharmony/native`. For a Rust-only cross-compile you can instead grab just the
   `native` NDK component from the public SDK (no account needed):

   ```bash
   curl -LO https://repo.huaweicloud.com/openharmony/os/6.0-Release/L2-SDK-MAC-M1-PUBLIC.tar.gz
   # extract → .../ohos-sdk/packages/ohos-sdk/darwin/native-*.zip → unzip → .../native
   export OHOS_NDK_HOME=/path/to/ohos-sdk/native
   ```

2. **Rust OHOS targets** (Tier 2 std via rustup; Homebrew rustc has none):

   ```bash
   rustup target add aarch64-unknown-linux-ohos x86_64-unknown-linux-ohos
   ```

   The NDK ships the linker wrappers `$OHOS_NDK_HOME/llvm/bin/<target>-clang`; point cargo at them
   with `CARGO_TARGET_<TARGET>_LINKER`. (`day build` sets this itself, along with `CC_<target>`/
   `AR_<target>` pointing at the NDK's clang and llvm-ar so `cc-rs` build scripts — e.g. `ring`
   under day-part-http's rustls fallback — cross-compile too; the exports only matter if you drive
   bare cargo.)

## Check your environment first

`day doctor --toolkit harmonyos` reports exactly which of the pieces below are present or missing,
with setup instructions:

```bash
day doctor --toolkit harmonyos
```

Bare `day doctor` (no `--toolkit`) scans every toolkit and reports a missing HarmonyOS setup as a
warning rather than an error, since you only need it if you build for HarmonyOS.

## Build & run

The build has two halves with different tool needs:

1. **The Rust cross-compile** (`libentry.so`) needs only the OpenHarmony **NDK** — the `native`
   component of the public SDK, which downloads without a Huawei account. Point `OHOS_NDK_HOME` at it.
   `hdc` (for install/launch) sits in the SDK's sibling `toolchains/` dir; `day` finds it there
   automatically, or you can put it on `PATH`.
2. **Packaging the `.hap`** needs `hvigor` + `ohpm`. These are NOT in the public SDK — they ship with
   the OpenHarmony **command-line-tools** (bundled with DevEco Studio, or the Linux-x64 bundle at
   `repo.huaweicloud.com/harmonyos/ohpm/<ver>/`). Put their `bin/` on `PATH`.

The showcase's `platform/ohos/` project targets **OpenHarmony** (`runtimeOS: "OpenHarmony"`,
`compileSdkVersion`/`compatibleSdkVersion` = the integer API level), which matches the Oniro emulator
and avoids the HMS-only `libimage_transcoder_shared` library that only DevEco Studio ships — so the
whole flow works login-free on macOS and Linux.

**macOS note.** The Linux command-line-tools hvigor/ohpm are pure JavaScript, so they run under
system `node` via a two-line wrapper even though the bundle is packaged for Linux:

```bash
cat > ~/ohos/bin/hvigorw <<'SH'
#!/usr/bin/env bash
exec node "$HOME/ohos-clt/command-line-tools/hvigor/bin/hvigorw.js" "$@"
SH
cat > ~/ohos/bin/ohpm <<'SH'
#!/usr/bin/env bash
exec node "$HOME/ohos-clt/command-line-tools/ohpm/bin/pm-cli.js" "$@"
SH
chmod +x ~/ohos/bin/hvigorw ~/ohos/bin/ohpm
```

Point `OHOS_NDK_HOME` at the public mac SDK's `native` dir and `OHOS_BASE_SDK_HOME` at a versioned
SDK layout (`<dir>/<api>/…`, e.g. a symlink `18 -> .../openharmony`).

```bash
cd apps/day-arkui-demo/platform/ohos

# 1) Cross-compile the app to libentry.so for the emulator (x86_64) and device (arm64):
./build.sh both                                # drops entry/libs/<abi>/libentry.so

# 2) Assemble an (unsigned) .hap with hvigor; build-profile.json5 declares no signingConfig:
ohpm install
hvigorw assembleHap --mode module -p product=default -p buildMode=debug --no-daemon

# 3) Patch + sign it with the OpenHarmony public release material (no account/secrets needed).
#    sign-hap.mjs rewrites module.json's compileSdkType to "OpenHarmony" so the emulator skips
#    code-sign verification (an OpenHarmony device does not trust the public cert's code signature,
#    install error 9568393, but skips the check entirely for OpenHarmony-declared apps), then signs
#    the provision profile + the .hap. Run from the platform/ohos/ project (it reads AppScope/app.json5):
node sign-hap.mjs entry/build/*/outputs/*/entry-default-unsigned.hap \
  entry/build/day-arkui-demo-signed.hap

# 4) Launch the HarmonyOS emulator, then install/run:
hdc install entry/build/day-arkui-demo-signed.hap
hdc shell aa start -b dev.daybrite.day.arkui.demo -a EntryAbility
```

(Opening `platform/ohos/` in DevEco Studio and pressing Run ▶ with auto-sign does all of 2–4 too.)

`day-arkui-demo` is a reactive counter that exercises container / label / button + native events.

In practice you don't run any of the above by hand — `day launch -p ohos-arkui` does the whole flow
(cross-compile → hvigor → sign → install → start), and `day` brings up the emulator too:

```bash
# A native OpenHarmony emulator window (QEMU cocoa on macOS; no VNC, no password, no DevEco).
# Point DAY_OHOS_EMULATOR at the Oniro image dir (default ~/ohos/emulator/images); --headless for CI.
day ohos emulator launch

# Then build + install + launch the app on every connected target (see "Multiple devices" below):
day launch --project apps/showcase -p ohos-arkui
```

## Multiple devices / architectures

`day launch -p ohos-arkui` enumerates every reachable `hdc` target, queries each one's arch
(`uname -m` → x86_64 emulator / arm64 device), builds a `libentry.so` for **each** arch, and packs
them all into the one `.hap` (`libs/x86_64/` + `libs/arm64-v8a/`) so it installs on any of them.
It then installs + starts the app on **every** connected target. Android (`adb`, per-device
`ro.product.cpu.abi`) and iOS (every booted simulator) do the same — one `day launch` fans out to
all connected devices, building whatever ABIs they need.

## Status

`ohos-arkui` is a **first-class, non-experimental** target. Pieces render as real ArkUI Native
NodeAPI nodes, verified on the Oniro emulator:

- **Nav shell** (`selector`) — a scrollable list that pushes detail pages.
- **Controls** — `Text`, `Button`, `TextInput`, native `Slider` / `Toggle`, a determinate `Progress`
  bar + an indeterminate `LoadingProgress` spinner, and `Divider` hairlines.
- **Canvas** (§11) — an `ARKUI_NODE_CUSTOM` node whose on-draw callback replays Day's display list
  with **OH_Drawing** (arcs, fills, strokes, rounded-rects, ellipses, text) — the gauge + shapes pages.
- **List** (§10) — an `ARKUI_NODE_LIST` driven by an `OH_ArkUI_NodeAdapter` with cell reuse, so a
  500-row list only builds the visible cells.
- **Tabs** — an `ARKUI_NODE_SWIPER` pager with a dot indicator.

Images/webview/lottie/map pieces are not yet wired (they render as placeholders). Packaging + signing
run headlessly via the command-line-tools (see Build & run) with no Huawei developer account: hvigor
assembles the `.hap` and `sign-hap.mjs` patches (compileSdkType → OpenHarmony) + signs it with the
public release material. `libentry.so` links the NDK's shared libc++ and `libnative_drawing`, both
packed into the `.hap`.

## CI

The `ohos-arkui` job in `.github/workflows/ci.yml` runs on every push/PR (`macos-14`). The build
gates **hard** (clippy + cross-compile + `hvigorw assembleHap` + sign + `day doctor`); only the
emulator boot + walkthrough are best-effort (`continue-on-error` per step) because the GitHub-hosted
TCG emulator is slow and occasionally flaky. It downloads + caches the OpenHarmony
**command-line-tools** (~2 GB) and then runs the real build pipeline:

1. clippy the backend for the OHOS target, and cross-compile the full Day app to `libentry.so` for
   the emulator (x86_64) and device (arm64) using the darwin SDK's NDK clang;
2. `ohpm install`, then `hvigorw assembleHap`, a full hvigor build of the ArkTS host + `.hap`;
3. patch + sign the `.hap` with `sign-hap.mjs` (compileSdkType → OpenHarmony + the public release
   material), then install/launch it on the Oniro emulator over `hdc` and drive the dayscript
   walkthrough, uploading screenshots for the gallery, like the other targets. CI boots the emulator
   with `day ohos emulator launch --headless` (the same Oniro v6.1 image openharmony-rs's
   emulator-action uses) rather than the action itself: the action's QEMU command has **no GPU
   device** (`-nographic`), so the guest has no display — the keyguard never dismisses, `aa start`
   is refused with error 10106102, and screenshots capture nothing. Day's launcher adds
   `-device virtio-gpu-pci` with `-display none`: a headless framebuffer the apps can foreground on
   and `uitest screenCap` can capture.

Declaring the app OpenHarmony (via the compileSdkType patch) is what lets it install: the emulator
enforces app code signing but doesn't trust the public cert, and OpenHarmony's BMS skips code-sign
verification for OpenHarmony-declared apps on devices without Huawei OH code signing.

The whole job — build gates and emulator screenshots — runs on a single **macOS** runner: the
x86_64 Oniro guest is TCG-emulated on every GitHub runner, and only the macOS ARM hosts run it
fast enough for the pipeline (first-boot keyguard render → wake + swipe-unlock → walkthrough);
the ubuntu hosts never got that far. Build steps gate hard; only the emulator boot + walkthrough
are per-step best-effort. The setup replicates the validated local macOS flow: the Linux
command-line-tools (hvigor/ohpm are pure JS) run through node wrapper scripts, the darwin
API-18 SDK from setup-ohos-sdk supplies the NDK + hdc + the versioned `OHOS_BASE_SDK_HOME`
view hvigor builds against, and Day's own QEMU launcher boots the image. `OHOS_BASE_SDK_HOME`
must be a **host-platform** SDK — hvigor spawns its native tools (`syscap_tool`, `restool`,
`es2abc`) directly, so pointing it at the Linux CLT's bundled SDK fails on macOS with
`spawn ENOEXEC` at `SyscapTransform`.

Four hard-won facts the scripted channel depends on (each was a silent total failure):

- **The default hdc forward port 55555 is often already occupied** — GitHub's macOS runners hold
  it, and so do some local services — and QEMU then dies instantly ("Could not set up host
  forwarding rule"), leaving no reachable target and blank screenshot sets. `day ohos emulator
  launch` probes and slides to the first free port, `tconn`s the chosen key (so device discovery
  finds it), and exports `DAY_OHOS_TARGET` through `GITHUB_ENV` so later CI steps target it too.

- **`ohos.permission.INTERNET` is required for the LOOPBACK dayscript socket** — without it in
  `module.json5` the engine's `TcpListener::bind` fails silently and no scripted run can ever
  connect (the Android manifest needs the same permission for the same reason).
- **The ability is a singleton**: a second `aa start` foregrounds the existing process, whose
  engine (if any) listens on the PREVIOUS run's port. `day launch` force-stops the bundle
  before every start so each run's engine params take effect.
- **The keyguard returns whenever the display sleeps** and `aa start` is refused while it shows;
  `day launch` wakes + swipe-unlocks (uitest + uinput) around every launch retry.

## Follow-ups

Wire the remaining pieces (image / webview / lottie / map), richer accessibility (roles beyond the
label), interactive tab-bar labels on the swiper, and the image/webview backends. The dayscript
engine's TCP channel is flaky on the TCG emulator (occasional connection resets), so scripted
walkthrough screenshots on OHOS are best-effort.
