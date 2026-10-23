<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# day-part-keychain

Small secrets in the platform's own secure store, by key.

A session token, a password, an API key: store it under a name and read it back, and the
platform keeps it the way it keeps its own. The keychain on iOS and macOS, the Android
Keystore, DPAPI on Windows, the Secret Service on Linux, the Universal Keystore on HarmonyOS.
An item can be readable while the device is unlocked, from the first unlock after boot, or
only after the person passes the device's own verification dialog, each time.

Parts are Day's small capability crates: no UI, just a plain Rust API over something the
platform already provides. This one works in any Rust program — you don't need a Day app
around it.

## Part of Day

This crate is one piece of [Day](https://daybrite.dev), a Rust framework for building apps
out of each platform's own widgets — AppKit, UIKit, Android's Material widgets, GTK 4, Qt 6,
XAML, and ArkUI — from one codebase. When you write `button("Save")`, macOS shows an
`NSButton` and Android shows a Material button. The framework also ships the tooling around
the app: the `day` CLI, a VS Code extension, GitHub CI workflows, localization,
accessibility, and dayscript automation.

New to Day? Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
