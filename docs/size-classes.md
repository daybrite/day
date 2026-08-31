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

A `selector` presents as `Split` (list beside detail), `Stack` (one page at a time,
back-navigable), `Tabs` (the rows as a tab bar) or `Rail` (the rows as a narrow strip). Left alone
it resolves that automatically:

```rust
selector(section)                       // automatic: follows the window
selector(section).presentation(NavPresentation::Split)   // pinned
```

Resolution answers four questions in order:

1. Did the app pin one? If so, that — clamped to something the toolkit can draw.
2. Can this toolkit draw split panes at all (`Cap::NavSplit`)? If not, it stays single-pane.
3. Which STYLE is it? `Tabs` is a tab bar at every size; `Sidebar` is the `Split` ↔ `Stack`
   ladder this document has always described.
4. `Automatic` walks the full ladder: `Split` when expanded, `Rail` at medium, and when compact
   either `Tabs` or `Stack` — `Cap::NavTabsAdaptive` decides which, because growing a tab bar
   from a narrowed window is idiomatic on the phones and the web and is not on any desktop
   ([docs/navigation.md](navigation.md)).

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

| pane | `Split` | `Stack` | `Tabs` | `Rail` |
|---|---|---|---|---|
| `Sidebar` | its own splitter pane | the root of the stack | not drawn — the rows ARE the tab bar | not drawn — the rows are the rail |
| `Detail` | the detail pane | pushed above the root | the tab's content area | the content beside the rail |

`Tabs` and `Rail` differ only in where the rows are drawn, which is why they share a code path in
the pieces layer and in every backend: both hide the sidebar PAGE and render its rows as chrome.

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

## Resizable windows on the phones

Both mobile platforms now resize app windows freely, and both vendors have stopped treating it as
optional. Android 16 (API 36) **ignores** `screenOrientation`, `resizeableActivity`,
`minAspectRatio` and `maxAspectRatio` on any display 600dp or wider — tablets, open foldables, and
desktop windowing on every form factor — and the temporary opt-out property
(`PROPERTY_COMPAT_ALLOW_RESTRICTED_RESIZABILITY`) is removed in API 37. iPadOS 26 deprecated
`UIRequiresFullScreen` and made every iPad window resizable; iOS 27 extends that to iPhone apps on
iPad and to iPhone Mirroring on the Mac.

Day needed little for this, because the class is **derived in day-core from
`Event::WindowResized`** and every backend already emitted one. What it needed was for that
geometry to be the window's own, and for the app to still be there afterwards.

### Android: survive the resize

Entering split-screen, or dragging a desktop-windowing edge across a size bucket, changes
`screenLayout` and `smallestScreenSize`. An activity that has not claimed those in
`android:configChanges` is **destroyed and recreated** — and day-android does not survive a second
`nativeStart` in one process, so the app comes back missing whatever it installed once at startup.
The scaffold's manifest claims them, `day lint` fails a manifest that does not
(`day::lint::android-not-resizable`), and `day::android::start` refuses a second launch with a
message naming the fix rather than half-relaunching into an undiagnosable state.

The `<layout>` element carries the window's minimum and its desktop-windowing default, and
`PROPERTY_SUPPORTS_MULTI_INSTANCE_SYSTEM_UI` (API 35+) is what lets the system UI offer a second
window for an app that supports one. `resizeableActivity` is deliberately not declared: it already
defaults to true at `targetSdk` 24+, and API 36 ignores it.

### iOS: measure the scene, not the screen

`UIScreen`'s bounds are the display. A scene has not filled the display since iPad multitasking,
and on iPadOS 26 it usually does not even at launch — measured on an iPad Pro 13-inch, a Day app
opens into a **635×1376pt window on a 1032×1376pt screen**. Sizing the window from the screen made
the first size class wrong by 397 points, so a nav host resolved a three-column split for a window
that had room for two and then corrected itself once the first layout pass ran.

day-uikit takes its launch geometry from the scene's own coordinate space —
`effectiveGeometry.coordinateSpace` on iOS 26+, the direct property below it, the same value either
way. `DayHolderView` reports every later change against **its own scene**, which matters as soon as
there are two windows: reporting a secondary's geometry against the primary re-framed the wrong
window's root view and re-bucketed the wrong window's class.

### Declaring a minimum size

One declaration, two layers — Android wants it in the manifest at build time, iOS wants it at run
time:

```toml
[window]
width = 960          # also the desktop-windowing default size
height = 640
min_width = 320
min_height = 400
```

`day build` writes the `<layout>` element's attributes and the iOS `Info.plist` keys day-uikit
reads back for `UIWindowScene.sizeRestrictions`. An app that sets `WindowOptions.min_size` in Rust
still wins over both. On iOS the minimum is a **preference the system satisfies on a best-effort
basis** — laying out sensibly at whatever size arrives is still the app's job.

### What still works on an older OS

Nothing here raises a floor. Android's `minSdk` stays 24 and everything used is API 24+ except the
multi-instance `<property>` (API 35+, and older platforms skip unknown tags). iOS deploys to 15.0
and needs no gated API on the critical path, because the class comes from geometry rather than from
UIKit traits: the scene's coordinate space reads the same from iOS 15 to iOS 27. The one API
newer than the floor — `UITabBarController.mode`, annotated `ios(18.0)` — is guarded on
`respondsToSelector:` rather than on a version number, which is the fact the call actually depends
on. It turns out to respond on iOS 15.5 and 17.5 as well; gating it on the version instead made
those releases *worse*, because the resolver's fallback lowers a different host shape.

## Row fit policies

A `row` keeps its children on one line no matter what. That is the right contract for a label
beside a value, and the wrong one for five buttons on a phone: the line overflows the window and
the tail lands offscreen, still green under every dayscript assertion because the synthetic rail
does not hit-test. `.fit(RowFit::…)` names what should happen instead — same children, same
call shape, four answers:

```rust
row((a, b, c)).spacing(8.0)                                  // RowFit::Clip, the default
row((chips,)).spacing(8.0).fit(RowFit::Wrap { run_spacing: 8.0 })
row((keys,)).spacing(8.0).fit(RowFit::WrapColumns { run_spacing: 8.0 })
row((label, control)).fit(RowFit::ColumnAt(WidthClass::Compact))
row((chips,)).spacing(8.0).fit(RowFit::Scroll)
```

`Clip` is the default: one line at natural sizes, and whatever does not fit lands offscreen. In
debug builds the engine logs the overflow once per container, naming the dayscript ids in reach
(`day layout: children overflow their container …`), so the silent version of this failure no
longer exists. Release builds skip the check entirely.

`Wrap` breaks onto additional lines where the next child would overflow, like wrapped text — the
shape a chip row, a button strip, or a tag cloud wants. Lines are `run_spacing` apart, and
children align within their line via `.align(VAlign::…)`. Wrapping replaces main-axis
negotiation, so `.grow()` and `spacer()` are inert, and a single child wider than the window
still overflows.

`WrapColumns` wraps the same way but into aligned COLUMNS: every cell takes the widest child's
width, and each line holds as many as the window fits. `Wrap` keeps each child at its natural
width, so the lines come out ragged — right for chips of unequal weight, wrong for a set of
peers that should read as a grid (a keypad, a palette, a row of equal choices):

```
Wrap          [Item 1][ Item 2 ][Item 3][ Item 4 ]     WrapColumns   [ Item 1 ][ Item 2 ][ Item 3 ]
              [ Item 5 ][Item 6][ Item 7 ]                           [ Item 4 ][ Item 5 ][ Item 6 ]
```

The column count follows the available width, so it re-flows as the window changes. An
authored, fixed column count with per-cell spans is a different job — that is [`grid`](grid.md),
whose children are rows rather than items.

`ColumnAt(class)` re-arranges the row into a leading-aligned column while the window's width
class is at or below `class` — the shape a label-plus-control-plus-result line wants, where
wrapping members independently would tear apart what reads as one sentence. The `size_class()`
read is tracked, so crossing the breakpoint re-arranges it live; app state lives in signals and
survives the rebuild.

`Scroll` keeps the single line and makes it a horizontal scroll strip: one row tall, filling the
width it is given, with the tail a swipe away instead of gone. The policy for rows whose order
matters more than their visibility — a timeline, a filmstrip, a rail of shortcuts.

The showcase's Layout page renders one row under each policy with a live component count, which
is the quickest way to feel the difference.

## Testing it

dayscript's `size_class:` step reports a class the way a backend would, without resizing anything:

```yaml
- size_class: { width: compact }              # height defaults to `expanded`
- assert_visible: { id: nav-list }
- size_class: { width: expanded, height: medium }
- size_class: { width: auto }                 # back to what the window reports
```

Everything downstream runs its real path — the host re-presents, a piece reading
`day::size_class()` rebuilds. What it does not change is the window's pixels, so a screenshot
after this step shows the new layout at the old size.

Where the geometry itself is under test, `resize:` moves it:

```yaml
- resize: { width: 1100, height: 900 }        # real points
- assert_visible: { id: content-list }
- resize: auto                                # back to the device's own geometry
```

The runner performs the resize and the engine half waits until the app has reported the new class,
so the next step cannot race the platform's resize animation. It is asserted as a **width class**:
what reaches day-core is the safe-area-inset CONTENT size, so a window resized to 900pt tall
reports about 830 once the status bar, the navigation bar and the app bar come out — and width is
what every re-presentation decision reads anyway. Aim for mid-bucket sizes; a width within a few
points of a breakpoint can fall the other side of it once insets are taken out.

Only android-mdc has a host-side lever today (`adb shell wm size`, which is also what delivers the
configuration change the manifest has to survive). Everywhere else the step **fails** rather than
passing one that moved nothing. iOS coverage for a width crossing therefore comes from running the
same walkthrough on an iPhone *and* an iPad, which is worth doing regardless: on iPadOS 26 an iPad
app opens **windowed**, so its scene is materially narrower than its screen.

> [!NOTE]
> **The iOS lever may be arriving.** `devicectl device appResize set --preferred-size <W>x<H>`
> exists, and a booted iOS 27 iPhone simulator advertises the capability behind it
> (`com.apple.coredevice.feature.resizableappmanagement`); `devicectl device info displays` shows
> such a device carrying a second display literally named **Resizable**. It is not usable yet:
> `appResize start` answers *"There is no foreground application to move to the resizable
> display"* even with the app launched and frontmost — through `simctl launch` and
> `devicectl device process launch` alike, headless and with Simulator.app running. Capabilities
> are only reported for BOOTED devices, which is why a survey of shut-down simulators finds
> nothing. Whoever picks this up starts there rather than from scratch.

Release the override with `width: auto` once the sweep is over. A forced class that outlives the
steps it was written for follows the script into everything after it: a phone left on `expanded`
lays out a two-pane split at 390pt, and the detail pane — the thing the next step is about to look
for — sits off the right edge of the screen. The layout is correct; the window is simply not that
wide.

The assertion worth writing is the two-part one: that the presentation changed **and** that the
state survived. A morph that silently drops the selected section passes a naive screenshot check
and fails the only thing this feature promises.
