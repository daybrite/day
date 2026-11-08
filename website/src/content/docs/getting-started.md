---
title: "1. Open your first window"
description: Install Day and create a localized desktop window.
order: 1
section: Tutorials
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

Build a Session counter in six lessons using AppKit on macOS, WinUI on Windows, or GTK on
Linux. Use the same host platform throughout. Prerequisites: basic Rust, a terminal, and a text
editor. A target combines an OS and UI toolkit; a Piece describes UI controls or containers.

## 1. Prepare your computer

Install Git and stable Rust through [rustup](https://rustup.rs), then install the platform tools:

<!-- tutorial-step: first-app/prerequisites -->

See [system requirements](/docs/reference/system-requirements). Select full Xcode on macOS;
use the MSVC Rust toolchain on Windows. The Day installation does not install SDKs.

## 2. Install Day

<!-- tutorial-step: first-app/install -->

This compiles the latest published CLI from crates.io into Cargo's executable directory.
If `day` is not found, open a new terminal or check [PATH](/docs/how-to/troubleshooting#day-is-not-found).
This desktop debug app requires neither mobile SDKs nor a developer signing account.

Check the CLI and toolkit installation:

<!-- tutorial-step: first-app/doctor -->

## 3. Create the project

Run in your projects directory. Use an unused name if `first-app` exists.

<!-- tutorial-step: first-app/create -->

All following paths are relative to this app directory. Create these files:

<!-- tutorial-step: first-app/cargo -->

The empty `[workspace]` isolates the app from enclosing Cargo workspaces. `[lib] name = "dayapp"`
names the library used by `main.rs` and native hosts. Create source files before adding dependencies.

<!-- tutorial-step: first-app/manifest -->

Use a unique app ID for a product. Window dimensions are in points. The manifest title is
packaging metadata; the visible window title comes from the Fluent catalog.

<!-- tutorial-step: first-app/build-script -->

<!-- tutorial-step: first-app/messages -->

The build script generates typed accessors for Fluent messages.

<!-- tutorial-step: first-app/ui -->

<!-- tutorial-step: first-app/entry -->

`day::resources!()` exposes `res`. `res::window_options()` installs the catalog and sets the
localized title and manifest geometry. `root()` defines the content; `Space::L` is a spacing token.
`day_start!` supplies the native-host entry point; `main.rs` supplies the Rust desktop entry point.
`welcome_label` identifies the label for automation independently of its translated text.

Add the dependencies:

<!-- tutorial-step: first-app/dependencies -->

`cargo add` records dependency requirements in `Cargo.toml`; Cargo locks exact versions in
`Cargo.lock`. Keep the lockfile. The `day-app` package exports the `day` library;
`--rename day` sets the dependency key. `day-build` is used by `build.rs`.

<!-- tutorial-step: first-app/features -->

The default `mock` feature supports editor checks. `day build` selects the target's toolkit feature.

## 4. Prepare the native host

On macOS, generate the Swift runner and host configuration:

<!-- tutorial-step: first-app/host -->

Keep the files under `platform/macos/`. Builds generate the Xcode project.
Windows and Linux use `main.rs` and skip this command.

## 5. Build and open the window

Run from `first-app`:

<!-- tutorial-step: first-app/build -->

<!-- tutorial-step: first-app/launch -->

Expect a successful build and a window titled **Session counter** displaying
**Count the sessions you finish today.** Debug builds may append build information to the title.

<!-- tutorial-step: first-app/quit -->

`day launch` also builds; use it after subsequent edits. A missing-icon notice is expected
because this app has no custom launcher icon.

## Troubleshooting

| Symptom | Check and recovery |
| --- | --- |
| `day: command not found` | Add Cargo's executable directory (`~/.cargo/bin` by default) to `PATH`; open a new terminal. |
| Doctor rejects Xcode | Select full Xcode 27 or newer with `DEVELOPER_DIR`; run `day doctor --toolkit appkit`. Command Line Tools alone are insufficient. |
| Cargo cannot find `day` | Check `day-app`, `day-build`, and connectivity to crates.io. |
| Missing `res::str` accessor | Check the catalog path and key; rebuild. Do not edit generated Rust. |
| Launch cannot find the project | Run from the directory containing `Day.toml`. |

## Editor support

The [VS Code extension](/docs/how-to/vscode) provides project and target commands.
See [agent instructions](/docs/reference/for-agents) for coding conventions.

## Next step

Continue to [2. Handle a button click](/docs/tutorials/events).
