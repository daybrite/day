---
title: "Pager"
description: "Retain page identity while selecting and reordering pages."
---

<!-- Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0 -->

# Pager

A pager presents related content as separate pages within the same area. It suits collections where
people move between distinct items or steps while keeping each page's content together.

Use [`pager`](https://docs.rs/day-pieces/latest/day_pieces/pager/fn.pager.html) to keep a fixed set
of page slots alive while changing which page is selected.

## Add to your app

This control is included in [`day-pieces`](https://github.com/daybrite/day/tree/main/crates/day-pieces)
and re-exported through `day::prelude::*`. No additional crate or control feature is required.

## Selection and lifetime

[`pager(state, busy, pages, settled)`](https://docs.rs/day-pieces/latest/day_pieces/pager/fn.pager.html) takes two signals, a vector of `AnyPiece` pages, and a
callback. [`PagerState`](https://docs.rs/day-spec/latest/day_spec/controls/pager/struct.PagerState.html)
contains the ordered slot identities, selected slot, and presentation epoch. The order indexes
the fixed page vector. Changing the order or selection preserves the pages' reactive scopes.

On UIKit, Android, and ArkUI, native paging gestures update `busy` and call the supplied callback with the settled slot
when a page settles. Increment the epoch when changing the presentation; stale completions
are rejected. Defer replacing slot contents until `busy` is false.

Other toolkits use a composed container. Only the selected slot in the active order is shown;
inactive pages remain mounted, with their native subtree hidden from input and accessibility.
This fallback supports programmatic selection but supplies no swipe gesture.

[`day::pager::support()`](https://docs.rs/day-pieces/latest/day_pieces/pager/fn.support.html) reports `Native` or `Emulated` after the runtime starts. A dayscript
`select` event uses the logical position in the current order, while settled callbacks use the
stable slot identity. Applications should use the callback to update their selected state.

## Progress

```rust
let progress = Signal::new(0.0);
column((
    pager(state, busy, pages, settle).progress(progress),
    page_dots(pages_len, progress),
))
```

[`.progress(binding)`](https://docs.rs/day-pieces/latest/day_pieces/pager/trait.PagerBuilder.html#tymethod.progress)
follows the pager's position in reading order, in pages: `0.0` is the first page and `1.5` is
halfway from the second to the third. A page indicator drawn from it tracks the finger.

Where `Cap::PagerProgress` is `Native`, the pager writes the position continuously while a
page is dragged, at most once per display frame. Everywhere else the value steps to the
selected page's position when the selection changes, so the same indicator still shows the
right page.

| Toolkit | Progress |
|---|---|
| [UIKit](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit") | `Native`: the page view controller's own scroll view, observed |
| [Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material") | `Native`: the pager's `onPageScrolled`, once per frame and once at rest |
| [ArkUI](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui") | `Native`: the swiper's content-scroll event, a whole page when it settles |
| [AppKit](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit") | steps with the selection (composed pager) |
| [GTK](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk") | steps with the selection (composed pager) |
| [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") | steps with the selection (composed pager) |
| [WinUI](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") | steps with the selection (composed pager) |
| [Web](https://daybrite.dev/docs/reference/platforms/web-dom "toolkit:web-dom") | steps with the selection (composed pager) |

The [navigation guide](/docs/reference/navigation) covers route and back-stack presentation.
A pager does not create routes or manage navigation history.
