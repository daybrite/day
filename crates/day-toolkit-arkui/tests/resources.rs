// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
//! Run the actual resource reader against a small NDK fake on Unix CI hosts.
#![cfg(unix)]
#![allow(non_snake_case, non_camel_case_types)]
extern crate self as napi_ohos;
extern crate self as ohos_sys;

pub mod sys {
    pub type napi_env = *mut std::ffi::c_void;
    pub type napi_value = *mut std::ffi::c_void;
}
mod node {
    pub fn cstr(s: &str) -> std::ffi::CString {
        std::ffi::CString::new(s).unwrap()
    }
}
pub mod rawfile {
    use std::ffi::{CStr, c_char, c_void};
    pub struct RawFileDescriptor {
        pub fd: i32,
        pub start: i64,
        pub length: i64,
    }
    pub mod raw_file_manager {
        use super::*;
        pub struct NativeResourceManager(pub u8);
        pub unsafe fn OH_ResourceManager_InitNativeResourceManager(
            _: *mut c_void,
            value: *mut c_void,
        ) -> *mut NativeResourceManager {
            Box::into_raw(Box::new(NativeResourceManager(value as usize as u8)))
        }
        pub unsafe fn OH_ResourceManager_ReleaseNativeResourceManager(
            p: *mut NativeResourceManager,
        ) {
            unsafe { drop(Box::from_raw(p)) };
        }
        pub unsafe fn OH_ResourceManager_OpenRawFile(
            manager: *mut NativeResourceManager,
            path: *const c_char,
        ) -> *mut u8 {
            if unsafe { CStr::from_ptr(path) }.to_bytes() != b"day/fixture.bin" {
                return std::ptr::null_mut();
            }
            Box::into_raw(Box::new(unsafe { (*manager).0 }))
        }
    }
    pub mod raw_file {
        use super::*;
        pub unsafe fn OH_ResourceManager_CloseRawFile(p: *mut u8) {
            unsafe { drop(Box::from_raw(p)) };
        }
        pub unsafe fn OH_ResourceManager_GetRawFileDescriptorData(
            _: *mut u8,
            _: *mut RawFileDescriptor,
        ) -> bool {
            false
        }
        pub unsafe fn OH_ResourceManager_ReleaseRawFileDescriptorData(_: *const RawFileDescriptor) {
        }
        pub unsafe fn OH_ResourceManager_GetRawFileSize(_: *mut u8) -> i64 {
            1
        }
        pub unsafe fn OH_ResourceManager_ReadRawFile(
            p: *mut u8,
            out: *mut c_void,
            _: usize,
        ) -> i32 {
            unsafe {
                *out.cast::<u8>() = *p;
            }
            1
        }
    }
}
#[path = "../src/resources.rs"]
mod resources;

#[test]
fn native_resources_are_readable_on_workers_and_survive_manager_replacement() {
    unsafe { resources::register(std::ptr::null_mut(), 7usize as *mut _) };
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (resume_tx, resume_rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        assert!(resources::available());
        assert!(resources::rawfile_exists("day/fixture.bin"));
        assert!(!resources::rawfile_exists("day/missing.bin"));
        let first = resources::open("day/fixture.bin").unwrap();
        assert!(resources::open("day/missing.bin").is_none());
        ready_tx.send(()).unwrap();
        resume_rx.recv().unwrap();
        // Keep the actual returned resource alive across replacement on the host thread.
        let (ptr, len) = first.as_ptr_len();
        assert_eq!(unsafe { std::slice::from_raw_parts(ptr, len) }, &[7]);
        let next = resources::open("day/fixture.bin").unwrap();
        let (ptr, len) = next.as_ptr_len();
        assert_eq!(unsafe { std::slice::from_raw_parts(ptr, len) }, &[9]);
    });
    ready_rx.recv().unwrap();
    unsafe { resources::register(std::ptr::null_mut(), 9usize as *mut _) };
    resume_tx.send(()).unwrap();
    worker.join().unwrap();
}
