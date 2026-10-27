---
title: Persistence
description: Typed SQLite persistence for Day, maintained as a first-party extension.
---

# Persistence

A database stores related records so an app can retrieve and update them across launches. Day's
persistence extension uses SQLite for structured data that needs queries and coordinated changes.

[day-persistence](https://github.com/daybrite/day-persistence) is a separately versioned,
first-party extension built on Day's public model and reactive APIs. Apps add it as an optional
dependency.

## Add to your app

Add both to your app:

```sh
cargo add day-app --rename day --features model
cargo add day-persistence
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
