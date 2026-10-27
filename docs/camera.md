---
title: "Camera"
description: "Show a camera viewfinder, request access, and capture photos into the app cache."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Camera

Show a live viewfinder and capture JPEG photos for attachments, scanning, or a profile image.
`day-piece-camera` exposes a [Piece](/docs/explanation/pieces) for the preview and observable
state for the capture controls. It does not save to the system photo library or record video.

## Add to your app

```toml
[dependencies]
day-piece-camera = "0.1"
```

[Crate](https://crates.io/crates/day-piece-camera) ·
[API reference](https://docs.rs/day-piece-camera/0.1.0/day_piece_camera/) ·
[Source](https://github.com/daybrite/day-piece-camera).
The [Day CLI](/docs/reference/cli) enables the selected toolkit's integration and packages its
native camera code. Declare the permission in `Day.toml`:

```toml
[permissions]
camera = true
```

Provide its rationale and the button label in the app's translation catalogs:

```ftl
permission_camera = Take photos to attach to your notes.
take_photo = Take photo
```

See [permissions](/docs/reference/permissions) for generated platform declarations and denial handling.

## Preview and capture

This component belongs in an app with [`day::resources!()`](https://daybrite.dev/api/day/macro.resources.html)
and the messages above. The shutter is enabled only while the session is running.

```rust
use day::prelude::*;
use day_piece_camera::{CameraState, Photo, camera};

pub fn camera_panel() -> impl Piece {
    let shutter = Trigger::new();
    let state = Signal::new(CameraState::Idle);
    let photo: Signal<Option<Photo>> = Signal::new(None);
    column((
        camera()
            .auto_request()
            .capture(shutter)
            .state(state)
            .photo(photo)
            .frame(320.0, 240.0),
        button(res::str::take_photo())
            .enabled(move || state.get() == CameraState::Running)
            .action(move || shutter.notify()),
    ))
}
```

[`auto_request()`](https://docs.rs/day-piece-camera/0.1.0/day_piece_camera/struct.Camera.html#method.auto_request)
opts into the system permission prompt when the session is wanted. Otherwise the app requests
access itself; an undecided permission leaves the session idle. The
[`active()`](https://docs.rs/day-piece-camera/0.1.0/day_piece_camera/struct.Camera.html#method.active)
binding starts or stops the session, and
[`facing()`](https://docs.rs/day-piece-camera/0.1.0/day_piece_camera/struct.Camera.html#method.facing)
selects the front or back camera. Removing the piece releases its session.

## Results and errors

[`CameraState`](https://docs.rs/day-piece-camera/0.1.0/day_piece_camera/enum.CameraState.html)
distinguishes idle, starting, running, unavailable hardware, denied permission, and a native
error. A compiled renderer is not evidence of a usable camera: check both
[`support()`](https://docs.rs/day-piece-camera/0.1.0/day_piece_camera/fn.support.html)
and runtime state. Translate app-facing error messages through the catalog; retain native
error details for diagnostics.

A [`Photo`](https://docs.rs/day-piece-camera/0.1.0/day_piece_camera/struct.Photo.html)
contains a cache-file path and its width and height.
[`bytes()`](https://docs.rs/day-piece-camera/0.1.0/day_piece_camera/struct.Photo.html#method.bytes)
reads the file synchronously; perform larger reads away from the UI thread. Move or copy photos
to persistent storage before treating them as saved attachments. Cache files can be removed
by the OS, and the piece does not manage a photo-library permission or save operation.

## Platform support

| Platform / toolkit | Behavior |
| --- | --- |
| iOS / UIKit | [AVFoundation capture](https://developer.apple.com/documentation/avfoundation/capture_setup). Capture needs a physical device; the simulator reports unavailable. Preview orientation is fixed portrait below iOS 17. |
| Android | [CameraX](https://developer.android.com/media/camera/camerax). Requires granted camera permission and usable hardware; virtual emulator cameras can exercise capture. |
| HarmonyOS / ArkUI | Native camera integration is present, but hardware behavior is unverified. Test on the intended device before relying on it. |
| macOS, Windows, Linux, Web | No camera renderer; the piece displays a placeholder and reports unsupported. |

Torch, zoom, and video recording are not exposed by this release. Use
[media playback](/docs/reference/media) to display an existing recording, and
[files and directories](/docs/reference/fs) to retain captured photos.
