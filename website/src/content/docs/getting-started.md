---
title: "1. Open your first window"
description: Install Day and open a localized native window on macOS, Windows, or Linux.
order: 1
section: Tutorials
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

Build a **Session counter** in six lessons: open a window, handle a button click, update a
count, arrange controls, add navigation, and finish and test the app. Choose your development
computer above; keep that choice as you work through the lessons.

You need basic Rust knowledge, a terminal, and a text editor. A **target** combines an operating
system with a UI toolkit. This tutorial uses AppKit on macOS, WinUI on Windows, and GTK on Linux.
A Day **Piece** describes part of the interface; the toolkit creates its native controls.

## 1. Prepare your computer

Install Git and current stable Rust through [rustup](https://rustup.rs), then prepare your
platform's tools:

<!-- tutorial-step: first-app/prerequisites -->

See [system requirements](/docs/reference/system-requirements) for installation details.
On macOS, select full Xcode as the active developer directory before continuing. On Windows,
use the MSVC Rust toolchain. These are prerequisites; installing Day does not install the SDKs.

## 2. Install Day

<!-- tutorial-step: first-app/install -->

This installs the latest published CLI from crates.io into Cargo's executable directory.
The first installation compiles the CLI and downloads dependencies. Open a new terminal if
`day` is not found, or follow [PATH troubleshooting](/docs/how-to/troubleshooting#day-is-not-found).
You do not need mobile SDKs or a developer signing account for this desktop debug app.

Check the installation and the selected toolkit's tools:

<!-- tutorial-step: first-app/doctor -->

## 3. Create the project

Choose a directory where you keep your projects. Use an unused name if `first-app` already exists.

<!-- tutorial-step: first-app/create -->

Create each file below. All file paths in this and subsequent lessons are relative to this
app directory. This small project is written out to teach each part; the CLI's general-purpose
scaffold has a larger starter app.

<!-- tutorial-step: first-app/cargo -->

The empty `[workspace]` makes this project independent of any enclosing Cargo workspace.
`[lib] name = "dayapp"` gives the shared library the name used by `main.rs` and generated native
hosts. Create the source files before adding dependencies.

<!-- tutorial-step: first-app/manifest -->

The app ID identifies this app. Use your own unique ID for a product. The dimensions
are initial and minimum window sizes in points, chosen for this desktop exercise. The manifest
title is packaging metadata; the window's visible title comes from the catalog below.

<!-- tutorial-step: first-app/build-script -->

<!-- tutorial-step: first-app/messages -->

Fluent catalogs hold user-facing text. The build script generates typed accessors from these
message keys, so you can translate the interface without changing the Rust code.

<!-- tutorial-step: first-app/ui -->

<!-- tutorial-step: first-app/entry -->

`day::resources!()` exposes the generated `res` module. `res::window_options()` installs the
catalog and supplies the localized window title and manifest geometry. `root()` describes the
window's content. `Space::L` is a framework spacing token. The `day_start!` macro provides the
entry point used by generated native hosts; `main.rs` supplies a Rust desktop entry point.
The symbolic `welcome_label` ID lets UI tests find this label even after its text is translated.

Now add Day from crates.io:

<!-- tutorial-step: first-app/dependencies -->

`cargo add` takes the newest release and records it in `Cargo.toml`, and Cargo records the exact
versions in `Cargo.lock`; keep that lockfile with your app. The umbrella crate's package is
`day-app`, and its library is `day`, so `--rename day` keeps the key `day` and the code says
`use day::prelude::*`. `day-build` is a build dependency, for `build.rs`.

Then give the app its toolkit features:

<!-- tutorial-step: first-app/features -->

The default `mock` feature supports editor checks; `day build` selects the feature for the
requested toolkit instead.

## 4. Prepare the native host

On macOS, create the Swift runner and native host configuration:

<!-- tutorial-step: first-app/host -->

Keep those starter files under `platform/macos/`. The build generates the Xcode project.
Windows and Linux use the Rust desktop entry point above and do not need this command.

## 5. Build and open the window

From `first-app`, run:

<!-- tutorial-step: first-app/build -->

<!-- tutorial-step: first-app/launch -->

The build must exit successfully. The launch opens a window titled **Session counter** with
the text **Count the sessions you finish today.** A debug build can append build information to
the title.

<!-- tutorial-step: first-app/quit -->

`day launch` builds again, so later
lessons need only that command after an edit. A note about a missing icon master is expected:
this app has no custom launcher icon.

## Troubleshooting

| Symptom | Check and recovery |
| --- | --- |
| `day: command not found` | Check that Cargo’s executable directory (`~/.cargo/bin` by default) is on `PATH`, then open a new terminal. |
| Doctor rejects Xcode | Select full Xcode 27 or newer with `DEVELOPER_DIR`, then run `day doctor --toolkit appkit`. Command Line Tools alone cannot build the host project. |
| Cargo cannot find `day` | Check the `day-app` and `day-build` dependencies and that your connection can reach crates.io. |
| A generated `res::str` name is missing | Check the catalog file path and exact key spelling, then rebuild. Do not write generated Rust files by hand. |
| The launch cannot find a project | Run it from the directory containing this app's `Day.toml`. |

## Editor support

The [VS Code extension](/docs/how-to/vscode) provides Day project and target commands in your editor.
For AI-assisted work, start with the [agent guide](/docs/reference/for-agents). Explore the
[Gallery](/gallery) for examples of larger apps.

## Next step

Continue to [2. Handle a button click](/docs/tutorials/events).
