---
title: Overview
description: "Meet pieces, signals, and the native controls behind a Day app."
order: 1
section: Start here
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

Day is a Rust framework for building apps with native platform controls. You write the
interface and application logic in Rust; Day connects them to the UI toolkit for each target.
A macOS build uses AppKit, an Android build uses Android views and Material controls, and a
web build uses HTML controls through WebAssembly.

If you want to try it, [create your first app](/docs/getting-started). If you are choosing a
framework, read [Is Day a good fit?](/docs/benefits) and check [platform support](/docs/platforms).

## What an interface looks like

Day calls a control or a group of controls a **piece**. Functions such as `label`, `button`,
and `column` create pieces; builder methods configure them. A function returning `impl Piece`
can become a reusable part of your interface.

```rust
use day::prelude::*;

pub fn root() -> impl Piece {
    let count = Signal::new(0i64);

    column((
        label(move || format!("Count: {}", count.get())),
        button("Add one").action(move || count.update(|n| *n += 1)),
    ))
    .spacing(12.0)
    .padding(16.0)
}
```

Here, `count` holds the current value. The button increments it, and the label’s closure
reads it. Day remembers that dependency and updates the label when the value changes.
The whole `root()` function does not run again.

These two ideas—[pieces](/docs/pieces) for composition and [signals](/docs/reactivity) for
change—are enough to start building screens. [Layout](/docs/layout) explains how those
screens are measured and arranged.

## What is shared, and what is native

Your Rust code describes the controls, their layout, and what happens when someone uses them.
Day creates native widgets, connects event handlers, and applies changes to their properties.
The platform toolkit supplies text input, selection, scrolling, and the controls’ appearance.

This gives you a shared interface, with visible differences between platforms. You can adjust
fonts, colors, and layout with [styling](/docs/styling), or use [platform-specific tweaks](/docs/tweaks)
when a native control needs extra configuration. Features outside Day’s common API may need
an integration for each platform you support.

## Working on an app

A project has a `Day.toml` manifest, a Cargo package, and the host files required by its
platforms. The Day CLI creates that structure, checks toolchains, builds and launches the app,
and prepares release packages. Code changes require a rebuild and relaunch.

Once your first screen works, add [navigation](/docs/navigation),
[translations](/docs/localization), and [accessibility labels](/docs/accessibility) as needed.
Use [dayscript](/docs/dayscript) to test interactions in a running app and capture screenshots.
The [documentation index](/docs) groups the guides by task.

<span id="the-targets"></span>

## Before you commit to a platform

Day’s targets have different levels of testing and feature coverage. The
[platform reference](/docs/platforms) is the source for support tiers, known limitations,
and links to setup instructions. Try the controls and integrations your app depends on,
and test them on each platform you intend to ship.
