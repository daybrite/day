---
title: "Deep link design history"
description: "Deep link design history: retained contributor discussion and its implementation boundaries."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Deep link design history

> [!NOTE]
> **Status: historical proposals; some superseded by implementation.** This contributor record preserves earlier design and implementation
> discussion. Examples here are not stable API guidance. See the [app documentation](../deep-links.md)
> for the current entry point. Platform execution claims are historical and were not rerun in Phase 2.

## macOS — Planned, with one structural caveat

Registration already ships in the platform/macos scaffold's Info.plist. Intake does not: the
runtime needs the Apple Event handler (`kAEGetURL`) registered at startup, emitting the same
cold/warm paths as iOS. Two macOS-specific concerns:

1. **Launch Services registers bundles, not binaries.** macos-appkit always builds as the
   Xcode `.app` now, so every `day launch` produces a bundle a scheme can reach; the caveat
   applies only to anyone running the raw executable out of `Contents/MacOS` by hand.
2. **Old copies shadow each other.** Launch Services indexes every copy of the bundle it
   has seen (a Debug build in `build/day/`, a packed copy in `/Applications`) and picks one
   by its own rules. During development, `open <url>` may target a copy other than the one
   just built. A `day doctor` note should cover this when the handler lands.

## Windows — Planned, the most work

Nothing ships yet. The pieces, in order of effort, are:

1. **Packaged (MSIX):** a `uap:Protocol` extension in the generated manifest, which is small
   and the store-grade path.
2. **Unpackaged (dev + NSIS):** protocol registration is an `HKCU` registry ProgId written at
   install or first run; `day launch` dev builds would self-register on start, which is a
   machine-state write day currently never does. This deserves an explicit opt-in.
3. **Single-instance forwarding** (below) is not optional here: without it every link spawns
   another copy of the app. Packaged apps can use `AppInstance` redirection; unpackaged needs
   a named mutex + pipe in the C++/WinRT shim. This is the bulk of the work.

## Linux — Planned, with one limit

Registration is two lines in the generated `.desktop` file (`MimeType=x-scheme-handler/<scheme>;`
plus `DBusActivatable=true`), and both packers already generate that file. Delivery concerns:

1. **AppImage integration is opt-in by the user.** An AppImage registers no `.desktop` entry
   unless the user runs an integration tool, so scheme links reaching an AppImage build are
   not dependable. Flatpak installs integrate normally and are the reliable path.
2. **Dev launches cannot receive links** for the same reason as macOS's cargo path: there is
   no installed `.desktop` pointing at the build tree. Testing needs an installed build, or
   the dayscript tier below.
3. Warm delivery should ride DBus activation (`org.freedesktop.Application.Open`), which both
   desktops' launchers use when `DBusActivatable` is set; a fallback single-instance socket
   would cover launchers that exec directly.



*Planned:* a `scheme = "…"` key under `[app]` in Day.toml, conveyed to each
host project the same way the id is; today the scheme is fixed at scaffold time.

*Planned:* the second tier, in which the launch runner delivers the URL from outside (the
commands above) and the script asserts via `assert_route`. That proves registration and intake,
costs a per-platform runner arm, and belongs in the per-target CI jobs rather than every
walkthrough. The split matters because tier-1 failures are app bugs and tier-2 failures are
packaging bugs; a single step doing both would leave every failure ambiguous.
