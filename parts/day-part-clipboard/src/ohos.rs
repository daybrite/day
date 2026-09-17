// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// HarmonyOS / OpenHarmony: the native Pasteboard C API (`libpasteboard.so`, oh_pasteboard.h,
// API 13+) with content typed through UDMF (`libudmf.so`): a plain-text write is an
// OH_UdsPlainText inside an OH_UdmfRecord inside an OH_UdmfData handed to OH_Pasteboard_SetData;
// a read is OH_Pasteboard_GetData + OH_UdmfData_GetPrimaryPlainText. Pure FFI, like macOS/iOS;
// no ArkTS bridge or Day runtime needed (unlike Android's ClipboardManager, which rides
// day-android's JVM/Context). Reading the pasteboard needs no permission.

use core::ffi::{CStr, c_char, c_int};
use std::ffi::CString;

// Opaque native handles (all created/destroyed through the API below).
#[repr(C)]
struct OhPasteboard {
    _opaque: [u8; 0],
}
#[repr(C)]
struct OhUdmfData {
    _opaque: [u8; 0],
}
#[repr(C)]
struct OhUdmfRecord {
    _opaque: [u8; 0],
}
#[repr(C)]
struct OhUdsPlainText {
    _opaque: [u8; 0],
}

/// UDMF_META_PLAIN_TEXT (udmf_meta.h), the plain-text uniform type id.
const PLAIN_TEXT_TYPE: &CStr = c"general.plain-text";
/// ERR_OK / UDMF_E_OK: both APIs return 0 on success.
const OK: c_int = 0;

#[link(name = "pasteboard")]
unsafe extern "C" {
    fn OH_Pasteboard_Create() -> *mut OhPasteboard;
    fn OH_Pasteboard_Destroy(pasteboard: *mut OhPasteboard);
    fn OH_Pasteboard_HasType(pasteboard: *mut OhPasteboard, type_id: *const c_char) -> bool;
    fn OH_Pasteboard_GetData(pasteboard: *mut OhPasteboard, status: *mut c_int) -> *mut OhUdmfData;
    fn OH_Pasteboard_SetData(pasteboard: *mut OhPasteboard, data: *mut OhUdmfData) -> c_int;
}

#[link(name = "udmf")]
unsafe extern "C" {
    fn OH_UdmfData_Create() -> *mut OhUdmfData;
    fn OH_UdmfData_Destroy(data: *mut OhUdmfData);
    fn OH_UdmfData_AddRecord(data: *mut OhUdmfData, record: *mut OhUdmfRecord) -> c_int;
    fn OH_UdmfData_GetPrimaryPlainText(
        data: *mut OhUdmfData,
        plain_text: *mut OhUdsPlainText,
    ) -> c_int;
    fn OH_UdmfRecord_Create() -> *mut OhUdmfRecord;
    fn OH_UdmfRecord_Destroy(record: *mut OhUdmfRecord);
    fn OH_UdmfRecord_AddPlainText(
        record: *mut OhUdmfRecord,
        plain_text: *mut OhUdsPlainText,
    ) -> c_int;
    fn OH_UdsPlainText_Create() -> *mut OhUdsPlainText;
    fn OH_UdsPlainText_Destroy(plain_text: *mut OhUdsPlainText);
    fn OH_UdsPlainText_SetContent(plain_text: *mut OhUdsPlainText, content: *const c_char)
    -> c_int;
    fn OH_UdsPlainText_GetContent(plain_text: *mut OhUdsPlainText) -> *const c_char;
}

pub fn set_text(text: &str) -> bool {
    // Interior NULs can't cross the C boundary; treat as failure rather than truncating.
    let Ok(content) = CString::new(text) else {
        return false;
    };
    unsafe {
        let pb = OH_Pasteboard_Create();
        if pb.is_null() {
            return false;
        }
        // Build plain-text uds → record → data; each Add* copies/attaches, then everything is
        // destroyed locally (SetData serializes the data into the system pasteboard service).
        let uds = OH_UdsPlainText_Create();
        let record = OH_UdmfRecord_Create();
        let data = OH_UdmfData_Create();
        let ok = !uds.is_null()
            && !record.is_null()
            && !data.is_null()
            && OH_UdsPlainText_SetContent(uds, content.as_ptr()) == OK
            && OH_UdmfRecord_AddPlainText(record, uds) == OK
            && OH_UdmfData_AddRecord(data, record) == OK
            && OH_Pasteboard_SetData(pb, data) == OK;
        if !data.is_null() {
            OH_UdmfData_Destroy(data);
        }
        if !record.is_null() {
            OH_UdmfRecord_Destroy(record);
        }
        if !uds.is_null() {
            OH_UdsPlainText_Destroy(uds);
        }
        OH_Pasteboard_Destroy(pb);
        ok
    }
}

pub fn get_text() -> Option<String> {
    unsafe {
        let pb = OH_Pasteboard_Create();
        if pb.is_null() {
            return None;
        }
        let mut status: c_int = 0;
        let data = OH_Pasteboard_GetData(pb, &mut status);
        let mut out = None;
        if !data.is_null() {
            if status == OK {
                let uds = OH_UdsPlainText_Create();
                if !uds.is_null() {
                    if OH_UdmfData_GetPrimaryPlainText(data, uds) == OK {
                        // GetContent borrows from the uds object; copy before destroying it.
                        let content = OH_UdsPlainText_GetContent(uds);
                        if !content.is_null() {
                            out = Some(CStr::from_ptr(content).to_string_lossy().into_owned());
                        }
                    }
                    OH_UdsPlainText_Destroy(uds);
                }
            }
            OH_UdmfData_Destroy(data);
        }
        OH_Pasteboard_Destroy(pb);
        out
    }
}

pub fn has_text() -> bool {
    unsafe {
        let pb = OH_Pasteboard_Create();
        if pb.is_null() {
            return false;
        }
        let has = OH_Pasteboard_HasType(pb, PLAIN_TEXT_TYPE.as_ptr());
        OH_Pasteboard_Destroy(pb);
        has
    }
}

use crate::{Content, Error, MAX_BYTES, Representation};
#[link(name = "udmf")]
unsafe extern "C" {
    fn OH_UdmfRecord_AddGeneralEntry(
        record: *mut OhUdmfRecord,
        ty: *const c_char,
        bytes: *mut u8,
        len: u32,
    ) -> c_int;
    fn OH_UdmfRecord_GetGeneralEntry(
        record: *mut OhUdmfRecord,
        ty: *const c_char,
        bytes: *mut *mut u8,
        len: *mut u32,
    ) -> c_int;
    fn OH_UdmfData_GetRecords(data: *mut OhUdmfData, count: *mut u32) -> *mut *mut OhUdmfRecord;
}
fn type_id(m: &str) -> CString {
    CString::new(match m {
        "image/png" => "general.png",
        "image/jpeg" => "general.jpeg",
        "image/bmp" => "com.microsoft.bmp",
        "image/gif" => "com.compuserve.gif",
        "image/tiff" => "general.tiff",
        "image/svg+xml" => "general.svg",
        other => other,
    })
    .unwrap()
}
pub fn write_content(content: &Content) -> Result<Vec<String>, Error> {
    unsafe {
        let pb = OH_Pasteboard_Create();
        let data = OH_UdmfData_Create();
        let rec = OH_UdmfRecord_Create();
        let mut types = Vec::new();
        if !pb.is_null() && !data.is_null() && !rec.is_null() {
            for r in &content.0 {
                if r.mime == "text/plain" {
                    if let Ok(text) = CString::new(r.bytes.as_slice()) {
                        let uds = OH_UdsPlainText_Create();
                        if !uds.is_null() {
                            if OH_UdsPlainText_SetContent(uds, text.as_ptr()) == OK
                                && OH_UdmfRecord_AddPlainText(rec, uds) == OK
                            {
                                types.push(r.mime.clone());
                            }
                            OH_UdsPlainText_Destroy(uds);
                        }
                    }
                    continue;
                }
                if OH_UdmfRecord_AddGeneralEntry(
                    rec,
                    type_id(&r.mime).as_ptr(),
                    r.bytes.as_ptr().cast_mut(),
                    r.bytes.len() as u32,
                ) == OK
                {
                    types.push(r.mime.clone());
                }
            }
            if types.is_empty()
                || OH_UdmfData_AddRecord(data, rec) != OK
                || OH_Pasteboard_SetData(pb, data) != OK
            {
                types.clear();
            }
        }
        if !rec.is_null() {
            OH_UdmfRecord_Destroy(rec);
        }
        if !data.is_null() {
            OH_UdmfData_Destroy(data);
        }
        if !pb.is_null() {
            OH_Pasteboard_Destroy(pb);
        }
        if types.is_empty() {
            Err(Error::Unavailable)
        } else {
            Ok(types)
        }
    }
}
pub fn read_content(preferred: &[&str]) -> Result<Option<Representation>, Error> {
    unsafe {
        let pb = OH_Pasteboard_Create();
        if pb.is_null() {
            return Err(Error::Unavailable);
        }
        let mut status = 0;
        let data = OH_Pasteboard_GetData(pb, &mut status);
        let result = (|| {
            if data.is_null() {
                return if status == OK {
                    Ok(None)
                } else {
                    Err(Error::Unavailable)
                };
            }
            let mut count = 0;
            let records = OH_UdmfData_GetRecords(data, &mut count);
            if records.is_null() {
                return Ok(None);
            }
            for mime in preferred {
                if *mime == "text/plain" {
                    let uds = OH_UdsPlainText_Create();
                    if !uds.is_null() {
                        let value = if OH_UdmfData_GetPrimaryPlainText(data, uds) == OK {
                            let p = OH_UdsPlainText_GetContent(uds);
                            if p.is_null() {
                                None
                            } else {
                                Some(CStr::from_ptr(p).to_bytes().to_vec())
                            }
                        } else {
                            None
                        };
                        OH_UdsPlainText_Destroy(uds);
                        if let Some(bytes) = value {
                            if bytes.len() > MAX_BYTES {
                                return Err(Error::TooLarge);
                            }
                            return Ok(Some(Representation::new(*mime, bytes)));
                        }
                    }
                }

                for i in 0..count as usize {
                    let mut bytes = std::ptr::null_mut();
                    let mut len = 0;
                    if OH_UdmfRecord_GetGeneralEntry(
                        *records.add(i),
                        type_id(mime).as_ptr(),
                        &mut bytes,
                        &mut len,
                    ) == OK
                        && !bytes.is_null()
                    {
                        if len as usize > MAX_BYTES {
                            return Err(Error::TooLarge);
                        }
                        return Ok(Some(Representation::new(
                            *mime,
                            std::slice::from_raw_parts(bytes, len as usize).to_vec(),
                        )));
                    }
                }
            }
            Ok(None)
        })();
        if !data.is_null() {
            OH_UdmfData_Destroy(data);
        }
        OH_Pasteboard_Destroy(pb);
        result
    }
}
