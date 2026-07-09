# Tweaks — per-toolkit configuration of built-in pieces

A **tweak** configures the native widget behind a Day-created piece — the extra `NSButton` or
WinUI `Button` method call that isn't worth a whole custom piece. A piece with a tweak applied is
a **Tweaked Piece**. Day keeps owning the widget's lifecycle, layout, and managed properties; the
tweak reaches in through the same handle Day manages.

Tweaks slot between styling and native pieces in the extension ladder:

```text
styling            .font/.background/…                portable, limited surface
tweaks             .tweak / per-toolkit ext traits    the native widget, case by case   ← this doc
native pieces      renderer! per toolkit              a NEW widget kind
```

## The portable surface

Everything below builds on two prelude items and one function:

```rust
// Runs once at mount, AFTER the native widget exists. The realized node is your key
// into the per-toolkit accessors below.
button("Save").tweak(|node| { /* … */ })

// Retained access for later (event handlers, timers). Clears on unmount; reads are
// REACTIVE (a binding that calls r.node() re-runs on mount/clear transitions).
let r = NativeRef::new();
slider(v).native_ref(&r);
r.with(|node| { /* … */ });          // None once the piece is disposed

// After any native call that changes the widget's intrinsic size:
day::invalidate_size(node);
```

## Per-toolkit access

Each toolkit crate has an `ext` module with a typed (or raw) accessor and a matching
`Decorate` extension trait. The support tiers, honestly:

| toolkit | accessor | you get | tier |
|---|---|---|---|
| AppKit  | `day_appkit::with_native` / `.appkit(…)` | `&Retained<NSView>` + `MainThreadMarker` (objc2 `downcast_ref` to the class) | typed |
| UIKit   | `day_uikit::with_native` / `.uikit(…)`   | `&Retained<UIView>` + marker | typed |
| GTK     | `day_gtk::with_native` / `.gtk(…)`       | `&gtk4::Widget` (`downcast_ref` to the class) | typed |
| Android | `day_android::with_native` / `.android(…)` | `&GlobalRef` + attached `&mut JNIEnv` | typed (JNI) |
| Qt      | `day_qt::with_native_raw` / `.qt_raw(…)` | the raw `QWidget*` — bring your own C++ (below) | raw |
| WinUI   | `day_winui::with_native_raw` / `.winui_raw(…)` | the **borrowed** `IUIElement*` ABI pointer — bring your own C++/WinRT (below) | raw |
| ArkUI   | `day_arkui::with_native_raw` / `.arkui_raw(…)` | the raw `ArkUI_NodeHandle` — NDK C API | raw |

The `windows` crate ships no `Windows.UI.Xaml` bindings, which is why WinUI is a raw tier: the
pointer is real and the C++/WinRT recipe below is short, but there is no typed Rust surface to
hand you.

```rust
// Inline, per-toolkit (each trait exists only under its backend's cargo feature):
use day_appkit::AppKitExt;
button("Save").appkit(|view, _mtm| {
    if let Some(btn) = view.downcast_ref::<objc2_app_kit::NSButton>() {
        unsafe { btn.setBezelStyle(objc2_app_kit::NSBezelStyle::Toolbar) };
    }
})
```

## Packaged tweaks (`day-tweak-*` crates)

For anything reusable, package the tweak: an ordinary crate whose modifier applies the native
calls per toolkit and **no-ops where it has no coverage** — the consuming app writes zero
`#[cfg]`. Three in-tree examples span the range:

| crate | scope | demonstrates |
|---|---|---|
| `tweaks/day-tweak-button-bezel` | AppKit only | the minimal shape: one enum of symbolic constants, one setter |
| `tweaks/day-tweak-label-selectable` | AppKit, GTK, Android | one modifier across three access tiers (objc2 / gtk4-rs / JNI) |
| `tweaks/day-tweak-slider-tickmarks` | AppKit, GTK, Android, Qt, WinUI, ArkUI | a configurable feature (`Tickmarks { count, snap, position }`), including the crate's OWN Qt C++, WinRT C++, and NDK C++ |

The Cargo shape mirrors piece crates: per-backend `[features]` gating optional deps, plus

```toml
[package.metadata.day.piece]
backends = ["appkit", "gtk", "widget", "qt", "winui", "arkui"]
```

so `day build` unions `<crate>/<backend>` into the app's features automatically (Tier A.2 —
`crates/day-cli/src/pieces.rs`). Apps that build with bare cargo wire the features explicitly,
as `apps/showcase/Cargo.toml` does.

## Bring-your-own native code (the raw tiers)

**Qt.** The handle IS the `QWidget*`. Compile a few lines of C++ in your crate's `build.rs` with
`cc` + `pkg-config Qt6Widgets` (Qt itself is already linked by day-qt-sys):

```cpp
#include <QtWidgets/QSlider>
extern "C" void my_ticks(void* w, int interval) {
    auto* s = static_cast<QSlider*>(w);
    s->setTickPosition(QSlider::TicksBelow);
    s->setTickInterval(interval);
}
```

**WinUI.** `with_native_raw` hands you a *borrowed* ABI pointer via the shim's `day_winui_unbox`
seam. In your C++/WinRT (compiled with `cc` against the Windows SDK's cppwinrt headers —
mirror `tweaks/day-tweak-slider-tickmarks/build.rs`):

```cpp
winrt::Windows::UI::Xaml::UIElement e{ nullptr };
winrt::copy_from_abi(e, abi);                       // AddRef for this call's duration
auto s = e.try_as<winrt::Windows::UI::Xaml::Controls::Slider>();
if (s) s.TickFrequency(10.0);
```

**ArkUI.** The handle is the NDK `ArkUI_NodeHandle`; resolve the node API with
`OH_ArkUI_GetModuleInterface` and `setAttribute` away (see
`tweaks/day-tweak-slider-tickmarks/src/ticks-arkui.cpp`).

## Rules

- **Main thread only.** Tweaks run at mount (already on the main thread); `NativeRef::with` from
  anywhere else is a checked no-op on Apple (`MainThreadMarker`) and undefined elsewhere — don't.
- **Never destroy or reparent** the widget; Day owns its lifecycle. Don't hold raw pointers or
  handle clones past the call — hold a `NativeRef` and re-resolve.
- **Managed properties can be clobbered.** Day re-applies what it manages (title, value, enabled,
  frame, a11y) on its next patch of that node. Unmanaged properties — bezel styles, tick marks,
  selectability — are stable. If you must re-assert, do it from an `Effect` or event handler via
  `NativeRef`.
- **Size changes need `invalidate_size(node)`** — Day cannot see native mutations it didn't make.
- **Report reality.** A packaged tweak documents per-toolkit coverage (and quirks like "Material
  sliders always snap when stepped") instead of pretending uniformity; where it has no coverage
  it must be a silent, safe no-op.

## How it works

`Toolkit::Handle` is `Clone + 'static`; the object-safe tree seam exposes
`node_handle_any(node) -> Option<Box<dyn Any>>` (a CLONE of the handle — a retain / gobject ref /
`GlobalRef` clone / `Copy` pointer), and each toolkit's `ext` module downcasts to its concrete
handle type. `.tweak` is an ordinary decorator: build the piece, hand the node to the closure —
by which point `realize` has already run. `NativeRef` is a `Cell<Option<RNode>>` plus a reactive
`Trigger`, set at build and cleared by the piece's scope cleanup; slotmap generations make a
stale node a clean `None` rather than a dangling pointer.
