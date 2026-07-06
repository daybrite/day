# HarmonyOS Next — the ArkUI backend (§9)

HarmonyOS Next dropped the AOSP layer; its UI framework is **ArkUI**. Day targets it with a native
backend (`day-arkui`) built on the **ArkUI Native NodeAPI** — the same "real native widgets, Day owns
layout" model as every other backend, adapted to HarmonyOS's ArkTS-hosted world.

## Architecture

It mirrors the Android backend: a managed UI runtime (ArkTS) hosts the window, native Rust builds the
widget tree over a thin bridge, and **Day owns absolute layout**.

```
ArkTS host (Index.ets)           libentry.so (Rust cdylib)
  NodeContent slot  ──start()──▶  day-arkui-sys  (C++ shim over the ArkUI NodeAPI + NAPI)
  ContentSlot(content)            day-arkui      (Toolkit/Platform: Stack/Text/Button/…)
        ▲                         day-core / day-pieces / day-fluent / day-script
        └── OH_ArkUI_NodeContent_AddNode ── the native tree Day builds is mounted here
```

- **`day-arkui-sys`** — a C++ shim (like `day-qt-sys`/`day-winui-sys`) exposing a flat C ABI over
  `arkui/native_node.h` (`createNode`/`setAttribute`/`addChild`/`registerNodeEvent`/`measureNode`),
  `arkui/native_node_napi.h` (`NodeContent`), and `napi/native_api.h`. It also registers the NAPI
  module (`entry`) whose `start(nodeContent, widthVp, heightVp, density)` kicks off Day, wires the
  global node-event receiver back to Rust, and posts to the main (JS) thread via libuv `uv_async`.
- **`day-arkui`** — the `Toolkit`/`Platform` impl. Pieces map to ArkUI node types
  (`ARKUI_NODE_STACK` for containers, `TEXT`, `BUTTON`, `TEXT_INPUT`, `TOGGLE`, `SLIDER`), children
  get an explicit position + size in **vp** (≈ Day points), and events (click / text / toggle /
  slider) come back through `day_arkui_on_event`.
- **`day::arkui_main!(root)`** — exports `day_arkui_start`, the symbol the shim's NAPI `start` calls;
  it mounts the app's root piece and runs the loop (`day::arkui::start` → `launch_with`).

## Local development environment

1. **OpenHarmony SDK / NDK.** Download the public SDK (no account needed) and extract the `native`
   component (the NDK — clang, sysroot, ArkUI/NAPI headers, the ArkUI runtime libs):

   ```bash
   curl -LO https://repo.huaweicloud.com/openharmony/os/6.0-Release/L2-SDK-MAC-M1-PUBLIC.tar.gz
   # extract → .../ohos-sdk/packages/ohos-sdk/darwin/native-*.zip → unzip → .../native
   export OHOS_NDK_HOME=/path/to/ohos-sdk/native
   ```

2. **Rust OHOS targets** (Tier 2 std via rustup — Homebrew rustc has none):

   ```bash
   rustup target add aarch64-unknown-linux-ohos x86_64-unknown-linux-ohos
   ```

   The NDK ships the linker wrappers `$OHOS_NDK_HOME/llvm/bin/<target>-clang`; point cargo at them
   with `CARGO_TARGET_<TARGET>_LINKER`.

## Build & run

```bash
# 1) Cross-compile the app to libentry.so for the emulator (x86_64) and device (arm64):
apps/day-arkui-demo/harmony/build.sh both     # drops entry/libs/<abi>/libentry.so

# 2) Package the .hap. Open apps/day-arkui-demo/harmony/ in DevEco Studio (it fills in the default
#    resources + a debug signing profile), or use the OpenHarmony command-line-tools:
#      hvigorw assembleHap

# 3) Sign (DevEco auto-sign, or a debug signing profile — HarmonyOS requires signed .haps), then
#    launch the HarmonyOS emulator and install/run:
hdc install entry/build/default/outputs/default/entry-default-signed.hap
hdc shell aa start -b dev.daybrite.day.arkui.demo -a EntryAbility
```

`day-arkui-demo` is a reactive counter that exercises container / label / button + native events.

## Status

Verified in this repo: the C++ shim compiles against the real ArkUI/NAPI headers, and the **complete
Day app cross-compiles and links to a loadable `libentry.so`** for both HarmonyOS arches (aarch64 +
x86_64), exporting `day_arkui_start` / `day_arkui_on_event` and registering the `entry` NAPI module.
The ArkTS host project + `build.sh` assemble the `.so` into a DevEco-buildable project.

Not yet done here (needs tooling this environment lacks): **packaging the `.hap`** (hvigor/ohpm — the
build system is npm-fetchable from `repo.harmonyos.com/npm`, but not bundled with the public SDK),
**app signing** (a HarmonyOS certificate/profile), and **running on the emulator** (the HarmonyOS
emulator ships with DevEco Studio and is gated behind a Huawei developer account). The `day` CLI
registers the `harmonyos-arkui` target (build/launch orchestration through DevEco/hvigor is a
follow-up); today the flow is `harmony/build.sh` + DevEco Studio.

## CI

The `ohos-arkui` job in `.github/workflows/ci.yml` runs on every push/PR. It always does the
verifiable half — fetch the OpenHarmony NDK (cached), then cross-compile the full Day app to
`libentry.so` for the emulator (x86_64) and device (arm64) and clippy the backend for the OHOS target
— and then **tries** the run against a locally-installed + configured HarmonyOS emulator: package the
`.hap` (if DevEco's hvigor is present), `hdc install`, launch the ability, and snapshot. Those run
steps no-op on a stock runner (no HarmonyOS target connected) and light up on a **self-hosted macOS
runner** that has DevEco Studio + a booted emulator; the job is `continue-on-error` so it stays
non-blocking. A pre-set `OHOS_NDK_HOME` (a self-hosted DevEco SDK) skips the download.

## Follow-ups

Accessibility (`accessibilityText`), the canvas display list (`ARKUI_NODE_CUSTOM` + a draw callback),
lists (`ARKUI_NODE_LIST` recycling), navigation (`Navigation`), scroll content sizing, and end-to-end
`day` CLI orchestration (cross-compile → hvigor → sign → `hdc install`).
