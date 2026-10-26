---
navTitle: "Frames"
title: "Display frames"
description: "Cancellable native frame callbacks for canvas animations and other UI clients."
---

<!-- Copyright © The Daybrite Project
     SPDX-License-Identifier: CC-BY-SA-4.0 -->

# Display frame callbacks

Schedule animation or rendering work in step with the display. A frame clock runs callbacks
on the UI thread and lets the app stop them by dropping their handle.

`day::frame` schedules UI-thread work at native display frame opportunities. It is independent
of canvas and pieces: capture a [`FrameClock::current()`](https://daybrite.dev/api/day_core/frame/struct.FrameClock.html#method.current) while building a page, or obtain one
from [`WindowHandle::frame_clock()`](https://daybrite.dev/api/day_core/windows/struct.WindowHandle.html#method.frame_clock), then keep the returned `FrameHandle` in your controller.

```rust,ignore
use day::frame::FrameClock;
use std::ops::ControlFlow;

let clock = FrameClock::current();
let handle = clock.subscribe(move |frame| {
    simulation.advance(frame.delta.as_secs_f64());
    repaint.notify(); // Only canvases tracking this trigger re-record.
    if simulation.is_moving() { ControlFlow::Continue(()) }
    else { ControlFlow::Break(()) }
});
// Own `handle` until done, or call handle.in_scope() for reactive-scope ownership.
```

[`clock.request(callback)`](https://daybrite.dev/api/day_core/frame/struct.FrameClock.html#method.request) delivers once. [`clock.subscribe(callback)`](https://daybrite.dev/api/day_core/frame/struct.FrameClock.html#method.subscribe) continues while the
callback returns `Continue(())`. `Break(())` pauses the subscription; [`handle.resume()`](https://daybrite.dev/api/day_core/frame/struct.FrameHandle.html#method.resume) wakes it.
[`pause()`](https://daybrite.dev/api/day_core/frame/struct.FrameHandle.html#method.pause) resets the elapsed-time baseline; [`cancel()`](https://daybrite.dev/api/day_core/frame/struct.FrameHandle.html#method.cancel) permanently removes a registration.
Dropping the handle cancels too. A one-shot callback cannot be replayed by resuming its handle.
The module-level [`request`](https://daybrite.dev/api/day_core/frame/fn.request.html) and [`subscribe`](https://daybrite.dev/api/day_core/frame/fn.subscribe.html) functions use the current window. Prefer capturing
the clock when building UI rather than discovering the current window later in an input handler.

The service coalesces clients into one outstanding native request **per window**, batches their
reactive writes, and flushes before requesting another frame. No native requests remain when all
clients pause, finish, or cancel. A request made inside a callback starts on a later frame.
Cancelling another client during dispatch prevents its callback in that dispatch. Native bridges
carry monotonically allocated integer tickets, so a late callback cannot touch a freed closure.
Callbacks and handles stay on the UI thread; callback panics are contained and stop that client.

`Frame` contains monotonic `timestamp`, subscription-local `delta`, and an optional
`target_timestamp` when the toolkit supplies a predicted presentation time. The epoch is native
and may differ across windows or platforms. Delivery is a **frame opportunity**, not proof of
presentation. Delta is zero on the first frame and after pause/resume or app foregrounding.
It is deliberately unclamped. Physics clients should choose a fixed-step/catch-up policy; the
Showcase ball uses 120 Hz simulation steps with at most 100 ms of catch-up per display callback.
Never assume 60 Hz or use callback counts to measure elapsed time.

App backgrounding cancels pending requests; foregrounding resumes active clients with a fresh
baseline. Closing a window cancels its registrations. Desktop focus loss alone does not pause
visible animations. Hidden/unmapped-window throttling follows the native scheduler; it is not a
portable per-view visibility signal. Clients should pause when their animation is no longer
needed (including retained, offscreen pages). [`in_scope()`](https://daybrite.dev/api/day_core/frame/struct.FrameHandle.html#method.in_scope) cancels at scope disposal.

| Toolkit | Native source |
| --- | --- |
| macos-appkit | View-associated [`CADisplayLink`](https://developer.apple.com/documentation/quartzcore/cadisplaylink) on macOS 14+; `CVDisplayLink` on supported macOS 13, marshaled to the main queue |
| ios-uikit | Screen-associated [`CADisplayLink`](https://developer.apple.com/documentation/quartzcore/cadisplaylink), in common run-loop modes |
| android-material | [`Choreographer.postFrameCallback`](https://developer.android.com/reference/android/view/Choreographer) / `removeFrameCallback` |
| windows-winui | One-shot registration with `Microsoft.UI.Xaml.Media.CompositionTarget.Rendering` |
| web-dom | `requestAnimationFrame` / `cancelAnimationFrame` |
| macos/linux-gtk | Host widget's `add_tick_callback`, timestamped by its [`GdkFrameClock`](https://docs.gtk.org/gdk4/class.FrameClock.html) |
| macos/linux-qt | Host [`QWindow::requestUpdate()`](https://doc.qt.io/qt-6/qwindow.html) / [`QEvent::UpdateRequest`](https://doc.qt.io/qt-6/qevent.html) |
| harmony-arkui | [`OH_NativeVSync_RequestFrame`](https://github.com/openharmony/docs/blob/master/en/application-dev/reference/apis-arkgraphics2d/capi-nativevsync.md), delivered back onto the ArkUI JS/UI loop |
| mock | Inert; core tests inject a manual timestamp source |

There is no Day timer fallback. [Qt documents](https://doc.qt.io/qt-6/qwindow.html#requestUpdate)
that `requestUpdate` is vsync-aligned where supported, with a Qt-managed timer elsewhere. This
does not promise hardware vsync on every Qt platform plugin. GTK similarly owns its frame-clock
pacing. Harmony's source uses the baseline NativeVSync API (API 9+) because Day's ArkUI node
handle does not expose an OS window ID; it is a system-vsync source rather than an explicit
window-ID association. macOS 13's Core Video adapter selects the host's display at each request;
modern AppKit follows the view's display automatically. Predicted timestamps are currently
available on CADisplayLink; other adapters leave them absent.

[`Toolkit::request_frame(host, callback) -> CancelFrame`](https://daybrite.dev/api/day_spec/trait.Toolkit.html#method.request_frame) is the toolkit seam. It must never
invoke the callback inline; delivery is one-shot and cancellation must be safe before or after
delivery. A toolkit can keep a paused native source during delivery to reuse it if Day re-arms,
but must release it when demand ends. Qt keeps only an inert event filter after cancellation;
its pending window update, if any, still belongs to Qt's backing-store machinery.

The existing [`frame_clock`](https://daybrite.dev/api/day_pieces/fn.frame_clock.html) piece uses this same service. Its compatibility contract still gives
an initial 1/60 s step and clamps subsequent deltas to 100 ms. New clients should use the explicit
handle API so they can stop while idle. Native-widget [`with_animation`](https://daybrite.dev/api/day_core/fn.with_animation.html) continues to use toolkit
animators; this primitive does not replace them. Interpolation, timing and a frame-driven
transition driver built on it are in [tween](tween.md).

Regression coverage lives in `day-core/src/frame.rs`: coalescing, callback reentrancy, cancellation,
late native delivery, independent windows, lifecycle suspension, malformed timestamps, and
unclamped elapsed time. Day-Showcase's Animation page and `dayscript/frame-animation.yaml`
exercise native delivery, pause/resume, settling, canvas taps, reset and navigation.
