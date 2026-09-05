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

The `day` CLI (modeled on the architecture of `flutter_tools`) creates, builds, launches, packs,
lints, and scripts projects. It works the same driven by hand, from CI, or from an IDE.

## The commands

```bash
day new                      # interactive: scaffold an app, a piece, or a part
day new app my-app           # scaffold a new app non-interactively (--no-website to skip the site config)
day app add-toolkit android-mdc   # add a target to an existing app
day localize list|add|remove # survey the project's locales, or add/remove one on every surface at once
day icon                     # generate every platform's app-icon set from one master (--check: CI drift gate)
day build   -p macos-appkit  # build one target
day launch  -p macos-gtk     # build + run on a target
day launch  --git <url>      # clone a repository and run the app in it — no checkout needed
day launch  --day-src <path|url>  # run this app against another day, for one build
day pack    -p macos-appkit  # build + sign + produce a distributable artifact (.dmg here)
day sign    --check          # report release-signing readiness without printing secrets
day rebuild <artifact>       # rebuild a shipped artifact from its provenance and compare the bytes
day lint                     # check ids, Fluent coverage, project shape (--fix applies what it can)
day devices list             # simulators, emulators and phones a mobile target can launch onto
day devices boot -p ios-uikit <id>  # start a simulator/AVD so it can be launched onto
day doctor                   # check toolchains for every target
day checkup                  # doctor, then scaffold + build + pack a throwaway app per target
day stop --all               # stop running launches (sessions in build/day/sessions.json)
day clean                    # remove all build artifacts (build/, target/, gradle/hvigor outputs); --dry-run lists them
day relaunch --all-running   # stop + rebuild + relaunch — "apply my changes"
day drive -p <t> --steps-json '…'   # drive a running app with dayscript steps
day patch --local <checkout> # build against a local day (or piece) checkout; repeatable (--check: verify)
day patch --git <url>[@<ref>] # build against a fork of day, for the whole graph; commit the table
day mcp-server               # serve Day tools to AI agents (Model Context Protocol, stdio)
day version                  # print the CLI version, build profile, and git ref (always the commit)
```

`day patch` switches an app from the published git dependency to a local checkout of day, or
of an external piece or part, or to a fork of day, and verifies the switch took; [Developing Day and an app together](/docs/local-development) covers
when and how to use it.

`day pack` produces a standalone, installable package per target. See
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
which platforms and toolkits to support. Every question has an equivalent flag, so the same choices
can be made non-interactively, e.g. `day new app my-app --toolkit ios-uikit --toolkit macos-appkit
--appid com.example.myapp --title "My App"`. Scaffolds currently depend on `day` from its git
remote (the framework crates are not yet published to crates.io); once they are, `--registry`
pins them to your CLI's version from crates.io and will become the default.

`day new app` scaffolds a working starter: a typed-route sidebar over four sample panels (a
reactive counter, a controls tour, a canvas dial, and a drill-down stack), with locales, a
dayscript walkthrough (`day launch -p <target> --script dayscript/demo.yaml`), and the thin native
host projects the mobile targets build through. The scaffold comes from a **template**: a plain
directory tree whose file contents *and paths* are rendered with mustache-style placeholders
(`{{name}}`, `{{ident}}`, `{{snake}}`, `{{pascal}}`, `{{title}}`, `{{id}}`, `{{scheme}}`,
`{{day_dep}}`, `{{day_build_dep}}`, `{{targets_toml}}`, `{{first_target}}`). The built-in template is embedded in the
CLI (a fresh `cargo install day-cli` scaffolds offline); bring your own with:

```bash
day new app my-app --template ./my-template          # a local directory
day new app my-app --template https://github.com/you/tpl#v1   # a git repo (optional #ref)
```

`day new --describe` prints the questions themselves (every kind's fields, their options, and the
flag each one fills) as a versioned JSON document. It takes no project, so an editor can read it
to build its own New Project dialog without copying the target list into a second place. The VS
Code extension's wizard is rendered entirely from it.

```bash
day new --describe | jq '.kinds[] | {id, fields: [.fields[].id]}'
```

Template conventions: a trailing `.hbs` on a filename is stripped after rendering (use
`Cargo.toml.hbs` so tooling doesn't mistake the template for a Rust package), `_gitignore`
becomes `.gitignore`, non-UTF-8 files (icons) copy verbatim, and an unknown `{{placeholder}}`
is an error rather than silent empty output. Files under `platform/<os>/` belong to that OS's
targets and are only scaffolded for targets that need them.

Add a platform later with **`day app add-toolkit <target>`** (repeatable / comma-separated):
it appends the target to `Day.toml`'s `[app] targets` array (via toml_edit, so your comments
and formatting survive) and materializes the target's native host project (`platform/android/`,
`platform/ios/`, `platform/ohos/`) from the same template, never overwriting existing files.
Pass the same `--template` the app was created with if it wasn't the built-in one.

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

`--day-src` points the app's `day` dependencies somewhere else for **one build**. It takes a path
to a day checkout, or a git URL with an optional `@<ref>` — a branch, a tag, a commit, or someone
else's fork. It's on `day build` and `day launch` both, since answering "does this branch fix the
bug?" means building with each version and looking at both:

```bash
day launch --day-src ../day                                               # a local checkout
day launch --day-src https://github.com/daybrite/day.git@experimental-nav  # a branch
day launch --day-src https://github.com/someone/day.git@fix-482            # a PR fork
```

Nothing in your project changes. `day patch` writes `.cargo/config.toml` and every later build
uses it until you delete it; `--day-src` computes the same `[patch]` table, hands it to one cargo
run, and leaves the project exactly as it found it — `Cargo.lock` included, which cargo rewrites
during the build and which the CLI puts back afterwards. Use `day patch` when you're developing
the framework and the app together for a while, and `--day-src` when you want one look.

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

There are two limits. On Android and HarmonyOS only the Rust half is isolated; Gradle and
hvigor keep their own shared build directories, so the packaging step re-runs when you switch. And
`day pack` takes no `--day-src`, so a shipped artifact always records the framework that built it.

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

### Simulators, emulators, and devices

Without a device flag, a launch goes to **every** runtime of that kind it can see: every booted
iOS simulator, every connected Android device and emulator. That is what a capture sweep wants.
When you mean one specific phone, name it. Selection is one flag per runtime, so a single command
can send each `-p` somewhere different:

| Flag | Selects | Find them with |
| --- | --- | --- |
| `--ios-device <name\|udid>` | a physical iPhone or iPad | `xcrun devicectl list devices` |
| `--ios-simulator <name\|udid>` | one booted simulator | `xcrun simctl list devices booted` |
| `--android-device <serial>` | one device or emulator | `adb devices` |
| `--ohos-device <key>` | one OpenHarmony device or emulator | `hdc list targets` |

Or ask Day, which covers all three in one listing and needs no project:

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
`devices`; simulators and AVDs that exist but are not running come back under `bootable`. A target
whose toolchain is missing reports `available: false` with a note, instead of looking like nothing
is plugged in.

`day devices boot` starts one of the `bootable` entries. That matters most on iOS, where an app
cannot be installed onto a shut-down simulator: booting one is the step between "none running" and
being able to launch at all.

`day devices shutdown` is the other direction, for when a simulator or emulator has stopped being
worth the memory it holds. Both spellings of an Android emulator work — the adb serial the listing
reports, or the AVD name you booted it by — and the command waits until the emulator has actually
gone, so the next listing describes the machine you are about to act on. Stopping something that
is already stopped succeeds. Physical phones are refused: unplugging one is the real action, and
powering off hardware someone is holding is a different thing entirely. The OpenHarmony emulator
has no stop yet; close its window.

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

Naming `--ios-device` changes the build, not just where it lands: the `iphoneos` SDK instead of
the simulator's, and code signing, which a simulator build does not do at all. Day signs the
bundle after the build against a **development provisioning profile** installed for the app's
bundle id. The profile supplies both the signing identity (matched by fingerprint, so a machine
holding several development certificates picks the right one) and the entitlements, so the
signature cannot claim something its profile does not grant.

So the prerequisites are a paired device and a profile that covers this app and lists that device.
Install one by double-clicking the `.mobileprovision`; without a match, the launch stops and says
so rather than falling back to a simulator. Push is the case where the two halves have to agree:
if `Day.toml` declares `notifications`, the build fails when the profile has no `aps-environment`,
instead of installing an app that cannot register.

One error deserves a mention, because Apple reports it as `RequestDenied`:

```
[ios-uikit] the device is locked — unlock it and run again (iOS will not launch an app onto a locked screen)
```

Installing works on a locked phone; launching does not.

## Checking the machine

`day doctor` reports what each toolkit needs and what's missing. `day checkup` tests the answer by
doing the work: it runs the doctor checks, then for every target this machine supports it scaffolds
a throwaway app in a temporary directory, builds it, and packages it. Each target's build time and
packaged size are printed at the end.

```bash
day checkup                                   # every target this machine can build
day checkup -p ios-uikit,macos-appkit         # only these
day checkup --no-pack --profile release       # stop after the build; use the release profile
day checkup --day-version 0.2.0               # check that release, not the CLI you have
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

`--day-version` picks which Day the checkup is about. It sets both halves (the `day` CLI that
scaffolds, builds, and packs, and the `day` your app depends on), so you never test one against the
other:

```bash
day checkup --day-version main       # the main branch on GitHub
day checkup --day-version 0.2.0      # that release
day checkup --day-version latest     # the newest release on crates.io
day checkup --day-version a1b2c3d    # that commit
```

Unless the CLI you're running is already the version you named, checkup installs it into the run's
temporary directory (`cargo install`), so nothing on your PATH changes. The same spec goes to
`day new --day-version`, which is available on its own if you only want to pin a project:

```bash
day new app my-app --day-version main       # day = { git = "…", branch = "main" }
day new app my-app --day-version 0.2.0      # day = { git = "…", tag = "v0.2.0" }
```

A release pins the matching `vX.Y.Z` git tag today, because the framework crates aren't on
crates.io yet; with `--registry` it pins the crates.io version instead.

## The conventional project

A Day project is a normal Cargo package plus a small `Day.toml`: the project marker and the
home of everything Day-specific. `name` and `version` are **derived from Cargo.toml's
`[package]`** and never restated, so identity can't drift. Any `[app]` property can be
**overridden per platform, per toolkit, or per target** (`[app.ios]`, `[app.qt]`,
`[app.macos-appkit]`), with the most specific table winning. The build tool reads
the resolved values when it derives platform metadata (an Android build's label and
applicationId, for example).

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
no known platform/toolkit/target are findings.

## Store listings

An app that ships to the App Store or Google Play keeps its listing under `store/<locale>/`, as
plain text files named for what they are — `name.txt`, `subtitle.txt`, `short.txt`,
`description.txt`, `keywords.txt`, `release-notes.txt` — one directory per locale, keyed the same
way `resource/locales/` is. `day new app` scaffolds it for any app with a mobile target, and
`day store init` adds it to an existing one.

The two stores differ in field names, length limits (release notes: 4000 characters on the App
Store, 500 on Google Play), which fields exist, and how a locale is spelled (`zh-CN` here is
`zh-Hans` to Apple). `day store stage` resolves all of it, generating a ready-to-run
fastlane project per target under `build/day/store/<target>/`, with `validate` and `upload` lanes.
`day pack` runs the same generation, so a packaged build already has its listing beside it.

`day lint` checks the listing against the stores' rules before an upload can reject it: length
limits per store, required fields, URL format, leftover `TODO` placeholders, and locale parity with
the app's own translations, so a new app locale also requires a listing in that locale. See
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
the app runs: a route nothing declares navigates nowhere, an undeclared permission terminates the
app on iOS, an unknown target in `Day.toml` is not read. Coverage gaps and store copy are
**warnings**. Both kinds fail `--strict`, so the split changes what you read rather than what CI
does.

One rule that would pass that test stays a warning anyway. `unknown-key` (a `tr("…")` with no
message, which renders the key itself on screen) is found by scanning for the literal after
`tr("`, and that two-character name turns up inside other identifiers, where what follows it is
not always a key. A text scan is weaker evidence than a parse, so the finding stays a warning.

Some findings come with a repair. `day lint --fix` applies them and reports each one:

```
$ day lint --fix
fixed   day::lint::store-whitespace     store/en/name.txt: Trim the surrounding whitespace
fixed   day::lint::store-bad-keywords   store/en/keywords.txt: Remove the spaces around commas
```

A rule proposes a fix only where there is one right answer and applying it cannot lose anything you
wrote (trimming stray whitespace around a store field, dropping the spaces in a keyword list).
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
      "message": "store/en/name.txt: leading or trailing whitespace",
      "waived": false,
      "file": "store/en/name.txt",
      "line": 1,
      "column": 1,
      "fix": {
        "title": "Trim the surrounding whitespace",
        "file": "store/en/name.txt",
        "contents": "Day Rise\n"
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

One backend feature is enabled per binary; `day launch -p <target>` selects it, so the AppKit build
contains only AppKit code and the Android build only its JNI bridge. The full directory anatomy,
the per-target build pipelines, and how resources are packaged are covered in
[Project structure & builds](/docs/project-structure).

## dayscript

**dayscript** is a YAML language that drives and asserts a *running* app over a socket, using the
same script on every platform. Pieces are addressed by the same stable `.id` you give them in
Rust, and routes are the same keys your `selector`/`stack` use, so one script exercises the app
identically everywhere. It has its own guide: [Testing with dayscript](/docs/dayscript).

## Continuous integration

Every push builds the showcase on every target and runs the walkthrough, uploading each target's
screenshots (and its installable packages) as artifacts. This site's [gallery](/gallery) is
assembled from those screenshot artifacts, so it always shows the latest captures from each
platform that succeeded. [Packaging & distribution](/docs/packaging) covers the artifact
pipeline, and [Platform support](/docs/platforms) reports what that CI shows, per target.
