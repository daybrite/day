<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-platform-android

Android runtime services shared by Day toolkits and platform-service crates.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

> [!NOTE]
> **Usually an indirect dependency.** You are unlikely to import this crate directly in an app.
> [`day-toolkit-material`](https://github.com/daybrite/day/tree/main/crates/day-toolkit-material)
> and [`day-part-http`](https://github.com/daybrite/day/tree/main/crates/day-part-http) use it as
> part of their implementation.

It manages JVM attachment, the application context and class loader, checked Java bindings, and
shared message encoding. UI controls live in
[`day-toolkit-material`](https://github.com/daybrite/day/tree/main/crates/day-toolkit-material).
Native hosts initialize this runtime before using its services.

## Documentation

- [Android setup](https://daybrite.dev/docs/reference/platforms/android-material/)
- [Android JNI contract](https://github.com/daybrite/day/blob/main/docs/android-jni.md/)
- [Rust API source](https://github.com/daybrite/day/blob/main/crates/day-platform-android/src/lib.rs)

[Source](https://github.com/daybrite/day/tree/main/crates/day-platform-android) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
