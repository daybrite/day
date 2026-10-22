---
title: "HarmonyOS backend"
description: "The ArkUI backend in depth: the C node API, the ArkTS shim, hvigor packaging, and emulator workflows."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# HarmonyOS Next: the ArkUI backend

HarmonyOS Next dropped the AOSP layer; its UI framework is **ArkUI**. Day targets it with a native
backend (`day-arkui`) built on the **ArkUI Native NodeAPI**, using the same "real native widgets, Day
owns layout" model as every other backend, adapted to HarmonyOS's ArkTS-hosted world. `harmony-arkui`
is [Tier 3](/docs/reference/platforms#support-tiers): tested, but not comprehensively, and
not yet exercised by shipping applications.

## Launch logs

Attached launches resolve the application's PID with `hdc shell pidof <app-id>` and
stream `hilog -P <pid>`. Each device has one reader. Starting another variant or stopping
the app kills and reaps the old reader, so locale/theme sweeps do not accumulate log streams.
The filter includes the current process's buffered startup messages, not earlier launches
or unrelated system services. Device-wide log buffers are never cleared.

If PID lookup fails, Day prints a diagnostic and skips streaming rather than falling back
to the entire device log. Separate WebView renderer/service processes are outside the filter;
use `hdc shell hilog` manually when those diagnostics are needed. See the
[OpenHarmony hilog options](https://github.com/openharmony/docs/blob/master/zh-cn/application-dev/dfx/hilog.md).

## Architecture

It mirrors the Android backend: a managed UI runtime (ArkTS) hosts the window, native Rust builds the
widget tree over a thin bridge, and Day owns absolute layout.

```text
ArkTS host (Index.ets)           libentry.so (Rust cdylib)
  NodeContent slot  ──start()──▶  day-arkui  (Toolkit/Platform: Stack/Text/Button/…)
  ContentSlot(content)              host_api  #[napi] exports + JS callbacks   (napi-ohos)
        ▲                           node / canvas / list / images / …          (ohos-sys)
        │                         day-core / day-pieces / day-fluent / day-script
        └── OH_ArkUI_NodeContent_AddNode ── the native tree Day builds is mounted here
```

There is no C or C++ in the backend. The NDK is bound by openharmony-rs's
[`ohos-sys`](https://crates.io/crates/ohos-sys) (ArkUI's node, gesture, animate and drag APIs,
OH_Drawing, the image kit, rawfile, VSync, UDMF, hilog), and the NAPI module the host imports is
registered and marshaled by ohos-rs's [`napi-ohos`](https://crates.io/crates/napi-ohos): every
export in `types/Index.d.ts` is a `#[napi]` function in `src/host_api.rs`, and the host's
callbacks (navigation, pickers, permissions, windows, ArkTS-built pieces, the daybridge record)
are held as NAPI references. `build.rs` names the module `entry` (`NAPI_BUILD_TARGET_NAME`) so
it matches the `libentry.so` the ArkTS runtime loads.

The ArkTS host is the framework's, not the app's. It lives in the day-arkui crate at
`toolkits/day-arkui/platform/harmony/` — `ets/` holds `EntryAbility` (which also reports the
app lifecycle, [docs/lifecycle](lifecycle.md), through the `lifecycle` export: its foreground/background,
memory and destroy callbacks plus the window stage's active/inactive events), `DayWindowAbility`,
the `Index` page and the secondary-window `DayWindow` page; `types/Index.d.ts` declares the native
module's exports, so it always matches the `#[napi]` exports the app links. `day build`
(and `day prepare`/`day open`) resolves the crate through cargo metadata and stages the
directory into `build/day/harmony/project/`: the pages and abilities under
`entry/src/main/ets/day/`, the typings (with their `oh-package.json5`) under
`entry/src/main/cpp/types/libentry/`, plus the generated page list and start-window colors.
The CLI derives the hvigor skeleton from its built-in template, including build profiles,
`AppScope/app.json5`, `module.json5`, string tables and hvigor configuration. New apps need
no checked-in `platform/harmony/` directory. A customized source host at that path takes
precedence. Day writes permission strings, shortcut labels
and profiles, manifest changes, and raw resources into the staged project, then runs hvigor
there. All generated output stays under `build/day/`. An app that predates this (2026-09) still carries its
own copies under `ets/pages/` and `ets/entryability/`; `day build` leaves those alone and
prints a note on how to adopt the staged host.

`day open -t harmony-arkui` prepares and opens the staged project in DevEco Studio. For
manual customization, first run `day project add-target harmony-arkui --persist-native-project`,
edit the resulting files under `platform/harmony/`, then run `day prepare -t harmony-arkui` again;
edits made only in the staged copy are overwritten. Each preparation refreshes the source
copy and removes obsolete resources while keeping hvigor's build and dependency caches.
Symlinked inputs, including generated icon media, are copied as files so native generators
cannot write through to their sources. The old `platform/ohos/` layout is rejected; use
`platform/harmony/` and `[app.harmony]` for platform identity overrides.
Flavors use `build/day/flavors/<name>/harmony/project/`.

Set `[harmony] vendor = "your-publisher"` in `Day.toml` to customize the vendor in
AppScope metadata without retaining a project skeleton.

Keep native module dependencies and custom build-script paths relative to the native host,
or use absolute paths for external tools. A path that climbs out of `platform/harmony/`
will resolve from a different directory in the staged project.

- **`day-arkui`**: the `Toolkit`/`Platform` impl (`src/lib.rs`) over a set of modules, one per
  NDK API: `node` (the `ArkUI_NativeNodeAPI_1` table: `createNode`/`setAttribute`/`addChild`/
  `registerNodeEvent`/`measureNode`, plus the attribute recipes the pieces need), `events` (the
  global node-event receiver, which turns ArkUI payloads into the shared bridge wire), `canvas`
  (OH_Drawing replay), `list` (the NodeAdapter), `images` (pixelmaps, the packer, snapshots),
  `fonts`, `anim` (`animateTo`), `gesture`, `transfer` (UDMF drag and drop), `resources`
  (rawfile), `vsync`, `hilog`, `main_thread` (a NAPI threadsafe function posts to the JS thread),
  `host_api` (the `#[napi]` exports and the host's callbacks) and `bridge` (daybridge's ArkTS
  arms). Pieces map to ArkUI node types (`ARKUI_NODE_STACK` for containers, `TEXT`, `BUTTON`,
  `TEXT_INPUT`, `TOGGLE`, `SLIDER`), children get an explicit position + size in **vp** (≈ Day
  points), and events (click / text / toggle / slider) come back through `on_event`. A standalone
  piece drives its own node through `day_arkui::node` and the raw `day_arkui::arkui_sys` bindings
  ([day-piece-datetime](/docs/reference/datepicker), [day-piece-pullrefresh](/docs/reference/pullrefresh), day-tweak-slider-tickmarks): no C++ of its own.
- **`day::day_start_arkui!(root)`**: exports `day_arkui_start`, the symbol the shim's NAPI `start` calls;
  it mounts the app's root piece and runs the loop (`day::arkui::start` → `launch_with`). Apps
  reach it through `day::day_start!("App Name", root)`, which expands to this macro and to every
  other platform's entry; the title argument is ignored here, since the label comes from
  `AppScope/resources/base/element/string.json`.

## Local development environment

1. **cargo-ohos.** The cross-compile runs through [cargo-ohos](https://crates.io/crates/cargo-ohos),
   openharmony-rs's cargo subcommand (the same org as the `ohos-sys` bindings). It locates the
   OpenHarmony SDK and sets up the linker, the `cc-rs` compilers, bindgen, CMake and pkg-config
   for the target, so `day build`, the lint script and a bare `cargo ohos clippy` share one cross
   environment:

   ```bash
   cargo install --locked cargo-ohos      # or: cargo binstall cargo-ohos (prebuilt binaries)
   ```

2. **OpenHarmony SDK / NDK.** `cargo ohos init sdk` downloads the public SDK (no account needed)
   into cargo-ohos's cache, where `cargo ohos` finds it with no variable set. The `native`
   component is the NDK (the cross clang and its sysroot), `toolchains` carries `hdc` and the
   signing material, and hvigor needs every component:

   ```bash
   cargo ohos init sdk --api 18                                    # everything, for hvigor too
   cargo ohos init sdk --api 18 --components native,toolchains     # a Rust-only cross-compile
   ```

   An SDK from elsewhere works as well: the Linux **command-line-tools** bundle (see Build & run)
   carries one at `sdk/default/openharmony`, and DevEco Studio installs one. Point
   `OHOS_SDK_NATIVE` at its `native` directory (cargo-ohos also reads `OHOS_NDK_HOME`,
   `OHOS_SDK_HOME` and `DEVECO_SDK_HOME`, and finds DevEco Studio's install on macOS by itself).
   The SDK must be built for this host OS: the Linux bundle's tools don't run on macOS, so take
   the download or the public mac SDK there.

3. **Rust OHOS targets** (Tier 2 std via rustup; Homebrew rustc has none):

   ```bash
   rustup target add aarch64-unknown-linux-ohos x86_64-unknown-linux-ohos
   ```

   `day build` adds a missing one itself. The one variable cargo-ohos does not set is
   `OHOS_NDK_HOME`, which napi-ohos's build script reads for its link search paths: `day build`
   and `scripts/ci/lint.sh` export it from the SDK cargo-ohos resolved; a bare `cargo ohos build`
   needs it set by hand.

## Check your environment first

`day doctor --toolkit harmonyos` reports exactly which of the pieces below are present or missing,
with setup instructions:

```bash
day doctor --toolkit harmonyos
```

It checks what the build and launch actually use, on Linux, macOS, and Windows alike:

| Check | Needed for | What it verifies |
|---|---|---|
| `cargo-ohos` | build | `cargo ohos --version` runs |
| `ohos-ndk` | build | cargo-ohos resolves an SDK `native` dir (`cargo ohos env`), and its clang is a binary for this host OS |
| `rust-ohos` | build | an OHOS Rust target is installed |
| `ohos-sdk` | build | `OHOS_BASE_SDK_HOME` is set, in the versioned `<dir>/<api>/` layout hvigor requires, holds the API level the current project's `compileSdkVersion` names, and its `toolchains/restool` runs on this host |
| `hvigorw`, `ohpm` | build | on PATH, including Windows' `.bat`/`.cmd` launchers |
| `node` | build | on PATH; signing runs `sign-hap.mjs` under it |
| `hdc` | launch | on PATH, or in the SDK's `toolchains/` |
| `qemu` | launch | `qemu-system-x86_64` on PATH |
| `emu-images` | launch | every file of the emulator image in `DAY_OHOS_EMULATOR` or `~/ohos/emulator/images`, Oniro (6.x) or ohos-qemu (7.0) layout |
| `kvm` | launch (Linux) | `/dev/kvm` opens read-write, so the emulator is accelerated |

The host-format checks catch the common cross-OS mistake before hvigor does: a Linux SDK on a
Mac fails deep in the build with `spawn ENOEXEC`, and a flat (unversioned) SDK root with "The SDK
management mode has changed". Launch checks are warnings: you need them only to run the app.

Bare `day doctor` (no `--toolkit`) scans every toolkit and reports a missing HarmonyOS setup as a
warning rather than an error, since you only need it if you build for HarmonyOS.

## Build & run

The build has two halves with different tool needs:

1. **The Rust cross-compile** (`libentry.so`) needs cargo-ohos and the OpenHarmony **NDK**, the
   `native` component of the public SDK: `cargo ohos init sdk --components native,toolchains`
   downloads it without a Huawei account, or point `OHOS_SDK_NATIVE` at one you have. `hdc` (for
   install/launch) sits in the SDK's sibling `toolchains/` dir; `day` finds it there
   automatically, or you can put it on `PATH`.
2. **Packaging the `.hap`** needs `hvigor` + `ohpm`. These are not in the public SDK; they ship with
   the OpenHarmony **command-line-tools** (bundled with DevEco Studio, or the Linux-x64 bundle at
   `repo.huaweicloud.com/harmonyos/ohpm/<ver>/`). Put their `bin/` on `PATH`.

The showcase's `platform/harmony/` project targets **OpenHarmony** (`runtimeOS: "OpenHarmony"`,
`compileSdkVersion`/`compatibleSdkVersion` = the integer API level), which matches the Oniro emulator
and avoids the HMS-only `libimage_transcoder_shared` library that only DevEco Studio ships, so the
whole flow works login-free on macOS and Linux.

**Linux note.** The bundle's `bin/hvigorw` and `bin/ohpm` run directly, using the node under
`tool/node/bin`; put both directories on `PATH` (Day's signing step runs `node`). Point
`OHOS_SDK_NATIVE` at `sdk/default/openharmony/native` and `OHOS_BASE_SDK_HOME` at a versioned view
of the same SDK:

```bash
C=~/ohos-clt/command-line-tools
mkdir -p ~/ohos/sdk && ln -sfn $C/sdk/default/openharmony ~/ohos/sdk/18
export OHOS_SDK_NATIVE=$C/sdk/default/openharmony/native OHOS_BASE_SDK_HOME=~/ohos/sdk
export PATH=$C/bin:$C/tool/node/bin:$C/sdk/default/openharmony/toolchains:$PATH
```

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

The SDK `cargo ohos init sdk --api 18` downloads is the public mac one, cached already in the
versioned layout hvigor wants: `cargo ohos` finds its `native` dir by itself, and
`OHOS_BASE_SDK_HOME` is the `<version>/darwin` directory the download prints
(`~/Library/Caches/cargo-ohos/ohos-sdk/5.1.0/darwin`, whose `18/` holds the components).

```bash
cd Day-Showcase
day build -t harmony-arkui

# Inspect the prepared native project in DevEco Studio:
day open -t harmony-arkui
```

For native packaging work after a Day build, run hvigor from the staged project:

```bash
cd build/day/harmony/project
ohpm install
hvigorw assembleHap --mode module -p product=default -p buildMode=debug --no-daemon
```

Use DevEco Studio's auto-sign configuration when installing on a physical device. Configure
it in the source host if it must survive the next preparation.

You don't run any of the above by hand; `day launch -t harmony-arkui` does the whole flow
(cross-compile → hvigor → sign → install → start), and `day` brings up the emulator too:

The emulator images are a separate download: unpack
[`oniro_emulator.zip`](https://github.com/eclipse-oniro4openharmony/device_board_oniro/releases/download/v6.1/oniro_emulator.zip)
(OpenHarmony 6.1; 1.4 GB, 5.5 GB unpacked) in `~/ohos/emulator`, which yields the default
`~/ohos/emulator/images`, and install `qemu-system-x86_64`.

**OpenHarmony 7.0.** `DAY_OHOS_EMULATOR` may instead name the images of a
[harmony-contrib/ohos-qemu](https://github.com/harmony-contrib/ohos-qemu) `x86_64_virt` package
(verified: v20260919, OpenHarmony 7.0.0.39, API 26). `EmulatorImage` in `ohos.rs` tells the two
layouts apart by the 7.0 image's extra `sys_prod.img`/`chip_prod.img` and supplies what differs:
six disks rather than four (in the vda–vdf order the kernel command line's mounts name), an
eng/developer-mode command line, hdc on guest port 5555 rather than 55555, and the virtio tablet
and keyboard. The host port stays `DAY_OHOS_TARGET`'s for both, so `day launch`, `day drive`, and
scripts need no change. The Showcase walkthrough passes 861/861 on both images (API-18 build).
Both images boot windowed with `-display sdl,gl=off,show-cursor=on` (the cursor: see "Over
remote desktop" below): the 7.0 guest aborts QEMU's GL display
(`surface_gl_create_texture: Assertion 'map_format(...)'`) during boot. ohos-qemu's arm64
packages (an `Image` kernel) are refused by name: they need `qemu-system-aarch64`.

```bash
# A native OpenHarmony emulator window (QEMU cocoa on macOS, SDL on Linux, as the Oniro image's own
# run.sh opens it; no VNC, no password, no DevEco). Point DAY_OHOS_EMULATOR at the Oniro image dir
# (default ~/ohos/emulator/images). --headless runs without a window, as CI does.
day devices boot -t harmony-arkui
# The same image on a landscape tablet panel. Oniro has no screen of its own: it draws at whatever
# size QEMU's virtio-gpu is told, so --device names a panel (phone 360x720, tablet 1280x800, or
# WxH) and --orientation turns it. The guest still calls itself a phone (const.product.devicetype
# is baked into the image), so this shows the app's wide layout, not the system's tablet behavior.
day devices boot -t harmony-arkui --device tablet --orientation landscape

# Then build + install + launch the app on every connected target (see "Multiple devices" below):
day launch --project Day-Showcase -t harmony-arkui
```

**Acceleration.** On an x86_64 Linux host the guest runs under KVM when this user can open
`/dev/kvm` read-write. That usually takes membership in the `kvm` group (`sudo usermod -aG kvm
$USER`, then log in again). Otherwise boot falls back to TCG and says so. TCG boots in several
minutes on a 4-core desktop. The guest gets 6 vCPUs, or the host's core count when it has fewer;
`DAY_OHOS_SMP` overrides that. `DAY_OHOS_ACCEL` forces a choice (`kvm`, or `tcg,thread=multi`).

**Windowed boot on Linux.** The window is QEMU's SDL display with GL off, which is how the Oniro
image's own `run.sh` opens it (its default screen is the 360×720 phone), and QEMU comes from the
distribution: `sudo apt install qemu-system-x86 qemu-system-gui` on Ubuntu, `qemu-system-x86-core
qemu-ui-sdl` on Fedora (verified 2026-09 on Ubuntu 24.04 with its QEMU 8.2, under KVM):

- The SDL window opens at the requested panel, and the guest boots at it. When the window manager
  clamps a window larger than the screen, the guest follows the window: a 1280×800 tablet on a
  1280×800 laptop comes up at the screen's usable 1214×731. `--headless` keeps the panel exactly.
- A QEMU built without SDL is refused, by `day devices boot` and by `day doctor`. Homebrew's
  QEMU is one: it has only GTK, whose window hands the guest its own size, starting from a
  640×480 placeholder, instead of the requested panel.
- `day launch` sizes its keyguard swipe from the guest's own screen (RenderService's
  `physical resolution`), not from the requested panel: a swipe aimed at 360×720 starts below a
  640×480 screen, the screen-lock service stays locked, and every `aa start` is refused with
  10106102.

**Over remote desktop.** QEMU hides the host cursor over its window by default, since the
guest draws its own, and an RDP client (Thincast here) takes a hidden cursor as a cue to send
relative "game" mouse input, which the window never receives: QEMU's input trace showed a few
positions as the pointer entered, then nothing. `day devices boot` keeps the cursor visible
(`-display sdl,gl=off,show-cursor=on`), so the client keeps sending positions.

- **OpenHarmony 7.0 (ohos-qemu)** gets a virtio tablet, an absolute pointer its input service
  maps to the screen, so the guest pointer follows the host's over RDP (verified 2026-09 over
  Thincast: about 2,300 positions traced where a hidden cursor let about 20 through).
- **Oniro 6.1** can't use one: its input service has no handling for absolute pointers
  (`MouseTransformProcessor` treats absolute motion as relative), so a tablet pins the pointer
  to the screen's edge. Its relative PS/2 mouse moves only while the window has grabbed the host
  pointer, which a remote-desktop session doesn't support. What works there is the image's own
  `run.sh --headless`, whose VNC server turns the viewer's pointer positions into relative motion
  with no grab (verified over Thincast, viewing with Remmina); the website's troubleshooting page
  has the steps. `day devices boot` doesn't serve VNC itself.

Don't put the guest to sleep with `power-shell suspend`: the whole guest suspends, `hdcd`
included, so every later `hdc` call hangs and nothing over hdc can wake it. Restart the
emulator.

## Multiple devices / architectures

`day launch -t harmony-arkui` enumerates every reachable `hdc` target, queries each one's arch
(`uname -m` → x86_64 emulator / arm64 device), builds a `libentry.so` for **each** arch, and packs
them all into the one `.hap` (`libs/x86_64/` + `libs/arm64-v8a/`) so it installs on any of them.
It then installs + starts the app on **every** connected target. Android (`adb`, per-device
`ro.product.cpu.abi`) and iOS (every booted simulator) do the same: one `day launch` fans out to
all connected devices, building whatever ABIs they need.

## Status

`harmony-arkui` is a maintained target. Pieces render as ArkUI Native NodeAPI nodes, verified
on the Oniro emulator:

- **Nav shell** (`nav`) — ArkUI's `Navigation`: the rows beside the detail where the window has
  room (a tablet, `NavigationMode.Auto` tiling its nav bar and content), a list that pushes detail
  pages where it has not; the mode is ArkUI's decision, observed and reconciled
  ([docs/size-classes](size-classes.md)).
- **Controls** — `Text`, `Button`, `TextInput`, native `Slider` / `Toggle`, a determinate `Progress`
  bar + an indeterminate `LoadingProgress` spinner, and `Divider` hairlines.
- **Inline links** — native clickable `SPAN` children route their original targets through
  `Event::LinkActivated`. Markdown, `TextBuilder` and standalone `link(...)` use the same
  path, including wrapped text and reactive updates. `Cap::TextLinks` reports `Native`.
  See [text-runs](text-runs.md) and the [native tap tests](testing.md#native-arkui-link-tests).
- **Text areas** — native `TextArea` with live read-only and selection policies. Day vetoes edits
  and restores text for input paths that bypass the veto; selection-disabled editors collapse
  native ranges and suppress selection menus/clipboard shortcuts. Both capabilities report
  `Emulated` because the API 18 C editor lacks equivalent persistent properties. Read-only
  text can still be copied, and app writes still replace its contents. Spell-check remains
  unsupported. See [textarea](textarea.md) and the [native-input tests](testing.md#native-arkui-text-area-tests).
  `.on_submit(...)` uses the native Send action and submit event. Shift+Enter dispatches the
  native newline action through the ArkTS host, retaining selection and undo/redo; plain
  text areas keep multiline Enter behavior.
- **Canvas** — an `ARKUI_NODE_CUSTOM` node whose on-draw callback replays Day's display list
  with **OH_Drawing** (arcs, fills, strokes, rounded-rects, ellipses, text): the gauge + shapes pages.
  Day holds a Stack containing the custom surface and a transparent, unpadded button with an
  empty label. The button receives focus and keys for the canvas; the Stack lays out both
  siblings and owns accessibility. Keeping drawing outside the button avoids its native
  switch between text layout and a focus scope with no focusable children. This preserves
  drawing and `.focused(..)` after native layout (`canvas::create`; [focus](focus.md)).
- **Switch alignment** — Day clears the native Toggle's default outer margins. ArkUI adds
  theme hot-zone margins otherwise, moving a switch below and to the right of its assigned
  frame and into neighboring form rows. Native inspection uses the painted window position:
  the layout-position query substitutes the explicit position and hides this offset. The
  `labeled-nontext-controls-center` and `row-nontext-controls-center` conformance cases compare
  native control/label centers and spacing, including after value changes.
- **List** — an `ARKUI_NODE_LIST` driven by an `OH_ArkUI_NodeAdapter` with cell reuse, so a
  500-row list only builds the visible cells. Native row containers fill the viewport width,
  including selection and hit testing. Reloads and programmatic scrolling run outside Day's
  tree borrow, since either can synchronously request a row binding. Row jumps reveal with
  minimal movement; scrolling to the end aligns the last row with the viewport bottom.
- **Tabs** — an `ARKUI_NODE_SWIPER` pager with a dot indicator.
- **Dialogs and prompts** — `Cap::Dialogs` is native. The ArkTS `DayDialogs` host presents
  native ArkUI controls through `PromptAction.openCustomDialog`: centered alerts/prompts and
  bottom-aligned action sheets, including destructive/cancel roles and editable prompt text.
  Requests are queued and closed by request ID, with Back/outside cancellation and cleanup
  after disappearance. Content is a themed ArkUI builder, rather than Day-rendered overlay
  controls. See [dialogs](dialogs.md#arkui-presentation-and-lifetime) for lifecycle and
  primary-window ownership, and [testing](testing.md#native-arkui-dialog-tests) for real-input tests.
- **Gestures** — `.on_tap` (NODE_ON_CLICK) and `.on_drag` (a native pan recognizer via the
  NDK gesture API, reported on the shared kind-11 wire in px). The existing toolkit hook
  `GestureKind::LongPress` uses ArkUI's one-finger, non-repeating, 500 ms recognizer and emits
  `Event::LongPress` once on acceptance, with coordinates local to the native target in Day
  points. Normal priority preserves ArkUI's gesture arbitration with scrolling and dragging.
  A child long-press target takes precedence over an enclosing draggable list row inside the
  child's hit region; the rest of the row remains draggable. Do not cover a reorderable row
  with a competing long-press target if dragging must work across the whole row. Also,
  repeated registration for the same node/event target is idempotent, and node disposal
  detaches and releases the recognizer. This is low-level hook support: Day does not yet expose
  a public `.on_long_press` modifier, and this does not add native context menus. UIKit and
  Android likewise use long presses internally for menus/dragging without implementing this
  generic hook. See [native-input regression tests](testing.md#native-arkui-long-press-tests).
- **Scrolling** — `scroll()` is a real `ARKUI_NODE_SCROLL` whose day children live in a
  shim-owned container sized by `set_scroll_content`; without it the Scroll measures a content
  extent of 0 (day's content nodes are layout-only) and neither touch nor `scroll_to` moves.
- **Frame clock** — `OH_NativeVSync_RequestFrame` drives `day::frame`, with delivery
  marshaled to the JS/UI loop. Integer request tickets make cancellation and late callbacks safe;
  the native source is released when idle. See [frames](frames.md) for timing and lifecycle.
- **Animation** — `Cap::Animation` is `Native`. `set_opacity`, `set_transform`, and an
  animated container background run inside the NDK's `animateTo` (`anim::animate`),
  so ArkUI interpolates `NODE_OPACITY`, `NODE_TRANSLATE`/`NODE_SCALE`/`NODE_ROTATE` (about
  `NODE_TRANSFORM_CENTER`), and `NODE_BACKGROUND_COLOR` on its compositor. Easing curves map to
  ArkUI's built-in ones; a spring becomes a custom curve that samples day's own analytic spring
  over the duration, so it overshoots and settles like every other backend, nudged to end exactly
  on the target (ArkUI jumps to the target when a curve ends elsewhere). `apply` runs instantly
  when the node has no UI context yet. `set_frame` is not animated.
- **Label measurement** — a label measures on a detached, reusable Text probe
  (`node::measure_label`): after a Text's `NODE_TEXT_CONTENT` changes, ArkUI's `measureNode`
  keeps returning the previous text's size until its own layout pass runs (`markDirty` and a
  changed constraint don't clear it), so a readout that grew was laid out at its old width and
  wrapped. The probe resets its text and font attributes between measurements, avoiding repeated
  native node allocation. Measured widths reserve one physical pixel, capped by the proposal,
  because ArkUI truncates fractional widths in `getMeasuredSize`; using the truncated width can
  wrap a short label onto a clipped second line. Styled labels (spans) still measure directly.
- **Button measurement** — a title button measures a fresh copy (`node::measure_button`), for
  the stale-size reason labels do. An icon button (the Row `apply_button_content` inserts) is
  sized from that Row plus the capsule's padding, calibrated once from a fresh "M" button against
  a fresh "M" text: ArkUI measures the Button at its 32 vp minimum without regard to the Row.
  Never read `NODE_FONT_*` from a Button node: those getters are Text-model accessors and crash
  (`TextModelNG::GetFontFamily`).
- **Canvas images** — `DrawOp::Image` draws the decoded pixelmap with
  `OH_Drawing_CanvasDrawPixelMapRect`, preceded by a fully transparent rect: a custom node's
  recording whose only content is pixel map draws renders nothing.
- **Remote images** — [`day-piece-remote-image`](/docs/reference/images) has a native Image-node renderer, automatically
  enabled by its `arkui` feature metadata. Reactive PNG/JPEG bytes replace the displayed image;
  absent, empty, or invalid bytes restore the placeholder. Fit/fill scaling and rounded or
  centered circular clipping apply to both content and placeholder. Drawables are retained
  until replacement or node disposal. See [images](images.md#reactive-remote-image-views)
  for behavior, conformance coverage, and decoding limits.
- **Fullscreen cover** ([docs/cover](cover.md)) — `Cap::Cover` answers `Emulated`: the cover node is
  re-homed onto the window root at full bounds (no transition, no gesture dismissal).

- **ArkTS-built pieces** ([docs/extending](extending.md)) — a piece whose component exists only in ArkTS (the
  declarative `Web` has no ArkUI C node kind) ships its own `.ets`; `day build` stages it into the
  hvigor project and generates `entry/src/main/ets/daypieces/DayPieces.ets`, which the host page
  registers with the shim (`registerPiece`) before `start()`. `day-piece-webview` is the first user.
  **ArkWeb does not run on the x86_64 emulator**: the image's `ArkWebCore.hap` carries arm64-only
  native libs, so `libarkweb_engine.so` never loads (`GetNWeb: web engine is nullptr`) and the
  component's RosenWeb surface wedges the window's compositor for the rest of the process; every
  later frame goes stale. The showcase walkthrough therefore skips the web-view page on
  harmony-arkui; the arm path is unaffected.

Images/lottie/map pieces are not yet wired (they render as placeholders). Packaging + signing
run headlessly via the command-line-tools (see Build & run) with no Huawei developer account: hvigor
assembles the `.hap` and `sign-hap.mjs` patches (compileSdkType → OpenHarmony) + signs it with the
public release material. `libentry.so` links the NDK's shared libc++ and `libnative_drawing`, both
packed into the `.hap`.

## CI

The `harmony-arkui` job in `.github/workflows/ci.yml` runs on every push/PR (`ubuntu-24.04`). The build
gates **hard** (clippy + cross-compile + `hvigorw assembleHap` + sign + `day doctor`); only the
emulator boot + walkthrough are best-effort (`continue-on-error` per step) because the GitHub-hosted
TCG emulator is slow and occasionally flaky. It downloads + caches the OpenHarmony
**command-line-tools** (~2 GB) and then runs the real build pipeline:

1. clippy the backend for the OHOS target, and cross-compile the full Day app to `libentry.so` for
   the emulator (x86_64) and device (arm64), both through cargo-ohos (pinned, installed with
   install-action) against the Linux SDK from setup-ohos-sdk, whose `native` component the job
   hands over as `OHOS_SDK_NATIVE`;
2. `ohpm install`, then `hvigorw assembleHap`, a full hvigor build of the ArkTS host + `.hap`;
3. patch + sign the `.hap` with `sign-hap.mjs` (compileSdkType → OpenHarmony + the public release
   material), then install/launch it on the Oniro emulator over `hdc` and drive the dayscript
   walkthrough, uploading screenshots for the gallery, like the other targets. CI boots the emulator
   with `day devices boot -t harmony-arkui --headless` (the same Oniro v6.1 image openharmony-rs's
   emulator-action uses) rather than the action itself: the action's QEMU command has **no GPU
   device** (`-nographic`), so the guest has no display: the keyguard never dismisses, `aa start`
   is refused with error 10106102, and screenshots capture nothing. Day's launcher adds
   `-device virtio-gpu-pci` with `-display none`: a headless framebuffer the apps can foreground on
   and `uitest screenCap` can capture.

Declaring the app OpenHarmony (via the compileSdkType patch) lets it install: the emulator
enforces app code signing but doesn't trust the public cert, and OpenHarmony's BMS skips code-sign
verification for OpenHarmony-declared apps on devices without Huawei OH code signing.

The whole job (build gates and emulator screenshots) runs on a single **x86_64 Linux** runner,
where `/dev/kvm` accelerates the x86_64 Oniro guest (under TCG only the macOS ARM hosts ever
booted it fast enough, which is where the job used to run). Build steps gate hard; only the
emulator boot + walkthrough are per-step best-effort. The setup replicates the local flow:
cargo-ohos drives the cross-compile, the Linux command-line-tools (hvigor/ohpm are pure JS) run
through node wrapper scripts, the API-18 SDK from setup-ohos-sdk supplies the NDK + hdc + the
versioned `OHOS_BASE_SDK_HOME` view hvigor builds against, and Day's QEMU launcher boots the
image. `OHOS_BASE_SDK_HOME`
must be a **host-platform** SDK: hvigor spawns its native tools (`syscap_tool`, `restool`,
`es2abc`) directly, so pointing it at the Linux CLT's bundled SDK fails on macOS with
`spawn ENOEXEC` at `SyscapTransform`.

The scripted channel depends on the following facts, each of which was a silent total failure
until diagnosed:

- **The default hdc forward port 55555 is often already occupied**, since GitHub's macOS runners
  hold it, and so do some local services; QEMU then dies instantly ("Could not set up host
  forwarding rule"), leaving no reachable target and blank screenshot sets. `day devices boot -t harmony-arkui` probes and slides to the first free port, `tconn`s the chosen key (so device discovery
  finds it), and exports `DAY_OHOS_TARGET` through `GITHUB_ENV` so later CI steps target it too.

- **`ohos.permission.INTERNET` is required for the loopback dayscript socket**; without it in
  `module.json5` the engine's `TcpListener::bind` fails silently and no scripted run can ever
  connect (the Android manifest needs the same permission for the same reason).
- **The ability is a singleton**: a second `aa start` foregrounds the existing process, whose
  engine (if any) listens on the previous run's port. `day launch` force-stops the bundle
  before every start so each run's engine params take effect.
- **The keyguard returns whenever the display sleeps** and `aa start` is refused while it shows;
  `day launch` wakes + swipe-unlocks (uitest + uinput) around every launch retry.
- **ArkUI Navigation diffs the old and new stacks by destination name**, so a same-turn pop+push
  of two destinations with the same generic name collapses into "page is not change — don't
  transition", leaving the old (already-detached, empty) destination on screen for good. The
  host page pushes each Day page under a unique `day-page-<key>` name, and dayscript-driven
  sessions push/pop **without** transition animation (ArkUI drops an operation issued across an
  in-flight transition; scripted bursts outrun one).
- **`uitest screenCap` trails the UI tree**, because the TCG guest's RenderService composites a
  pushed page seconds after it has laid out, so a shot taken right after a navigation shows the
  previous page. The runner waits for `ui_idle` (the pushed destination's first area report),
  sleeps `DAY_OHOS_SHOT_SETTLE_MS` (default 4000) before capturing, and re-captures when a shot
  comes out byte-identical to the run's previous one (the first push after app start can lag
  past the settle while the render tree warms up).

## Follow-ups

Wire the remaining pieces (image / lottie / map), richer accessibility (roles beyond the
label), interactive tab-bar labels on the swiper, and the image/webview backends. The dayscript
engine's TCP channel is flaky on the TCG emulator (occasional connection resets), so scripted
walkthrough screenshots on OHOS are best-effort. Canvas fonts and the font list
([docs/fonts](fonts.md)) compile against `OH_Drawing_FontMgr` and are verified by CI on a
device. A family the ability registered through `font.registerFont` is NOT visible to
`OH_Drawing_FontMgrMatchFamilyStyle` (the 2026-09 showcase drew canvas text in the system face
while its labels were right), so the toolkit builds a typeface from each bundled font's bytes
and the canvas draws from that.
