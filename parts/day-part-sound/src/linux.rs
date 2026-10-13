// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// Desktop Linux (GTK and Qt alike): libcanberra, the freedesktop event-sound library, which plays
// through whatever sound server the desktop runs (PulseAudio, PipeWire's Pulse layer, ALSA). It is
// loaded at run time rather than linked, the way day-part-speech loads speech-dispatcher: a linked
// library is a DT_NEEDED entry, and an app should start on a desktop without libcanberra and stay
// silent there (docs/bridge.md "Linking").
//
// libcanberra plays files, and bundled assets live inside the binary (GResource, Qt's .rcc), so each
// clip is written once to the user's cache directory and played from there. Preloading also asks
// the sound server to cache the sample, so a play starts without decoding the file again.
//
// libcanberra plays a file from its start to its end and has no way to repeat one or to change a
// sound that is playing, so a loop is emulated, in one of two ways.
//
// Where libpulse's simple API can be loaded and reaches a server (PulseAudio, or PipeWire's Pulse
// layer: the same server libcanberra plays through on most desktops), the loop is a playback
// stream of its own, fed by a thread that writes the clip's samples round and round. Every
// repeat is sample-exact, and a new volume is applied to the samples as they are written.
//
// Elsewhere (an ALSA-only desktop) the clip is laid end to end into one long file
// (`super::unrolled`), inside which every repeat is sample-exact, and libcanberra streams that
// file. A helper thread asks for it again a moment before it runs out, and the two playings cross
// over the fades at the file's ends, which a steady tone can be heard dipping through. A new
// volume cancels the loop and starts it again.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::{CString, c_char, c_int, c_void};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use super::{AssetName, RATE, unrolled};

type Create = unsafe extern "C" fn(*mut *mut c_void) -> c_int;
type Sets = unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> c_int;
type Destroy = unsafe extern "C" fn(*mut c_void) -> c_int;
type Finished = unsafe extern "C" fn(*mut c_void, u32, c_int, *mut c_void);
type PlayFull =
    unsafe extern "C" fn(*mut c_void, u32, *mut c_void, Option<Finished>, *mut c_void) -> c_int;
type CacheFull = unsafe extern "C" fn(*mut c_void, *mut c_void) -> c_int;
type Cancel = unsafe extern "C" fn(*mut c_void, u32) -> c_int;

/// The id each playing is given, which is what cancels it. One counter for clips and loops.
static NEXT_ID: AtomicU32 = AtomicU32::new(1);

unsafe extern "C" {
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

const RTLD_LAZY: c_int = 1;

/// The few libcanberra entry points this arm uses, resolved from the loaded library.
#[derive(Clone, Copy)]
struct Canberra {
    context: *mut c_void,
    proplist_create: Create,
    proplist_sets: Sets,
    proplist_destroy: Destroy,
    play_full: PlayFull,
    cache_full: CacheFull,
    cancel: Cancel,
}

// SAFETY: the function pointers are plain code addresses, and libcanberra documents its context
// as thread-safe, which is what lets the loop thread play through the context the UI thread
// made.
unsafe impl Send for Canberra {}
// SAFETY: as above; nothing here is mutated after `load`.
unsafe impl Sync for Canberra {}

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
                cancel: std::mem::transmute::<*mut c_void, Cancel>(sym(c"ca_context_cancel")?),
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

// ---------------------------------------------------------------------------
// Loops through libpulse's simple API
// ---------------------------------------------------------------------------

type SimpleNew = unsafe extern "C" fn(
    *const c_char,
    *const c_char,
    c_int,
    *const c_char,
    *const c_char,
    *const SampleSpec,
    *const c_void,
    *const BufferAttr,
    *mut c_int,
) -> *mut c_void;
type SimpleWrite = unsafe extern "C" fn(*mut c_void, *const c_void, usize, *mut c_int) -> c_int;
type SimpleFlush = unsafe extern "C" fn(*mut c_void, *mut c_int) -> c_int;
type SimpleFree = unsafe extern "C" fn(*mut c_void);

/// `pa_sample_spec`.
#[repr(C)]
struct SampleSpec {
    format: c_int,
    rate: u32,
    channels: u8,
}

/// `pa_buffer_attr`. `u32::MAX` leaves a field to the server.
#[repr(C)]
struct BufferAttr {
    maxlength: u32,
    tlength: u32,
    prebuf: u32,
    minreq: u32,
    fragsize: u32,
}

/// `PA_SAMPLE_S16LE` and `PA_STREAM_PLAYBACK`.
const SAMPLE_S16LE: c_int = 3;
const STREAM_PLAYBACK: c_int = 1;

/// The loop thread writes this many frames at a time (20 ms), and the stream holds about five
/// writes (100 ms): short enough that a new volume or a stop is heard at once, long enough that
/// a late thread does not run it dry.
const CHUNK: usize = RATE as usize / 50;
const STREAM_BYTES: u32 = (RATE / 10) * 2;

/// libpulse-simple's entry points, resolved from the loaded library. Function pointers only, so
/// the loop threads can hold a copy.
#[derive(Clone, Copy)]
struct Pulse {
    new: SimpleNew,
    write: SimpleWrite,
    flush: SimpleFlush,
    free: SimpleFree,
}

impl Pulse {
    fn load() -> Option<Pulse> {
        // SAFETY: dlopen/dlsym with NUL-terminated names; each symbol is cast to the signature
        // `pulse/simple.h` declares for it, and a missing one means no stream loops.
        unsafe {
            let lib = dlopen(c"libpulse-simple.so.0".as_ptr(), RTLD_LAZY);
            if lib.is_null() {
                return None;
            }
            let sym = |name: &std::ffi::CStr| {
                let p = dlsym(lib, name.as_ptr());
                (!p.is_null()).then_some(p)
            };
            Some(Pulse {
                new: std::mem::transmute::<*mut c_void, SimpleNew>(sym(c"pa_simple_new")?),
                write: std::mem::transmute::<*mut c_void, SimpleWrite>(sym(c"pa_simple_write")?),
                flush: std::mem::transmute::<*mut c_void, SimpleFlush>(sym(c"pa_simple_flush")?),
                free: std::mem::transmute::<*mut c_void, SimpleFree>(sym(c"pa_simple_free")?),
            })
        }
    }

    /// Open a mono playback stream at [`RATE`], or `None` when no server answers.
    fn open(&self, app: &CString) -> Option<Stream> {
        let spec = SampleSpec {
            format: SAMPLE_S16LE,
            rate: RATE,
            channels: 1,
        };
        let attr = BufferAttr {
            maxlength: u32::MAX,
            tlength: STREAM_BYTES,
            prebuf: u32::MAX,
            minreq: u32::MAX,
            fragsize: u32::MAX,
        };
        // SAFETY: every pointer is to a live local or a NUL-terminated string, and null is the
        // documented "default" for the server, the device, the channel map and the error slot.
        let stream = unsafe {
            (self.new)(
                std::ptr::null(),
                app.as_ptr(),
                STREAM_PLAYBACK,
                std::ptr::null(),
                c"loop".as_ptr(),
                &spec,
                std::ptr::null(),
                &attr,
                std::ptr::null_mut(),
            )
        };
        (!stream.is_null()).then_some(Stream(stream))
    }
}

/// A `pa_simple` connection, made on the UI thread and handed to the one thread that feeds it.
struct Stream(*mut c_void);

// SAFETY: a `pa_simple` may be used from any one thread at a time; this one is moved into its
// loop thread before its first write and never touched from anywhere else.
unsafe impl Send for Stream {}

/// What the UI thread tells a stream loop's thread.
struct Feed {
    stop: AtomicBool,
    /// The volume, as an `f32`'s bits.
    volume: AtomicU32,
}

impl Feed {
    fn volume(&self) -> f32 {
        f32::from_bits(self.volume.load(Ordering::Relaxed))
    }
}

/// Write `samples` round and round into `stream` until told to stop, at the volume `feed` holds,
/// easing to a new volume across one write so it does not step.
fn feed(pulse: Pulse, stream: Stream, samples: Arc<Vec<i16>>, feed: Arc<Feed>) {
    let mut at = 0;
    let mut gain = feed.volume();
    let mut chunk = [0i16; CHUNK];
    while !feed.stop.load(Ordering::Relaxed) {
        let want = feed.volume();
        for (i, out) in chunk.iter_mut().enumerate() {
            let g = gain + (want - gain) * (i as f32 / CHUNK as f32);
            *out = (f32::from(samples[at]) * g) as i16;
            at += 1;
            if at == samples.len() {
                at = 0;
            }
        }
        gain = want;
        // SAFETY: the stream is this thread's, and the chunk is `CHUNK` live 16-bit samples.
        // The write blocks while the stream is full, which is what paces this loop.
        let sent = unsafe {
            (pulse.write)(
                stream.0,
                chunk.as_ptr().cast(),
                CHUNK * 2,
                std::ptr::null_mut(),
            )
        };
        if sent < 0 {
            // The server went away; there is nothing left to feed.
            break;
        }
    }
    // SAFETY: dropping what the stream still holds, then closing it, on its own thread.
    unsafe {
        (pulse.flush)(stream.0, std::ptr::null_mut());
        (pulse.free)(stream.0);
    }
}

// ---------------------------------------------------------------------------
// Loops through libcanberra, for a desktop without libpulse
// ---------------------------------------------------------------------------

/// libcanberra takes its volume in decibels.
fn decibels(volume: f32) -> Option<CString> {
    CString::new(format!("{:.2}", 20.0 * volume.max(1e-4).log10())).ok()
}

/// One emulated loop: the long file of repeats, and the playings of it that may be sounding.
struct Emulated {
    id: i32,
    file: CString,
    event: CString,
    volume: CString,
    /// How long the file runs, less the crossover: the gap between one playing and the next.
    period: Duration,
    /// When the file is asked for again.
    next_at: Instant,
    /// The playing just started and the one before it, which is still fading out.
    playing: [u32; 2],
}

/// What the UI thread and the loop thread share: the loops, and a way to wake the thread when
/// they change.
struct Shared {
    ca: Canberra,
    loops: Mutex<Vec<Emulated>>,
    wake: Condvar,
}

impl Shared {
    fn loops(&self) -> MutexGuard<'_, Vec<Emulated>> {
        // Nothing under the lock can leave the list half-written, so a poisoned lock is usable.
        self.loops.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Start the file from its top, and note when it will next be wanted.
    fn sound(&self, l: &mut Emulated) {
        let role = c"game".to_owned();
        // A file this long is streamed from disk rather than uploaded to the server's cache.
        let never = c"never".to_owned();
        let n = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let (context, play) = (self.ca.context, self.ca.play_full);
        self.ca.with_props(
            &[
                (c"event.id", &l.event),
                (c"media.filename", &l.file),
                (c"media.role", &role),
                (c"canberra.volume", &l.volume),
                (c"canberra.cache-control", &never),
            ],
            // SAFETY: the context and property list are live for the call; no callback.
            |props| unsafe {
                play(context, n, props, None, std::ptr::null_mut());
            },
        );
        l.playing = [n, l.playing[0]];
        l.next_at = Instant::now() + l.period;
    }

    /// Cut off every playing of `l`.
    fn silence(&self, l: &Emulated) {
        for n in l.playing {
            if n != 0 {
                // SAFETY: cancelling by id on the live context; an id no longer playing is a
                // no-op.
                unsafe { (self.ca.cancel)(self.ca.context, n) };
            }
        }
    }

    /// The loop thread: sleep until the earliest file runs out, play it again, and sleep on. With
    /// no loops it waits to be woken, so an app that never loops never has this thread at all and
    /// one that stopped looping has it parked.
    fn run(&self) {
        let mut loops = self.loops();
        loop {
            let now = Instant::now();
            for l in loops.iter_mut() {
                if l.next_at <= now {
                    self.sound(l);
                }
            }
            loops = match loops.iter().map(|l| l.next_at).min() {
                Some(at) => {
                    let wait = at.saturating_duration_since(Instant::now());
                    self.wake
                        .wait_timeout(loops, wait)
                        .map(|(guard, _)| guard)
                        .unwrap_or_else(|e| e.into_inner().0)
                }
                None => self.wake.wait(loops).unwrap_or_else(|e| e.into_inner()),
            };
        }
    }
}

struct Engine {
    ca: Canberra,
    /// Each clip's cached file and the event id the sound server knows it by.
    files: HashMap<String, (CString, CString)>,
    /// libpulse-simple, looked for when the first loop starts: `Some(None)` is "not installed".
    pulse: Option<Option<Pulse>>,
    /// The loops playing as streams of their own, by id.
    streams: HashMap<i32, Arc<Feed>>,
    /// Each looped clip's samples, for the streams.
    samples: HashMap<String, Arc<Vec<i16>>>,
    /// Each looped clip's file of repeats and how long that file runs, for the fallback.
    unrolled: HashMap<String, (CString, Duration)>,
    dir: PathBuf,
    /// The fallback's loops and their thread, made when the first of them starts.
    shared: Option<Arc<Shared>>,
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

    /// The clip's file of repeats (`super::unrolled`), written beside its plain file the first
    /// time it loops, with how long it runs and how long its fades are.
    fn unrolled(&mut self, path: &str) -> Option<(CString, Duration)> {
        if let Some(f) = self.unrolled.get(path) {
            return Some(f.clone());
        }
        let samples = super::load(path)?;
        let (bytes, frames, edge) = unrolled::file(&samples, RATE)?;
        let file = self.dir.join(format!("{path}.loop.wav"));
        let fresh = std::fs::metadata(&file).is_ok_and(|m| m.len() == bytes.len() as u64);
        if !fresh {
            let written = file
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| std::fs::write(&file, &bytes));
            if let Err(e) = written {
                super::warn_once(path, &format!("cannot cache its loop: {e}"));
                return None;
            }
        }
        let name = CString::new(file.to_string_lossy().into_owned()).ok()?;
        // The next playing starts one fade before this one ends.
        let period = Duration::from_secs_f64((frames - edge) as f64 / RATE as f64);
        self.unrolled
            .insert(path.to_owned(), (name.clone(), period));
        Some((name, period))
    }

    /// The shared loop state, starting its thread the first time a loop needs it.
    fn shared(&mut self) -> Option<Arc<Shared>> {
        if self.shared.is_none() {
            let shared = Arc::new(Shared {
                ca: self.ca,
                loops: Mutex::new(Vec::new()),
                wake: Condvar::new(),
            });
            let theirs = shared.clone();
            std::thread::Builder::new()
                .name("day-part-sound-loops".into())
                .spawn(move || theirs.run())
                .ok()?;
            self.shared = Some(shared);
        }
        self.shared.clone()
    }

    /// Start loop `id` as a stream of its own. False when libpulse is not installed, no server
    /// answers, or the clip cannot be read, so the caller falls back to libcanberra.
    fn stream_start(&mut self, id: i32, path: &str, volume: f32) -> bool {
        let Some(pulse) = *self.pulse.get_or_insert_with(Pulse::load) else {
            return false;
        };
        let samples = match self.samples.get(path) {
            Some(s) => s.clone(),
            None => {
                let Some(loaded) = super::load(path).filter(|s| !s.is_empty()) else {
                    return false;
                };
                let samples: Arc<Vec<i16>> =
                    Arc::new(loaded.iter().map(|s| (s * 32_767.0) as i16).collect());
                self.samples.insert(path.to_owned(), samples.clone());
                samples
            }
        };
        let app = std::env::current_exe()
            .ok()
            .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
            .and_then(|s| CString::new(s).ok())
            .unwrap_or_else(|| c"day".to_owned());
        let Some(stream) = pulse.open(&app) else {
            return false;
        };
        let ctl = Arc::new(Feed {
            stop: AtomicBool::new(false),
            volume: AtomicU32::new(volume.to_bits()),
        });
        let theirs = ctl.clone();
        let spawned = std::thread::Builder::new()
            .name("day-part-sound-loop".into())
            .spawn(move || feed(pulse, stream, samples, theirs));
        if spawned.is_err() {
            // The closure, and the stream inside it, were dropped with the error; the server
            // closes the connection when the process ends.
            return false;
        }
        self.streams.insert(id, ctl);
        true
    }

    fn loop_start(&mut self, id: i32, path: &str, volume: f32) {
        if self.stream_start(id, path, volume) {
            return;
        }
        let Some((file, period)) = self.unrolled(path) else {
            return;
        };
        let (Some(volume), Ok(event), Some(shared)) = (
            decibels(volume),
            CString::new(format!("day-part-sound-loop-{path}")),
            self.shared(),
        ) else {
            return;
        };
        let mut l = Emulated {
            id,
            file,
            event,
            volume,
            period,
            next_at: Instant::now(),
            playing: [0, 0],
        };
        shared.sound(&mut l);
        shared.loops().push(l);
        // The thread may be sleeping toward a later deadline, or waiting for a first loop.
        shared.wake.notify_one();
    }

    fn loop_set(&mut self, id: i32, volume: f32) {
        if let Some(feed) = self.streams.get(&id) {
            feed.volume.store(volume.to_bits(), Ordering::Relaxed);
            return;
        }
        let (Some(shared), Some(volume)) = (self.shared.clone(), decibels(volume)) else {
            return;
        };
        let mut loops = shared.loops();
        if let Some(l) = loops.iter_mut().find(|l| l.id == id) {
            // No call changes a playing sound's volume: cut it off and start it at the new one.
            shared.silence(l);
            l.volume = volume;
            l.playing = [0, 0];
            shared.sound(l);
        }
        drop(loops);
        shared.wake.notify_one();
    }

    fn loop_stop(&mut self, id: i32) {
        if let Some(feed) = self.streams.remove(&id) {
            // Its thread sees this within one write, flushes the stream and closes it.
            feed.stop.store(true, Ordering::Relaxed);
            return;
        }
        let Some(shared) = self.shared.clone() else {
            return;
        };
        let mut loops = shared.loops();
        if let Some(i) = loops.iter().position(|l| l.id == id) {
            let l = loops.remove(i);
            shared.silence(&l);
        }
    }

    fn loops_stop(&mut self) {
        for (_, feed) in self.streams.drain() {
            feed.stop.store(true, Ordering::Relaxed);
        }
        if let Some(shared) = self.shared.clone() {
            for l in shared.loops().drain(..) {
                shared.silence(&l);
            }
        }
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
        let Some(db) = decibels(volume) else {
            return;
        };
        let role = c"game".to_owned();
        let permanent = c"permanent".to_owned();
        let n = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let (context, play) = (self.ca.context, self.ca.play_full);
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
                pulse: None,
                streams: HashMap::new(),
                samples: HashMap::new(),
                unrolled: HashMap::new(),
                dir: cache_dir(),
                shared: None,
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

pub fn loop_start(id: i32, path: &str, volume: f32, _pan: f32) {
    with_engine(|e| e.loop_start(id, path, volume));
}

pub fn loop_set(id: i32, volume: f32, _pan: f32) {
    with_engine(|e| e.loop_set(id, volume));
}

pub fn loop_stop(id: i32) {
    // Reached from `Loop`'s destructor, which may run while the thread and this thread-local are
    // being torn down: `try_with`, and never look for libcanberra just to stop something.
    let _ = ENGINE.try_with(|e| {
        if let Ok(mut e) = e.try_borrow_mut()
            && let Some(engine) = e.as_mut()
        {
            engine.loop_stop(id);
        }
    });
}

pub fn unload_all() {
    // The sound server keeps its cache; forgetting the files here only means a later play checks
    // them again.
    ENGINE.with(|e| {
        if let Some(engine) = e.borrow_mut().as_mut() {
            engine.loops_stop();
            engine.files.clear();
            engine.samples.clear();
            engine.unrolled.clear();
        }
    });
}
