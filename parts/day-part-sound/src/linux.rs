// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// Desktop Linux (GTK and Qt alike): libcanberra, the freedesktop event-sound library, which plays
// through whatever sound server the desktop runs (PulseAudio, PipeWire's Pulse layer, ALSA). It is
// loaded at RUN time rather than linked, the way day-part-speech loads speech-dispatcher: a linked
// library is a DT_NEEDED entry, and an app should start on a desktop without libcanberra and simply
// stay silent there (docs/bridge.md "Linking").
//
// libcanberra plays files, and bundled assets live inside the binary (GResource, Qt's .rcc), so each
// clip is written once to the user's cache directory and played from there. Preloading also asks
// the sound server to cache the sample, so a play starts without decoding the file again.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::{CString, c_char, c_int, c_void};
use std::path::PathBuf;

use super::AssetName;

type Create = unsafe extern "C" fn(*mut *mut c_void) -> c_int;
type Sets = unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> c_int;
type Destroy = unsafe extern "C" fn(*mut c_void) -> c_int;
type Finished = unsafe extern "C" fn(*mut c_void, u32, c_int, *mut c_void);
type PlayFull =
    unsafe extern "C" fn(*mut c_void, u32, *mut c_void, Option<Finished>, *mut c_void) -> c_int;
type CacheFull = unsafe extern "C" fn(*mut c_void, *mut c_void) -> c_int;

unsafe extern "C" {
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

const RTLD_LAZY: c_int = 1;

/// The few libcanberra entry points this arm uses, resolved from the loaded library.
struct Canberra {
    context: *mut c_void,
    proplist_create: Create,
    proplist_sets: Sets,
    proplist_destroy: Destroy,
    play_full: PlayFull,
    cache_full: CacheFull,
}

impl Canberra {
    fn load() -> Option<Canberra> {
        // SAFETY: dlopen/dlsym with NUL-terminated names; each symbol is cast to the signature
        // libcanberra's header declares for it, and a missing one abandons the whole engine.
        unsafe {
            let mut lib = dlopen(c"libcanberra.so.0".as_ptr(), RTLD_LAZY);
            if lib.is_null() {
                lib = dlopen(c"libcanberra.so".as_ptr(), RTLD_LAZY);
            }
            if lib.is_null() {
                return None;
            }
            let sym = |name: &std::ffi::CStr| {
                let p = dlsym(lib, name.as_ptr());
                (!p.is_null()).then_some(p)
            };
            let context_create: Create = std::mem::transmute(sym(c"ca_context_create")?);
            let mut ca = Canberra {
                context: std::ptr::null_mut(),
                proplist_create: std::mem::transmute::<*mut c_void, Create>(sym(
                    c"ca_proplist_create",
                )?),
                proplist_sets: std::mem::transmute::<*mut c_void, Sets>(sym(c"ca_proplist_sets")?),
                proplist_destroy: std::mem::transmute::<*mut c_void, Destroy>(sym(
                    c"ca_proplist_destroy",
                )?),
                play_full: std::mem::transmute::<*mut c_void, PlayFull>(sym(
                    c"ca_context_play_full",
                )?),
                cache_full: std::mem::transmute::<*mut c_void, CacheFull>(sym(
                    c"ca_context_cache_full",
                )?),
            };
            if context_create(&mut ca.context) != 0 || ca.context.is_null() {
                return None;
            }
            Some(ca)
        }
    }

    /// Run `f` with a property list holding `props`, then free it.
    fn with_props(&self, props: &[(&std::ffi::CStr, &CString)], f: impl FnOnce(*mut c_void)) {
        let mut list: *mut c_void = std::ptr::null_mut();
        // SAFETY: a proplist created, filled with NUL-terminated strings, used, and destroyed here.
        unsafe {
            if (self.proplist_create)(&mut list) != 0 || list.is_null() {
                return;
            }
            for (key, value) in props {
                (self.proplist_sets)(list, key.as_ptr(), value.as_ptr());
            }
            f(list);
            (self.proplist_destroy)(list);
        }
    }
}

struct Engine {
    ca: Canberra,
    /// Each clip's cached file and the event id the sound server knows it by.
    files: HashMap<String, (CString, CString)>,
    dir: PathBuf,
    next_id: u32,
}

thread_local! {
    static ENGINE: RefCell<Option<Engine>> = const { RefCell::new(None) };
    static LOOKED: Cell<bool> = const { Cell::new(false) };
}

/// `$XDG_CACHE_HOME/day-part-sound/<executable>`, where each clip is written.
fn cache_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(std::env::temp_dir);
    let app = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "app".into());
    base.join("day-part-sound").join(app)
}

impl Engine {
    /// The clip's cached file, written (or rewritten, when the bundled bytes changed size) first.
    fn file(&mut self, path: &str) -> Option<(CString, CString)> {
        if let Some(f) = self.files.get(path) {
            return Some(f.clone());
        }
        let Some(res) = day_spec::resource(AssetName::dynamic(path)) else {
            super::warn_once(path, "no such bundled asset");
            return None;
        };
        let bytes = res.as_slice();
        let file = self.dir.join(path);
        let fresh = std::fs::metadata(&file).is_ok_and(|m| m.len() == bytes.len() as u64);
        if !fresh {
            let written = file
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| std::fs::write(&file, bytes));
            if let Err(e) = written {
                super::warn_once(path, &format!("cannot cache it: {e}"));
                return None;
            }
        }
        let name = CString::new(file.to_string_lossy().into_owned()).ok()?;
        let id = CString::new(format!("day-part-sound-{path}")).ok()?;
        self.files
            .insert(path.to_owned(), (name.clone(), id.clone()));
        Some((name, id))
    }

    fn preload(&mut self, path: &str) {
        let Some((file, id)) = self.file(path) else {
            return;
        };
        let context = self.ca.context;
        let cache = self.ca.cache_full;
        self.ca.with_props(
            &[(c"event.id", &id), (c"media.filename", &file)],
            // SAFETY: the context and property list are live for the call.
            |props| unsafe {
                cache(context, props);
            },
        );
    }

    fn play(&mut self, path: &str, volume: f32) {
        let Some((file, id)) = self.file(path) else {
            return;
        };
        // libcanberra takes its volume in decibels.
        let Ok(db) = CString::new(format!("{:.2}", 20.0 * volume.max(1e-4).log10())) else {
            return;
        };
        let role = c"game".to_owned();
        let permanent = c"permanent".to_owned();
        self.next_id = self.next_id.wrapping_add(1);
        let (context, play, n) = (self.ca.context, self.ca.play_full, self.next_id);
        self.ca.with_props(
            &[
                (c"event.id", &id),
                (c"media.filename", &file),
                (c"media.role", &role),
                (c"canberra.volume", &db),
                (c"canberra.cache-control", &permanent),
            ],
            // SAFETY: the context and property list are live for the call; no callback.
            |props| unsafe {
                play(context, n, props, None, std::ptr::null_mut());
            },
        );
    }
}

fn with_engine(f: impl FnOnce(&mut Engine)) {
    ENGINE.with(|e| {
        let mut e = e.borrow_mut();
        if e.is_none() && !LOOKED.with(|l| l.replace(true)) {
            *e = Canberra::load().map(|ca| Engine {
                ca,
                files: HashMap::new(),
                dir: cache_dir(),
                next_id: 0,
            });
            if e.is_none() {
                log::info!(target: "day_part_sound", "no libcanberra; sound is off");
            }
        }
        if let Some(engine) = e.as_mut() {
            f(engine);
        }
    });
}

pub fn supported() -> bool {
    let mut ok = false;
    with_engine(|_| ok = true);
    ok
}

pub fn preload(path: &str) {
    with_engine(|e| e.preload(path));
}

pub fn play(path: &str, volume: f32, _pan: f32, _priority: i32) {
    with_engine(|e| e.play(path, volume));
}

pub fn unload_all() {
    // The sound server keeps its cache; forgetting the files here only means a later play checks
    // them again.
    ENGINE.with(|e| {
        if let Some(engine) = e.borrow_mut().as_mut() {
            engine.files.clear();
        }
    });
}
