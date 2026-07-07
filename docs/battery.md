# Battery (headless capability crate)

> **Status: implemented** as `day-part-battery` (in `parts/`, the headless counterpart of `pieces/`) — a **headless** day-ecosystem crate (no UI Piece): a shared
> cross-platform API for reading the device battery through each platform's NATIVE API. Any Rust code
> can depend on it and call `day_part_battery::status()`. Verified on macOS (real battery), the iOS
> simulator, and the Android emulator; HarmonyOS cross-compiles + links against the native
> `libohbattery_info.so`.

## Authoring

```rust
if let Some(b) = day_part_battery::status() {
    println!("{:?} · {}%", b.state, b.percent().unwrap_or(0));
}
```

`status() -> Option<BatteryStatus>` returns `None` where there's no battery API (or no battery).
`BatteryStatus { level: Option<f32>, state: BatteryState }`; `level` is `0.0..=1.0` (`percent()` gives
`0..=100`); `BatteryState` is `Charging | Discharging | Full | NotCharging | Unknown`.

There are **no features** — platform selection is purely `#[cfg(target_os)]`, because a battery is an OS
concern, not a widget-toolkit one. `parts/day-part-battery/examples/battery.rs` is a plain `main` that uses
it with no Day framework at all.

## Per-platform native realization

| OS | API | dependency |
|---|---|---|
| macOS | IOKit `IOPowerSources` (CoreFoundation) | `core-foundation`, IOKit framework |
| iOS | `UIDevice.batteryLevel` / `batteryState` | `objc2-ui-kit` |
| Windows | `GetSystemPowerStatus` | raw FFI (kernel32) |
| Linux | `/sys/class/power_supply` | std only |
| HarmonyOS | native `OH_BatteryInfo_GetCapacity` / `GetPluggedType` (`libohbattery_info.so`) | raw FFI (BasicServicesKit) |
| Android | `BatteryManager` via a Java shim | `day-android` + `[package.metadata.day.android]` |

iOS reads on the main thread (`UIDevice` is `MainThreadOnly`); off it, `status()` returns `None`. The
simulator has no battery → `level: None, state: Unknown` (the API path still runs).

HarmonyOS is `target_os = "linux"` but sandboxes `/sys` away, so it's gated on `target_env = "ohos"`
and uses the native BasicServicesKit C API instead — pure FFI, needing neither a permission nor the
Day runtime (unlike Android). The native API exposes capacity + plugged type but no explicit charge
state, so the state is inferred from whether external power is connected. The `day-arkui-demo` app
shows a live readout (docs/harmonyos.md).

## What it shows about the extension system

This is the first **headless** external crate — no UI Piece, nothing registered into any backend's
`RENDERERS` slice. It demonstrates that the standalone-piece **backend-contribution** mechanism (see
[extending.md](extending.md)) already accommodates headless capability crates: `day-part-battery` contributes
its Android Java through `[package.metadata.day.android]` exactly like the UI pieces, but registers no
renderer — the Java staging is independent of rendering. On Android the crate rides on the Day runtime
(it uses day-android's cached JVM + `DayBridge.ctx`); on every other platform it is fully day-independent.
