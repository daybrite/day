<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# day-part-downloads

A download manager for large files, over day-part-http's client.

Downloads wait in a queue, and each one writes to a partial file beside a journal. A pause, a
dropped connection or a relaunch continues from the partial file with a validated `Range`
request. A finished file is checked against its expected length and SHA-256 before it moves to
its destination. Transient failures retry with backoff that honors `Retry-After`, and a watch
reports progress about ten times a second.

A download can also be handed to the OS: a background `URLSession` on macOS and iOS,
`DownloadManager` on Android, the request agent on HarmonyOS, and the Background Intelligent
Transfer Service on Windows. The OS then keeps transferring while the app is suspended or closed.

Parts are Day's small capability crates: a plain Rust API over something the platform already
provides. This one works in any Rust program, with or without a Day app around it. The
[reference](https://github.com/daybrite/day/blob/main/docs/downloads.md) covers every option.

## Part of Day

This crate is one piece of [Day](https://daybrite.dev), a Rust framework for building apps out of
each platform's own widgets (AppKit, UIKit, Android's Material widgets, GTK 4, Qt 6, XAML, and
ArkUI) from one codebase. When you write `button("Save")`, macOS shows an `NSButton` and Android
shows a Material button. The framework also ships the tooling around the app: the `day` CLI, a VS
Code extension, GitHub CI workflows, localization, accessibility, and dayscript automation.

New to Day? Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
