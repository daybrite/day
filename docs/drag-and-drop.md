---
title: "Drag and drop"
description: "Native transfers, representation types, destination policy, and platform boundaries."
---

<!-- Copyright © The Daybrite Project; SPDX-License-Identifier: CC-BY-SA-4.0 -->

# Drag and drop

Let people drag text, images, files, and other data between your app and other apps. Sources
offer data in one or more formats; drop targets choose the formats they can accept.

`day::transfer` attaches native system transfers to pieces. A source supplies an owned
`Offer` when the native drag starts. An offer contains separate `Item`s; each item's MIME
representations are alternative encodings of the same object. Arbitrary binary data is
preserved, including zero bytes. No clipboard writes or private inter-process connection
are needed.

```rust
use day::{
    prelude::*,
    transfer::{Item, Offer, Operation, Representation, Target},
};
use std::rc::Rc;

let received = Signal::new(String::new());
let source = label(res::str::drag()).drag_source(|_| {
    Some(Offer {
        items: vec![Item::new(vec![Representation::new(
            "application/vnd.example.card",
            vec![0, 255, 42],
        )])],
    })
});
let target = label(move || received.get()).drop_target(Target {
    types: vec!["application/vnd.example.card".into()],
    accept: Rc::new(|location| {
        if location.position.x >= 24. && location.has("application/vnd.example.card") {
            Operation::Copy
        } else {
            Operation::None
        }
    }),
    receive: Rc::new(move |drop| {
        if drop.items.len() != 1 {
            return false;
        }
        let Some(bytes) = drop.items[0].get("application/vnd.example.card") else {
            return false;
        };
        received.set(res::str::received_bytes(bytes.len() as i64).format());
        true
    }),
});
```

Localize user-facing strings in real applications. Apply a canvas's gesture/test [`.id(...)`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.id)
before [`.drop_target(...)`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.drop_target) when the identifier must still address the canvas itself: the
transfer decorator creates a native wrapper. The source and target may be the same piece.

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](snippet-catalog.md) and [catalog setup](/docs/how-to/localization).

## Acceptance and completion

The synchronous policy receives target-local logical coordinates, advertised types, native
allowed operations, and native locality. It is called during hover and again before delivery.
Return `None` for a forbidden position or disabled destination. Policy cannot escalate the
native operation mask. Validate actual bytes in `receive`; a type name is not proof of content.
Callbacks run on the UI thread inside their owning piece scope and become inactive when that
scope is disposed. An asynchronous task must capture any scene state it uses after an await;
enter the captured scope for model operations that resolve ambient state at commit time.

Adapters currently advertise **Copy**. A `true` receipt acknowledges ownership of the bytes;
it does not promise that an asynchronous document import has committed. Do not delete source
data after launching asynchronous work. Showcase performs same-page moves within one model
transaction, using native locality plus a page identifier. Other windows and processes receive
copies. The `Move` and `Link` enum variants reserve negotiation vocabulary but are not currently
advertised by these adapters.

Day-to-Day transfers preserve ordered items and alternatives in the versioned native
`application/vnd.day.transfer` representation. Standard first-item formats are also published
for other applications. The bundle is bounded to 64 MiB, 256 items, and 32 alternatives per
item; malformed, duplicate, truncated, or oversized representations are rejected. Callers
must use bare MIME types without parameters. Standard foreign formats have toolkit-specific
item/alternative coverage; multiple native file references can be encoded in one URI list.

## Native adapters and limitations

| Toolkit | Native mechanism | File and timing notes |
|---|---|---|
| AppKit | NSDraggingSession, [NSPasteboard](https://developer.apple.com/documentation/appkit/nspasteboard) | File URLs; synchronous receipt |
| GTK | DragSource, DropTargetAsync, GIO streams | URI lists; reads cancel after 30 seconds |
| Qt | [QDrag](https://doc.qt.io/qt-6/qdrag.html), QMimeData | URLs; synchronous receipt; source survives nested event-loop disposal safely |
| WinUI | CanDrag/AllowDrop in-app, host-window [`IDropTarget`](https://learn.microsoft.com/en-us/windows/win32/api/oleidl/nn-oleidl-idroptarget) for external drags, DataPackage | StorageItems; drag/drop deferrals; CF_HDROP imported as `text/uri-list` |
| UIKit | UIDragInteraction, UIDropInteraction, NSItemProvider | Encoded byte providers; pending loads cancel after 30 seconds |
| Android | startDragAndDrop, ClipData, ContentProvider | Read-only URI grants; bounded reads on a worker; permissions released at receipt/timeout |
| ArkUI | native node drag events, UDMF | General byte records and file URI records; compile checked against API 18 |
| Web | HTML draggable and DataTransfer | Files read at drop; HTML custom strings carry the Day bundle as base64 |

macOS MIME/UTI conversion is shared with the system mapping used by GTK, so custom types
survive AppKit/Qt/GTK process boundaries. Windows (WinUI) uses named DataPackage streams; the
WinUI/Qt/GTK cross-process matrix still requires a Windows runtime check.

HTML protected mode only exposes metadata during hover. File objects and string read requests
are captured synchronously during drop, before awaiting bytes. Browser pages can exchange Day
binary bundles, but HTML custom strings do **not** provide universal native binary export or
filesystem paths. Check capabilities rather than assuming desktop file behavior on web.

The current API is eager bytes. File promises, asynchronous application-supplied providers,
portable file leases, and externally committed moves are not supported. UIKit does not yet import promised files through
NSItemProvider file representations; Android content URIs are read into bytes. File references
are not a portable promise of sandbox access. The `DragFilePromises` and `DragExternalMove`
capabilities therefore remain unsupported. Existing canvas and collection gestures remain
separate from system data transfers.

> [!WARNING]
> **Windows / WinUI: external file drops.**
> Explorer file drops have not been verified on WinUI.
> Cross-process transfers between Windows toolkits also remain unverified.

> [!WARNING]
> **Linux and HarmonyOS: native transfers.**
> Native drag-and-drop behavior remains unverified. Test cross-application transfers,
> cancellation, and the formats your app accepts.

## Platform guidance

- [Apple: drag and drop](https://developer.apple.com/design/human-interface-guidelines/drag-and-drop)
- [Android: drag and drop](https://developer.android.com/develop/ui/views/touch-and-input/drag-drop)
