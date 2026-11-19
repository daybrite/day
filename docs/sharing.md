---
navTitle: "Sharing"
---

# Native URL sharing

Sharing lets people send content to a destination they choose, such as another app or a contact. A
system share interface offers the actions available for that content on the device.

Build a request with [`Share::url(url)`](https://docs.rs/day-pieces/latest/day_pieces/struct.Share.html)
and [`present`](https://docs.rs/day-pieces/latest/day_pieces/struct.Share.html#method.present) it
directly from a user action after checking
[`day::share_support()`](https://docs.rs/day-pieces/latest/day_pieces/fn.share_support.html). It
presents the toolkit's native chooser and returns whether presentation started. It does not send
the URL automatically, report completion or treat cancellation as an error. Sharing targets and
permissions belong to the operating system.

```rust
Share::url(page.url())
    .title(page.title())
    .action(share_action(res::str::add_bookmark()).symbol(Symbol::Bookmark).action(add_bookmark))
    .action(share_action(res::str::find_on_page()).symbol(Symbol::Search).action(open_find))
    .anchor(id!(share))
    .present();
```

## The app's own actions

[`.action(share_action(label))`](https://docs.rs/day-pieces/latest/day_pieces/fn.share_action.html)
offers one of the app's commands in the chooser, after the system's targets: Safari's Add
Bookmark, Add to Favorites and Find on Page are this kind of action. Each has a title (a literal
or a `res::str::…` text, resolved when the chooser opens), an optional
[`symbol`](https://docs.rs/day-pieces/latest/day_pieces/struct.ShareAction.html#method.symbol) and
the work it does, which runs once the chooser has closed. Probe `Cap::ShareActions`:

| Platform | App actions |
|---|---|
| [iOS](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit") [UIKit](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit") | Native: a `UIActivity` per action, listed with the sheet's own actions, its SF Symbol beside the title. The action runs from the sheet's completion handler, after the sheet is dismissed |
| [macOS](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit") [AppKit](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit") | Native: an `NSSharingService` per action, added by the picker's delegate after the system's services |
| [Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material") 14 and later | Native: a `ChooserAction` per action in the system chooser's action row |
| [Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material") 13 and earlier, [Windows](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui"), web, [GTK](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk"), [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt"), [ArkUI](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui") | Unsupported: the chooser shows the system's targets only |

Where the actions are unsupported the chooser leaves them out, so offer the same commands in the
app's own menu too. Day registers each action's handler for as long as its chooser can report
a choice; opening the next chooser retires the previous one's actions.

## Pointing at the control

[`.anchor(id!(share))`](https://docs.rs/day-pieces/latest/day_pieces/struct.Share.html#method.anchor)
names the control the chooser points at where it opens as a popover: an iPad's share sheet and
[macOS](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit")'s sharing picker point at the button that was pressed. Without an anchor the chooser points
at the window's top trailing corner. Phones and the other platforms present it the same way
either way.

## Platforms

| Platform | Presentation |
|---|---|
| [macOS](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit") [AppKit](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit") / [GTK](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk") / [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") | NSSharingServicePicker |
| [iOS](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit") [UIKit](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit") | UIActivityViewController, with an iPad popover anchor |
| [Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material") | ACTION_SEND system chooser |
| [Windows](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") ([WinUI](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") / [GTK](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk") / [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt")) | DataTransferManager desktop share UI |
| [Web](https://daybrite.dev/docs/reference/platforms/web-dom "toolkit:web-dom") | Web Share, when supported and called during user activation |
| [Linux](https://daybrite.dev/docs/reference/platforms "toolkit:linux-gtk,linux-qt") [GTK](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk") / [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt"); [OpenHarmony](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui") [ArkUI](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui") | Unsupported |
| mock | Records each request and its anchor; [`MockProbe::choose_share_action`](https://docs.rs/day-mock/latest/day_mock/struct.MockProbe.html#method.choose_share_action) chooses an action |

The WebView demo uses a button labeled Copy Link on unsupported platforms, backed by
Day's asynchronous clipboard API. It never silently labels copying as native sharing.
The API offers no completion future. [Windows](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") and [macOS](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit") keep the latest chooser/registration
alive until replacement or toolkit/process teardown.

## Platform guidance

- [Apple: activity views](https://developer.apple.com/design/human-interface-guidelines/activity-views)
- [Android: sending simple data](https://developer.android.com/training/sharing/send)
