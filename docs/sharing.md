---
navTitle: "Sharing"
---

# Native URL sharing

Sharing lets people send content to a destination they choose, such as another app or a contact. A
system share interface offers the actions available for that content on the device.

Call [`day::share_url(url, title)`](https://docs.rs/day-app/latest/day/fn.share_url.html) directly
from a user action after checking
[`day::share_support()`](https://docs.rs/day-app/latest/day/fn.share_support.html). It presents the
toolkit's native chooser and returns whether presentation started. It does not send the URL
automatically, report completion or treat cancellation as an error. Sharing targets and permissions
belong to the operating system.

| Platform | Presentation |
|---|---|
| macOS AppKit / GTK / Qt | NSSharingServicePicker |
| iOS UIKit | UIActivityViewController, with an iPad popover anchor |
| Android | ACTION_SEND system chooser |
| Windows (WinUI / GTK / Qt) | DataTransferManager desktop share UI |
| Web | Web Share, when supported and called during user activation |
| Linux GTK / Qt; OpenHarmony ArkUI; mock | Unsupported |

The WebView demo uses a button labeled Copy Link on unsupported platforms, backed by
Day's asynchronous clipboard API. It never silently labels copying as native sharing.
Native chooser presentation is window-based; the current API does not accept a per-control
anchor or offer a completion future. Windows and macOS keep the latest chooser/registration
alive until replacement or toolkit/process teardown.

## Platform guidance

- [Apple: activity views](https://developer.apple.com/design/human-interface-guidelines/activity-views)
- [Android: sending simple data](https://developer.android.com/training/sharing/send)
