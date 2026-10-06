// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Drag and drop (docs/drag-and-drop.md) on ArkUI's drag events and the UDMF data framework
//! (`ohos_sys::arkui` `drag_and_drop`, `ohos_sys::udmf`).
//!
//! A drag from a Day source carries the whole offer as one `application/vnd.day.transfer`
//! record (the encoded packet another Day target decodes intact) alongside each
//! representation as its native UDMF type, so apps that do not know Day still get images and
//! files. A drop reads the packet back when present, else assembles one item from whatever
//! records the system delivered.

// Node handles are opaque runtime tokens (see node.rs), never dereferenced here.
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::ffi::{CStr, c_char};
use std::ptr;

use day_spec::Point;
use day_spec::sidetable::SideTable;
use day_spec::transfer::{Item, Location, Offer, Operation, Representation, Source, Target};
use ohos_sys::arkui::drag_and_drop::{
    ArkUI_DragEvent, ArkUI_DragResult, ArkUI_DropOperation, OH_ArkUI_AllowNodeAllDropDataTypes,
    OH_ArkUI_DisallowNodeAnyDropDataTypes, OH_ArkUI_DragEvent_GetDataTypeCount,
    OH_ArkUI_DragEvent_GetDataTypes, OH_ArkUI_DragEvent_GetTouchPointXToWindow,
    OH_ArkUI_DragEvent_GetTouchPointYToWindow, OH_ArkUI_DragEvent_GetUdmfData,
    OH_ArkUI_DragEvent_SetData, OH_ArkUI_DragEvent_SetDragResult,
    OH_ArkUI_DragEvent_SetSuggestedDropOperation, OH_ArkUI_NodeEvent_GetDragEvent,
    OH_ArkUI_SetNodeDraggable,
};
use ohos_sys::arkui::native_node::{
    ArkUI_NodeEvent, ArkUI_NodeEventType as Ev, OH_ArkUI_NodeEvent_GetEventType,
    OH_ArkUI_NodeEvent_GetNodeHandle, OH_ArkUI_NodeUtils_GetPositionWithTranslateInWindow,
};
use ohos_sys::arkui::native_type::ArkUI_IntOffset;
use ohos_sys::udmf::data_management_framework::{
    OH_UdmfData_AddRecord, OH_UdmfData_Create, OH_UdmfData_Destroy, OH_UdmfData_GetRecords,
    OH_UdmfRecord_AddFileUri, OH_UdmfRecord_AddGeneralEntry, OH_UdmfRecord_Create,
    OH_UdmfRecord_Destroy, OH_UdmfRecord_GetFileUri, OH_UdmfRecord_GetGeneralEntry,
    OH_UdmfRecord_GetTypes,
};
use ohos_sys::udmf::data_struct::{
    OH_UdsFileUri_Create, OH_UdsFileUri_Destroy, OH_UdsFileUri_GetFileUri, OH_UdsFileUri_SetFileUri,
};

use ohos_sys_opaque_types::OH_UdmfData;

use crate::AHandle;
use crate::node::{self, Handle};

day_core::tls_group! {
    static SOURCES: SideTable<Source> = SideTable::new();
    static TARGETS: SideTable<Target> = SideTable::new();
    /// The node whose drag is in flight, so a drop knows it is local.
    static ACTIVE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// The in-flight drag's data. `OH_ArkUI_DragEvent_SetData` keeps the pointer, not a
    /// copy: ArkUI reads the records after the start callback has returned, when it stores
    /// the drag with the UDMF service (`ProcessDragDropData` → `UdmfClient::SetData`), so the
    /// data lives until the drag ends. Destroying it on the way out of the callback, as the
    /// first port did, left the service reading freed memory: every real drag of the
    /// Showcase's logo crashed inside `SetData` (2026-10).
    static DRAG_DATA: std::cell::Cell<*mut OH_UdmfData> = const { std::cell::Cell::new(ptr::null_mut()) };
}

/// Release the data of the last drag, if any; the next drag's replaces it.
fn release_drag_data() {
    let data = DRAG_DATA.with(|d| d.replace(ptr::null_mut()));
    if !data.is_null() {
        // SAFETY: data this module created, and no drag is in flight to read it.
        unsafe { OH_UdmfData_Destroy(data) };
    }
}

const LIMIT: usize = 64 * 1024 * 1024;
/// The UDMF type carrying the whole encoded offer.
const BUNDLE: &CStr = c"application/vnd.day.transfer";

/// The UDMF type id a day record is stored under: the MIME text itself, except for the
/// standard uniform types whose bytes mean the same thing (a PNG file is a PNG file), so a
/// record other apps can read is typed the way they expect. Custom ids are what UDMF's
/// general entry exists for; the registry is never asked, because resolving an id it does
/// not know through the NDK (`OH_Utd_Create` on the `flex.…` id it mints for an unknown MIME)
/// read freed memory inside `libudmf` on the 7.0 emulator and took the app down with the
/// drop (2026-10). Within one app the day bundle carries every representation anyway.
fn native(mime: &str) -> &str {
    match mime {
        "image/png" => "general.png",
        "image/jpeg" => "general.jpeg",
        "image/gif" => "general.gif",
        "image/webp" => "general.webp",
        "text/html" => "general.html",
        "text/uri-list" => "general.file-uri",
        other => other,
    }
}

/// The MIME type for a record's type id (the inverse of [`native`]): a standard id Day
/// knows, else the id itself, which for a day record is already the MIME text.
fn mime(type_id: &str) -> String {
    match type_id {
        "general.png" => "image/png",
        "general.jpeg" => "image/jpeg",
        "general.gif" => "image/gif",
        "general.webp" => "image/webp",
        "general.html" => "text/html",
        "general.file-uri" => "text/uri-list",
        other => other,
    }
    .to_owned()
}

unsafe fn add_entry(data: *mut OH_UdmfData, mime_type: &str, bytes: &[u8]) {
    // SAFETY: a fresh record, filled and destroyed here; UDMF copies the bytes.
    unsafe {
        let record = OH_UdmfRecord_Create();
        if record.is_null() {
            return;
        }
        let ty = node::cstr(native(mime_type));
        // A record the registry would not describe is left out rather than stored: see
        // `native`.
        if OH_UdmfRecord_AddGeneralEntry(
            record,
            ty.as_ptr(),
            bytes.as_ptr().cast_mut(),
            bytes.len() as u32,
        ) == 0
        {
            OH_UdmfData_AddRecord(data, record);
        }
        OH_UdmfRecord_Destroy(record);
    }
}

/// The pointer position of a drag event, node-relative and in vp.
unsafe fn position(n: Handle, drag: *mut ArkUI_DragEvent) -> Point {
    let mut origin = ArkUI_IntOffset { x: 0, y: 0 };
    // SAFETY: a live node and drag event.
    unsafe {
        OH_ArkUI_NodeUtils_GetPositionWithTranslateInWindow(n, &mut origin);
        let d = node::density();
        Point::new(
            (f64::from(OH_ArkUI_DragEvent_GetTouchPointXToWindow(drag)) - f64::from(origin.x)) / d,
            (f64::from(OH_ArkUI_DragEvent_GetTouchPointYToWindow(drag)) - f64::from(origin.y)) / d,
        )
    }
}

pub fn forget(n: Handle) {
    let key = n as usize;
    SOURCES.with(|t| t.remove(key));
    TARGETS.with(|t| t.remove(key));
    ACTIVE.with(|a| {
        if a.get() == key {
            a.set(0);
        }
    });
}

pub fn source(h: &AHandle, source: Source) {
    SOURCES.with(|t| t.insert(h.0 as usize, source));
    // SAFETY: a live node.
    unsafe { OH_ArkUI_SetNodeDraggable(h.0, true) };
    node::register_event(h.0, Ev::NODE_ON_DRAG_START, 0);
    node::register_event(h.0, Ev::NODE_ON_DRAG_END, 0);
}

pub fn target(h: &AHandle, target: Target) {
    TARGETS.with(|t| t.insert(h.0 as usize, target));
    // SAFETY: a live node.
    unsafe { OH_ArkUI_AllowNodeAllDropDataTypes(h.0) };
    for ev in [
        Ev::NODE_ON_DRAG_ENTER,
        Ev::NODE_ON_DRAG_MOVE,
        Ev::NODE_ON_DRAG_LEAVE,
        Ev::NODE_ON_DROP,
    ] {
        node::register_event(h.0, ev, 0);
    }
}

/// Handle a drag event for a registered source or target. True when consumed.
pub fn event(ev: *mut ArkUI_NodeEvent) -> bool {
    // SAFETY: a live event for the callback's duration.
    let (n, kind) = unsafe {
        (
            OH_ArkUI_NodeEvent_GetNodeHandle(ev),
            OH_ArkUI_NodeEvent_GetEventType(ev),
        )
    };
    let key = n as usize;
    let is_source = SOURCES.with(|t| t.get(key).is_some());
    let is_target = TARGETS.with(|t| t.get(key).is_some());
    if !is_source && !is_target {
        return false;
    }
    if !matches!(
        kind,
        Ev::NODE_ON_DRAG_START
            | Ev::NODE_ON_DRAG_END
            | Ev::NODE_ON_DRAG_ENTER
            | Ev::NODE_ON_DRAG_MOVE
            | Ev::NODE_ON_DRAG_LEAVE
            | Ev::NODE_ON_DROP
    ) {
        return false;
    }
    // SAFETY: a live event.
    let drag = unsafe { OH_ArkUI_NodeEvent_GetDragEvent(ev.cast()) };
    if drag.is_null() {
        return false;
    }
    // SAFETY: as above.
    let at = unsafe { position(n, drag) };
    if kind == Ev::NODE_ON_DRAG_END {
        ACTIVE.with(|a| a.set(0));
        release_drag_data();
        return true;
    }
    if kind == Ev::NODE_ON_DRAG_START {
        if is_source {
            start(n, drag, at);
        }
        return true;
    }
    if !is_target {
        return false;
    }
    if kind == Ev::NODE_ON_DRAG_LEAVE {
        // SAFETY: a live node.
        unsafe { OH_ArkUI_AllowNodeAllDropDataTypes(n) };
        return true;
    }
    hover_or_drop(n, drag, at, kind == Ev::NODE_ON_DROP);
    true
}

/// A drag begins on a source: publish its offer.
fn start(n: Handle, drag: *mut ArkUI_DragEvent, at: Point) {
    let Some(offer) = SOURCES.with(|t| t.get(n as usize)).and_then(|f| f(at)) else {
        return;
    };
    let Some(packet) = offer.encode() else {
        return;
    };
    if packet.len() > LIMIT {
        return;
    }
    // SAFETY: UDMF objects created, filled and destroyed here.
    unsafe {
        let data = OH_UdmfData_Create();
        if data.is_null() {
            return;
        }
        let bundle = BUNDLE.to_str().unwrap_or_default();
        add_entry(data, bundle, &packet);
        for rep in offer
            .items
            .iter()
            .flat_map(|i| i.representations.iter())
            .take(32)
        {
            if rep.mime == "text/uri-list" {
                let list = String::from_utf8_lossy(&rep.bytes);
                for line in list.lines() {
                    let uri = line.trim_end_matches('\r');
                    if uri.is_empty() || uri.starts_with('#') {
                        continue;
                    }
                    let file = OH_UdsFileUri_Create();
                    let rec = OH_UdmfRecord_Create();
                    if !file.is_null() && !rec.is_null() {
                        let c = node::cstr(uri);
                        OH_UdsFileUri_SetFileUri(file, c.as_ptr());
                        OH_UdmfRecord_AddFileUri(rec, file);
                        OH_UdmfData_AddRecord(data, rec);
                    }
                    if !rec.is_null() {
                        OH_UdmfRecord_Destroy(rec);
                    }
                    if !file.is_null() {
                        OH_UdsFileUri_Destroy(file);
                    }
                }
            } else {
                add_entry(data, &rep.mime, &rep.bytes);
            }
        }
        // The drag owns the data from here (see `DRAG_DATA`); a drag that never started
        // releases it at the next one.
        release_drag_data();
        if OH_ArkUI_DragEvent_SetData(drag, data) == 0 {
            ACTIVE.with(|a| a.set(n as usize));
            DRAG_DATA.with(|d| d.set(data));
        } else {
            OH_UdmfData_Destroy(data);
        }
    }
}

/// The types a drag event carries, as MIME types.
unsafe fn drag_types(drag: *mut ArkUI_DragEvent) -> Option<Vec<String>> {
    let mut count = 0i32;
    // SAFETY: the buffers are sized as the API requires and outlive the call.
    unsafe {
        OH_ArkUI_DragEvent_GetDataTypeCount(drag, &mut count);
        if !(0..=256).contains(&count) {
            return None;
        }
        let mut buffers: Vec<Vec<c_char>> = (0..count).map(|_| vec![0; 256]).collect();
        let mut names: Vec<*mut c_char> = buffers.iter_mut().map(|b| b.as_mut_ptr()).collect();
        if OH_ArkUI_DragEvent_GetDataTypes(drag, names.as_mut_ptr(), count, 256) != 0 {
            return None;
        }
        Some(
            buffers
                .iter()
                .map(|b| mime(&CStr::from_ptr(b.as_ptr()).to_string_lossy()))
                .collect(),
        )
    }
}

/// A hover (enter/move) or a drop on a target.
fn hover_or_drop(n: Handle, drag: *mut ArkUI_DragEvent, at: Point, is_drop: bool) {
    let local = ACTIVE.with(|a| a.get()) != 0;
    // SAFETY: a live drag event.
    let Some(types) = (unsafe { drag_types(drag) }) else {
        return;
    };
    let location = |types: Vec<String>| Location {
        local,
        position: at,
        types,
        allowed: vec![Operation::Copy],
    };
    let accepted = TARGETS
        .with(|t| t.get(n as usize))
        .is_some_and(|t| t.proposal(&location(types.clone())) == Operation::Copy);
    // SAFETY: a live node and drag event.
    unsafe {
        if accepted {
            OH_ArkUI_AllowNodeAllDropDataTypes(n);
            OH_ArkUI_DragEvent_SetSuggestedDropOperation(
                drag,
                ArkUI_DropOperation::ARKUI_DROP_OPERATION_COPY,
            );
        } else {
            OH_ArkUI_DisallowNodeAnyDropDataTypes(n);
        }
        if !is_drop {
            return;
        }
        OH_ArkUI_DragEvent_SetDragResult(drag, ArkUI_DragResult::FAILED);
        if !accepted {
            return;
        }
        let Some(offer) = read_offer(drag) else {
            return;
        };
        let delivered = TARGETS
            .with(|t| t.get(n as usize))
            .is_some_and(|t| t.deliver(location(offer.types()), offer));
        if delivered {
            OH_ArkUI_DragEvent_SetDragResult(drag, ArkUI_DragResult::SUCCESSFUL);
        }
    }
}

/// The offer a drop carries: the Day packet when a record has one, else one item assembled
/// from the records' native representations.
unsafe fn read_offer(drag: *mut ArkUI_DragEvent) -> Option<Offer> {
    // SAFETY: UDMF objects created and destroyed here; entries are read before the data is
    // destroyed.
    unsafe {
        let data = OH_UdmfData_Create();
        if data.is_null() {
            return None;
        }
        let mut offer = None;
        if OH_ArkUI_DragEvent_GetUdmfData(drag, data) == 0 {
            let mut count = 0u32;
            let records = OH_UdmfData_GetRecords(data, &mut count);
            let mut reps: Vec<Representation> = Vec::new();
            let mut total = 0usize;
            if !records.is_null() && count <= 256 {
                'records: for i in 0..count as usize {
                    let record = *records.add(i);
                    let mut p: *mut u8 = ptr::null_mut();
                    let mut len = 0u32;
                    if OH_UdmfRecord_GetGeneralEntry(record, BUNDLE.as_ptr(), &mut p, &mut len) == 0
                        && !p.is_null()
                        && len as usize <= LIMIT
                    {
                        offer = Offer::decode(std::slice::from_raw_parts(p, len as usize));
                        break;
                    }
                    let mut num = 0u32;
                    let types = OH_UdmfRecord_GetTypes(record, &mut num);
                    if types.is_null() || num > 32 {
                        continue;
                    }
                    for j in 0..num as usize {
                        let ty = *types.add(j);
                        if ty.is_null() {
                            continue;
                        }
                        let m = mime(&CStr::from_ptr(ty).to_string_lossy());
                        if m == "text/uri-list" {
                            let f = OH_UdsFileUri_Create();
                            if !f.is_null() {
                                if OH_UdmfRecord_GetFileUri(record, f) == 0 {
                                    let uri = OH_UdsFileUri_GetFileUri(f);
                                    if !uri.is_null() {
                                        let mut v =
                                            CStr::from_ptr(uri).to_string_lossy().into_owned();
                                        v.push_str("\r\n");
                                        reps.push(Representation::new(m, v.into_bytes()));
                                    }
                                }
                                OH_UdsFileUri_Destroy(f);
                            }
                        } else if OH_UdmfRecord_GetGeneralEntry(record, ty, &mut p, &mut len) == 0
                            && !p.is_null()
                            && len as usize <= LIMIT
                        {
                            total += len as usize;
                            if total > LIMIT {
                                break 'records;
                            }
                            reps.push(Representation::new(
                                m,
                                std::slice::from_raw_parts(p, len as usize).to_vec(),
                            ));
                        }
                    }
                }
            }
            if offer.is_none() && total <= LIMIT && !reps.is_empty() && reps.len() <= 256 {
                offer = Some(Offer {
                    items: vec![Item::new(reps)],
                });
            }
        }
        OH_UdmfData_Destroy(data);
        offer
    }
}
