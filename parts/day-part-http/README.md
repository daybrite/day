# day-part-http

Fetch like the platform, not around it.

This crate does HTTP through each platform's own networking stack — NSURLSession on macOS and
iOS, OkHttp on Android, WinHTTP on Windows — so requests pick up everything the OS
already knows: system proxies and PAC scripts, VPN routing, Low Data Mode, enterprise certificate
stores. On Linux and HarmonyOS, where no OS-level HTTP API exists, a bundled ureq + rustls
fallback keeps the same API working; `tier()` tells you which world you're in.

The surface is small on purpose: blocking `fetch`, callback `fetch_async`, streaming
`fetch_to_file` and `fetch_streamed` (progress, cancellation, hash-as-you-go). HTTP error
statuses are responses, not errors, and a long download is never cut off by the request timeout —
it bounds progress, not the transfer.

Parts are Day's small capability crates: no UI, just a plain Rust API over something the platform
already provides. This one works in any Rust program — you don't need a Day app around it.

## Part of Day

This crate is one piece of [Day](https://daybrite.dev), a Rust framework for building apps out of
each platform's real native widgets — AppKit, UIKit, Android's Material widgets, GTK 4, Qt 6,
WinUI, and ArkUI — from one codebase. There is no web view and no bundled rendering engine: when
you write `button("Save")`, macOS shows an `NSButton` and Android shows a Material button.

New to Day? Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
