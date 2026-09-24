---
title: CLI & projects
description: The Day command-line tool, the conventional project layout, Day.toml, and dayscript.
order: 30
section: Build & ship
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

The `day` command manages a project from creation through testing and packaging. The same
commands work in a terminal, an editor, or CI. This reference covers the commands and the
project settings they use.

## The commands

`day --help` groups commands by task. Use `day help <command>` for its options and subcommands,
for example `day help icon new`. A target flag accepts `-p`, `--platform`, or `--target`.
`icon`, `sign`, and `project` require a subcommand; bare `day doctor` runs quick toolchain checks.
`day doctor verify` builds and packages test apps and can take several minutes.

```bash
day new                      # interactive: scaffold an app, a piece, or a part
day new app my-app           # scaffold a new app non-interactively (--no-website to skip the site config)
day project add-target android-mdc   # add a target to an existing app
day localize list|add|remove # survey the project's locales, or add/remove one on every surface at once
day prepare                  # render the derived host files (icon catalogs, mipmaps) under build/day/host (--check: CI gate)
day open -p <target>         # prepare, then open the host project in Xcode / Android Studio / DevEco
day icon build              # render platform icons from the source icon
day icon new                # create a seeded source icon and render its platform icons
day icon check              # check for icon drift without writing files (exit 5)
day build   -p macos-appkit  # build one target
day launch  -p macos-gtk     # build + run on a target
day launch  --git <url>      # clone a repository and run the app in it — no checkout needed
day launch  --day-src <path|url>  # run this app against another day, for one build
day build   --flavor custom  # build the Day-custom.toml flavor of this app (/docs/flavors)
day pack    -p macos-appkit  # build + sign + produce a distributable artifact (.dmg here)
day sign check              # report release-signing readiness without printing secrets
day rebuild <artifact>       # rebuild a shipped artifact from its provenance and compare the bytes
day lint                     # check ids, Fluent coverage, project shape (--fix applies what it can)
day devices list             # simulators, emulators and phones a mobile target can launch onto
day devices boot -p ios-uikit <id>  # start a simulator/AVD so it can be launched onto
day doctor                   # check toolchains for every target
day doctor verify           # doctor, then scaffold + build + pack a throwaway app per target
day stop --all               # stop running launches (sessions in build/day/sessions.json)
day clean                    # remove all build artifacts (build/, target/, gradle/hvigor outputs); --dry-run lists them
day relaunch --all-running   # stop + rebuild + relaunch — "apply my changes"
day drive -p <t> --steps-json '…'   # drive a running app with dayscript steps
day patch --local <checkout> # build against a local day (or piece) checkout; repeatable (--check: verify)
day patch --git <url>[@<ref>] # build against a fork of day, for the whole graph; commit the table
day mcp-server               # serve Day tools to AI agents (Model Context Protocol, stdio)
day version                  # print the CLI version, build profile, and git ref (always the commit)
```

`--flavor <name>` applies to every command, not only `build`: it layers `Day-<name>.toml` over
`Day.toml` so one source tree ships as several apps. [Build flavors](/docs/flavors) covers the
file and what each key changes.

`day patch` switches an app from the published git dependency to a local checkout of day, or
of an external [piece](/docs/glossary#piece) or part, or to a fork of day, and verifies the switch took; [Developing Day and an app together](/docs/local-development) covers
when and how to use it.

`day pack` produces a standalone, installable package per [target](/docs/glossary#target). See
[Packaging & distribution](/docs/packaging) for formats, signing, and CI:

| target | artifact |
|---|---|
| `macos-appkit` | `.dmg` (codesign → notarize → staple) |
| `ios-uikit` | `.ipa` (App Store export; without signing config, an unsigned device `.ipa` named `<stem>-ios-uikit-unsigned.ipa`) |
| `android-mdc` | `.apk` + `.aab` (release-signed) |
| `linux-gtk` / `linux-qt` | single-file `.flatpak` bundle **and** a `.appimage` |
| `windows-xaml` | `.msix` + NSIS `-setup.exe` |
| `harmony-arkui` | `.hap` |
| `web-dom` | none — `day build` already emits a self-contained static `dist/` |

Run `day new` with no arguments to be walked through choosing what to create (app / piece / part) and
which platforms and [toolkits](/docs/glossary#toolkit) to support. Every question has an equivalent flag, so the same choices
can be made non-interactively, e.g. `day new app my-app --toolkit ios-uikit --toolkit macos-appkit
--appid com.example.myapp --title "My App"`. Scaffolds currently depend on `day` from its git
remote (the framework crates are not yet published to crates.io); once they are, `--registry`
pins them to your CLI's version from crates.io and will become the default.

`--appid` has to name an id every chosen target accepts, since one id serves them all. Android and
HarmonyOS read it as a Java package name, so each segment starts with a letter and carries no
hyphen (`io.github.fair_starter`, not `io.github.fair-starter`); Apple also takes hyphens. `day
new` refuses an id a chosen target would reject, and derives one that works when the flag is
left out.

`day new app` scaffolds a working starter: a typed-route [sidebar](/docs/glossary#sidebar) over four sample panels (a
[reactive](/docs/glossary#reactive) counter, a controls tour, a canvas dial, and a drill-down stack), with [locales](/docs/glossary#locale), a
[dayscript](/docs/glossary#dayscript) [walkthrough](/docs/glossary#walkthrough) (`day launch -p <target> --script dayscript/demo.yaml`), and the native
host projects the mobile targets build through. The scaffold comes from a **template**: a plain
directory tree whose file contents *and paths* are rendered with mustache-style placeholders:

| placeholder | what it renders to |
|---|---|
| `{{name}}` | the cargo package name, lowercase kebab |
| `{{repo}}` | the name as typed, case intact — the scaffold directory and the Pages path |
| `{{ident}}` | the crate's Rust extern name (hyphens → underscores) |
| `{{snake}}` / `{{pascal}}` | a snake_case stem and its PascalCase form |
| `{{title}}` | the app's display name |
| `{{id}}` | the application id, reverse-DNS |
| `{{org}}` | the id's organization segment, which the generated website claims as its Pages host |
| `{{scheme}}` | the deep-link scheme derived from the name |
| `{{day_dep}}` / `{{day_build_dep}}` / `{{day_piece_deps}}` | dependency lines for the source the app was scaffolded against (git, crates.io, or a local checkout) |
| `{{targets_toml}}` | the chosen targets, quoted, for `Day.toml` |
| `{{targets_list}}` | the same targets bare, for a CI workflow's `targets:` input |
| `{{first_target}}` | the first chosen target, for the commands a README prints |

The built-in template is embedded in the
CLI (a fresh `cargo install day-cli` scaffolds offline); bring your own with:

```bash
day new app my-app --template ./my-template          # a local directory
day new app my-app --template https://github.com/you/tpl#v1   # a git repo (optional #ref)
```

`day new piece` also writes `demo/` beside the crate: the same app template, cut to one page that
shows the piece, with a `dayscript/demo.yaml` walkthrough. A native piece's demo targets the
platforms its toolkits draw on, and a composite piece's demo targets all of them. The demo depends
on the piece by path, so `day launch -p <target> --script dayscript/demo.yaml`, run from `demo/`,
builds the code you are editing. `--no-demo` leaves it out.

`day new --describe` prints the questions themselves (every kind's fields, their options, and the
flag each one fills) as a versioned JSON document. It takes no project, so an editor can read it
to build its own New Project dialog without copying the target list into a second place. The VS
Code extension's wizard is rendered entirely from it.

```bash
day new --describe | jq '.kinds[] | {id, fields: [.fields[].id]}'
```

Template conventions: a trailing `.hbs` on a filename is stripped after rendering (use
`Cargo.toml.hbs` so tooling doesn't mistake the template for a Rust package), `_gitignore`
becomes `.gitignore`, `_vscode/` and `_github/` become `.vscode/` and `.github/`, non-UTF-8 files
(icons) copy verbatim, and an unknown `{{placeholder}}` is an error rather than silent empty
output. A literal `{{` is written `\{{`, which is what a GitHub Actions expression in a template's
workflow needs (`flavors: $\{{ github.ref }}` scaffolds as `flavors: ${{ github.ref }}`), and
`{{!-- a note --}}` is a comment the scaffolded app never sees. Files under `platform/<os>/` belong
to that OS's targets and are only scaffolded for targets that need them.

A template repository's own `.git`, `target/` and `.github/` are skipped on load: the workflows
under `.github/` are that repository's CI, and the ones a scaffolded app should get travel as
`_github/`.

Add a platform later with **`day project add-target <target>`** (repeatable / comma-separated):
it appends the target to `Day.toml`'s `[app] targets` array (via toml_edit, so your comments
and formatting survive) and materializes the target's native host project (`platform/android/`,
`platform/ios/`, `platform/ohos/`) from the same template, never overwriting existing files.
Pass the same `--template` the app was created with if it wasn't the built-in one.
`day project migrate-xcode` moves Xcode build settings into `DayApp.xcconfig` files without
building the app. `day build` also performs this migration when needed.

Use `day sign check` to check signing configuration, or `day sign status <id>` to query a
notarization submission. See [signing configuration](/docs/packaging#signing-configuration).

CI and editor integrations check for the commands they need. If a command is unavailable,
update the CLI from the same Day checkout or Git revision as the integration. Metadata JSON
and MCP tool names remain stable.

### Running a repository directly

`day launch --git <url>` runs an app you haven't checked out. It clones the repository, finds the
Day project inside it, and launches that. With no `-p`, it builds for your host's default
toolkit, so trying a sample app is one command:

```bash
day launch --git https://github.com/daybrite/Day-Rise.git
day launch --git https://github.com/daybrite/Day-Rise.git@main       # a branch, tag, or commit
day launch --git https://github.com/daybrite/Day-Skies.git -p ios-uikit --env WEATHER_MOCK=1
```

The checkout is cached per URL and ref, under `$XDG_CACHE_HOME/day/git/…` when that's set and
your platform's cache directory otherwise (`~/Library/Caches/day` on macOS, `~/.cache/day` on
Linux, `%LOCALAPPDATA%\day\cache` on Windows). Every run prints the path:

```text
     Cloning daybrite/Day-Rise @ main
    Checkout ~/Library/Caches/day/git/github.com/daybrite/Day-Rise/main
  Defaulting macos-appkit (no --platform given)
    Building macos-appkit (xcodebuild Debug, macosx)
   Launching macos-appkit
```

That path is a working checkout, so `cd` there and start editing. The build tree lives inside it,
which is why the second run is an incremental compile rather than a fresh one. A later run
fetches and fast-forwards; once you've edited or committed in the checkout, it stops updating and
builds what's on disk, telling you so. Nothing is ever reset or force-updated. `Cargo.lock` is the
one exception: building here is what rewrites it, so it doesn't count as your edit, and it's
discarded when an incoming commit carries a new one. `--dir <d>` clones somewhere you name instead
of the cache, and `day stop --project <that path>` ends a `--detach`ed run.

Each ref gets its own checkout, and a build tree runs to a couple of GB per target, so
`Day-Rise.git` and `Day-Rise.git@main` cost twice what one of them does; pick a spelling and keep
to it. `day clean` works on projects, not on this cache; to reclaim the space, delete the printed
directory, or all of them at once:

```bash
rm -rf "${XDG_CACHE_HOME:-~/Library/Caches}/day/git"    # ~/.cache/day/git on Linux
```

For a repository holding more than one Day project, `--project` selects one by its path within the
repo; without it, an ambiguous repository lists what it found.

`--script` works too, and a relative path that isn't in your current directory is looked up in the
checkout, so a repository's own walkthrough runs by the name it has there:

```bash
day launch --git https://github.com/daybrite/Day-Showcase.git --script dayscript/walkthrough.yaml
```

`--git` builds and runs code from a URL. Pass URLs you trust, the same way you would with
`cargo install --git`.

### Trying another version of Day itself

`--day-src` points the app's `day` dependencies somewhere else for one build. It takes a path to a
day checkout, or a git URL with an optional `@<ref>` — a branch, a tag, a commit, or someone else's
fork. It's on `day build` and `day launch` both, since answering "does this branch fix the bug?"
means building with each version and looking at both:

```bash
day launch --day-src ../day                                               # a local checkout
day launch --day-src https://github.com/daybrite/day.git@experimental-nav  # a branch
day launch --day-src https://github.com/someone/day.git@fix-482            # a PR fork
```

`day patch` writes `.cargo/config.toml` and every later build uses it until you delete it;
`--day-src` computes the same `[patch]` table, hands it to one cargo run, and leaves the project
exactly as it found it — `Cargo.lock` included, which cargo rewrites during the build and which the
CLI puts back afterwards. Use `day patch` when you're developing the framework and the app together
for a while, and `--day-src` when you want one look.

Each day-src gets its own build tree under `build/day/day-src/<slug>/`, so two versions can be
compared without either one's compile throwing away the other's:

```text
     Day src https://github.com/daybrite/day.git @ main
    Checkout ~/Library/Caches/day/git/github.com/daybrite/day/main
     Patched 33 day crate(s) → https://github.com/daybrite/day.git @ main
   Launching macos-appkit
```

Both apps can run at once, and in a debug build each window's title says which framework it came
from — `Day Rise (0.1.0+main-2d77edbf/appkit)` beside `Day Rise (0.1.0+day-4aea8304/appkit)`.
Switching back to a version you've already built is an incremental compile, not a fresh one.

On Android and HarmonyOS only the Rust half is isolated; Gradle and hvigor keep their own shared
build directories, so the packaging step re-runs when you switch. And `day pack` takes no
`--day-src`, so a shipped artifact always records the framework that built it.

`day launch` streams the app's stdout/stderr back to your terminal and can drive it with a script:

```bash
# run a dayscript walkthrough after launch, capturing localized screenshots
day launch -p macos-gtk --script dayscript/walkthrough.yaml --locale fr

# capture variants of the same walkthrough: `--variant` names the screenshot subdirectory
# (build/day/screenshots/<target>/<variant>/) and DAY_THEME forces the theme on every backend
day launch -p macos-gtk --script dayscript/walkthrough.yaml --variant dark --env DAY_THEME=dark

# variant loops share one binary (theme and locale are runtime inputs): build once, then
# `--skip-build` reuses the artifact — on iOS this pays xcodebuild once instead of per variant
day build -p ios-uikit
day launch -p ios-uikit --skip-build --script dayscript/walkthrough.yaml --variant dark --env DAY_THEME=dark

# --record captures what you do into a replayable dayscript: drive the app by hand, and the file
# is rewritten continuously (see the dayscript "Recording" guide)
day launch -p macos-appkit --record recording.yaml
```

CI runs each showcase walkthrough once per theme × locale (`light`/`dark` × en/fr/ar/zh-CN) with
one command: `day launch --themes light,dark --locales en,fr,ar,zh-CN --script …` builds once and
expands the matrix internally, naming each run's variant `<theme>` (for the default locale) or
`<theme>-<locale>`. The [gallery](/gallery) lets you flip every screenshot between those variants.

A scripted run captures the desktop toolkits and the web build at 2560×1600 pixels, a
1280×800-point window at 2×. `--capture-size 2880x1800`, the `DAY_CAPTURE_SIZE` variable, or a
`[screenshots]` table in `Day.toml` changes it, and `window` captures at the app's own
`[window]` size ([capture size](/docs/dayscript#capture-size)).

### Simulators, emulators, and devices

For HarmonyOS, run `day devices boot -p harmony-arkui --headless` to start the configured
Oniro image without a window. It always waits for boot readiness, so `--wait` is accepted but
unnecessary. `ID`, `--device`, `--os`, and `--orientation` are rejected for this target.


Without a device flag, a launch goes to every runtime of that kind it can see: every booted iOS
simulator, every connected Android device and emulator. That suits a capture sweep. To target one
phone, name it with a flag. Selection is one flag per runtime, so a single command can send each
`-p` somewhere different:

| Flag | Selects | Find them with |
| --- | --- | --- |
| `--ios-device <name\|udid>` | a physical iPhone or iPad | `xcrun devicectl list devices` |
| `--ios-simulator <name\|udid>` | one booted simulator | `xcrun simctl list devices booted` |
| `--android-device <serial>` | one device or emulator | `adb devices` |
| `--ohos-device <key>` | one OpenHarmony device or emulator | `hdc list targets` |

Or ask Day, which lists all three from any directory:

```bash
day devices list                      # every mobile target
day devices list -p ios-uikit         # just one
day --format json devices list        # for editors and scripts

# start something to launch onto: a simulator, an AVD, or the OpenHarmony emulator
day devices boot -p ios-uikit  C4C903E3-95E1-40F3-A3F8-45D3EAE035BB
day devices boot -p android-mdc Pixel_9_API_36

# and stop one when you are done with it
day devices shutdown -p ios-uikit  "iPhone 16 Pro"
day devices shutdown -p android-mdc Pixel_9_API_36
```

Booted simulators, attached phones, running emulators and reachable hdc targets come back under
`devices`; simulators and AVDs that exist but are not running come back under `bootable`. Both
halves name the flag that selects a device, so an editor can show a row for one before it has
booted. A target whose toolchain is missing reports `available: false` with a note, instead of
looking like nothing is plugged in.

`day devices boot` starts one of the `bootable` entries. On iOS an app cannot be installed onto a
shut-down simulator, so boot one before `day launch`. With `--wait`, booting an Android emulator
also turns off its "isn't responding" and crash dialogs and its "Viewing full screen" hint, as
`day launch` does on every emulator, so they stay out of screenshots. Physical devices keep their
own settings.

`--headless` boots with no window, which is what a CI runner wants: it starts an Android emulator
without one and keeps the iOS simulator's UI app closed. Leave it off at a desk, where the boot
opens that app so you can watch your app arrive.

Which app shows the simulator depends on the Xcode. Up to Xcode 26 it is `Simulator.app`, opened
directly. Xcode 27 replaced it with **Device Hub**, which shows the one device it is told about,
so Day opens it on the device being booted
(`open -a DeviceHub.app "devices://manage/select?id=<udid>"`). Day picks whichever the selected
Xcode ships, and `day doctor` names it as `simulator-ui`.

`day launch -p ios-uikit` brings that window up too when it is launching onto a single simulator,
so an app started against an already-booted device is something you can watch. A run that targets
several simulators at once (a capture sweep) opens none of them.

Device Hub shows nothing for a device that is still booting and does not correct itself
afterwards, so a boot that is going to open a window waits for the device first. `--headless`
skips both the wait and the window.

`day devices shutdown` is the other direction. Both spellings of an Android emulator work — the adb
serial the listing reports, or the AVD name you booted it by — and the command waits until the
emulator has gone, so the next listing describes the machine you are about to act on. Stopping
something that is already stopped succeeds. Physical phones are refused; unplug one instead. The
OpenHarmony emulator has no stop yet; close its window.

`--android-device` and `--ohos-device` take precedence over `ANDROID_SERIAL` and
`DAY_OHOS_TARGET`, so an exported value keeps working as the default and the flag overrides it for
one run. `--device` is an accepted alias for `--ios-simulator`.

Whichever device a run names is also the one its dayscript talks to and its screenshots come
from; the port forward and the capture follow the selection rather than whichever device
enumerated first.

```bash
# every booted simulator — the default, and what a screenshot sweep wants
day launch -p ios-uikit

# one booted simulator, by name or UDID
day launch -p ios-uikit --ios-simulator "iPhone 16 Pro"

# a physical iPhone
day launch -p ios-uikit --ios-device "iPhone 13 mini"

# one Android device or emulator, by adb serial
day launch -p android-mdc --android-device 19091FDF600BAY

# one OpenHarmony device or emulator, by hdc connect key
day launch -p harmony-arkui --ohos-device 127.0.0.1:55555

# both phones at once, from one command, with the logs interleaved
day launch -p ios-uikit    --ios-device "iPhone 13 mini" \
           -p android-mdc  --android-device 19091FDF600BAY

# start them and get the shell back rather than staying attached to the logs
day launch -p ios-uikit --ios-device "iPhone 13 mini" --detach

# drive a device run with a dayscript, the same as a simulator run
day launch -p ios-uikit --ios-device "iPhone 13 mini" --script dayscript/demo.yaml

# a phone and a desktop together, to compare the same screen side by side
day launch -p ios-uikit --ios-device "iPhone 13 mini" -p macos-appkit
```

Every target reports the same way. Day reports each step itself, and the tools underneath it
(`adb`, `devicectl`, `simctl`) stay quiet unless they fail, at which point their output is the
diagnostic. The two-phone command above prints:

```
     Signing Showcase.app (Day Showcase iOS Development)
  Installing ios-uikit on iPhone 13 mini
   Launching ios-uikit (dev.daybrite.showcase) on device iPhone 13 mini
  Installing android-mdc on 19091FDF600BAY
   Launching android-mdc (dev.daybrite.showcase) on 19091FDF600BAY (arm64-v8a)
```

and then streams both apps' stdout and stderr, each line prefixed with the target it came from
(`[ios-uikit]`, `[android-mdc]`), so two phones running at once read apart. Ctrl-C stops the run
and takes the log watchers with it.

### What a physical iOS device needs

Naming `--ios-device` also changes the build: the `iphoneos` SDK instead of the simulator's, and
signing against a real identity, where a simulator build signs ad-hoc. Day signs the bundle after the build
against a development provisioning profile installed for the app's bundle id. The profile supplies
both the signing identity (matched by fingerprint, so a machine holding several development
certificates picks the right one) and the entitlements, so the signature cannot claim something its
profile does not grant.

So the prerequisites are a paired device and a profile that covers this app and lists that device.
Install one by double-clicking the `.mobileprovision`; without a match, the launch stops and says
so rather than falling back to a simulator. Push is the case where the two halves have to agree:
if `Day.toml` declares `notifications`, the build fails when the profile has no `aps-environment`,
instead of installing an app that cannot register.

Pressing Run in Xcode needs one thing more, because Xcode signs during the build rather than after
it: a development team. Set it in `platform/ios/DayApp.local.xcconfig`, which `DayApp.xcconfig`
includes last and `.gitignore` already covers:

```text
DEVELOPMENT_TEAM = ABCDE12345
```

That file belongs to your checkout, so a fork can sign with its own team, and with a bundle id its
profile covers, while the committed project stays as it is.

Apple reports a locked device as `RequestDenied`; Day translates it:

```
[ios-uikit] the device is locked — unlock it and run again (iOS will not launch an app onto a locked screen)
```

Installing works on a locked phone; launching does not.

## Checking the machine

`day doctor` reports what each toolkit needs and what's missing. `day doctor verify` tests the answer by
doing the work: it runs the doctor checks, then for every target this machine supports it scaffolds
a throwaway app in a temporary directory, builds it, and packages it. Each target's build time and
packaged size are printed at the end.

```bash
day doctor verify                                   # every target this machine can build
day doctor verify -p ios-uikit,macos-appkit         # only these
day doctor verify --no-pack --profile release       # stop after the build; use the release profile
day doctor verify --day-version 0.2.0               # check that release, not the CLI you have
```

Run it with no arguments and a target whose prerequisites are missing is skipped, with the same fix
line `day doctor` would print. Name targets with `-p` and a missing prerequisite is an error
instead, since you said those targets work here. `--strict` fails the run on any target this
machine could have checked but isn't set up for (a target that only builds on another OS is never
counted).
The scheduled workflow in the `day` repository uses it to check each platform-toolkit pair
against a freshly installed CLI. Under GitHub Actions the per-target table goes to the job summary.

The scaffolded projects are deleted at the end unless you pass `--keep`.

### Checking a specific version of Day

`--day-version` picks which Day to verify. It sets both halves (the `day` CLI that
scaffolds, builds, and packs, and the `day` your app depends on), so you never test one against the
other:

```bash
day doctor verify --day-version main       # the main branch on GitHub
day doctor verify --day-version 0.2.0      # that release
day doctor verify --day-version latest     # the newest release on crates.io
day doctor verify --day-version a1b2c3d    # that commit
```

Unless the CLI you're running is already the version you named, the command installs it into the run's
temporary directory (`cargo install`), so nothing on your PATH changes. The same spec goes to
`day new --day-version`, which is available on its own if you only want to pin a project:

```bash
day new app my-app --day-version main       # day = { git = "…", branch = "main" }
day new app my-app --day-version 0.2.0      # day = { git = "…", tag = "v0.2.0" }
```

A release pins the matching `vX.Y.Z` git tag today, because the framework crates aren't on
crates.io yet; with `--registry` it pins the crates.io version instead.

## The conventional project

A Day project is a normal Cargo package plus a small `Day.toml`: the project marker and the home of
everything Day-specific. `name` and `version` are derived from Cargo.toml's `[package]` and never
restated, so identity can't drift. Any `[app]` property can be overridden per platform, per toolkit,
or per target (`[app.ios]`, `[app.qt]`, `[app.macos-appkit]`), with the most specific table winning.
The build tool reads the resolved values when it derives platform metadata (an Android build's label
and applicationId, for example).

```toml
# Day.toml
schema = 1

[app]
id = "dev.daybrite.showcase"
title = "Day Showcase"
build = 1
targets = [
  "macos-appkit",
  "macos-gtk",
  "macos-qt",
  "ios-uikit",
  "android-mdc",
]

[window]
width = 480
height = 640

# Example: a different display title on iOS only.
[app.ios]
title = "Showcase Mobile"
```

`day metadata` prints the project's identity, targets, and per-target resolved values;
`--json` emits a versioned, machine-readable envelope (this is what the VS Code extension
consumes instead of parsing Day.toml itself, and it also carries the full target catalog).
`day lint` validates the manifest's structure. Unknown targets and override tables that name
no known platform/toolkit/target are findings, and so is an `[app] id` a declared target does not
accept: `day::lint::app-id` resolves the id per target and holds Android and HarmonyOS to a Java
package name, which is the check `day new` runs on `--appid`.

## Store listings

An app that ships to the App Store or Google Play keeps its whole listing in `store/storefront.toml`
(or `store/storefront.yaml`, the same tree in YAML): the submission info, the text in every locale, and
the screenshots each listing shows. The text is `[storefront.metadata]` (the default locale) with
a table per other locale carrying what differs, `name`, `subtitle`, `short`, `description`,
`keywords` (a list), `release-notes` and the URLs; a target's or a store's own `metadata` table
specializes the wording for that storefront; any field may be `<field>-ref = "store/…"`, a
project file holding the text; and a locale may live in `store/storefront.<tag>.toml` beside the main
file. `day new app` scaffolds it for any app with a mobile target, `day store init` adds it to an
existing one, and `day store migrate` folds the older `store/<locale>/*.txt` layout into it.

The two stores differ in field names, length limits (release notes: 4000 characters on the App
Store, 500 on Google Play), which fields exist, and how a locale is spelled (`zh-CN` here is
`zh-Hans` to Apple). `day store stage` resolves all of it, generating a ready-to-run
fastlane project per target under `build/day/store/<target>/`, with `validate` and `upload` lanes.
`day pack` runs the same generation, so a packaged build already has its listing beside it.

`day lint` checks the listing against the stores' rules before an upload can reject it: length
limits per store, the fields each store's record cannot do without, URL format, leftover `TODO`
placeholders, and locale parity with the app's translations, so a new app locale also requires a
listing in that locale. Each finding names its place, `store/storefront.toml [storefront.metadata.fr]
description` or the referenced file.

`day store export` writes the listing resolved per target, store and locale as one JSON document,
with the stores' rules in force, which is what the app's website is built from and what a release
carries as `storefront.json`. `day store stage` refuses a listing with any lint error, and
placeholder text unless `--allow-placeholders`. Everything the CLI knows about a store is data
(`store-rules.toml`: label, targets, layout, fields and limits, locale spellings, screenshot
sizes), replaceable with `--rules FILE`, `DAY_STORE_RULES` or a project's `store/rules.toml`. See
[Store submission](/docs/guide-store-submission) for the release walk-through.

The listing's screenshots come from the walkthrough's captures, chosen in `store/storefront.toml`
`[storefront.<target>.<store>.screenshots]`: per device kind (or `default`), the `screenshot:`
steps to show, in order, each in the light theme or `{ name = "…", theme = "dark" }`; the
target's own `[storefront.<target>.screenshots]` is what its website page shows, one row per
device kind. Submission metadata sits beside it in `submission-info` tables, shared, per target
or per store. `day screenshot index` resolves that into
`gallery.json`, `day store screenshots <gallery.json | URL>` checks each list against the
store's sizes and coverage rules (the rules file the CLI embeds; `--rules FILE` names another),
and `day store stage --screenshots <gallery.json | URL>` places it where fastlane uploads it. See
[Store listings](/docs/internal/store) for the full field table and the credential variables.

In CI, `day lint --strict` turns any finding into a failure (exit 10). A fresh scaffold trips one
rule, because the listing text it ships is still `TODO`. Pass `--allow store-placeholder` to let
that one code stand while every other rule still fails the run. An allowed code is still reported,
as one line carrying its count and a sample, so an `--allow` nobody has revisited stays visible.

## Linting

`day lint` reads the project's sources, catalogs and manifest, and reports what it finds. Each
finding carries a code you can `--allow`, and the file, line and column it is about:

```
error   day::lint::unknown-route     navigate: route "settings/theme" starts with "settings", which no `.item(…)` or `routes! { … }` declares (src/lib.rs:88)
warning day::lint::unused-key        resource/locales/en: history_hint is never referenced (resource/locales/en/app.ftl:434)
```

A finding is an **error** when it names something that does not exist, or that will misbehave once
the app runs: a [route](/docs/glossary#route) nothing declares navigates nowhere, an undeclared permission terminates the
app on iOS, an unknown target in `Day.toml` is not read. Coverage gaps and store copy are
**warnings**. Both kinds fail `--strict`, so the split changes what you read rather than what CI
does.

One rule that would pass that test stays a warning anyway. `unknown-key` (a `tr("…")` with no
message, which renders the key itself on screen) is found by scanning for the literal after `tr("`,
and that two-character name turns up inside other identifiers, where what follows it is not always a
key.

Some findings come with a repair. `day lint --fix` applies them and reports each one:

```
$ day lint --fix
fixed   day::lint::store-whitespace     store/metadata/en/description.txt: Trim the surrounding whitespace
```

A rule proposes a fix only where there is one right answer and applying it cannot lose anything you
wrote (trimming stray whitespace around a store text kept in its own file).
Anything that would need a decision, or that would add text you did not write, reports and waits
for you. A code you passed to `--allow` is never rewritten.

`day lint --json` emits a versioned envelope instead of the report (every finding with its place,
its severity, and its fix); the
[VS Code extension](/docs/getting-started#2-install-the-day-extension-for-vs-code) draws its
squiggles and quick fixes from it:

```json
{
  "schema": 1,
  "findings": [
    {
      "code": "day::lint::store-whitespace",
      "severity": "warning",
      "message": "store/metadata/en/description.txt ([storefront.metadata] description-ref): leading or trailing whitespace",
      "waived": false,
      "file": "store/metadata/en/description.txt",
      "line": 1,
      "column": 1,
      "fix": {
        "title": "Trim the surrounding whitespace",
        "file": "store/metadata/en/description.txt",
        "contents": "What Day Rise does.\n"
      }
    }
  ],
  "counts": { "errors": 0, "warnings": 1, "waived": 0, "fixable": 1 }
}
```

Waived findings appear too, marked `"waived": true`, so a tool can show them greyed instead of
hiding an `--allow` that has outlived its reason.

Under GitHub Actions, findings also become annotations on the lines they name, plus a summary table
on the run page.

One [backend](/docs/glossary#backend) feature is enabled per binary; `day launch -p <target>` selects it, so the AppKit build
contains only AppKit code and the Android build only its JNI bridge. The full directory anatomy,
the per-target build pipelines, and how [resources](/docs/glossary#resource) are packaged are covered in
[Project structure & builds](/docs/project-structure).

## dayscript

**dayscript** is a YAML language that drives and asserts a *running* app over a socket, using the
same script on every platform. Pieces are addressed by the same stable `.id` you give them in Rust,
and routes are the same keys your `nav`/`nav_stack` use. It has its own guide: [Testing with
dayscript](/docs/dayscript).

## Continuous integration

Every push builds the showcase on every target and runs the walkthrough, uploading each target's
screenshots (and its installable packages) as artifacts. This site's [gallery](/gallery) is
assembled from those screenshot artifacts, so it always shows the latest captures from each
platform that succeeded. [Packaging & distribution](/docs/packaging) covers the artifact
pipeline, and [Platform support](/docs/platforms) reports what that CI shows, per target.
