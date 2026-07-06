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

1. **OpenHarmony SDK / NDK.** Easiest: install the **command-line-tools** (see Build & run) — one
   bundle carrying the NDK + hvigor + ohpm + node + signing material — and point `OHOS_NDK_HOME` at
   its `sdk/default/openharmony/native`. For a Rust-only cross-compile you can instead grab just the
   `native` NDK component from the public SDK (no account needed):

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

You need the OpenHarmony **command-line-tools** — one self-contained bundle carrying the SDK/NDK,
hvigor, ohpm, a bundled node, `hdc`, and the default debug signing material. It downloads without a
Huawei developer account from `repo.huaweicloud.com/harmonyos/ohpm/<ver>/` (Linux x64; on macOS use
DevEco Studio's bundled copy). Point `OHOS_NDK_HOME` at `sdk/default/openharmony/native` and put
`bin/` (hvigor/ohpm) + `sdk/default/openharmony/toolchains` (hdc) on `PATH`.

```bash
cd apps/day-arkui-demo/harmony

# 1) Cross-compile the app to libentry.so for the emulator (x86_64) and device (arm64):
./build.sh both                                # drops entry/libs/<abi>/libentry.so

# 2) Assemble an (unsigned) .hap with hvigor — build-profile.json5 declares no signingConfig:
ohpm install
hvigorw assembleHap --mode module -p product=default -p buildMode=debug --no-daemon

# 3) Sign it with the bundled default OpenHarmony DEBUG material (no account/secrets needed).
#    sign-hap.sh runs generate-app-cert + sign-profile + sign-app for this bundle id:
./sign-hap.sh entry/build/*/outputs/*/entry-default-unsigned.hap \
  entry/build/day-arkui-demo-signed.hap dev.daybrite.day.arkui.demo

# 4) Launch the HarmonyOS emulator, then install/run:
hdc install entry/build/day-arkui-demo-signed.hap
hdc shell aa start -b dev.daybrite.day.arkui.demo -a EntryAbility
```

(Opening `harmony/` in DevEco Studio and pressing Run ▶ — with auto-sign — does all of 2–4 too.)

`day-arkui-demo` is a reactive counter that exercises container / label / button + native events.

## Status

Verified in this repo: the C++ shim compiles against the real ArkUI/NAPI headers, and the **complete
Day app cross-compiles and links to a loadable `libentry.so`** for both HarmonyOS arches (aarch64 +
x86_64), exporting `day_arkui_start` / `day_arkui_on_event` and registering the `entry` NAPI module.
The ArkTS host project + `build.sh` assemble the `.so` into a DevEco-buildable project.

**Packaging + signing now run headlessly** via the command-line-tools (see Build & run) — no Huawei
developer account: hvigor assembles the `.hap` and `sign-hap.sh` signs it with the bundled default
debug material. What still needs real hardware is **running on the emulator** (the HarmonyOS emulator
ships with DevEco Studio, Huawei-account-gated; no HarmonyOS emulator is connectable to `hdc` in this
sandbox). The `day` CLI registers the `harmonyos-arkui` target (end-to-end build/launch orchestration
through hvigor is a follow-up); today the flow is the `harmony/` scripts above or DevEco Studio.

## CI

The `ohos-arkui` job in `.github/workflows/ci.yml` runs on every push/PR (`ubuntu-24.04`,
`continue-on-error` so it stays non-blocking). It downloads + caches the OpenHarmony
**command-line-tools** (~2 GB) and then runs the real build pipeline:

1. clippy the backend for the OHOS target, and cross-compile the full Day app to `libentry.so` for
   the emulator (x86_64) and device (arm64) using the CLT's native NDK clang;
2. `ohpm install`, then `hvigorw assembleHap` — a genuine hvigor build of the ArkTS host + `.hap`;
3. sign the `.hap` with the bundled default debug material (`sign-hap.sh`), uploaded as an artifact.

The final step **tries** a run against a locally-configured HarmonyOS emulator (`hdc install` → launch
→ snapshot); it no-ops on a stock GitHub runner (no HarmonyOS target connected) and lights up on a
**self-hosted runner** with a booted emulator. A pre-set `DEVECO_SDK_HOME`/`OHOS_NDK_HOME` (a
self-hosted DevEco install) is used as-is.

## Follow-ups

Accessibility (`accessibilityText`), the canvas display list (`ARKUI_NODE_CUSTOM` + a draw callback),
lists (`ARKUI_NODE_LIST` recycling), navigation (`Navigation`), scroll content sizing, and end-to-end
`day` CLI orchestration (cross-compile → hvigor → sign → `hdc install`).
