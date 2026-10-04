# Native URL sharing

Call `day::share_url(url, title)` directly from a user action after checking
`day::share_support()`. It presents the toolkit's native chooser and returns whether
presentation started. It does not send the URL automatically, report completion or treat
cancellation as an error. Sharing targets and permissions belong to the operating system.

| Platform | Presentation |
|---|---|
| macOS AppKit / GTK / Qt | NSSharingServicePicker |
| iOS UIKit | UIActivityViewController, with an iPad popover anchor |
| Android | ACTION_SEND system chooser |
| Windows XAML / WinUI / GTK / Qt | DataTransferManager desktop share UI |
| web-dom | Web Share, when supported and called during user activation |
| Linux GTK / Qt; OpenHarmony ArkUI; mock | Unsupported |

The WebView demo uses a button labelled Copy Link on unsupported platforms, backed by
Day's asynchronous clipboard API. It never silently labels copying as native sharing.
Native chooser presentation is window-based; the current API does not accept a per-control
anchor or offer a completion future. Windows and macOS keep the latest chooser/registration
alive until replacement or backend/process teardown.
