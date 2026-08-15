---
title: "Size classes"
description: "Compact to expanded: how day re-presents navigation when the window crosses a size-class boundary."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Size classes and re-presenting navigation

A window's width decides how much can be on screen at once. A `selector` in a wide window shows
its list beside the selected page; the same selector in a narrow one shows the list, then pushes
the page over it. Day resolves that from the window's **size class** and re-resolves it whenever
the class changes, so one `selector` is right on a desktop, a tablet, a phone, and a browser
window someone is dragging narrower as they read.

## The breakpoints

`SizeClass` buckets a window's size in points. The numbers are Android's window size classes, used
verbatim on every backend:

| `WidthClass` | points | typically |
|---|---|---|
| `Compact` | `< 600` | phone portrait |
| `Medium` | `600–839` | tablet portrait, a narrow desktop window |
| `Expanded` | `840–1199` | tablet landscape, a typical desktop window |
| `Large` | `1200–1599` | a large desktop window |
| `ExtraLarge` | `≥ 1600` | maximized on a big display |

| `HeightClass` | points | typically |
|---|---|---|
| `Compact` | `< 480` | phone landscape |
| `Medium` | `480–899` | phone portrait, tablet landscape |
| `Expanded` | `≥ 900` | tablet portrait, most desktop windows |

One table across every backend means one answer: a 700pt window is `Medium` on a Mac, in a
browser, and on a tablet, so an app that lays out from the class gets the same layout at the same
size everywhere. Apple publishes two buckets rather than five; those map onto this table instead
of replacing it, with `Compact` the compact one and everything wider regular.

Apps read the class with `day::size_class()`. The read is tracked, so a piece that lays out from
it rebuilds when the window crosses a breakpoint. It answers `None` on a backend that reports no
window geometry.

```rust
let two_up = day::size_class().is_some_and(|c| c.width >= WidthClass::Expanded);
```

## Per-window, not per-app

The class belongs to a **window**. One process can show two windows in different classes at the
same moment — a narrow window beside a wide one, iPadOS Stage Manager, Android split-screen — and
an app keyed off a single global would lay the second window out for the first one's size. The
signal is keyed by window root, alongside the safe-area insets, and both scope the way toolbars do
(§`day_core::toolbar`): a read during a window's content build means *that* window.

The value is derived in day-core from `Event::WindowResized`, which every backend already emits.
A toolkit reports geometry; it never reports a class. That is what keeps the breakpoint table in
one place rather than nine.

## What a nav host does with it

A `selector` presents as `Split` (list beside detail) or `Stack` (one page at a time,
back-navigable). Left alone it resolves that automatically:

```rust
selector(section)                       // automatic: follows the window
selector(section).presentation(NavPresentation::Split)   // pinned
```

Resolution answers three questions in order:

1. Can this toolkit draw split panes at all (`Cap::NavSplit`)? If not, `Stack`, always.
2. Did the app pin one? If so, that.
3. Otherwise: `Split` when the window is wider than compact.

Pin one when the content only works one way — a settings sidebar whose detail is meaningless on
its own, a wizard that has to stay a stack. A pin is still a preference: a toolkit with no split
container stacks whatever it is asked for.

### Re-presenting, not rebuilding

When the class crosses a breakpoint the host is **re-presented**: `NavPatch::Presentation` tells
the toolkit to rebuild its own chrome and re-home the pages it already has. No page is torn down
and rebuilt, which is the whole point — a rebuild would drop every scroll offset, text selection,
and focused field, and would restart any animation in flight.

That works because a page's `Pane` is a fact about the model rather than about the current
drawing. A selector's list page is `Pane::Sidebar` whether the host is split or stacked; what the
presentation decides is where the pane lands:

| pane | `Split` | `Stack` |
|---|---|---|
| `Sidebar` | its own splitter pane | the root of the stack |
| `Detail` | the detail pane | pushed above the root |

Selection is carried across, with one asymmetry:

- **Narrowing** keeps the selection. The detail simply becomes the top of the stack, which is
  where the user already was.
- **Widening** with nothing selected picks the first item, because a split presentation has no
  way to draw an empty detail pane. This is the same rule the initial build uses.

## Per-backend support

`Cap::NavRepresent` says whether a toolkit re-presents a live host. It gates more than the patch:
on `Unsupported`, presentation resolves from `Cap::NavSplit` alone and the window's size never
enters into it. A toolkit that cannot change its presentation must not have it decided by
something that changes underneath, or a window launched narrow would be stuck stacked with no way
back.

| backend | `NavSplit` | `NavRepresent` | notes |
|---|---|---|---|
| web-dom | ✅ | ✅ | the shim rebuilds chrome; page elements move between containers intact |
| macos-appkit | ✅ | ✅ | one `NSSplitViewController` either way — a stack is that split with its sidebar item collapsed |
| linux-qt / windows-qt / macos-qt | ✅ | ✅ | one `QSplitter` either way, back header installed in both |
| linux-gtk / windows-gtk | ✅ | — | see below |
| windows-xaml | ✅ | — | a `NavigationView` owns its own `PaneDisplayMode`; re-presenting means driving that rather than re-homing pages |
| ios-uikit | ✅ | **observed** | `UISplitViewController`, both columns navigation controllers |
| android-mdc | ✅ | **observed** | `SlidingPaneLayout`, list pane beside a detail pane |
| harmony-arkui | — | — | `NavigationMode.Auto` pending; see below |

GTK is the odd one out among the desktops. Everywhere else both presentations are the same
container with different chrome, so a morph re-homes pages inside a host Day already holds. On
GTK they are different WIDGETS — `AdwOverlaySplitView` for the split, `AdwNavigationView` for the
stack — and Day holds the host handle, so it cannot be swapped underneath. The route there is
`AdwNavigationSplitView` and its `collapsed` property, which is exactly the GNOME adaptive idiom
(it is what an `AdwBreakpoint` drives in Nautilus and Text Editor). That is a restructure of a
working backend rather than an addition to it.

> [!NOTE]
> The mobile backends run the **opposite** policy from the desktop ones, which is what
> `NavRepresent = Emulated` records. Their platforms ship adaptive containers that already do this
> morph natively, with the right animation and the right gesture: `UISplitViewController` collapses
> and expands on its own as a Pro Max iPhone rotates between compact and regular width, and
> `SlidingPaneLayout` decides at MEASURE time whether both panes fit. Day observes and reconciles;
> it never pushes a presentation at them, because that would be a second source of truth racing
> the platform's own animation.

One lowering rule falls out of this policy: an `Emulated` toolkit's adaptive host is lowered
with `presentation: Split` — "build the adaptive container" — even when the window is compact at
build time, because the container collapses itself. `Stack` in `NavProps` is thereby literal: it
marks a host that is a stack at *every* size (a pinned request, or the nested `stack()` piece
under a split host), and the backend realizes it as a plain navigation controller.

Traps worth knowing, all found the hard way and all silent:

- **iOS.** The split's PRIMARY column must be a `UINavigationController`, not a bare view
  controller. UIKit merges the secondary column *into the primary's stack* when it collapses —
  with nothing to merge into, it drops the navigation bar entirely on phones and breaks
  first-responder handling, so `becomeFirstResponder` fails quietly. Which controller owns the
  stack therefore depends on the presentation.
- **iOS, again.** Day-initiated stack changes are ONE `setViewControllers:animated:` derived
  from the mirrored stack, never incremental push/pop calls. A selection change is a pop AND a
  push; issued separately while the first still animates, `viewControllers` reports a transient
  state and any decision read from it (a count, a top page) can wipe or double an entry — the
  atomic set is idempotent, so however calls interleave the last one applies the final model.
- **iOS, a third time.** Never *animate* to an empty stack (deselecting in the expanded split
  empties the detail column): with no destination controller the transition sets up but never
  completes, the stack keeps its old contents, and the orphaned transition coordinator reports
  busy forever. Pass `animated: false` when the target is empty.
- **Every toolkit with an adaptive container.** Never nest one inside a pane. A
  `UISplitViewController` assumes it owns the window; embedded in a detail column its column
  layout collapses into garbage. This is what the `Stack`-is-literal lowering rule exists for —
  the nested host realizes as a plain navigation controller instead.

## Testing it

dayscript's `size_class:` step reports a class the way a backend would, without resizing anything:

```yaml
- size_class: { width: compact }              # height defaults to `expanded`
- assert_visible: { id: nav-list }
- size_class: { width: expanded, height: medium }
```

Everything downstream runs its real path — the host re-presents, a piece reading
`day::size_class()` rebuilds. What it does not change is the window's pixels, so a screenshot
after this step shows the new layout at the old size. Where the geometry itself is under test,
drive a real resize from the runner instead: Playwright's `setViewportSize` on web, the
simulator's rotation on iOS.

The assertion worth writing is the two-part one: that the presentation changed **and** that the
state survived. A morph that silently drops the selected section passes a naive screenshot check
and fails the only thing this feature promises.
