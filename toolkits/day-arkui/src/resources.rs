// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Bundled data resources (§18.3) from the app's rawfile store, through the NDK's
//! `OH_ResourceManager_*` API (`ohos_sys::rawfile`).
//!
//! The native resource manager can only be made from the ArkTS `resourceManager` object, so the
//! host hands it over once (`registerResourceManager`, src/host_api.rs); until then nothing
//! here can read, and `resource(name)` answers `None`.

use std::ffi::c_void;
use std::ptr;
use std::sync::Mutex;

use ohos_sys::rawfile::RawFileDescriptor;
use ohos_sys::rawfile::raw_file::{
    OH_ResourceManager_CloseRawFile, OH_ResourceManager_GetRawFileDescriptorData,
    OH_ResourceManager_GetRawFileSize, OH_ResourceManager_ReadRawFile,
    OH_ResourceManager_ReleaseRawFileDescriptorData,
};
use ohos_sys::rawfile::raw_file_manager::{
    NativeResourceManager, OH_ResourceManager_InitNativeResourceManager,
    OH_ResourceManager_OpenRawFile, OH_ResourceManager_ReleaseNativeResourceManager,
};

// Initialization uses NAPI on the host thread; the resulting NDK manager owns a native
// shared ResourceManager, not a NAPI value. Native reads may run on resource-provider
// workers. Serialize each complete read against replacement/release of the manager.
struct Manager(*mut NativeResourceManager);
// SAFETY: no NAPI operations occur through this pointer after initialization, and every
// NDK access (including release) holds MANAGER's mutex. RawFile handles never escape it.
unsafe impl Send for Manager {}
static MANAGER: Mutex<Manager> = Mutex::new(Manager(ptr::null_mut()));

/// Take the ArkTS resource manager (a NAPI value) and keep its native handle for the process.
///
/// # Safety
/// `env` and `value` are the live NAPI environment and the `resourceManager` object.
pub unsafe fn register(env: napi_ohos::sys::napi_env, value: napi_ohos::sys::napi_value) {
    // SAFETY: per the caller's contract; NAPI initialization stays on the host thread.
    let mgr = unsafe { OH_ResourceManager_InitNativeResourceManager(env.cast(), value.cast()) };
    let mut manager = MANAGER.lock().unwrap();
    if !manager.0.is_null() {
        // SAFETY: no reader can still be using the old manager while this lock is held.
        unsafe { OH_ResourceManager_ReleaseNativeResourceManager(manager.0) };
    }
    manager.0 = mgr;
}

pub fn available() -> bool {
    !MANAGER.lock().unwrap().0.is_null()
}

/// Whether rawfile `path` (e.g. `"day/home.svg"`) exists in the app package; false before the
/// entry ability registers the resource manager (docs/vectors.md).
pub fn rawfile_exists(path: &str) -> bool {
    let manager = MANAGER.lock().unwrap();
    let mgr = manager.0;
    if mgr.is_null() {
        return false;
    }
    let path = crate::node::cstr(path);
    // SAFETY: a live manager and a valid C string; the file is closed before returning.
    unsafe {
        let f = OH_ResourceManager_OpenRawFile(mgr, path.as_ptr());
        if f.is_null() {
            return false;
        }
        OH_ResourceManager_CloseRawFile(f);
        true
    }
}

/// One opened rawfile's bytes: a zero-copy mmap of the uncompressed `.hap` entry where the
/// descriptor allows it, else a heap copy. Unmapped or freed on drop.
pub enum Mapped {
    Mmap {
        base: *mut c_void,
        len: usize,
        offset: usize,
    },
    Heap(Vec<u8>),
}

impl Mapped {
    pub fn as_ptr_len(&self) -> (*const u8, usize) {
        match self {
            // SAFETY: the mapping is `len` bytes from `base`, and the entry starts `offset` in.
            Mapped::Mmap { base, len, offset } => {
                (unsafe { base.cast::<u8>().add(*offset) }, len - offset)
            }
            Mapped::Heap(v) => (v.as_ptr(), v.len()),
        }
    }
}

impl Drop for Mapped {
    fn drop(&mut self) {
        if let Mapped::Mmap { base, len, .. } = self {
            // SAFETY: a mapping this module created with exactly these arguments.
            unsafe { libc::munmap(*base, *len) };
        }
    }
}

/// Open rawfile `path` (e.g. `"day/numbers.bin"`, relative to the rawfile root).
///
/// The CLI stages resources uncompressed, so the entry has a real fd/offset/length inside the
/// `.hap` to mmap (the offset need not be page-aligned, so the mapping starts at the page below
/// and the view is biased). If the descriptor or the mapping is unavailable, the whole file is
/// read into a heap buffer instead.
pub fn open(path: &str) -> Option<Mapped> {
    let manager = MANAGER.lock().unwrap();
    let mgr = manager.0;
    if mgr.is_null() {
        return None;
    }
    let cpath = crate::node::cstr(path);
    // SAFETY: a live manager, a valid C string, and rawfile handles closed on every path.
    unsafe {
        let mut rf = OH_ResourceManager_OpenRawFile(mgr, cpath.as_ptr());
        if rf.is_null() {
            return None;
        }
        let mut fd = RawFileDescriptor {
            fd: -1,
            start: 0,
            length: 0,
        };
        let have_fd = OH_ResourceManager_GetRawFileDescriptorData(rf, &mut fd);
        if have_fd && fd.fd >= 0 && fd.length > 0 {
            let page = libc::sysconf(libc::_SC_PAGESIZE);
            let misalign = if page > 0 { fd.start % page } else { 0 };
            let map_len = (fd.length + misalign) as usize;
            let base = libc::mmap(
                ptr::null_mut(),
                map_len,
                libc::PROT_READ,
                libc::MAP_PRIVATE,
                fd.fd,
                (fd.start - misalign) as libc::off_t,
            );
            // The descriptor owns a dup'd fd; release it: the mapping survives the close.
            OH_ResourceManager_ReleaseRawFileDescriptorData(&fd);
            OH_ResourceManager_CloseRawFile(rf);
            if base != libc::MAP_FAILED {
                return Some(Mapped::Mmap {
                    base,
                    len: map_len,
                    offset: misalign as usize,
                });
            }
            // The mapping failed: reopen and fall through to the copy.
            rf = OH_ResourceManager_OpenRawFile(mgr, cpath.as_ptr());
            if rf.is_null() {
                return None;
            }
        } else if have_fd {
            // A descriptor obtained but unusable: release it so the dup'd fd isn't leaked.
            OH_ResourceManager_ReleaseRawFileDescriptorData(&fd);
        }
        let size = OH_ResourceManager_GetRawFileSize(rf);
        if size <= 0 {
            OH_ResourceManager_CloseRawFile(rf);
            return None;
        }
        let mut buf = vec![0u8; size as usize];
        let read = OH_ResourceManager_ReadRawFile(rf, buf.as_mut_ptr().cast(), buf.len());
        OH_ResourceManager_CloseRawFile(rf);
        if read <= 0 {
            return None;
        }
        buf.truncate(read as usize);
        Some(Mapped::Heap(buf))
    }
}
