---
title: Overview
description: What Day is, how it works, and the platforms it targets.
order: 1
---

**Day** is a Rust framework for building applications that look and behave like native
applications on every platform, because they are native applications.

You write your UI once, in Rust, as a declarative tree of **Pieces** (what SwiftUI calls
a View and Flutter calls a Widget). Each Piece is realized by a native component
(`NSTextField`, `UILabel`, `android.widget.Button`, `GtkEntry`, a Qt `QSlider`, a WinUI `TextBox`)
through a per-platform **toolkit** backend. Day owns layout, reactivity, localization, accessibility
policy, and scripting; the platform owns pixels, text input, scrolling physics, and assistive
technology.

```rust
use day::prelude::*;

fn counter() -> AnyPiece {
    let count = Signal::new(0i64);
    column((
        label(move || format!("{} clicks", count.get())),
        button("Tap me").action(move || count.update(|c| *c += 1)),
    ))
    .spacing(12.0)
    .padding(16.0)
    .any()
}
```

That same function renders as a stack of native labels and buttons on macOS, iOS, Android, Linux,
and Windows. There's no web view, custom renderer, or per-platform fork involved.

## The targets

A *target* is an `(OS, toolkit)` pair whose toolkit supports that OS. Day ships ten:

| Target | OS | Toolkit |
|---|---|---|
| `macos-appkit` | macOS | AppKit |
| `ios-uikit` | iOS | UIKit |
| `android-widget` | Android | android.widget / android.view |
| `linux-gtk` | Linux | GTK 4 · libadwaita |
| `linux-qt` | Linux | Qt 6 Widgets |
| `windows-winui` | Windows | WinUI 3 |
| `macos-gtk`, `macos-qt` | macOS | GTK 4, Qt 6 |
| `windows-gtk`, `windows-qt` | Windows | GTK 4, Qt 6 |

Because GTK and Qt are themselves portable, the non-default combinations are valid targets too.
One `day launch -p <target>` builds and runs your app on any of them.

## How it works: build once, bind forever

Most declarative UI frameworks re-run your view functions and diff the result on every state
change. Day does not. It builds the native widget tree once, then binds reactive
values directly to native attributes. When a `Signal` changes, only the widgets that read it are
updated. There is no virtual tree, no reconciliation pass, and no re-execution of your view code.

- A `Signal<T>` is a `Copy`, cheap-to-clone reactive cell.
- `bind`, `when`, and reactive closures wire a signal to a native attribute or a subtree.
- Changes are batched and flushed to the native toolkit at safe points in its own run loop.

The result is SwiftUI-style authoring with the runtime profile of hand-written native code: no
diffing on the hot path, and one native widget per Piece.

## One binary per target

Day compiles exactly one toolkit backend into each binary (selected by a Cargo feature). There is
no runtime toolkit abstraction to pay for: the AppKit build contains only AppKit code, and the
Android build only the JNI bridge. This keeps binaries small and calls direct.

## What Day is not

- **Not a renderer.** Day never rasterizes text or widgets itself. The `canvas` Piece delegates to
  the platform's native 2D API. It doesn't bundle Skia, vello, or an embedded web view for core UI.
- **Not pixel-identical across platforms.** A Day app looks like a Mac app on macOS and a Material
  app on Android. The goal is consistent behavior and information architecture with native look
  and feel on each platform, rather than one skin everywhere.
- **Not lowest-common-denominator.** Where platforms diverge, the Piece API exposes per-target
  styling; where a platform lacks a control, the toolkit composes one from primitives.

Continue to [Why Day](/docs/benefits), or jump into the [API tour](/docs/api-tour).
