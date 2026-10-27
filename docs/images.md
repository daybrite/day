---
title: "Images"
description: "Raster images from bytes: decoding a PNG or JPEG the app already holds into the platform's own image type, drawing it, reading what it is, and writing it back out."
---

<!-- Copyright © The Daybrite Project
     SPDX-License-Identifier: CC-BY-SA-4.0 -->

# Images

An [`image(res::images::logo)`](https://docs.rs/day-pieces/latest/day_pieces/fn.image.html) draws a picture the build staged. This page is about the other half:
bytes the app already holds — a download, a file the user picked, a paste, a row out of a database
— turned into the platform's own image type, drawn, measured, and written back out.

One currency covers both halves. [`ImageSource`] is what the [`image`](https://docs.rs/day-pieces/latest/day_pieces/fn.image.html) piece takes:

```rust
image(res::images::logo)        // Named  — the staged asset, resolved per backend
image(bytes)                    // Bytes  — an encoded PNG/JPEG the app holds
image(&bitmap)                  // Decoded — a handle from `day::decode_image`
```

`Named` is unchanged from [docs/resources](resources.md). The other two are new, and the rest of
this page is about them.

## Decoding

```rust
day::task(async move {
    match day::decode_image(bytes).await {
        Ok(bitmap) => shown.set(Some(bitmap)),
        Err(e) => log::warn!("not an image: {e}"),
    }
});
```

[`decode_image`](https://docs.rs/day-app/latest/day/fn.decode_image.html) takes an `Arc<Vec<u8>>` and answers a `Bitmap`. It is **asynchronous** because one
toolkit genuinely is: a browser decodes through `createImageBitmap`, which resolves on a later
turn. Every other toolkit answers inline, and the future completes without ever yielding.

`Bitmap` is a **handle, not the pixels**. The decoded image lives in the toolkit — an [`NSImage`](https://developer.apple.com/documentation/appkit/nsimage), a
[`GdkTexture`](https://docs.gtk.org/gdk4/class.Texture.html), a [`QImage`](https://doc.qt.io/qt-6/qimage.html), an [`android.graphics.Bitmap`](https://developer.android.com/reference/android/graphics/Bitmap) — and the handle releases it when the last
clone drops. Cloning is cheap; copy it into as many pieces as you like.

```rust
let info = bitmap.info();       // pixel size, scale, format, alpha
bitmap.id()                     // what `ImageSource::Decoded` carries
```

Two failures are worth telling apart, and the error says which: `ImageError::Unsupported` means the
toolkit has no byte decoder at all (probe [`Cap::ImageDecode`] first), while `ImageError::Decode`
means it has one and these bytes are not an image it reads.

## Drawing on a canvas

```rust
let photo = day::decode_image(bytes).await?;          // once
canvas(move |d, size| d.image(&photo, Rect::from_size(size)))
```

[`Draw::image`](https://docs.rs/day-pieces/latest/day_pieces/struct.Draw.html#method.image) takes the **handle**, never bytes. A canvas re-records on every tracked read, so a
buffer in the op would hand the toolkit a megabyte to compare — and re-decode — on every frame. The
app decodes once and draws a number. [`Draw::image_with_opacity`](https://docs.rs/day-pieces/latest/day_pieces/struct.Draw.html#method.image_with_opacity) multiplies the image's own alpha.
Images follow the canvas's top-left origin and current affine transform, including rotation and
zoom. AppKit respects its flipped canvas context, keeping image rows upright.

A released bitmap draws nothing rather than a placeholder: a handle can be dropped between the
record and the replay, and a frame that flashes a grey box is worse than one that omits the image.

## Reading what it is

[`Bitmap::info()`](https://docs.rs/day-core/latest/day_core/image/struct.Bitmap.html#method.info) answers from the decode, synchronously:

| field | meaning |
| --- | --- |
| `pixels` | the real pixel size — not points. A 144-DPI photo reports its pixels, not its print size |
| `scale` | pixels per point. Always `1.0` for decoded bytes; a staged `@2x` asset is the other path |
| `format` | the container the bytes came in, by magic number |
| `has_alpha` | whether the image carries an alpha channel |

[`Bitmap::properties()`](https://docs.rs/day-core/latest/day_core/image/struct.Bitmap.html#method.properties) goes further — EXIF orientation, DPI, capture timestamp, and whatever else
the platform's own reader names — but only where [`Cap::ImageProperties`] says so. A toolkit with
no metadata reader answers `None` to the whole call rather than an empty struct, because "this file
records nothing" and "nobody looked" are different answers.

## Writing it back out

```rust
let png = bitmap.encode(EncodeSpec::default()).await?;                 // PNG
let jpeg = bitmap.encode(EncodeSpec {
    format: ImageFormat::Jpeg,
    quality: Some(0.8),
    fit: Some(Size::new(1024.0, 1024.0)),
    ..Default::default()
}).await?;
```

`fit` scales the longest side down first, preserving aspect. A box **larger** than the original is
ignored: upscaling on an export path inflates the bytes without adding detail.

**Ask before you offer.** Every platform here reads more formats than it writes, and the asymmetry
is the platform's own — which is why [`image_encode_formats()`](https://docs.rs/day-core/latest/day_core/image/fn.image_encode_formats.html) exists rather than a constant list:

```rust
if day::image_encode_formats().contains(&ImageFormat::Jpeg) { … }
```

## Rendering a drawing into one

A canvas drawing rendered offscreen is a third source of a `Bitmap`, beside staged assets and
decoded bytes: [`Drawing::record`](https://docs.rs/day-pieces/latest/day_pieces/struct.Drawing.html#method.record)`(size, draw)`[`.to_bitmap`](https://docs.rs/day-pieces/latest/day_pieces/struct.Drawing.html#method.to_bitmap)`(scale)` replays the drawing through the
toolkit's own canvas painter and returns a handle in the same table, so everything on this page
applies to it, encoding included ([canvas](canvas.md#exporting)).

## Toolkit support

Platform guidance: [Apple: images](https://developer.apple.com/design/human-interface-guidelines/images); [Android: drawables](https://developer.android.com/develop/ui/views/graphics/drawables).

Everything above works on every toolkit except where noted.

| Toolkit | Decodes | Writes | Notes |
| --- | --- | --- | --- |
| **macos-appkit** | everything ImageIO reads | PNG, JPEG, TIFF, BMP, GIF | The only toolkit that answers [`properties()`](https://docs.rs/day-core/latest/day_core/image/struct.Bitmap.html#method.properties): [`NSBitmapImageRep`](https://developer.apple.com/documentation/appkit/nsbitmapimagerep) carries the container's own metadata dictionary |
| **ios-uikit** | everything ImageIO reads | PNG, JPEG | [`UIImage`](https://developer.apple.com/documentation/uikit/uiimage) writes nothing else. `has_alpha` is derived from the container, not measured — asking UIKit would mean reaching through [`CGImage`](https://developer.apple.com/documentation/coregraphics/cgimage) for an alpha flag |
| **macos-gtk**, **linux-gtk** | whatever gdk-pixbuf's loaders read | PNG, JPEG, TIFF, BMP | PNG and TIFF come from the texture's own writers; the rest through gdk-pixbuf |
| **macos-qt**, **linux-qt** | whatever Qt's image plugins read | PNG, JPEG, TIFF, BMP | |
| **android-material** | everything `BitmapFactory` reads | PNG, JPEG | The only toolkit that **reads** `has_alpha` rather than inferring it. WebP decodes but is not offered for encode: `WEBP_LOSSY` is API 30 and the scaffold's `minSdk` is 24 |
| **web-dom** | whatever the engine reads | PNG, JPEG, and WebP where the engine writes it | The one toolkit where decoding **and** encoding are asynchronous. [`image_encode_formats()`](https://docs.rs/day-core/latest/day_core/image/fn.image_encode_formats.html) asks the engine — Chromium writes WebP, WebKit does not — and an encode the engine would quietly turn into PNG is refused instead. `has_alpha` is derived from the container: an `ImageBitmap` exposes no way to ask |
| **harmony-arkui** | whatever the image framework reads | PNG, JPEG | `EncodeSpec::fit` is ignored: `OH_PixelmapNative_Scale` rescales in place, and fitting would resize the very bitmap every later draw shares |
| **windows-winui** | whatever WIC reads | PNG | Win2D canvas bitmaps and WIC PNG encoding, with transparency and optional fit/downscaling. Image dimensions are read from the container header |
| **mock** | a synthetic answer per format | PNG, JPEG | Deterministic sizes a test can predict, in the spirit of its synthetic text metrics |

`Cap::ImageProperties` is Native on macos-appkit alone. Everywhere else [`properties()`](https://docs.rs/day-core/latest/day_core/image/struct.Bitmap.html#method.properties) answers
`None` — including toolkits whose platform *could* report metadata (ArkUI exposes
`OH_ImageSourceNative_GetImageProperty`) but where nothing reads it yet.

## Showing one in a piece

An [`image()`](https://docs.rs/day-pieces/latest/day_pieces/fn.image.html) piece takes any of the three sources, and a source **swap repaints the same view**
rather than rebuilding the subtree — so an [`image()`](https://docs.rs/day-pieces/latest/day_pieces/fn.image.html) bound to a signal shows new pixels without
disturbing anything around it:

```rust
let shown: Signal<Option<day::Bitmap>> = Signal::new(None);
image(move || match shown.get() {
    Some(b) => ImageSource::from(b.id()),
    None => ImageSource::Named(String::new()),
})
```

One limit worth stating: [`VectorBuilder::tint`](https://docs.rs/day-pieces/latest/day_pieces/trait.VectorBuilder.html#tymethod.tint) applies to **named vector** sources only. A recolor re-reads the
source art — an SVG's paths on ArkUI and WinUI, the file on GTK — and bytes have no such art to
re-read.

For asynchronously loaded images in recycled rows, bind the source to a bundled placeholder
until the current item's image is ready; see [list](./list.md#asynchronous-row-images).
UIKit and Android also treat the default source (`ImageSource::Named(String::new())`) as an
explicit clear when patching an existing image view. An unresolved nonempty name or a source
the toolkit cannot decode retains the preceding image, so it is not a loading placeholder.

## Reactive images with placeholders

[`async_image`](https://docs.rs/day-pieces/latest/day_pieces/async_image/fn.async_image.html) accepts a
`Signal<Option<Arc<Vec<u8>>>>`. It uses the core decoder and canvas, with no HTTP dependency.
`None`, a pending decode, and an invalid image show the placeholder. A new source cancels the
previous decode and clears its picture; a disposed scope cancels the task. Completed bitmap
handles remain alive while the drawing uses them.

```rust
let bytes = Signal::new(None);
async_image(bytes).circle().frame(48.0, 48.0)
```

The view grows in both axes. Constrain avatars and thumbnails with [`.frame(width, height)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.frame).
Its default scaling is `ContentMode::Fill`; choose `Fit` to preserve the whole picture or
`Stretch` to fill without preserving its proportions. [`.placeholder_color(color)`](https://docs.rs/day-pieces/latest/day_pieces/async_image/trait.AsyncImageBuilder.html#tymethod.placeholder_color) changes
the empty surface. [`.rounded(radius)`](https://docs.rs/day-pieces/latest/day_pieces/async_image/trait.AsyncImageBuilder.html#tymethod.rounded) clips the image and placeholder to a rounded rectangle;
[`.circle()`](https://docs.rs/day-pieces/latest/day_pieces/async_image/trait.AsyncImageBuilder.html#tymethod.circle) uses a centered circle whose diameter is the shorter side, including after resize.

For a URL, enable `day`'s optional `http-image` feature and call
[`day::http_image(url)`](https://docs.rs/day-app/latest/day/fn.http_image.html). The helper uses the
[HTTP service](/docs/reference/http), accepts only successful HTTP responses, and cancels the
request when the owning scope is disposed. Transport failure leaves the placeholder.
This is a convenience for one request, with no shared response cache or retry policy.

The ordinary [`image`](https://docs.rs/day-pieces/latest/day_pieces/fn.image.html) API retains the
previous picture on an invalid replacement. Choose it when that behavior fits the app; use
`async_image` when an explicit loading or empty state is required.

## Lifetime

A `Bitmap` releases its pixels when the last clone drops, through [`Toolkit::release_image`](https://docs.rs/day-spec/latest/day_spec/trait.Toolkit.html#method.release_image). The
release waits for a safe point when it has to: a handle can die inside the tree's own borrow (a
removed node's handler owning the last clone), and the release then runs at the next pump rather
than re-entering the tree.

Who keeps a decode alive:

- [`image(&bitmap)`](https://docs.rs/day-pieces/latest/day_pieces/fn.image.html) — the piece does. It holds a clone for the node's life, so the app may drop
  its own handle the moment the piece is built.
- [`Draw::image(&bitmap, …)`](https://docs.rs/day-pieces/latest/day_pieces/struct.Draw.html#method.image) and a `Signal<ImageSource>` carrying `ImageSource::Decoded(id)` —
  the app does. Both name the handle without owning it; keep the `Bitmap` alive as long as
  anything draws it. On the web the `<img>` behind a released id points at an object URL that no
  longer exists.
- [`bitmap.encode(…)`](https://docs.rs/day-core/latest/day_core/image/struct.Bitmap.html#method.encode) — the future does, for its whole life, so dropping the handle after asking
  for an export is fine.
- A `DecodeFuture` dropped before its answer — nobody, and that is handled: a decode that
  completes with no owner left releases itself rather than leaking.

A handle that outlives its tree releases nothing, because there is no toolkit left to tell. That
only arises when a tree is torn down while bitmaps are still alive — [`uninstall_tree`](https://docs.rs/day-core/latest/day_core/fn.uninstall_tree.html) in tests,
not an app, which exits the process instead.
