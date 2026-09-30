// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Raster images (docs/images.md) and the window image (docs/window-image.md) on the image
//! kit (`ohos_sys::multimedia::image_kit`): decoding bytes into pixelmaps, packing pixelmaps
//! back into PNG/JPEG, and handing an image node a decoded bitmap.
//!
//! Decoded bitmaps live in a registry keyed by the id day-core minted: a pixelmap outlives any
//! one node and may be drawn by several at once. Entries leave only through [`release`], which
//! day-core calls when the app drops its last handle.

// Node handles are opaque runtime tokens (see node.rs), never dereferenced here.
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::ptr;

use ohos_sys::arkui::drawable_descriptor::OH_ArkUI_DrawableDescriptor_CreateFromPixelMap;
use ohos_sys::arkui::native_node::{ArkUI_NodeAttributeType as Attr, OH_ArkUI_GetNodeSnapshot};
use ohos_sys::multimedia::image_kit::native_image::common::{Image_MimeType, ImageResult};
use ohos_sys::multimedia::image_kit::native_image::image_packer::{
    OH_ImagePackerNative_Create, OH_ImagePackerNative_PackToDataFromPixelmap,
    OH_ImagePackerNative_Release, OH_PackingOptions_Create, OH_PackingOptions_Release,
    OH_PackingOptions_SetMimeType, OH_PackingOptions_SetQuality,
};
use ohos_sys::multimedia::image_kit::native_image::image_source::{
    OH_DecodingOptions_Create, OH_DecodingOptions_Release, OH_ImageSourceNative_CreateFromData,
    OH_ImageSourceNative_CreatePixelmap, OH_ImageSourceNative_Release,
};
use ohos_sys::multimedia::image_kit::native_image::pixelmap::{
    OH_PixelmapImageInfo_Create, OH_PixelmapImageInfo_GetAlphaType, OH_PixelmapImageInfo_GetHeight,
    OH_PixelmapImageInfo_GetWidth, OH_PixelmapImageInfo_Release, OH_PixelmapNative_GetImageInfo,
    OH_PixelmapNative_Release, PIXELMAP_ALPHA_TYPE,
};
pub use ohos_sys_opaque_types::OH_PixelmapNative;

use crate::node::Handle;

thread_local! {
    static BITMAPS: RefCell<HashMap<u64, *mut OH_PixelmapNative>> = RefCell::new(HashMap::new());
}

pub fn bitmap(id: u64) -> *mut OH_PixelmapNative {
    BITMAPS.with(|b| b.borrow().get(&id).copied().unwrap_or(ptr::null_mut()))
}

/// A pixelmap's width, height and alpha type (`PIXELMAP_ALPHA_TYPE`).
pub fn info(pm: *mut OH_PixelmapNative) -> (u32, u32, i32) {
    let (mut w, mut h, mut alpha) = (0u32, 0u32, 0i32);
    // SAFETY: an info object created and released here, filled from a live pixelmap.
    unsafe {
        let mut info = ptr::null_mut();
        if OH_PixelmapImageInfo_Create(&mut info) == ImageResult::SUCCESS && !info.is_null() {
            if OH_PixelmapNative_GetImageInfo(pm, info) == ImageResult::SUCCESS {
                OH_PixelmapImageInfo_GetWidth(info, &mut w);
                OH_PixelmapImageInfo_GetHeight(info, &mut h);
                OH_PixelmapImageInfo_GetAlphaType(info, &mut alpha);
            }
            OH_PixelmapImageInfo_Release(info);
        }
    }
    (w, h, alpha)
}

/// Decode `bytes` into a pixelmap the caller owns, with its width and height; `None` when the
/// bytes are not an image the platform decodes.
fn decode_bytes(bytes: &[u8]) -> Option<(*mut OH_PixelmapNative, u32, u32, i32)> {
    // SAFETY: the source reads the borrowed bytes during the call; every object created here is
    // released except the pixelmap, which the caller owns.
    unsafe {
        let mut src = ptr::null_mut();
        if OH_ImageSourceNative_CreateFromData(bytes.as_ptr().cast_mut(), bytes.len(), &mut src)
            != ImageResult::SUCCESS
            || src.is_null()
        {
            return None;
        }
        let mut opts = ptr::null_mut();
        let _ = OH_DecodingOptions_Create(&mut opts);
        let mut pm = ptr::null_mut();
        let rc = OH_ImageSourceNative_CreatePixelmap(src, opts, &mut pm);
        if !opts.is_null() {
            OH_DecodingOptions_Release(opts);
        }
        OH_ImageSourceNative_Release(src);
        if rc != ImageResult::SUCCESS || pm.is_null() {
            return None;
        }
        let (w, h, alpha) = info(pm);
        if w == 0 || h == 0 {
            OH_PixelmapNative_Release(pm);
            return None;
        }
        Some((pm, w, h, alpha))
    }
}

/// Decode into the registry under `id`: (width, height, has alpha), read from the pixelmap's
/// alpha type rather than inferred from the container.
pub fn decode(id: u64, bytes: &[u8]) -> Option<(f64, f64, bool)> {
    let (pm, w, h, alpha) = decode_bytes(bytes)?;
    // Replacing an id releases what it held. day-core mints a fresh id per decode, so this
    // only fires if one is ever reused.
    let old = BITMAPS.with(|b| b.borrow_mut().insert(id, pm));
    if let Some(old) = old.filter(|p| !p.is_null()) {
        // SAFETY: a pixelmap this registry owned.
        unsafe { OH_PixelmapNative_Release(old) };
    }
    Some((
        f64::from(w),
        f64::from(h),
        alpha != PIXELMAP_ALPHA_TYPE::PIXELMAP_ALPHA_TYPE_OPAQUE.0 as i32,
    ))
}

/// Drop a decoded bitmap.
pub fn release(id: u64) {
    if let Some(pm) = BITMAPS
        .with(|b| b.borrow_mut().remove(&id))
        .filter(|p| !p.is_null())
    {
        // SAFETY: a pixelmap this registry owned.
        unsafe { OH_PixelmapNative_Release(pm) };
    }
}

/// Pack a pixelmap as `mime` ("image/png" / "image/jpeg") at `quality` 0..100 (`None` = the
/// format's default). The packer writes into a caller-sized buffer and reports what it used,
/// so the capacity is an upper bound: the raw RGBA size plus room for the container's own
/// headers; a real UI compresses far below it.
pub fn pack(pm: *mut OH_PixelmapNative, mime: &str, quality: Option<u32>) -> Option<Vec<u8>> {
    let (w, h, _) = info(pm);
    if w == 0 || h == 0 {
        return None;
    }
    let mut mime_bytes = mime.as_bytes().to_vec();
    // SAFETY: packer and options created and released here; the buffer outlives the pack.
    unsafe {
        let mut packer = ptr::null_mut();
        if OH_ImagePackerNative_Create(&mut packer) != ImageResult::SUCCESS || packer.is_null() {
            return None;
        }
        let mut opts = ptr::null_mut();
        if OH_PackingOptions_Create(&mut opts) != ImageResult::SUCCESS || opts.is_null() {
            OH_ImagePackerNative_Release(packer);
            return None;
        }
        let mut mt = Image_MimeType {
            data: mime_bytes.as_mut_ptr().cast(),
            size: mime_bytes.len(),
        };
        OH_PackingOptions_SetMimeType(opts, &mut mt);
        if let Some(q) = quality {
            OH_PackingOptions_SetQuality(opts, q);
        }
        let cap = w as usize * h as usize * 4 + 65536;
        let mut buf = vec![0u8; cap];
        let mut used = cap;
        let rc = OH_ImagePackerNative_PackToDataFromPixelmap(
            packer,
            opts,
            pm,
            buf.as_mut_ptr(),
            &mut used,
        );
        OH_PackingOptions_Release(opts);
        OH_ImagePackerNative_Release(packer);
        if rc != ImageResult::SUCCESS || used == 0 {
            return None;
        }
        buf.truncate(used);
        Some(buf)
    }
}

/// Re-encode a decoded bitmap (docs/images.md). `EncodeSpec::fit` is not honored:
/// `OH_PixelmapNative_Scale` rescales in place, so fitting would resize the very bitmap every
/// later draw shares.
pub fn encode(id: u64, mime: &str, quality: Option<u32>) -> Option<Vec<u8>> {
    let pm = bitmap(id);
    if pm.is_null() {
        return None;
    }
    pack(pm, mime, quality)
}

/// Point an image node at a decoded bitmap: `NODE_IMAGE_SRC` takes a drawable descriptor object
/// rather than a `resource://` string. The node takes its own reference; disposing the
/// descriptor here would blank the image.
pub fn node_set_bitmap(n: Handle, id: u64) {
    let pm = bitmap(id);
    if n.is_null() || pm.is_null() {
        return;
    }
    // SAFETY: a live pixelmap; the descriptor becomes the node's.
    let dd = unsafe { OH_ArkUI_DrawableDescriptor_CreateFromPixelMap(pm) };
    if !dd.is_null() {
        crate::node::set_object(n, Attr::NODE_IMAGE_SRC, dd.cast());
    }
}

/// Point an image node at raw encoded bytes: realize has no `BitmapId` to look up (the app never
/// decoded these through day-core), so they are decoded here, handed to the node as a drawable
/// descriptor, and the temporary pixelmap released: the node holds its own reference by then.
pub fn node_set_bytes(n: Handle, bytes: &[u8]) {
    if n.is_null() || bytes.is_empty() {
        return;
    }
    let Some((pm, _, _, _)) = decode_bytes(bytes) else {
        return;
    };
    // SAFETY: a pixelmap decoded above, released once the node holds its descriptor.
    unsafe {
        let dd = OH_ArkUI_DrawableDescriptor_CreateFromPixelMap(pm);
        if !dd.is_null() {
            crate::node::set_object(n, Attr::NODE_IMAGE_SRC, dd.cast());
        }
        OH_PixelmapNative_Release(pm);
    }
}

/// Render a mounted node to PNG bytes (docs/window-image.md), entirely native and SYNCHRONOUS:
/// `OH_ArkUI_GetNodeSnapshot` renders the node into a pixelmap and the image packer encodes it.
/// The ArkTS image kit has no synchronous packer, which would have forced `day::window_image()`
/// to be async on every backend to satisfy this one.
pub fn snapshot_png(n: Handle) -> Option<Vec<u8>> {
    if n.is_null() {
        return None;
    }
    let mut pm = ptr::null_mut();
    // SAFETY: a live node; the snapshot pixelmap is released after packing.
    let rc = unsafe { OH_ArkUI_GetNodeSnapshot(n, ptr::null_mut(), &mut pm) };
    if rc != 0 || pm.is_null() {
        return None;
    }
    let out = pack(pm, "image/png", Some(100));
    // SAFETY: the snapshot pixelmap this call owns.
    unsafe { OH_PixelmapNative_Release(pm) };
    out
}
