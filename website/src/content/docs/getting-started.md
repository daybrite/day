---
title: "1. Open your first window"
description: Install Day and open a localized native window on macOS with AppKit.
order: 1
section: Tutorials
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

Build a **Session counter** in six lessons: open a window, handle a button click, update a
count, arrange controls, add navigation, and finish and test the app. Start with macOS and
AppKit. [Other platforms](/docs/tutorials/other-platforms) are an optional branch after the app works.

On Windows or Linux, start with [Windows setup](/docs/reference/platforms/windows-winui),
[Linux GTK setup](/docs/reference/platforms/linux-gtk), or [Linux Qt setup](/docs/reference/platforms/linux-qt).
This walkthrough uses macOS commands.

You need basic Rust knowledge, a terminal, and a text editor. No previous Day knowledge is
required. A **target** is an operating system and UI toolkit pair; `macos-appkit` uses Apple's
AppKit toolkit. A Day **Piece** describes part of the interface; the backend creates its native
controls. A window hosts that interface.

## 1. Prepare your Mac

Use an Apple silicon Mac with full Xcode 27 or newer, Git, and a current stable Rust toolchain
installed through [rustup](https://rustup.rs). Open Xcode once to finish its installation.
See [system requirements](/docs/reference/system-requirements#macos) for SDK setup details.

If `xcodebuild` selects Command Line Tools instead of full Xcode, select your Xcode installation
for this terminal session:

```sh
export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
```

Adjust that path if you installed Xcode elsewhere. This setting does not install Xcode.

## 2. Install Day

```sh
cargo install --git https://github.com/daybrite/day day-cli
```

This installs the `day` command in Cargo's executable directory. The first installation compiles
the CLI and downloads its dependencies, so it may take a few minutes. You do not need mobile SDKs
or a developer signing account for this local debug app.

If your terminal says `day: command not found`, open a new terminal first. For a default Rust
installation, you can also make it available in the current terminal:

```sh
export PATH="$HOME/.cargo/bin:$PATH"
day --version
```

Or run `"$HOME/.cargo/bin/day" --version` directly without changing PATH. See
[PATH troubleshooting](/docs/how-to/troubleshooting#day-is-not-found) for a permanent fix or a
custom installation directory.

Check the installation and AppKit tools:

```sh
day --version
day doctor --toolkit appkit
```

## 3. Create the project

Choose a directory where you keep your projects. Use an unused name if `first-app` already exists.

```sh
mkdir -p first-app/src first-app/resource/locales/en
cd first-app
```

Create each file below. All file paths in this and subsequent lessons are relative to this
app directory. This small project is written out to teach each part; the CLI's general-purpose
scaffold has a larger starter app.

Create `Cargo.toml`:

<!-- tutorial-file: Cargo.toml -->
```toml
[package]
name = "first-app"
version = "0.1.0"
edition = "2024"

[workspace]

[lib]
name = "dayapp"

[features]
default = ["mock"]
mock = ["day/mock"]
appkit = ["day/appkit"]

[dependencies]
day = { package = "day-app", version = "0.6" }

[build-dependencies]
day-build = "0.6"
```

Cargo fetches the Day crates from crates.io and records the selected versions in `Cargo.lock`.
Keep that lockfile with your app. The umbrella crate's package is `day-app`, and its library is `day`,
so the dependency keeps the key `day` and the code says `use day::prelude::*`. The empty `[workspace]` makes this project independent of any enclosing
Cargo workspace. The default `mock` feature supports editor checks; `day build -t macos-appkit`
selects the real AppKit backend instead. `[lib] name = "dayapp"` gives the shared library the
name used by `main.rs` and generated native hosts.

Create `Day.toml`:

<!-- tutorial-file: Day.toml -->
```toml
schema = 1

[app]
id = "dev.daybrite.tutorial.session-counter"
title = "Session counter"
build = 1
targets = ["macos-appkit"]

[window]
width = 640
height = 480
min_width = 400
min_height = 320
```

The app ID identifies this app. Use your own unique ID for a product. The dimensions
are initial and minimum window sizes in points, chosen for this desktop exercise. The manifest
title is packaging metadata; the window's visible title comes from the catalog below.

Create `build.rs`:

<!-- tutorial-file: build.rs -->
```rust
fn main() {
    day_build::prebuild_project().expect("day-build: prebuild");
}
```

Create `resource/locales/en/app.ftl`:

<!-- tutorial-file: resource/locales/en/app.ftl -->
```ftl
app_title = Session counter
welcome = Count the sessions you finish today.
```

Fluent catalogs hold user-facing text. The build script generates typed accessors from these
message keys, so you can translate the interface without changing the Rust code.

Create `src/lib.rs`:

<!-- tutorial-file: src/lib.rs -->
```rust
use day::prelude::*;

day::resources!();
day::day_start!(options: res::window_options(), root);

pub fn root() -> impl Piece {
    label(res::str::welcome())
        .padding(Space::L)
        .id(id!(welcome_label))
}
```

Create `src/main.rs`:

<!-- tutorial-file: src/main.rs -->
```rust
fn main() {
    day::launch(dayapp::res::window_options(), dayapp::root);
}
```

`day::resources!()` exposes the generated `res` module. `res::window_options()` installs the
catalog and supplies the localized window title and manifest geometry. `root()` describes the
window's content. `Space::L` is a framework spacing token. The `day_start!` macro provides the
entry point used by generated native hosts; `main.rs` supplies a Rust desktop entry point.
The symbolic `welcome_label` ID lets UI tests find this label even after its text is translated.

## 4. Create the native host

Create the native host source files from the CLI's built-in template:

```sh
day project add-target macos-appkit
```

The CLI reports that the target is already declared and materializes missing files under
`platform/macos/`, including `DayApp.xcconfig` and the Swift runner. Keep these generated starter
sources. The build derives the Xcode project; you do not need to create it manually.

## 5. Build and open the window

From `first-app`, run:

```sh
day build -t macos-appkit
day launch -t macos-appkit
```

The build must exit successfully. The launch opens a window titled **Session counter** with
the text **Count the sessions you finish today.** A debug build can append build information to
the title. Quit the app with **Command-Q** before continuing. `day launch` builds again, so later
lessons need only that command after an edit. A note about a missing icon master is expected:
this app has no custom launcher icon.

## Troubleshooting

| Symptom | Check and recovery |
| --- | --- |
| `day: command not found` | Check that Cargo’s executable directory (`~/.cargo/bin` by default) is on `PATH`, then open a new terminal. |
| Doctor rejects Xcode | Select full Xcode 27 or newer with `DEVELOPER_DIR`, then run `day doctor --toolkit appkit`. Command Line Tools alone cannot build the host project. |
| Cargo cannot find `day` | Check the Git URLs in `Cargo.toml` and that your connection can reach GitHub. |
| A generated `res::str` name is missing | Check the catalog file path and exact key spelling, then rebuild. Do not write generated Rust files by hand. |
| The launch cannot find a project | Run it from the directory containing this app's `Day.toml`. |

## Editor support

The [VS Code extension](/docs/how-to/vscode) provides Day project and target commands in your editor.
For AI-assisted work, start with the [agent guide](/docs/reference/for-agents). Explore the
[Gallery](/gallery) for examples of larger apps.

## Next step

Continue to [2. Handle a button click](/docs/tutorials/events).
