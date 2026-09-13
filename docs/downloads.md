---
title: "Downloads"
description: "Large downloads with pause, resume, SHA-256 checks and retries via day-part-downloads, run by the app or handed to the OS."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Downloads (headless capability crate)

> **Status: implemented** as `day-part-downloads` (in `parts/`), a headless crate over
> [day-part-http](http.md)'s client. It queues large downloads, writes each to a partial file
> beside a journal, resumes with validated ranges, checks length and SHA-256, and retries
> transient failures. A download can instead be handed to the OS, which keeps transferring while
> the app is suspended or closed. The showcase's Network & HTTP page downloads 32 MiB from its
> local test server, pauses, resumes and verifies it, then repeats the download through the OS;
> `dayscript/network.yaml` runs that flow on macOS, iOS and Android.

## Authoring

```rust
use day_part_downloads::{Download, Downloads};
use day_part_http::Request;

let downloads = Downloads::open(day_part_fs::data_dir()?.join("downloads"))?;
let id = downloads.enqueue(
    Download::new(Request::get("https://example.com/big.bin"), &dest)
        .expect_len(32 << 20)
        .expect_sha256("9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"),
)?;

// Progress arrives on the thread that reported it; a Setter carries it to the UI.
let fraction = progress_signal.setter();
let watch = downloads.watch(move |p| {
    if p.id == id {
        fraction.set(p.fraction().unwrap_or(0.0));
    }
});

downloads.pause(id)?;
downloads.resume(id)?;
downloads.cancel(id)?;
```

`Downloads` is a cheap handle: clones share one manager. Keep the `Watch` for as long as you
want callbacks; dropping it stops them. `Downloads::with_client(dir, client)` runs every
download through your own [`Client`](http.md#the-client), so its cookies, challenge handler and
pins apply.

## A download's states

`Progress::state` is one of these, and `State::label()` gives the lowercase name the showcase
displays and its dayscript asserts:

| State | Meaning |
|---|---|
| `Queued` | waiting for a free slot |
| `Running` | transferring |
| `Paused` | stopped by `pause`; its partial file waits for `resume` |
| `Retrying` | waiting out a backoff after a transient failure |
| `Verifying` | checking the finished file |
| `Done` | verified and moved to its destination |
| `Failed` | out of retries or failed a check; `Progress::error` says why, and `resume` tries again |
| `Cancelled` | stopped for good; its partial file is deleted |

`Progress` also carries `received`, `total`, `bytes_per_second` (over the last three seconds),
`remaining`, `attempt`, `resumed_from`, the finished file's `sha256`, and the `tier` that ran it.
Watches hear every state change, and at most ten updates a second while bytes arrive.

## Resume

Each download writes to `<id>.part` in the manager's directory, and the destination only ever
receives a whole, verified file. When an attempt stops, the next one asks for the rest with
`Range: bytes=<n>-` and `If-Range` carrying the validator the server first sent: a strong `ETag`,
else `Last-Modified`. A `206` whose `Content-Range` starts at `n` continues the file. A `200`
means the file changed on the server, or the server ignores ranges, so the partial file starts
over. A partial file with no validator also starts over, because nothing shows the server still
has the same bytes.

The journal, `downloads.journal`, records every download on one line and is replaced whole on each
change. `Downloads::open` reads it back: running and retrying downloads return to the queue and
paused ones stay paused. The bytes already on disk are hashed on a worker thread before new bytes
arrive, so the SHA-256 still covers the whole file.

## Checks, retries and limits

- **Length.** `expect_len` sets the length the finished file must have. It also lets the manager
  compare the bytes still needed with the volume's free space (`statvfs`, or
  `GetDiskFreeSpaceExW` on Windows) before a transfer starts, failing with `NoSpace`.
- **SHA-256.** `expect_sha256` sets the digest, hashed as bytes arrive. A mismatch fails with
  `Integrity` and deletes the partial file, so the next attempt downloads everything again.
- **Retries.** A timeout, a refused or dropped connection, a DNS failure, `408`, `429` and `5xx`
  retry up to `retries` times (default 5). The wait starts at `retry_delay` (default one second),
  doubles on each attempt up to a minute, and varies by a fifth either way; a `Retry-After` header
  sets it instead. Other statuses fail at once with `DownloadError::Http`.
- **Limits.** `limits(total, per_host)` caps how many downloads run at once, three and two by
  default. Downloads handed to the OS do not count against them.

Backoff waits on day-async's timer, and bytes arrive on the client's transport, so the manager
owns no thread while downloads run.

## Handing a download to the OS

`Download::in_background(true)` asks the OS to run the transfer where
`Downloads::system_tier()` is true. The manager keeps the journal, the progress it reports and
the checks it makes on the finished file; the OS keeps the connection and the bytes until the
body is whole, then the manager verifies it and moves it into place. Elsewhere the download runs
in the app, and `Progress::tier` says which tier ran it.

| Platform | Service | Pause | Notes |
|---|---|---|---|
| macOS, iOS | a background `URLSession`, one per process, keyed to the bundle identifier | yes, from resume data | Each task's description names the manager directory and download, so a transfer that finishes while the app is away lands in place when the app next opens the manager. A binary with no bundle identifier has no system tier. |
| Android | `DownloadManager` | no: `pause` returns `DownloadError::Unsupported` and the transfer keeps running | The OS shows its own notification. The file lands in the app's external files directory and moves into the manager's directory when it is whole. |
| HarmonyOS | a background task of the request agent (`request.agent`) | yes | The file lands in the app's cache directory first. |
| Windows | the Background Intelligent Transfer Service | yes | Each download is a BITS job owned by the user, so it continues after the app quits. |
| Linux, the web | none | | Downloads run in the app. |

The OS sends only the request's URL and headers, so the client's cookies, redirect and challenge
handlers, and pins do not apply to a download it runs. Its transient failures retry like the
in-app tier's, and each retry starts a new transfer, from resume data on Apple. The walkthrough
exercises the Apple and Android services; the HarmonyOS and Windows services build for their
targets.

On iOS the app does not yet forward `application(_:handleEventsForBackgroundURLSession:)`, so a
download that finishes while the app is suspended is reported the next time the app opens its
manager.

## The web

A page has no files for the manager to own, so `Downloads::open` returns
`DownloadError::Unsupported` on wasm32. The rest of the API compiles there, which keeps shared
code building for the web.

## Errors

`DownloadError` covers the manager's calls and a download's failure:

| Variant | When |
|---|---|
| `Io(message)` | a file or directory the manager needs could not be used |
| `Unknown(id)` | no download has this id |
| `Unsupported` | the web, or pausing an Android system-tier transfer |
| `Http(HttpError)` | the request failed, or answered with a status that does not retry |
| `Integrity { expected, actual }` | the SHA-256 differed |
| `Length { expected, actual }` | the length differed |
| `NoSpace { needed, available }` | the volume lacks room for the rest of the file |

## Files in the manager's directory

| File | Written by |
|---|---|
| `downloads.journal` | the manager, on every change |
| `<id>.part` | an in-app attempt, or the OS's finished body before verification |
| `<id>.resume` | the Apple system tier, when a paused or failed transfer left resume data |
| `<id>.part.bits` | the Windows system tier, while BITS completes a job |

Put the directory under [day-part-fs](fs.md)'s `data_dir()`, which keeps it beside the app's other
files and out of the user's documents.
