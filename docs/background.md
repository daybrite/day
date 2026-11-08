---
title: "Background work"
description: "Schedule persistent OS tasks or delayed work within a running app process."
navTitle: "Background work"
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Background work

Background tasks handle short jobs that can wait, such as refreshing a cache or retrying a sync.
A registered handler runs independently of a page, using persisted inputs and checking whether
it has been asked to stop.

## Add to your app

```sh
cargo add 'day-part-background@0.*'
```

Build with the [Day CLI](/docs/reference/cli) so the host includes native scheduling and process
startup code. Declare permitted task identifiers on iOS as shown below.

[API reference](https://docs.rs/day-part-background/latest/day_part_background/) ·
[Source and demo](https://github.com/daybrite/day-part-background).

## Choose a scheduler

[`capabilities`](https://docs.rs/day-part-background/latest/day_part_background/fn.capabilities.html)
reports persistent and in-process scheduling separately.

| Need | Scheduler | Platforms |
| --- | --- | --- |
| Retain a request after the app process exits | Persistent OS scheduling | iOS and Android |
| Run delayed work while the process remains alive | In-process scheduling | iOS, Android, macOS, Windows, and Linux |

Persistent scheduling never falls back to in-process work. Desktop persistent scheduling is
unsupported; web and HarmonyOS support neither scheduler. In-process scheduling does not keep
a suspended app running or restart a closed app.

## Register a handler

Use a stable task identifier and a handler that does not need UI state:

```rust
use day_part_background::{Context, Outcome, Schedule};

day_part_background::register_task!("dev.example.refresh", refresh);

fn refresh(context: Context) -> Outcome {
    if context.is_cancelled() { return Outcome::Retry; }
    // Process a bounded chunk of persisted work; check cancellation between chunks.
    Outcome::Success
}

async fn schedule_refresh() -> Result<(), day_part_background::Error> {
    let mut request = Schedule::new("dev.example.refresh");
    request.requires_network = true;
    request.interval_secs = Some(15 * 60);
    day_part_background::schedule(&request).await
}
```

[`register_task!`](https://docs.rs/day-part-background/latest/day_part_background/macro.register_task.html)
registers the handler before any page mounts. The handler runs on a worker thread. Persist its
inputs before scheduling, keep writes safe to repeat, and check the cancellation flag between
bounded units of work. Cancellation cannot forcibly terminate arbitrary Rust code.

[`Outcome`](https://docs.rs/day-part-background/latest/day_part_background/enum.Outcome.html)
reports success, a transient failure to retry, or a permanent failure. The OS decides when to
run persistent work and can defer or stop it. Keep a foreground path for work users need now.

## Persistent scheduling rules

[`schedule`](https://docs.rs/day-part-background/latest/day_part_background/fn.schedule.html)
submits a request to the OS. [`cancel`](https://docs.rs/day-part-background/latest/day_part_background/fn.cancel.html)
removes pending work and asks active handlers to stop.

- Scheduling replaces pending work with the same identifier. Android also cancels an executing
  instance; it may take time to observe the cancellation flag.
- Repeats request an interval of at least 15 minutes. A delay is a minimum, not a deadline.
  Network and charging constraints can postpone execution further.
- Payloads are UTF-8 strings of at most 4,096 bytes. Store larger data separately and pass an ID.
- Cancellation cannot undo results already written by a handler.

## Work within the running process

Use the same registered handler with
[`in_process::schedule`](https://docs.rs/day-part-background/latest/day_part_background/in_process/fn.schedule.html)
for a delayed refresh or retry that can be discarded when the process exits:

```rust
fn schedule_in_process() -> Result<(), day_part_background::Error> {
    let mut request = day_part_background::Schedule::new("dev.example.refresh");
    request.delay_secs = 30;
    day_part_background::in_process::schedule(&request)
}
```

[`in_process::cancel`](https://docs.rs/day-part-background/latest/day_part_background/in_process/fn.cancel.html)
affects only the in-process schedule with that identifier. Network and charging constraints
return an unsupported error. Repeats wait at least the requested interval after the handler
returns; retry waits 15 minutes, and permanent failure ends the schedule.

A replacement can begin before a cancelled handler returns. At most 64 worker threads can be
active, including cancelled handlers that have not yet returned.

## Native configuration

| Platform | Setup and limits |
| --- | --- |
| iOS / UIKit | Uses `BGProcessingTaskRequest`. List task identifiers in `BGTaskSchedulerPermittedIdentifiers` and include `processing` in `UIBackgroundModes`. Repeating tasks resubmit before execution. |
| Android / Material | Uses WorkManager, with dependencies and manifest permissions contributed by the package. A Worker can start the Rust handler without an Activity. Force-stop prevents execution until the user opens the app again. |

In the iOS host configuration, declare each registered identifier:

```xml
<key>BGTaskSchedulerPermittedIdentifiers</key>
<array><string>dev.example.refresh</string></array>
<key>UIBackgroundModes</key>
<array><string>processing</string></array>
```

See Day's [native project configuration](/docs/how-to/native-projects), Apple's
[Background Tasks](https://developer.apple.com/documentation/backgroundtasks), and Android's
[WorkManager guide](https://developer.android.com/develop/background-work/background-tasks/persistent-work).
This package does not provide continuous execution, foreground services, or background file
transfers. OS scheduling does not guarantee eventual execution.

## Exercise a handler

Background Lab records a completion count. Schedule work, leave the app, then return and refresh
the count. Its **Run handler now** control invokes
[`run_now`](https://docs.rs/day-part-background/latest/day_part_background/fn.run_now.html)
for diagnostics; that synchronous call runs on the caller's thread and does not test OS scheduling.

The portable walkthrough checks controls and cancellation. Unit tests cover schedule validation,
replacement, retries, and cooperative cancellation. Process-restart behavior requires a separate
native test.
