---
title: Tweaks
description: "Configuring the native widget behind a built-in piece — per toolkit, case by case — without writing a custom piece."
order: 26
section: Guides
---

Sometimes the widget Day gives you is exactly right except for one platform-specific detail: you
want the standard button, but with AppKit's toolbar bezel; the standard slider, but with WinUI's
tick marks. Writing a whole custom piece for two method calls is disproportionate — so Day has
**tweaks**: a supported way to reach the real native widget behind a built-in piece and configure
it, while Day keeps owning layout, lifecycle, and everything else. A piece with a tweak applied
is a **Tweaked Piece** — same widget, same handle, a little more configured.

The showcase's Tweaks page (in the [gallery](/gallery)) demonstrates everything on this page.

## Applying a tweak

The portable entry point is a modifier that runs once at mount, after the native widget exists:

```rust
button("Save").tweak(|node| {
    // `node` is the realized node; per-toolkit accessors turn it into a native handle.
})
```

Each toolkit crate adds a typed extension trait over that, which is what you'll normally use. The
closure gets the native widget **and its concrete class name**, so a tweak knows exactly what it's
poking:

```rust
use day_appkit::AppKitExt;   // exists only in the appkit build

button("Save").appkit(|view, class, _mtm| {   // class == "NSButton"
    if let Some(btn) = view.downcast_ref::<objc2_app_kit::NSButton>() {
        unsafe { btn.setBezelStyle(objc2_app_kit::NSBezelStyle::Toolbar) };
    }
})
```

`.gtk(|widget, class| …)`, `.uikit(|view, class, mtm| …)`, and `.android(|view, class, jni_env| …)`
follow the same shape with each platform's own types. Qt, WinUI, and ArkUI sit behind C shims, so
their accessors hand out the raw native pointer (plus the class) instead, with a short
bring-your-own-C++ recipe — honest tiers, spelled out in the [tweaks reference](/docs/internal/tweaks).

That class name is what makes tweaks robust. On the typed tiers it's the *live* widget's runtime
class, so if a piece ever has more than one native backing — a plain `label` as `UILabel`, a
link-bearing one as `UITextView` — the tweak can `match` on the class instead of guessing a
downcast. On the raw tiers, where Rust can't introspect an opaque pointer, it's the metadata your
C++ needs: pass it across the shim and guard the cast, rather than blindly reinterpreting the
pointer as the wrong control.

Two rules cover most of what can go wrong. Day re-applies the properties it *manages* (a
button's title, a slider's value) on its next update, so tweak the properties Day doesn't touch —
bezels, tick marks, selectability — and they're stable. And if a native call changes the widget's
intrinsic size, tell layout with `day::invalidate_size(node)`, because Day can't see mutations it
didn't make.

## Reaching a widget later

A mount-time hook covers configuration; for imperative access afterward — from an event handler,
say — capture a `NativeRef`:

```rust
let save_ref = NativeRef::new();

column((
    button("Save").native_ref(&save_ref),
    button("Flash the save button").action(move || {
        save_ref.with(|node| { /* per-toolkit accessor on `node` */ });
    }),
))
```

The ref clears automatically when the piece unmounts, so a late timer or async completion is a
safe `None`, never a dangling widget. Reads are reactive, too: a label whose closure calls
`save_ref.node()` re-renders when the referenced piece mounts or disappears.

## Packaged tweaks

Anything worth reusing is worth packaging: a `day-tweak-*` crate wraps the per-toolkit calls in
one modifier and no-ops on toolkits it doesn't cover — so the *app* using it stays completely
free of `#[cfg]`. Three in-tree examples span the range from trivial to fully cross-platform:

```rust
use day_tweak_button_bezel::{Bezel, ButtonBezelTweak};
use day_tweak_label_selectable::LabelSelectableTweak;
use day_tweak_slider_tickmarks::{SliderTickmarksTweak, Tickmarks};

button("Save").bezel(Bezel::Toolbar);          // AppKit only; stock elsewhere
label("Copy me").selectable();                 // AppKit, GTK, Android
slider(v).tickmarks(Tickmarks::count(11).snap(true));  // six toolkits, incl. its own C++
```

The tick-marks crate is the one to study when you write your own: it configures a native feature
on six toolkits through every access tier Day has — objc2, gtk4-rs, JNI, and its *own* compiled
Qt C++, WinRT C++, and ArkUI NDK code — and it documents per-platform reality plainly (Material
sliders always snap when stepped; UIKit has no native tick API, so there it's a no-op). Publishing
one is publishing a crate: consumers add a dependency, and `day build` wires the per-toolkit
features automatically.

The [tweaks reference](/docs/internal/tweaks) has the full per-toolkit matrix, the native-code
recipes, and the mechanics underneath. For a genuinely new widget rather than a configured
existing one, you want a [native piece](/docs/extending) instead.
