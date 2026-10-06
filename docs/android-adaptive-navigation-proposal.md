---
title: "Android adaptive navigation proposal"
description: "A researched, unimplemented design for compact predictive Back on Android section navigation and native fold-aware list/detail placement."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Android adaptive navigation proposal

Status: researched design, October 2026. This is not the implemented behavior. The
[current audit](https://github.com/daybrite/Day-News/blob/main/tests/android-navigation-audit.md) describes the existing
backend and the fixed stale-pop bookkeeping. This proposal addresses the remaining
compact section predictive-Back gap and native pane placement for foldables.

## Recommendation and evidence

Use a real AndroidX fragment stack for every full-screen compact destination, and
native pane containers for destinations displayed together. Preserve Day's logical
selection/path across both presentations. Do not implement gesture physics or seek
SlidingPaneLayout's private drag controller.

Google's [Material Views list/detail sample](https://github.com/material-components/material-components-android/blob/master/catalog/java/io/material/catalog/adaptive/AdaptiveListViewDemoFragment.java)
uses `replace` in both forms, adding `addToBackStack` only when the detail occupies
the screen instead of sitting beside the list. Its old orientation-based layout
switch is an example of the transaction rule, not the proposed breakpoint policy.

Android's [fragment animation guidance](https://developer.android.com/guide/fragments/animate)
supports predictive fragment transitions with Fragment 1.7+ and Transition 1.5+ on
Android 14+. Day already declares Fragment 1.8.5, Transition 1.5.1 and Material
1.14.0. Keep MaterialSharedAxis and native FragmentManager progress/cancellation.

The [two-pane guide](https://developer.android.com/develop/ui/views/layout/twopane)
and AbstractListDetailFragment are useful references, but the documented pane Back
callback only closes on commit. Adopting that container alone would not solve the
peek gap. SlidingPaneLayout also has [maintenance-only status](https://developer.android.com/jetpack/androidx/releases/slidingpanelayout).
The [Views predictive progress APIs](https://developer.android.com/guide/navigation/custom-back/support-animations-views)
are a supported alternative, but custom progress handling is unnecessary for normal
page navigation when fragments can do it.

## Navigation and presentation contracts

Keep one logical hierarchy: feed selector → selected section's article list →
selected article/dashboard. Section and article identity must not be inferred from
fragment entry counts. Pane role and navigation depth are separate facts.

| Available layout | Presentation | Native history |
| --- | --- | --- |
| One pane | Feeds, article list and reader each occupy the same page container | Real replace/back-stack transactions for both forward levels |
| Two panes | Master and subordinate content visible together; other ancestor panes collapse | History only for navigation that replaces a visible pane |
| Three panes | Feeds, article list and reader/dashboard visible together | Sidebar selections replace content; no sequence of visited feeds to unwind |

The exact two-pane role priority is Day policy. For Day-News, favor article list and
reader when reading, with the feeds selector available through native collapsible
navigation. Other Day apps may favor sidebar and detail instead. Do not conflate
pane priority with physical device type.

On compact phones the feed selector must be the actual root fragment in the same
container as the section and reader. Keeping it in another SlidingPaneLayout child
and recording a detail-only pop would still leave an empty incoming container.
Place the root's search field with its content so the real root can be previewed.
The app bar can remain host chrome, as it does for existing article Back.

A cancelled native Back changes neither selection nor logical path, disposes no
page scope and triggers no refresh or article side effects. After a committed pop
and settled fragment state, report one `NavBack { already_popped: true }` for the
actual removed page. Retain the page-specific acknowledgement/removal fix.
Ordinary toolbar Up and programmatic Back use the same native pop path. Explicit
application guards can still intercept Back; they must not shadow ordinary compact
navigation. Wide reader dismissal may retain Day-News's existing policy of showing
the dashboard without clearing the selected section.

## Smallest useful framework changes

1. Refactor DayNavHost's compact presentation into a normal FragmentContainerView
   and real root/list/detail stack. Remove the commit-only section Back callback
   from unguarded compact navigation.
2. Keep page identity/title/role and Day-owned content separate from their fragment
   presentation wrappers. Reuse content across layout changes. Do not change an
   existing fragment's container ID or dispose Rust scopes simply to move a pane.
3. Add Android placement for the existing `Pane::List`/`.content_list(...)` contract.
   Today Android advertises NavContentList unsupported and the shared layer builds
   list and reader together. That prevents the toolkit from independently placing
   those panes around hinges or deciding which pane must collapse. This needs real
   implementation before changing the capability flag, not just declaring support.
   Forward NavPageProps.pane into the Java bridge, along with the existing list-width
   configuration. Compare the merged-pane contract used by UIKit before choosing
   the advertised support level. If the current messages cannot express focused
   pane/detail visibility separately from overall Stack/Split, add that small shared
   contract explicitly; do not guess focus from page count or make Day-News alone
   coordinate native panes.
4. Use one presentation decision derived from available space and folding features.
   Reconcile the native containers/history from the existing logical hierarchy only
   when that decision changes. Layout transactions are presentation changes, not
   user pops; they must not emit logical Back or deselect feeds/articles.
5. Use a host-owned child FragmentManager where feasible, with the host as primary
   navigation fragment and lifecycle-bound callbacks/listeners. This removes the
   shared activity-history prefix arbitration for independent hosts. A NavController
   migration is optional: Day already owns routing, and adding a second route graph
   solely for animation would increase scope without solving pane placement.

A layout change preserves selected scope, selected article, list scroll position,
search text, loaded reader DOM and WebView session. Native fragment wrappers may
need replacement, but the underlying page state must survive. Coalesce resize
notifications. Defer structural reconciliation while a predictive transaction is
in flight, then apply the latest layout after commit/cancellation settles, using
[FragmentManager's public callbacks](https://developer.android.com/reference/androidx/fragment/app/FragmentManager.OnBackStackChangedListener).
Activity/process recreation restores logical identities before constructing the
appropriate native hierarchy; live-view retention alone is not process restoration.

## Window sizes and folds

Use current app-window dimensions and actual pane minimum widths, including safe
insets, rather than device names, orientation alone, or maximum display metrics.
Android's [size-class guidance](https://developer.android.com/develop/adaptive-apps/guides/use-window-size-classes)
places compact below 600dp, medium at 600–839dp and expanded at 840dp+. These are
starting layout categories, not a guarantee that three readable Day-News columns
fit at 840dp. The configured list width plus sidebar and reader minima must fit.
Check height as well, particularly for landscape phones and half-open devices.

Use Jetpack WindowManager's [Java callback adapter](https://developer.android.com/reference/androidx/window/java/layout/WindowInfoTrackerCallbackAdapter)
for lifecycle-bound WindowInfoTracker observation. Declare required window/window-java
artifacts in day-android's existing Gradle metadata channel; do not rely on the old
transitive WindowManager dependency from SlidingPaneLayout.

The proposed fold policies are:

- For a vertical separating/occluding hinge, map its window bounds into host content
  coordinates and place panes in the usable rectangles beside it. If either side
  cannot fit a pane's minimum, collapse that pane. A third pane can share one side
  only if it fits there independently.
- For horizontal tabletop posture, use vertically arranged content/list or controls
  when usable heights permit; otherwise show the focused content in one usable
  region. Do not force three horizontal columns because total width is expanded.
- A flat, nonseparating, nonoccluding crease need not create two independent panes.
  A dual-screen hinge must still be respected when the reported state is FLAT.
- Folding, unfolding, rotation and multi-window resizing change layout, not logical
  navigation. Collapsing while an article is open must yield the real compact path
  feeds → article list → reader, so the next two Back operations have proper previews.

These policies apply Android's [fold-feature guidance](https://developer.android.com/develop/adaptive-apps/guides/foldables/make-your-app-fold-aware)
(bounds, orientation, separation and occlusion) to Day's reading hierarchy. Window
size class alone does not carry that information.

## Acceptance criteria

Test partial progress, cancel and commit at both compact levels. Assert the actual
incoming content is visible at intermediate progress, not just that Back ends at a
correct title. Cancellation must restore the same selected article, pane and DOM;
commit changes the logical path once. Also test toolbar Up, three-button Back,
root Back-to-home, section replacement, guarded Back and repeated mixed operations.

Test narrow/wide/three-pane boundaries, rotation, background resize, process
recreation and an open/actively loading reader across those changes. Verify no
extra network reload merely from pane re-presentation. Fold tests need vertical
separating hinges, horizontal half-open folds, FLAT dual-screen hinges and narrow
usable regions, including a layout change during predictive Back.

Android provides [WindowLayoutInfoPublisherRule and test folding features](https://developer.android.com/training/testing/different-screens/tools),
so hinge/layout tests can run on a normal Android emulator. Use a foldable AVD or
physical device as additional acceptance coverage, not the only way to exercise
fold-aware layout. The previous completed-swipe tests do not establish peek or
cancellation correctness for this new design.
