---
title: "Foldable support plan"
description: "Research and proposed Day architecture for iPhone Duo, Android foldables, and future HarmonyOS support."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Foldable support plan

Day should make adaptive layout a property of every window and content container. Extend its
existing native navigation with explicit usable regions, preserve application state through
display and host changes, and expose optional hardware capabilities separately. This supports
iPhone Duo and Android foldables while leaving room for multiple hinges and HarmonyOS.

This is an implementation proposal dated October 23, 2026. Proposed Day symbols and configuration
are design sketches, not APIs available in the current framework. Breaking API changes are in
scope. The intended reader is a Day framework or app maintainer.

The recommended order is:

1. Establish precise geometry, lifecycle, and restoration contracts.
2. Deliver native Duo bars and region-aware navigation on iOS and Android.
3. Add reusable adaptive arrangements and migrate representative Day apps.
4. Add optional hinge input, camera direction, and secondary-display sessions.
5. Implement and certify the same contracts on HarmonyOS as its product priority warrants.

## Platform findings

### iPhone Duo

Apple describes an inner and outer display, partial-fold poses, Split View multitasking, and
camera regions that affect layout. Preserve tasks and functionality across poses; expand the
existing information hierarchy as space permits. System bars move to an edge on the outer
display and inner landscape display. Their physical placement does not mirror with text
direction. The inner camera creates a reserved region while active. These are significant
differences from treating an unfolded phone as a small tablet.
[Apple Duo design guidance](https://developer.apple.com/design/human-interface-guidelines/designing-for-iphone-duo)

The linked SDK matters independently of the deployment target. Apple documents compatibility
layouts for builds using iOS 26 or earlier, dynamic resizing with iOS 27, and full-display use
with vertical bars for iOS 27.1 builds. Size content from its current view/window, obtain scale
from traits, and respond to subsequent geometry changes. Neither device idiom nor orientation
identifies the available layout.
[Apple preparation checklist](https://developer.apple.com/iphone-duo/prepare/)

The main native integration points are:

| Capability | Apple API or behavior | Consequence for Day |
| --- | --- | --- |
| Native adaptive navigation | `UISplitViewController`, `UINavigationController`, `UITabBarController` | Keep native container ownership and observe the resulting presentation |
| Interior layout restrictions | `UIView.reservedRegions(kind:options:)`, `UIView.ReservedRegion` | Represent division and occlusion separately from edge insets |
| Two related content areas | `UIArrangementViewController`, split and overlay arrangements | Offer a content arrangement abstraction with native lowering where containment permits |
| Vertical toolbar adaptation | Native controller-owned bars; `verticalBarEdge`, axis behavior, visibility priority | Express item semantics and let UIKit choose placement |
| Hinge interaction | `UIHingeInteraction`, `UIHinge` | Optional input stream, independent of layout policy |
| Additional windows | Scene activation, `UIWindowSceneActivationAction` | Availability can change during a session; activation can fail |
| Camera display accessory | Camera capture accessory scene | Conditional, camera-scoped supplementary content |
| Camera direction | `AVCaptureDeviceDirectionCoordinator` in AVKit | Select relative to the presenting view when that is the application's intent |

Reserved regions have identity, activity, and a view-local frame. Apple distinguishes divisions
from occlusions; inactive regions can be queried. The returned frame already includes the
region's interaction margins. Regions can partially overlap a view and extend beyond its
bounds. Region reads in `updateConstraints()` participate in automatic invalidation. Scrolling
containers do not automatically split rows around a division.
[UIView reserved-region reference](https://developer.apple.com/documentation/uikit/uiview/reservedregion)

Native arrangements handle a pair of related views using split or overlay policies. The API
reference cautions against placing them inside split, scroll, or collection containers, and
against embedding navigation split/tab containers inside them. Therefore Day cannot lower every
nested adaptive layout to an arrangement controller. Use one as a supported content controller,
for example under a navigation stack; retain direct region-aware layout for other compositions.
[Arrangement controller reference](https://developer.apple.com/documentation/uikit/uiarrangementviewcontroller)

Vertical bars require the bars owned by navigation/tab controllers. A standalone `UIToolbar`
does not gain the same behavior. Custom toolbar views need suitable representations for the
available axis. UIKit provides visibility priority, overflow integration, and compression
preferences; keyboard accessory controls remain attached to the keyboard. In split navigation,
the detail column participates in vertical bars while other columns retain their local bars.
[Apple toolbar session](https://developer.apple.com/videos/play/tech-talks/111462/)

Duo permits new app scenes on its inner display, with activation unavailable on the outer
display. Scene accessories are dynamically available. The camera capture accessory requires
an active camera session and a fullscreen app on the inner display. This does not establish a
general-purpose right to put arbitrary app UI on both displays.
[Apple scenes and displays session](https://developer.apple.com/videos/play/tech-talks/111464/)

`UIHingeInteraction` is associated with a view hierarchy and can deliver an unavailable hinge
when that association changes. `UIHinge.angle` is in radians; the SDK does not promise a fixed
frequency or precision. Layout should follow regions and arrangements, while hinge data can
drive optional interaction effects.
[Hinge interaction reference](https://developer.apple.com/documentation/uikit/uihingeinteraction),
[hinge reference](https://developer.apple.com/documentation/uikit/uihinge),
[Apple adaptive layout session](https://developer.apple.com/videos/play/tech-talks/111463/)

Apple's virtual front camera switches between the physical inner and outer front cameras. Its
feature set is constrained by the constituent cameras. Applications choosing physical cameras
must manage switching. The direction coordinator describes cameras relative to a view and
provides sendable descriptors for capture work outside the main actor. A fixed physical
front/back label does not always describe a camera's direction toward the person using a display.
[Apple camera session](https://developer.apple.com/videos/play/tech-talks/111465/)

### Android foldables

Android encompasses continuous folding displays, displays separated by a physical hinge,
book-style devices, flip-style devices, and device-specific outer displays. Fold/unfold can
change window size and trigger recreation. Multi-window operation and continuity are baseline
requirements; tabletop and book layouts provide additional opportunities.
[Android foldable fundamentals](https://developer.android.com/develop/adaptive-apps/guides/foldables/learn-about-foldables)

The first supplied Android URL has a malformed host, `developer.android.coout`. The relevant
official starting point is [Android foldables](https://developer.android.com/foldables), alongside
the supplied adaptive-app guide.

Jetpack WindowManager exposes folding features with bounds, orientation, `FLAT`/`HALF_OPENED`
state, `isSeparating`, and occlusion. A half-open fold separates areas; a dual-screen hinge can
remain separating when flat. A flat, nonseparating crease need not split content. The feature
API does not supply a continuous hinge angle. An empty feature list does not prove the hardware
has no hinge: it is window information, not a universal hardware inventory.
[Android fold-aware guide](https://developer.android.com/develop/adaptive-apps/guides/foldables/make-your-app-fold-aware)

Day uses Android Views and Java. Integrate `WindowInfoTracker` through
`WindowInfoTrackerCallbackAdapter`, with listeners owned by each activity/window lifecycle.
The documented stable WindowManager version is 1.5.1 at the research date; declare explicit
`window` and `window-java` dependencies and use `window-testing` for platform tests. Do not
depend on a transitive version from SlidingPaneLayout or copy an alpha Compose API into Day's
Java toolkit.
[WindowManager release notes](https://developer.android.com/jetpack/androidx/releases/window)

Android's width classes are compact below 600dp, medium below 840dp, expanded below 1200dp,
large below 1600dp, and extra-large thereafter. Height classes break at 480dp and 900dp.
These describe current window space rather than a device category. Height matters for wide,
short windows and half-open devices.
[Android window size classes](https://developer.android.com/develop/adaptive-apps/guides/use-window-size-classes)

Rear-display mode transfers an activity to the outward display and turns off the inner display.
Dual-screen mode presents on both. `WindowAreaController` exposes operation-specific
availability and sessions. Rear-display transfer uses a system confirmation dialog. Availability
depends on device and system support; these modes are not guaranteed across Android foldables.
[Android display modes](https://developer.android.com/develop/adaptive-apps/guides/foldables/support-foldable-display-modes)

For apps targeting Android 17/API 37, the system removes the developer opt-out from large-screen
orientation, aspect-ratio, and resizability changes, subject to documented exceptions such as
games and user settings. Day already compiles and targets API 37. Resizing is a current shipping
requirement, not a future manifest option.
[Android 17 behavior](https://developer.android.com/about/versions/17/changes/ff-restrictions-ignored)

Configuration-change handling can preserve a live activity, but it does not eliminate activity
or process recreation. Resource configuration and display density can also change. Test both
in-place adaptation and rebuilding from saved state.
[Android configuration changes](https://developer.android.com/guide/topics/resources/runtime-changes)

### HarmonyOS and multiple hinges

Huawei documents foldability, fold status, display mode, and crease-region queries, with both
ArkTS and selected C APIs. Its guidance distinguishes observing fold status from observing
which display mode is active.
[Huawei fold-state guidance](https://developer.huawei.com/consumer/cn/doc/doccenter-dev-faq/faqs-purax-12)

OpenHarmony's public display reference includes multiple crease rectangles and statuses for
devices with two fold axes. It also documents that `MAIN` and `SUB` have different inner/outer
meanings for different device families. Treat these as platform facts requiring mapping, not
portable Day labels. OpenHarmony references inform the adapter design; they do not certify
every API on every commercial HarmonyOS device.
[OpenHarmony display reference](https://github.com/openharmony/docs/blob/master/en/application-dev/reference/apis-arkui/js-apis-display.md)

The display-mode setter is documented as a system API. A normal Day app must not depend on it
to force inner/outer or simultaneous-display operation.
[OpenHarmony system display APIs](https://github.com/openharmony/docs/blob/master/en/application-dev/reference/apis-arkui/js-apis-display-sys.md)

### Common contracts and platform differences

| Concern | Shared Day contract | Preserve platform differences |
| --- | --- | --- |
| Available space | Current window and local container geometry | UIKit traits, Android dp, ArkUI vp, and native chrome remain distinct inputs |
| Hinge layout | Collections of divisions and occlusions | An Apple reserved division is not necessarily a physical hole; an Android hinge can be |
| Continuity | Stable task, document, route, and pane identity | Native scene/activity recreation mechanisms differ |
| Pane layout | Content minima, role priority, focused pane, usable regions | UIKit controls native split presentation; Day must supply Android placement policy |
| Chrome | Semantic commands, groups, overflow, accessible labels | Duo vertical edge bars are not a rule for all Android foldables |
| Hinge input | Optional, timestamped readings with explicit availability | Angle units, precision, reporting, and associations differ |
| Multiple displays | Optional scoped session with revocable availability | Camera accessory, activity transfer, and presentation are different operations |
| Future devices | Zero or more features and regions | Do not bake one hinge, two screens, or three postures into the model |

## Day implementation audit

The audit uses Day revision `314c19daf143bf844211298811653db23b350c3f` plus the existing working
tree. These are code observations, not hardware certification. Xcode reports 27.1, build
`27A9275`; its installed iPhoneOS 27.1 headers contain the new UIKit and AVKit interfaces.

| Area | Existing foundation | Work needed |
| --- | --- | --- |
| Size classes | Five width and three height classes in [`day-spec`](../crates/day-spec/src/lib.rs); `SPLIT_MIN_WIDTH = 600`, `prefers_split()` | Keep useful thresholds; remove the implication that a width bucket proves two panes fit |
| Ambient geometry | Per-window class and safe-area signals in [`ambient.rs`](../crates/day-core/src/ambient.rs) | Add coherent metrics, local layout context, interior regions, source availability, and revisioning |
| Layout | [`layout.rs`](../crates/day-core/src/layout.rs) supports adaptive rows and wrapping | Base nested adaptation on local constraints; add region-aware placement without rebuilding child scopes |
| UIKit | Scene geometry, page safe-area updates, split/navigation/tab controllers in [`day-toolkit-uikit`](../crates/day-toolkit-uikit/src/lib.rs) | Bind reserved regions and new bar traits; handle dynamic scene failures; audit custom toolbar content |
| Android hosts | Insets, resize handling, navigation saved state in [`DayActivity`](../crates/day-toolkit-material/java/dev/daybrite/day/bridge/DayActivity.java) | Add explicit window-feature observation and activity-scoped geometry; broaden restoration beyond routes |
| Android secondary windows | [`DayWindowActivity`](../crates/day-toolkit-material/java/dev/daybrite/day/bridge/DayWindowActivity.java) uses a Rust node ID and closes when the process is not running | Restore a durable window descriptor before mounting native content |
| Android navigation | Fragment-based host and SlidingPaneLayout in [`DayNavHost`](../crates/day-toolkit-material/java/dev/daybrite/day/bridge/DayNavHost.java) | Independent content-list placement, horizontal division support, correct compact predictive Back |
| Android build | [`DayAndroidPlugin`](../crates/day-toolkit-material/gradle-plugin/src/main/java/dev/daybrite/day/gradle/DayAndroidPlugin.java) sets compile/target SDK 37; toolkit metadata declares native dependencies | Add pinned WindowManager dependencies through metadata and test the merged application |
| ArkUI | Native Navigation and area-change reporting in [`platform/harmony`](../crates/day-toolkit-arkui/platform/harmony) | Connect fold/display/crease observations and replace default-display assumptions where window-specific data is needed |
| Camera | External `day-piece-camera` has physical front/back selection, native preview, and photo capture | Add view-relative intent, dynamic capability changes, and explicit preview policy; current API does not expose video recording |
| Testing | DayScript `size_class` override and Android `resize` tooling | Add real posture/display scenarios and geometric hit-test assertions; a class override does not resize pixels |

Day's [size-class reference](size-classes.md) already explains native presentation observation
and preservation of page identity. Retain those strengths. Its current description of mapping
Apple compact/regular traits onto numeric buckets should be replaced during implementation:
Apple's native traits and Day's numeric classes are separate facts.

The existing [Android adaptive navigation proposal](android-adaptive-navigation-proposal.md)
already identifies independent `Pane::List` placement, compact fragment history, and predictive
Back requirements. This plan extends it with shared geometry, lifecycle, and display contracts.
Keep its detailed transaction rules rather than creating a competing Android navigation design.

The UIKit window-open path checks `supportsMultipleScenes()` and requests scene activation
without an error handler. On Duo, runtime availability and failed requests need explicit
completion. A pending open must not wait forever when the system refuses a new scene.

## Proposed geometry and environment API

### Separate window metrics from local layout

Replace ambiguous ambient size reads with two explicit levels. `Window::metrics()` observes
the current native window. A `LayoutContext` belongs to the exact container being laid out.
The latter is what most reusable pieces need.

The following is a proposed type outline; omitted types and methods are not implemented:

```text
WindowMetrics {
    window_id: WindowId,
    revision: GeometryRevision,
    bounds: LogicalRect,
    display_scale: f64,
    size_class: WindowSizeClass,
    system_insets: SystemInsets,
    keyboard: KeyboardGeometry,
    native_traits: NativeLayoutTraits,
    capabilities: AdaptiveCapabilities,
}

LayoutContext {
    window_id: WindowId,
    revision: GeometryRevision,
    coordinate_space: LayoutSpaceId,
    bounds: LogicalRect,
    safe_content_rect: LogicalRect,
    size_class: ContainerSizeClass,
    reserved_regions: Observation<Vec<ReservedRegion>>,
    usable_regions: Vec<UsableRegion>,
    layout_direction: LayoutDirection,
    chrome: ChromeGeometry,
}

Observation<T> = Unknown | Unsupported | Available(T)

ReservedRegion {
    id: RegionId,
    kind: Division | Occlusion,
    avoidance_bounds: LogicalRect,
    active: bool,
    separation: Unknown | NonSeparating | Separating,
    occlusion: Unknown | None | Full,
    axis: Option<Axis>,
    source: RegionSource,
}
```

`Available([])` means an observed empty set for this space. `Unknown` means that geometry has
not arrived or cannot currently be established. `Unsupported` means the provider cannot report
it. These states must not collapse to an invented flat posture.

Region and native display identifiers are scoped to the current observation/session unless the
platform guarantees more. Never persist them as document or window identity. If a provider lacks
stable feature IDs, match observations conservatively within a window and reset that association
when its display changes. Unknown topology uses ordinary measured layout and native safe-area
behavior; diagnostics must distinguish that fallback from verified region-aware placement.

Use one logical-unit convention at the toolkit boundary: UIKit points, Android pixels divided
by the current window density, and ArkUI vp. Window scale is observable and can change after
launch. Do not use a startup global density to translate later geometry or pointer input.

Define the coordinate contracts before implementation:

1. Window bounds use the current native client coordinate space, before Day removes system or
   keyboard insets. This is the stable input to `WindowSizeClass`.
2. Container bounds are local to the content host after its parent allocation. Explicit
   transforms map window features into that space. Rotation, window origin, and density belong
   to the transform; subtracting top inset alone is insufficient.
3. Edge insets describe edge restrictions. Interior regions remain rectangles, never four
   synthetic insets. Keep IME geometry separate, including floating keyboards where reported.
4. `avoidance_bounds` includes the provider's required interaction clearance exactly once.
   Apple frames already include it. Android's raw hinge bounds can receive a separately defined
   Day interaction clearance policy; this must not alter the reported physical bounds in diagnostics.
5. Preserve zero-width/zero-height division lines. Generic rectangle-area filtering must not
   discard a valid separating fold. Clip partial intersections for placement without losing
   feature identity or orientation.
6. Hardware coordinates are physical. Mirror semantic leading/trailing placement for RTL,
   but do not mirror a physical camera, hinge, or system-selected bar edge.
7. Insets already consumed by a native parent must be marked as consumed. A child must not
   subtract the same safe area or native split exclusion again.

`usable_regions` is a computed layout aid. Divide around active separating features; avoid
occlusions within each resulting area. Do not treat a small camera cutout as a reason to turn
the whole screen into two navigation panes. Permit multiple divisions, asymmetric regions,
partial intersections, and overlapping exclusion sources. Content with unusual geometry can
inspect the original regions instead of relying on rectangular decomposition.

### Keep useful classes and remove device assumptions

Retain the existing numeric defaults:

| Axis | Proposed default classes in logical units |
| --- | --- |
| Width | Compact `<600`; Medium `600–<840`; Expanded `840–<1200`; Large `1200–<1600`; ExtraLarge `>=1600` |
| Height | Compact `<480`; Medium `480–<900`; Expanded `>=900` |

Use different type names for `WindowSizeClass` and `ContainerSizeClass` even when the thresholds
match. A 1000-unit window can contain a compact reader or inspector. Compute the container
class from its allocated content area; compute each usable region's size separately.

Expose optional native horizontal/vertical traits without reconstructing them from the numeric
table. Native UIKit controls follow native traits. Portable custom layout uses local dimensions,
content minima, and regions. Do not add `Foldable`, `Duo`, `Phone`, or `Tablet` size classes.

Remove `SizeClass::prefers_split()` and rename or remove `SPLIT_MIN_WIDTH`. A breakpoint is not a
proof that a list, reader, and sidebar fit. For example, 1000 units split by a 30-unit hinge can
yield two 485-unit regions; a 500-unit minimum reader cannot fit on either side despite an
expanded total width. Typography, toolbar placement, and height can reduce capacity further.

Keep window-level classification stable while an IME appears, but update the local content and
keyboard geometry. Default navigation should avoid changing its hierarchy merely because the
keyboard opened. A height-sensitive arrangement may still adapt to the genuinely usable height.

Allow application-specific layout thresholds through `AdaptivePolicy`, rather than mutating
global definitions of compact/medium. Prefer content fit and min/preferred/max sizes over a
large collection of arbitrary named buckets.

### Deliver coherent updates

Add `WindowMetricsChanged` and layout-environment invalidation to `day-spec`, with an atomic
snapshot per window revision. Toolkits may collect multiple native callbacks, but should publish
a coherent revision at the next appropriate UI/layout boundary. Track observation timestamps
and unknown values rather than pairing new bounds with stale features from another display.

Apply updates on the UI thread. Coalesce redundant notifications; invalidate only affected
layout scopes. A local environment read should relayout or update bindings, without destroying
native pages, WebViews, media sessions, editor documents, or reactive owners. Do not invoke
application builders from a native layout callback while the Day tree is borrowed.

Keep native transition coordination separate from geometry. Use the platform's animation where
it owns placement; avoid a second Day animation fighting it. Defer structural navigation changes
while an interactive Back transaction is active, then reconcile to the latest revision.

## Proposed layout and navigation API

### Adaptive arrangements

Add `arrangement(primary, secondary)` for two related content areas. Keep ordinary `row`,
`column`, and `stack` as predictable general-purpose layout primitives. An arrangement adds
explicit adaptation policy:

| Proposed setting | Meaning |
| --- | --- |
| `ArrangementStyle::Split` | Place related content beside or above/below each other when it fits |
| `ArrangementStyle::Overlay` | Foreground controls/content over a background, with separation when a division requires it |
| `axes(...)` | Allowed layout axes, with automatic selection from space and regions |
| `primary_constraints(...)`, `secondary_constraints(...)` | Minimum, preferred, and maximum usable dimensions |
| `secondary_visibility(...)` | Application-owned visibility state, preserved across adaptation |
| `compact_behavior(...)` | Explicit overlay, sheet, or single-pane fallback with an accessible route to secondary content |
| `region_policy(...)` | Automatic avoidance, focused-region preference, or an explicit advanced assignment |

Provide higher-level recipes for list/detail, supporting content, and tabletop media. Keep
navigation outside the content arrangement. A split arrangement is not a navigation stack and
must not create Back entries when its geometry changes.

Use native UIKit arrangements only for valid controller containment and compatible semantics.
The primary/secondary meaning differs between split and overlay, so the adapter must map
semantic content roles explicitly. Inside a native split column or scroll container, use the
local region solver or a specialized piece rather than nesting a prohibited controller.
For Android Views, implement placement over retained native child views. There is no need to
move the whole toolkit to Compose to implement these contracts.

Add local region-aware grid and overlay policies. A grid should avoid splitting an interactive
card across a division and can prefer balanced columns when useful. An overlay chooses a
region containing its anchor or focused content, then clamps to usable bounds. If a dialog
cannot fit, use a scrollable or fullscreen presentation rather than placing its action buttons
in a hinge. Keep native alerts, menus, and sheets in control where their placement already works.

### Adaptive navigation

Separate navigation chrome from content placement. The current `Stack`/`Split`/`Tabs`/`Rail`
presentation combines two decisions: how destinations are selected and how content panes are
shown. Replace this with `NavigationChrome` and a `PaneLayout` result.

`PaneLayout` should describe visible roles, their rectangles, the focused role, hidden roles
that remain navigable, and the resulting compact path. It must accommodate sidebar, list,
detail, and supporting content without assuming each role occupies a physical display.

Resolve placement in this order:

1. Obtain local bounds, native layout ownership, active regions, and content constraints.
2. Remove occupied regions and account for native chrome once.
3. Preserve the currently focused task and fit the highest-priority visible roles.
4. Add other panes only where their minima fit within a usable region.
5. Choose navigation chrome using platform conventions, available height, and destination count.
6. Produce a presentation transaction that preserves logical selection and route history.

For a book-style division, place appropriate panes on either side. For a horizontal division,
offer a vertical supporting-content layout if both regions are useful. Otherwise keep the
focused content in a usable region and provide navigation to the remaining content. Do not
force three columns just because total width is expanded.

On UIKit, native split/tab/navigation containers remain the placement authority where used;
Day expresses preferences and observes settled results. On Android, complete the existing
navigation proposal: real compact fragment history, independent list/detail roles, and a host
that can place retained views into regions. SlidingPaneLayout remains useful for its supported
two-pane case, but its horizontal two-pane contract does not cover the general solver.
[Android two-pane guide](https://developer.android.com/develop/ui/views/layout/twopane)

Continuity is an acceptance contract, not just an animation goal. Folding must preserve selected
document, route stack, scroll anchor, focused field, text selection, search text, draft edits,
undo history, and playback/capture intent. Use item/content anchors where text reflows rather
than persisting only a pixel scroll offset. A cancelled Back gesture changes none of these.

Keep an unselected detail state valid on wide layouts. Do not select an arbitrary first document
just because a pane becomes visible. Applications can opt into an explicit initial selection.
This changes the current widening behavior and should be part of the migration guide.

### Toolbar and tweak changes

Extend semantic toolbar items with axis suitability, visibility priority, content role, and
compact representation. Preserve labels even for symbol-only display so accessibility and
overflow menus use the same localized command name. Supply optional compact custom content
without creating a second command identity.

Expose the resolved bar axis and physical edge through `ChromeGeometry`. Use that for content
layout and custom toolbar representation; never infer it from device orientation. Add
platform-specific UIKit tweaks for exact vertical-bar behavior, axis preference, and toolbar
compression when the portable command model cannot express a native choice. Default them to
system behavior. An override that makes sense for a calculator should not become the global
Duo default.

Keep Android navigation bar/rail/drawer decisions independent from Duo bar behavior. On all
platforms, overflow must preserve every command, its enabled state, badge, label, and action.
Audit search, segmented controls, title views, text-only actions, and fixed spacers under short
vertical bars and large text.

## Hinge input and display sessions

### Optional hinge observations

Expose a subscription such as `Window::hinges()` or a view-scoped equivalent, returning
observations with a hinge identifier, reported status, optional angle, timestamp, source, and
association with a region when known. A collection supports more than one hinge. Do not invent
a region-to-sensor association when an OEM API cannot establish it.

Choose radians for Day's public angle unit. UIKit already reports radians; Android's optional
`TYPE_HINGE_ANGLE` sensor reports degrees and needs conversion. Sensor availability is a runtime
query. Android folding-feature reporting and angle-sensor reporting remain independent.
[Android hinge sensor](https://developer.android.com/reference/android/hardware/Sensor#TYPE_HINGE_ANGLE)

Only subscribe at the requested rate while visible/active, release listeners on detach, and
expose unavailable status when the hierarchy or hardware changes. Use angles for optional
effects, never to calculate undocumented thresholds for native layout. Respect reduced-motion
preferences and supply a normal control for any action otherwise driven by folding.

### Explicit display operations

Add session-oriented APIs with separate operations:

```text
DisplayOperation =
    OpenPeerWindow
  | TransferToRearDisplay
  | PresentOnSecondaryDisplay
  | CameraCaptureAccessory

Availability = Unsupported | Unavailable(reason) | Available | Active(session_id)

Window::display_capability(operation) -> observed Availability
Window::request_display_session(request) -> asynchronous Result<DisplaySession, DisplayError>
DisplaySession::close() -> asynchronous completion
```

These are proposed interfaces. The system can deny a request after it was reported available.
Model cancellation, denial, loss of availability, and session-ended callbacks. Drop/close should
release native resources exactly once; retain a clear owner for callbacks arriving afterward.
Associate each presentation with its own metrics, focus, lifecycle, input context, and content
tree. Share application models, not native views that can have only one parent.

Map the Apple camera accessory only to `CameraCaptureAccessory`; do not silently substitute it
for arbitrary secondary-display content. Map Android transfer and presentation to the matching
WindowArea operations, preserving system confirmation. Use normal scene/window APIs for peer
windows. Treat HarmonyOS display operations as unsupported until a public app-level API and
device tests establish the required contract.

## Lifecycle and state restoration

Make `WindowSession` a durable identity separate from `RNode`, activity instance, scene object,
or display ID. Store a typed descriptor containing the window kind, document/task identifier,
route state, and a versioned app restoration payload. Application models and long-running work
outlive presentation hosts; native views and listeners belong to the current host.

Add a restoration registration point, for example `App::restore_window(descriptor)`, alongside
typed save/restore hooks for pieces that own transient interaction state. Do not serialize
native object pointers or arbitrary Rust closures. Bound platform saved-state payloads and keep
large documents/media in app storage, referenced by identity.

For Android, rebuild a secondary activity from its descriptor after process loss. Replace the
current finish-on-missing-Rust-node behavior. Audit `DayBridge.ctx`, global density, navigation
saved-state maps, and focus/keyboard access for accidental primary-window ownership. Route
window operations through the activity associated with the window ID.

Retain `configChanges` handling where it serves native continuity, but stop treating it as the
mechanism that makes state safe. Repair restoration first, then reevaluate each claimed
configuration flag. Locale, density, font scale, and theme changes still need correct resource
and layout refresh when the activity handles them itself.

For UIKit, use the scene session restoration lifecycle and explicit failure completion for
scene activation. Dynamically update new-window commands. Switching displays must re-read
geometry and scale without turning it into a document-close event.

Distinguish visible, resumed, focused, and resource-eligible states. In Android multi-window,
multiple activities may be visible/resumed while camera ownership changes. A media session can
continue while its preview surface is replaced; a camera can report interruption and recover
without dropping the user's capture intent. Avoid using a generic focus-lost callback to stop
all application work.

## Toolkit and component work

### UIKit implementation

- Add typed bindings for reserved regions, arrangements, hinge interaction, vertical-bar traits,
  and relevant toolbar properties. The local SDK exposes Objective-C entry points such as
  `reservedRegionsOfKind:options:`; these do not require a SwiftUI rewrite. Use a small native
  bridge only where the Rust binding generator lacks coverage.
- Query region geometry at the content host that positions the affected children. Integrate
  UIKit's invalidation hooks and scene transitions with Day's coalesced layout updates.
- Audit `DayNavPageView`, `DayHolderView`, safe-area consumption, scroll-root special cases,
  toolbar title/custom views, and covers/sheets for side insets and interior restrictions.
- Use native containers for Duo bars and split presentation. Add a separate arrangement host
  with checked controller containment; report composed behavior where native lowering cannot fit.
- Add per-scene restoration, activation errors, and runtime display capability observation.
- Build with the iOS 27.1 SDK or newer for full Duo integration. Decide the deployment floor
  separately: older OS support can retain geometry-only behavior through availability guards.
  Permission to break the Day API does not require abandoning older ordinary phones.

### Android Views implementation

- Add lifecycle-bound WindowInfoTracker observation to primary and secondary activities using
  explicit WindowManager dependencies in `day-toolkit-material/Cargo.toml` metadata.
- Combine current window metrics, insets, display density, and folding-feature observations.
  Translate window-space feature bounds into the actual Day content host before conversion to
  logical layout regions. Recompute when the host moves even if its size is unchanged.
- Implement the navigation proposal's compact stack and independent pane roles. Use retained
  page models and appropriate native view/fragment lifecycles during region reassignment.
- Support horizontal divisions and more than two usable regions in the general layout path.
  A native two-pane container can remain an optimization when its contract matches.
- Reconcile predictive Back, focus, accessibility, and fragment restoration after topology
  changes. Never move a fragment by rewriting its container ID or manipulating private drag state.
- Add WindowArea transfer/presentation sessions as optional capabilities after baseline
  continuity works. Test unsupported, unavailable, denied, active, and externally ended states.
- Treat OEM cover-screen launch restrictions as platform limitations. Day can adapt wherever
  the OS presents the app; it cannot promise unrestricted cover-screen execution on every device.

### HarmonyOS implementation

Use the existing ArkTS host and Rust bridge. Observe window size and avoid areas together with
fold status, display-mode changes, and crease-region queries. Physical fold status may precede
the resulting display mode, so publish geometry after resolving the current window/display
association. Convert display-space px to window-local vp with current transforms.

Represent all crease rectangles and preserve unknown states. Map only confirmed separating or
occluding behavior; do not infer a physical gap merely from a crease. Keep ArkUI Navigation as
the native navigation authority where it owns layout. Evaluate native fold layout containers
against the common contract, then use composed placement when they cannot express it.

Confirm APIs against the selected commercial HarmonyOS SDK and representative devices before
advertising support. Compile-check `harmony-arkui` locally. Run emulator acceptance tests in CI,
as required by the repository instructions, and use physical devices for crease/display-mode
validation. Do not run a local HarmonyOS emulator for this work.

### Camera, media, WebView, and drawing

Overhaul the external camera piece's physical `Facing` selection into an explicit choice between
physical camera identity and view-relative direction intent. Preserve physical front/back for
applications that need that meaning. Add observed camera descriptors/capabilities, preview
fit/fill/focal policy, mirroring policy, and interruption/reconfiguration state.

On Duo, use the virtual front camera for ordinary self-facing capture, and the direction
coordinator when physical-device choice or view-relative direction matters. Pass descriptors
to the capture queue; format localized UI results on the main thread. Offer the camera accessory
through the display-session contract. Video capture would be an additional feature of the
current camera piece and must be scoped and tested explicitly.

On Android, audit CameraX preview rotation, crop, target aspect, and rebinding through display
changes. Keep preview orientation and capture orientation consistent. Media players should
retain playback position and session identity while moving controls below a horizontal division.
Do not recreate the player solely because its surface dimensions changed.

WebView and canvas pieces need current local bounds, scale, and hit-test transforms. Keep the
WebView session/DOM alive during presentation changes. For app-owned embedded web UI, offer an
explicit bridge carrying local geometry/regions; do not assume the web engine exposes native
fold information. Supply web translations from generated `res::str` accessors and open bundled
documents through their generated `AssetDir` constants.

For drawing, preserve document-space coordinates and transforms through resize. Native pointer
events must map to the same region/scale revision as drawing. Keep tool palettes and selection
handles accessible while permitting decorative backgrounds to span nonoccluding folds.

## Build tooling and documentation

Update `day-cli`, native dependency aggregation, templates, and `day doctor` together. The
generated application, including its merged manifest and native linker inputs, is the product
to validate. Adding an API to Rust while leaving existing app scaffolds incompatible is incomplete.

| Deliverable | Required behavior |
| --- | --- |
| Apple SDK validation | Report linked SDK separately from deployment minimum; require 27.1+ for the full Duo feature profile |
| Native binding generation | Generate/check public SDK bindings; isolate unavailable APIs; exercise simulator and device builds |
| Android dependencies | Pin the tested stable WindowManager family through toolkit metadata and verify packaged dependencies |
| Host migration | Update existing Day apps through an explicit scaffold migration, including scene roles/restoration and Android hosts |
| Adaptive lint | Flag layout from main-screen/device-model/orientation assumptions, fixed aspect/orientation constraints, and unsupported custom bar composition |
| Geometry diagnostics | Display window/local bounds, consumed insets, region IDs, capability status, scale, and revision in developer tooling |
| Capture tooling | Record posture, native window geometry, SDK/runtime, region state, and device in screenshot provenance |
| Documentation | Replace superseded contracts in size classes, layout, navigation, windows, lifecycle, toolbars, camera, and testing |

Keep ordinary resizability automatic. Use capability declarations only for optional behaviors
that require native registration, such as camera accessories or restoration factories. Do not
add a `supports_foldables = true` switch that makes a correctness requirement optional.

Introduce an implementation-time migration guide mapping `size_class()` to explicit window or
local reads, `NavPresentation` to chrome plus pane placement, physical camera selection to the
new selection model, and node-based restoration to session descriptors. Remove obsolete APIs
after updating in-repository consumers; compatibility shims are unnecessary for this project.

All app-owned text, accessibility labels, developer-overlay UI, errors, units, empty states,
and overflow items must use generated localization accessors. Reference bundled visual and data
assets through generated resource constants. Diagnostic protocol identifiers and synthetic test
fixtures remain separate from UI text. Add examples for RTL, long translations, and large text.

## Test and release plan

### Shared conformance tests

Add deterministic geometry tests to `day-mock`/conformance and protocol coverage to DayScript.
Synthetic fixtures should identify themselves as fixtures and never masquerade as real device
measurements. Proposed script operations should distinguish injected geometry from physical
device actions, for example `layout_fixture`, `device_posture`, and `display_session`.

Test at least:

- Every width/height boundary immediately below, at, and above the threshold; same-sized
  containers on different platforms; compact panes inside expanded windows.
- Zero-width divisions, full hinges, flat nonseparating creases, partial intersections,
  camera occlusions, inactive regions, multiple hinges, and asymmetric segment sizes.
- Pixel/logical conversion at multiple densities, moved window origins, rotation, and RTL.
- Simultaneous geometry changes and feature updates; stale callbacks from destroyed hosts;
  detach, reattach, and unsupported/unavailable reporting.
- IME resize/animation, side chrome, nested inset consumption, large text, and reduced motion.
- Layout invariants: critical controls stay in usable regions; every hidden pane remains
  reachable; role minima are respected or the documented fallback is selected.
- State invariants across repeated folding: selection, focused field, draft text, undo state,
  scroll anchor, WebView identity, and media position survive.
- Independent windows with different metrics, and activity/process recreation with durable
  restoration descriptors rather than live native handles.

Property-based tests should generate regions, window sizes, and transitions, checking that
placements remain finite and within available space, avoiding active full occlusions. A visual
snapshot alone cannot establish hit-test correctness or preservation of state.

### Native device matrix

| Platform | Minimum representative coverage | Critical scenarios |
| --- | --- | --- |
| iOS | Duo simulator and physical Duo; ordinary compact iPhone; iPad in a window | Outer/inner transfer, all supported poses, camera-region activation, vertical bars, Split View, second-scene refusal, accessory loss |
| Android | Book and flip emulators; physical book and flip devices from different vendors; a dual-screen hinge fixture/device where available | Fold while editing, horizontal/vertical divisions, density/size changes, multi-window, predictive Back cancellation, system recreation |
| Optional Android display modes | A device that reports each requested WindowArea operation | Confirmation, refusal, transfer, dual presentation, external termination |
| HarmonyOS | CI emulator plus physical single- and dual-fold-axis devices before certification | Fold-status/display-mode ordering, multiple creases, window transforms, lifecycle restoration |
| Shared regression | Desktop and web windows | Continuous resizing, local adaptive layouts, no-feature fallback, multi-window independence |

Use Xcode Device Hub/Duo poses and the resizable iOS simulator for genuine native coverage.
Day's current iOS `resize` script limitation must be addressed by a supported simulator/UI test
adapter or explicitly reported as unsupported. Do not call a forced size class a successful
native fold test. Android display-size overrides likewise do not exercise hinge reporting or
outer/inner screen transitions.
[Apple preparation and testing](https://developer.apple.com/documentation/technologyoverviews/preparing-your-app-for-iphone-duo)

Use WindowManager test injection for Java tests and physical/emulated posture controls for
end-to-end tests. Device command IDs must be discovered per device, not hardcoded from one
vendor. Always restore display/posture overrides after a run.

Android's adaptive quality guidance separates basic readiness, optimized layouts, and
differentiated experiences. Aim for optimized layout and continuity across supported devices;
certify differentiated tabletop/dual-display behavior for the applications that expose it.
[Adaptive app quality guidelines](https://developer.android.com/docs/quality-guidelines/adaptive-app-quality)

### Release acceptance

1. All critical workflows remain available in every supported window/posture configuration,
   including the smallest usable regions and large accessibility text.
2. Folding produces no logical Back/pop, unwanted selection change, duplicate mount, or
   lost edit. Process recreation restores the expected task independently of live-view retention.
3. Interactive Back shows real incoming content and cancellation preserves state, including
   when a fold occurs during gesture progress.
4. Active full occlusions contain no essential content or hit targets. Physical side bars and
   camera regions remain correct in RTL and multitasking.
5. Window/display requests always complete or cancel. Runtime capability loss releases only
   the associated presentation/resources, leaving the primary task usable.
6. Measure native resize/fold frame pacing, input latency, memory, and session counts on real
   devices. Set numerical budgets from representative app baselines; require zero unintended
   view/session remounts and no sustained memory growth over repeated transitions.
7. Run the repository's formatting, lint, generated-matrix, and toolkit-specific CI gates for
   implementation changes. Publish a device/OS capability matrix rather than an unqualified
   promise that every optional feature works on every foldable.

## Implementation work packages

Estimates below are planning ranges in engineer-weeks, including focused tests and documentation.
They are not delivery commitments. Shared geometry and navigation/lifecycle changes form the
critical path; hardware availability and OEM behavior can extend the schedule.

| Phase | Work and main files | Depends on | Exit condition | Estimate |
| --- | --- | --- | --- | --- |
| 0 | Native probes and recorded geometry: Duo UIKit, Android WindowManager, current Day apps | None | Confirm coordinate/ownership contracts and capture failing continuity cases | 1–2 |
| 1 | Geometry types/events in `day-spec`; window/local environment in `day-core`; mock fixtures | 0 | Deterministic region solver, coordinate, and per-window tests pass | 2–3 |
| 2 | UIKit bindings, reserved regions, native bars, scene errors; Android feature bridge and dependency metadata | 1 | Both toolkits publish coherent geometry and correctly adapt baseline native chrome | 2–4 |
| 3 | Navigation/pane policy and Android proposal; durable window/session restoration | 1–2 | Compact/wide/fold transitions and process recreation preserve task state | 3–5 |
| 4 | Arrangements, grid/overlay avoidance, component audits, app migrations | 2–3 | News, media/camera demo, and drawing workflows pass native scenarios | 2–4 |
| 5 | Optional hinge input, camera direction, display sessions, device-specific validation | 2–4 | Each advertised optional operation handles success, denial, and revocation | 2–4 |
| 6 | HarmonyOS adapter and physical certification | 1, 3–4 | Common conformance and platform acceptance pass on the declared device set | 2–4 |
| 7 | Tooling, migration docs, regression coverage, release evidence | Throughout | Existing apps migrate cleanly and the published capability matrix matches tests | 1–2 additional |

Budget approximately 13–24 engineer-weeks for the iOS/Android program including optional
features, plus 2–4 for HarmonyOS. This is work effort, not elapsed time; phases overlap only
after their contracts stabilize. Re-estimate after the native probes and Android navigation
prototype. Avoid a general Compose migration or a new cross-platform rendering engine in this
scope; neither is necessary to establish these contracts.

The first complete milestone is baseline continuity plus readable native navigation/chrome on
both platforms. The second is content layouts that use divisions and supporting regions. The
third is optional camera/display/hinge functionality. Do not delay the first milestone until
every vendor-specific capability is available.

## Representative Day app adoption

These are proposed product adaptations; individual app workflows need a focused audit during
implementation.

| App or component | Initial adaptive behavior | Validation focus |
| --- | --- | --- |
| Day News | Article on compact; list/reader where they fit; sidebar only with sufficient space; retain the reading task when folding | Article identity, reading anchor, loaded WebView, search, predictive Back, empty selection |
| Day Skies | Current conditions with forecast; add supporting charts/maps as space permits | Chart legibility, region-local controls, no hidden essential forecast actions |
| Day Sketch | Drawing canvas with movable tools/inspector; preserve a continuous document coordinate system | Stroke continuity, stylus coordinates, zoom, selection, undo, palette reachability |
| Camera demo | Preview and controls arranged around a horizontal division; view-relative camera selection where requested | Preview/capture orientation, interruptions, camera-region activation, accessory lifecycle |
| Media demo | Playback surface plus supporting controls/transcript where useful | No playback restart during reparenting, keyboard focus, background session behavior |
| Framework showcase | Geometry inspector and reproducible examples for every contract | Serves as conformance evidence rather than relying on app-specific screenshots |

## Decisions to settle during implementation

- Establish exact region invalidation behavior for frame-based Day layout under UIKit's native
  containers. Compile-time availability is confirmed; native integration still needs probes.
- Decide how much per-piece transient state Day restores automatically and what remains an
  application responsibility. A route string alone cannot restore an editor or reading position.
- Define the role-priority defaults for two- and three-pane navigation using real Day apps.
  Make priorities explicit so unusual applications can choose differently.
- Confirm native binding coverage and the smallest bridge necessary for accessory registration.
  Prefer the existing metadata and native-source aggregation mechanisms.
- Verify Android WindowManager/WindowArea support and multiple-feature reporting on the selected
  OEM devices. Preserve a no-feature layout when the OS supplies no topology information.
- Pin the HarmonyOS SDK and verify public-app access to required operations before offering
  advanced display features. Multiple-hinge geometry support can ship before those operations.

Supporting all foldables should mean that Day adapts correctly to the window, regions, lifecycle,
and capabilities the OS actually exposes. Advanced features should be discoverable and optional.
That contract remains useful when a device has an unfamiliar aspect ratio, a restricted cover
display, two hinges, or no public hinge information at all.
