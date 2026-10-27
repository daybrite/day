---
title: "Size classes"
description: "Compact to expanded: how day re-presents navigation when the window crosses a size-class boundary."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Size classes and re-presenting navigation

The space available to an app changes as windows resize, devices rotate, or another app shares the
screen. Size classes group those changes into layout choices, such as showing related panes together
or one at a time.

Day resolves navigation layout from the window’s **size class** and updates it when the class
changes. A [`nav`](https://docs.rs/day-pieces/latest/day_pieces/fn.nav.html) in a wide window shows
its list beside the selected page; in a narrow window, it shows the list and then opens the selected
page over it.

## The breakpoints

`SizeClass` buckets a window's size in points. The numbers are Android's window size classes, used
verbatim on every toolkit:

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

One table across every toolkit means one answer: a 700pt window is `Medium` on a Mac, in a
browser, and on a tablet, so an app that lays out from the class gets the same layout at the same
size everywhere. Apple publishes two buckets rather than five; those map onto this table instead
of replacing it, with `Compact` the compact one and everything wider regular.

Apps read the class with [`day::size_class()`](https://docs.rs/day-app/latest/day/fn.size_class.html). The read is tracked, so a piece that lays out from
it rebuilds when the window crosses a breakpoint. It answers `None` on a toolkit that reports no
window geometry.

```rust
let two_up = day::size_class().is_some_and(|c| c.width >= WidthClass::Expanded);
```

## Window scope

The class belongs to a **window**. One process can show two windows in different classes at the
same moment (a narrow window beside a wide one, iPadOS Stage Manager, Android split-screen), and
an app keyed off a single global would lay the second window out for the first one's size. The
signal is keyed by window root, alongside the safe-area insets, and both scope the way toolbars do
(`day_core::toolbar`): a read during a window's content build means *that* window.

The value is derived in day-core from `Event::WindowResized`, which every toolkit already emits.
A toolkit reports geometry and never a class, which keeps the breakpoint table in day-core alone.

## Adaptive navigation

A [`nav`](https://docs.rs/day-pieces/latest/day_pieces/fn.nav.html) presents as `Split` (list beside detail), `Stack` (one page at a time,
back-navigable), `Tabs` (the rows as a tab bar) or `Rail` (the rows as a narrow strip). Left alone
it resolves that automatically:

```rust
nav(section)                       // automatic: follows the window
nav(section).presentation(NavPresentation::Split)   // pinned
```

Resolution answers four questions in order:

1. Did the app pin one? If so, that one, clamped to something the toolkit can draw.
2. Can this toolkit draw split panes at all (`Cap::NavSplit`)? If not, it stays single-pane.
3. Which style is it? `Tabs` is a tab bar at every size; `Sidebar` is the `Split` ↔ `Stack`
   presentation.
4. `Automatic` walks the full ladder: `Split` when expanded, `Rail` at medium, and when compact
   either `Tabs` or `Stack`; `Cap::NavTabsAdaptive` decides which, because growing a tab bar
   from a narrowed window is idiomatic on the phones and the web and is not on any desktop
   ([docs/navigation](navigation.md)).

Pin one when the content only works one way, such as a settings sidebar whose detail is
meaningless on its own or a wizard that has to stay a stack. A pin is still a preference: a
toolkit with no split container stacks whatever it is asked for.

### Preserving page state

When the class crosses a breakpoint the host is **re-presented**: `NavPatch::Presentation` tells
the toolkit to rebuild its own chrome and re-home the pages it already has. No page is torn down
and rebuilt, because a rebuild would drop every scroll offset, text selection, and focused
field, and would restart any animation in flight.

That works because a page's `Pane` is a fact about the model rather than about the current
drawing. A nav host's list page is `Pane::Sidebar` whether the host is split or stacked; the
presentation decides only where the pane lands:

| pane | `Split` | `Stack` | `Tabs` | `Rail` |
|---|---|---|---|---|
| `Sidebar` | its own splitter pane | the root of the stack | not drawn; the rows are the tab bar | not drawn; the rows are the rail |
| `Detail` | the detail pane | pushed above the root | the tab's content area | the content beside the rail |

`Tabs` and `Rail` differ only in where the rows are drawn, which is why they share a code path in
the pieces layer and in every toolkit: both hide the sidebar page and render its rows as chrome.

Selection is carried across, with one asymmetry:

- **Narrowing** keeps the selection. The detail becomes the top of the stack, which is
  where the user already was.
- **Widening** with nothing selected picks the first item, because a split presentation has no
  way to draw an empty detail pane. This is the same rule the initial build uses.

## Per-toolkit support

`Cap::NavRepresent` says whether a toolkit re-presents a live host. It gates more than the patch:
on `Unsupported`, presentation resolves from `Cap::NavSplit` alone and the window's size never
enters into it. A toolkit that cannot change its presentation must not have it decided by
something that changes underneath, or a window launched narrow would be stuck stacked with no way
back.

| toolkit | `NavSplit` | `NavRepresent` | notes |
|---|---|---|---|
| web-dom | ✅ | ✅ | the shim rebuilds chrome; page elements move between containers intact |
| macos-appkit | ✅ | ✅ | one [`NSSplitViewController`](https://developer.apple.com/documentation/appkit/nssplitviewcontroller) either way — a stack is that split with its sidebar item collapsed |
| linux-qt / macos-qt | ✅ | ✅ | one [`QSplitter`](https://doc.qt.io/qt-6/qsplitter.html) either way, back header installed in both |
| linux-gtk | ✅ | — | see below |
| windows-winui | ✅ | — | a [`NavigationView`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.navigationview) owns its own `PaneDisplayMode`; re-presenting means driving that rather than re-homing pages |
| ios-uikit | ✅ | **observed** | [`UISplitViewController`](https://developer.apple.com/documentation/uikit/uisplitviewcontroller), both columns navigation controllers |
| android-material | ✅ | **observed** | `SlidingPaneLayout`, list pane beside a detail pane |
| harmony-arkui | ✅ | **observed** | ArkUI `Navigation` in `NavigationMode.Auto`: the nav bar (the rows) tiles beside the content where `navBarWidth` + `minContentWidth` fit, stacks otherwise; `onNavigationModeChange` reports which, through `navModeChanged`. A literal `Stack` host (a nested [`nav_stack`](https://docs.rs/day-pieces/latest/day_pieces/fn.nav_stack.html)) runs `NavigationMode.Stack` |

GTK is the odd one out among the desktops. Everywhere else both presentations are the same
container with different chrome, so a morph re-homes pages inside a host Day already holds. On
GTK they are different widgets (nested [`GtkPaned`](https://docs.gtk.org/gtk4/class.Paned.html)s for the split, [`AdwNavigationView`](https://gnome.pages.gitlab.gnome.org/libadwaita/doc/main/class.NavigationView.html) for the
stack), and Day holds the host handle, so it cannot be swapped underneath. The route there is
a stack presentation re-homing the split's pages under the same host, which is a restructure
of a working toolkit rather than an addition to it. What a narrowing GTK window does get is
the AppKit-style fold ([docs/navigation](navigation.md), toolkit notes): the sidebar
collapses when the window has no room for it beside the other panes' minimums, and returns
when there is room again — the presentation stays `Split` throughout.

> [!NOTE]
> The mobile toolkits run the **opposite** policy from the desktop ones; `NavRepresent = Emulated`
> records that. Their platforms ship adaptive containers that already do this
> morph natively, with the right animation and the right gesture: [`UISplitViewController`](https://developer.apple.com/documentation/uikit/uisplitviewcontroller) collapses
> and expands on its own as a Pro Max iPhone rotates between compact and regular width, and
> `SlidingPaneLayout` decides at measure time whether both panes fit. Day observes and reconciles;
> it never pushes a presentation at them, because that would be a second copy of the state racing
> the platform's animation.

This policy determines one lowering rule: an `Emulated` toolkit's adaptive host is lowered
with `presentation: Split` (meaning "build the adaptive container") even when the window is compact at
build time, because the container collapses itself. `Stack` in `NavProps` is thereby literal: it
marks a host that is a stack at *every* size (a pinned request, or the nested [`nav_stack()`](https://docs.rs/day-pieces/latest/day_pieces/fn.nav_stack.html) piece
under a split host), and the toolkit realizes it as a plain navigation controller.

## Mobile windows

Mobile apps can run in resizable windows, split-screen layouts, and rotated or folded devices.
Use the app window's available size rather than its device name or physical screen size.

### Android: survive the resize

Entering split-screen, or dragging a desktop-windowing edge across a size bucket, changes
`screenLayout` and `smallestScreenSize`. An activity that has not claimed those in
`android:configChanges` is **destroyed and recreated**. Day survives that (a recreation re-mounts
the tree, the path a theme switch takes on purpose: [settings](settings.md), "Appearance on
Android"), but a resize is not a reason to rebuild the app, so the scaffold's manifest claims them,
`day lint` fails a manifest that does not (`day::lint::android-not-resizable`), and the resize
reaches Day as a window resize with everything still in place.

The `<layout>` element carries the window's minimum and its desktop-windowing default, and
`PROPERTY_SUPPORTS_MULTI_INSTANCE_SYSTEM_UI` (API 35+) lets the system UI offer a second
window for an app that supports one. `resizeableActivity` is not declared, because it already
defaults to true at `targetSdk` 24+ and API 36 ignores it.

### iOS: measure the scene, not the screen

[`UIScreen`](https://developer.apple.com/documentation/uikit/uiscreen)'s bounds are the display. A scene has not filled the display since iPad multitasking,
and on iPadOS 26 it usually does not even at launch. Measured on an iPad Pro 13-inch, a Day app
opens into a **635×1376pt window on a 1032×1376pt screen**. Sizing the window from the screen made
the first size class wrong by 397 points, so a nav host resolved a three-column split for a window
that had room for two and then corrected itself once the first layout pass ran.

day-toolkit-uikit takes its launch geometry from the scene's own coordinate space
(`effectiveGeometry.coordinateSpace` on iOS 26+, the direct property below it; the same value either
way). `DayHolderView` reports every later change against **its own scene**, which matters as soon as
there are two windows: reporting a secondary's geometry against the primary re-framed the wrong
window's root view and re-bucketed the wrong window's class.

### Declaring a minimum size

One declaration serves both layers. Android reads it from the manifest at build time and iOS at
run time:

```toml
[window]
width = 960          # also the desktop-windowing default size
height = 640
min_width = 320
min_height = 400
```

`day build` writes the `<layout>` element's attributes and the iOS `Info.plist` keys day-toolkit-uikit
reads back for [`UIWindowScene.sizeRestrictions`](https://developer.apple.com/documentation/uikit/uiwindowscene). An app that sets `WindowOptions.min_size` in Rust
still wins over both. On iOS the minimum is a **preference the system satisfies on a best-effort
basis**; laying out sensibly at whatever size arrives is still the app's job.

### What still works on an older OS

Nothing here raises a floor. Android's `minSdk` stays 24 and everything used is API 24+ except the
multi-instance `<property>` (API 35+, and older platforms skip unknown tags). iOS deploys to 15.0
and needs no gated API on the critical path, because the class comes from geometry rather than from
UIKit traits: the scene's coordinate space reads the same from iOS 15 to iOS 27. The one API
newer than the floor ([`UITabBarController.mode`](https://developer.apple.com/documentation/uikit/uitabbarcontroller), annotated `ios(18.0)`) is guarded on
`respondsToSelector:` rather than on a version number, which is the fact the call depends
on. It responds on iOS 15.5 and 17.5 as well; gating it on the version instead made
those releases *worse*, because the resolver's fallback lowers a different host shape.

## Row fit policies

A [`row`](https://docs.rs/day-pieces/latest/day_pieces/fn.row.html) keeps its children on one line no matter what. That is the right contract for a label
beside a value, and the wrong one for five buttons on a phone: the line overflows the window and
the remaining controls land offscreen. [`.fit(RowFit::…)`](https://docs.rs/day-pieces/latest/day_pieces/trait.RowBuilder.html#tymethod.fit) names what should happen instead, with the same children and the same
call shape:

```rust
row((a, b, c)).spacing(Space::S)                                  // RowFit::Clip, the default
row((chips,)).spacing(Space::S).fit(RowFit::Wrap { run_spacing: 8.0 })
row((keys,)).spacing(Space::S).fit(RowFit::WrapColumns { run_spacing: 8.0 })
adaptive((label, control)).spacing(Space::S)                 // a column on a compact window
row((chips,)).spacing(Space::S).fit(RowFit::Scroll)
```

`Clip` is the default: one line at natural sizes, and whatever does not fit lands offscreen. In
debug builds the engine logs the overflow once per container, naming the dayscript ids in reach
(`day layout: children overflow their container …`), so the silent version of this failure no
longer exists. Release builds skip the check entirely.

`Wrap` breaks onto additional lines where the next child would overflow, like wrapped text, the
shape a chip row, a button strip, or a tag cloud wants. Lines are `run_spacing` apart, and
children align within their line via [`.align(VAlign::…)`](https://docs.rs/day-pieces/latest/day_pieces/trait.AdaptiveBuilder.html#tymethod.align). Wrapping replaces main-axis
negotiation, so [`.grow()`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.grow) and [`spacer()`](https://docs.rs/day-pieces/latest/day_pieces/fn.spacer.html) are inert, and a single child wider than the window
still overflows.

`WrapColumns` wraps the same way but into aligned columns: every cell takes the widest child's
width, and each line holds as many as the window fits. `Wrap` keeps each child at its natural
width, so the lines come out ragged, which is right for chips of unequal weight and wrong for a set
of peers that should read as a grid (a keypad, a palette, a row of equal choices):

```text
Wrap          [Item 1][ Item 2 ][Item 3][ Item 4 ]     WrapColumns   [ Item 1 ][ Item 2 ][ Item 3 ]
              [ Item 5 ][Item 6][ Item 7 ]                           [ Item 4 ][ Item 5 ][ Item 6 ]
```

The column count follows the available width, so it re-flows as the window changes. Left at
that, the columns keep the widest child's width and whatever width is left over trails the last
column. When any child grows ([`.grow_w()`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.grow_w)), the widest child's width becomes the narrowest a column
gets instead, and the columns stretch to share the whole width. That is the adaptive grid a
gallery of tiles wants, SwiftUI's `GridItem(.adaptive(minimum:))`: tiles that can grow fill each
line edge to edge, and a line with fewer tiles than columns leaves the rest of its columns
empty. No column is wider than the line, though: on a window narrower than the widest child, the
cells take the window's width and their text wraps.

```rust
row((tiles,))
    .spacing(Space::L)
    .fit(RowFit::WrapColumns { run_spacing: 16.0 })
// each tile: …min_width(120.0).grow_w(), with anything inside that should scale sized from it
```

An authored, fixed column count with per-cell spans is a different job; that is
[`grid`](grid.md), whose children are rows rather than items.

A row that should become a leading-aligned column while the window's width class is compact
(the shape a label-plus-control-plus-result line wants, where wrapping members independently
would tear apart what reads as one sentence) is [`adaptive`](https://docs.rs/day-pieces/latest/day_pieces/fn.adaptive.html)`((label, control, result))`[`.spacing`](https://docs.rs/day-pieces/latest/day_pieces/trait.AdaptiveBuilder.html#tymethod.spacing)`(Space::S)`,
with [`.stack_at(class)`](https://docs.rs/day-pieces/latest/day_pieces/trait.AdaptiveBuilder.html#tymethod.stack_at) naming the widest class that stacks (default `Compact`) and [`.align(VAlign)`](https://docs.rs/day-pieces/latest/day_pieces/trait.AdaptiveBuilder.html#tymethod.align)
the cross-axis alignment of the row arrangement (stacked, the children align leading). An [`adaptive`](https://docs.rs/day-pieces/latest/day_pieces/fn.adaptive.html)
is re-laid out, never rebuilt, when the window crosses the breakpoint, so its children keep
their local state (a focused field stays focused).

`Scroll` keeps the single line and makes it a horizontal scroll strip: one row tall, filling the
width it is given, with the tail a swipe away instead of gone. It is the policy for rows whose order
matters more than their visibility, such as a timeline, a filmstrip, or a rail of shortcuts.

The showcase's Layout page renders one row under each policy with a live component count, which
is the quickest way to see the difference.

## Platform guidance

- [Apple: layout](https://developer.apple.com/design/human-interface-guidelines/layout)
- [Android: adaptive layouts](https://developer.android.com/guide/topics/large-screens/support-different-screen-sizes)
