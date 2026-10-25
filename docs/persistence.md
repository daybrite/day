---
title: Persistence
description: Typed SQLite persistence for Day, maintained as a first-party extension.
---

# Persistence

[day-persistence](https://github.com/daybrite/day-persistence) is the independently versioned,
first-party SQLite persistence extension for Day. It builds on Day's public model and reactive
APIs; apps that do not use it do not depend on it.

## Add to your app

Add these entries to your app’s `Cargo.toml`:

```toml
[dependencies]
day = { package = "day-app", version = "0.6", features = ["model"] }
day-persistence = "0.1"
```

The default features provide bundled SQLite and the live-list adapter. Other options include
system SQLite, SQLCipher, and date/time codecs. [`day-model`](/docs/explanation/model) remains part of Day and works without
persistence.

- [Full persistence guide](https://github.com/daybrite/day-persistence/blob/main/docs/persistence.md)
- [Persistence Lab demo](https://github.com/daybrite/day-persistence/tree/main/demo): database
  management, SQL execution and tracing, typed records, and JSON import/export.
- [Regression tests](https://github.com/daybrite/day-persistence/tree/main/crates/day-persistence/tests)

## The web

The extension's `day-sqlite-worker` crate owns its SQLite engine, OPFS worker, and browser
protocol. Day discovers its browser files through `[package.metadata.day.web]`, just like any
other extension. See the [web engine guide](https://github.com/daybrite/day-persistence/blob/main/docs/persistence.md#the-web).

## Compatibility and local development

Use the same Day source/version for the application and extension. To work on both checkouts,
apply local Cargo patches at the consuming workspace root; do not compile a second copy of
Day's model/reactive crates. See the extension's README for setup and validation.
