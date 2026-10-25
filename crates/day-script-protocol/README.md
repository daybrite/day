<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# day-script-protocol

The types a dayscript is made of: the YAML document format and the JSON request and reply
exchanged between the `day` CLI and a running app.

A `Step` is one operation (`tap`, `input`, `assert_text`, `screenshot`, `navigate`, and the
rest), a `Request` carries one step plus the session token, and a `Reply` reports success,
a retryable or final error, and any screenshot bytes. `Script::from_yaml` parses and
validates a whole `flow:` document before anything runs, so a typo in step twelve is
reported up front rather than after eleven steps have already run.

The crate depends only on serde, JSON, and YAML. It links no UI, no localization, and no
transport, which is what lets the CLI, the in-app engine, and any other tool that reads or
writes dayscripts share one definition of every step. If you are building tooling that
generates or inspects dayscripts, this is the crate to use. To run them, use
[`day-script`](https://crates.io/crates/day-script) through
[`day-cli`](https://crates.io/crates/day-cli).

## Part of Day

This crate is one piece of [Day](https://daybrite.dev), a Rust framework for building apps
out of each platform's own widgets — AppKit, UIKit, Android's Material widgets, GTK 4, Qt 6,
WinUI, and ArkUI — from one codebase. When you write `button("Save")`, macOS shows an
`NSButton` and Android shows a Material button. The framework also ships the tooling around
the app: the `day` CLI, a VS Code extension, GitHub CI workflows, localization,
accessibility, and dayscript automation.

New to Day? Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
