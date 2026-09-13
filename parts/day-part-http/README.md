<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# day-part-http

Fetch like the platform, not around it.

This crate does HTTP through each platform's own networking stack: URLSession on macOS and iOS,
OkHttp on Android, WinHTTP on Windows, the system's libcurl on Linux, the Network Kit on
HarmonyOS, and the browser's `fetch` and `WebSocket` on the web. Requests pick up what the OS
already knows: system proxies and PAC scripts, VPN routing, Low Data Mode, and enterprise
certificate stores.

The crate-root functions send one request at a time: blocking `fetch`, callback `fetch_async`,
awaitable `fetch_future` (dropping it cancels the request), and `fetch_to_file` and
`fetch_streamed` for bodies that belong on disk. A `Client` adds the rest of a modern HTTP API on
the same stacks: response bodies streamed with backpressure, uploads from files, readers and
multipart forms, redirect and authentication callbacks, public-key pins and server trust
decisions, client certificates, cookies, caching, transfer metrics, and WebSockets.
`capabilities()` reports what the platform offers. HTTP error statuses are responses rather than
errors, and the request timeout bounds progress, so a long download that keeps moving runs to
the end. The crate also ships the local test server its tests and the showcase use.

Parts are Day's small capability crates: a plain Rust API over something the platform already
provides. This one works in any Rust program, with or without a Day app around it.

## Part of Day

This crate is one piece of [Day](https://daybrite.dev), a Rust framework for building apps out of
each platform's own widgets — AppKit, UIKit, Android's Material widgets, GTK 4, Qt 6, XAML, and
ArkUI — from one codebase. When you write `button("Save")`, macOS shows an `NSButton` and Android
shows a Material button. The framework also ships the tooling around the app: the `day` CLI, a VS
Code extension, GitHub CI workflows, localization, accessibility, and dayscript automation.

New to Day? Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
