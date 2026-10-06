---
title: "Haptics"
description: "Haptic feedback via day-part-haptics: impact styles per platform, and where haptics are silently unavailable."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Haptic feedback (headless capability crate)

> **Status: implemented** as `day-part-haptics` (in `parts/`, the headless counterpart of
> `pieces/`). It's a **headless** day-ecosystem crate (no UI Piece): a shared cross-platform API for
> playing haptic feedback through each platform's native API. Any Rust code can depend on it and call
> `day_part_haptics::play(Haptic::Success)`. Verified clippy-clean on macOS (real AppKit
> `NSHapticFeedbackManager`), the iOS Simulator target, Android (Rust side), and the HarmonyOS target.

## Authoring

```rust
use day_part_haptics::Haptic;

if day_part_haptics::is_supported() {
    day_part_haptics::play(Haptic::Success);
}
```

`play(Haptic)` is **fire-and-forget**: it returns nothing, never blocks, never errors, and never
panics. `is_supported() -> bool` reports whether the platform has a haptic engine wired here (it is
API availability, not a live hardware probe; a Simulator still reports `true`). Where support is
`false`, `play` is a no-op.

`Haptic` is modeled on iOS's three feedback-generator families so the same call maps to a native
pattern everywhere:

| Style | Meaning |
|---|---|
| `Light` / `Medium` / `Heavy` | physical *impact* intensities |
| `Success` / `Warning` / `Error` | *notification* outcomes |
| `Selection` | the light tick as a value scrolls past a detent |

There are no cargo features; platform selection is purely `#[cfg(target_os)]`, because a haptic
engine is an OS concern rather than a toolkit one. `parts/day-part-haptics/examples/haptics.rs` is a
plain `main` that uses it with no Day framework at all.

## Per-platform native realization

| OS | API | dependency |
|---|---|---|
| iOS | `UIImpactFeedbackGenerator` / `UINotificationFeedbackGenerator` / `UISelectionFeedbackGenerator` | `objc2` + `objc2-ui-kit` |
| macOS | `NSHapticFeedbackManager.defaultPerformer` (Force Touch trackpad) | `objc2-app-kit` |
| Android | `Vibrator` / `VibrationEffect` via a Java shim | `day-android` + `[package.metadata.day.android]` |
| HarmonyOS | `vibrator.startVibration` (Sensor Service Kit) via an ArkTS arm | `day-bridge` + `[package.metadata.day.ohos]` |
| Windows · desktop Linux (GTK/Qt) | — (no haptic engine wired) | none (no-op, `is_supported() == false`) |

## How each platform realizes the styles

- **iOS**: a direct one-to-one mapping. Light/Medium/Heavy pick the matching
  `UIImpactFeedbackStyle`; Success/Warning/Error pick the matching `UINotificationFeedbackType`;
  Selection calls `selectionChanged()`. The generators are `MainThreadOnly` and are `prepare()`d
  before firing to minimize latency; day runs on the main thread, so a call off it is a safe no-op.
  The Simulator has no Taptic engine, so the calls are silently ignored there.
- **macOS**: `NSHapticFeedbackManager` offers only three patterns, so the seven styles fold onto
  them: Light/Selection → `Alignment` (the subtlest snap), Medium/Heavy → `LevelChange` (a firmer
  detent), Success/Warning/Error → `Generic`. A Mac without a Force Touch trackpad feels
  nothing; the call is harmless.
- **Android**: a Java shim (`DayHaptics.java`) resolves the `Vibrator` (via `VibratorManager` on
  API 31+), and on API 29+ plays a predefined `VibrationEffect`: `EFFECT_TICK` (Light/Selection),
  `EFFECT_CLICK` (Medium), `EFFECT_HEAVY_CLICK` (Heavy/Warning), `EFFECT_DOUBLE_CLICK`
  (Success/Error). Older APIs fall back to a short one-shot buzz whose length stands in for
  intensity. Requires `android.permission.VIBRATE`, a normal install-time permission the crate
  contributes to the manifest itself (see below).
- **HarmonyOS**: an ArkTS daybridge arm ([docs/bridge.md](bridge.md)) plays the system's own haptic presets
  through the Sensor Service Kit's `vibrator`: `haptic.effect.soft` (Light), `haptic.clock.timer`
  (Medium/Selection), `haptic.effect.hard` (Heavy/Warning/Error), `haptic.effect.sharp` (Success).
  A device without a preset gets a timed buzz whose length stands in for strength, like Android's
  pre-API-29 fallback. The C vibrator API takes only durations, which is why the arm is ArkTS.
  Requires `ohos.permission.VIBRATE`, a `system_grant` permission the crate contributes through
  `[package.metadata.day.ohos] permissions`. The OpenHarmony emulator has no vibrator, so the call
  is accepted and nothing is felt.

## What it shows about the extension system

Like `day-part-battery` and `day-part-network`, this is a headless external crate: it has no UI
Piece and registers nothing into any backend's `RENDERERS` slice. It exercises the
**manifest-permission overlay** ([docs/extending.md](extending.md)): `[package.metadata.day.android]` stages its own
Java shim and contributes `android.permission.VIBRATE`, which `day build` merges into the app
manifest with no edits to any core day crate. On Android the crate rides on the Day runtime
(day-android's cached JVM + `DayBridge.ctx`); on every other platform it is fully day-independent.
