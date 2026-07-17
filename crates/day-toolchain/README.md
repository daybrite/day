# day-toolchain

One crate that knows where developer toolchains and SDKs live on your machine.

The Android SDK, NDK, and a usable JDK; Windows kits and C++/WinRT; the OpenHarmony NDK;
NSIS; rustup homes — every lookup checks the conventional environment variables first
(`ANDROID_HOME`, `JAVA_HOME`, and so on) and only then probes the places installers put
things. The `day` CLI, the backend build scripts, and generated projects all ask this
crate instead of hard-coding paths, so when a vendor moves something, there is one place
to fix it.

## Part of Day

This crate is one piece of [Day](https://daybrite.dev), a Rust framework for building apps
out of each platform's real native widgets — AppKit, UIKit, Android's Material widgets,
GTK 4, Qt 6, WinUI, and ArkUI — from one codebase. There is no web view and no bundled
rendering engine: when you write `button("Save")`, macOS shows an `NSButton` and Android
shows a Material button.

New to Day? Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
