---
navTitle: "Spinners"
title: "Activity spinner"
description: "Show indeterminate progress, change its running state, and choose its size."
---

<!-- Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0 -->

# Activity spinner

Use [`spinner()`](https://daybrite.dev/api/day_pieces/fn.spinner.html) when work has no known
completion percentage. It is part of the core [progress API](/docs/reference/progress) and
available through `day::prelude::*` without an additional dependency.

```rust
let running = Signal::new(true);
spinner().animating(running).large()
```

[`animating`](https://daybrite.dev/api/day_pieces/trait.ProgressBuilder.html#tymethod.animating)
accepts a Boolean, signal, or closure. It defaults to `true` and changes without rebuilding the
view. Stopping preserves its layout space. Apple and GTK show a stopped indicator; Android and
ArkUI can hide the indicator while retaining its space. Qt uses a busy progress bar and stops
it by selecting a fixed range. The browser pauses its CSS animation.

[`large`](https://daybrite.dev/api/day_pieces/trait.ProgressBuilder.html#tymethod.large) selects
the platform's larger indicator. A determinate [`progress(fraction)`](https://daybrite.dev/api/day_pieces/fn.progress.html)
uses the same `Progress` type; animation and size options apply to indeterminate indicators.

| Platforms / toolkits | Representation |
| --- | --- |
| AppKit, UIKit, GTK, Android, WinUI, ArkUI | Native activity indicator. |
| Qt | Native progress bar in busy mode. |
| Web | Animated browser element. |

The implementation lives in each toolkit's `controls/spinner.rs` and the core progress
protocol. There is no separate activity crate or activity node kind.
