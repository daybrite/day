# day-part-wakelock

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

Keep the screen on while something is showing: a timer, a recipe, a game round played with the
phone held up.

```rust
let lock = day_part_wakelock::keep_screen_on();
// The screen stays on until the lock is dropped.
drop(lock);
```

Locks nest, and the screen may sleep again once the last one is dropped.

| Platform | Mechanism |
|---|---|
| iOS | `UIApplication.isIdleTimerDisabled` |
| macOS | an IOKit power assertion |
| Windows | `SetThreadExecutionState` |
| Android | `FLAG_KEEP_SCREEN_ON` on the activity's window |
| HarmonyOS | `Window.setWindowKeepScreenOn` |
| web | the Screen Wake Lock API |
| Linux | not yet |

See [docs/wakelock.md](../../docs/wakelock.md).
