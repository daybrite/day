// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// Desktop Linux (GTK and Qt alike): libpulse, the client library of the PulseAudio sound server.
// PipeWire speaks the same protocol through its Pulse layer, so one library reaches the server on
// every current desktop. It is loaded at run time rather than linked, the way day-part-speech
// loads speech-dispatcher: a linked library is a DT_NEEDED entry, and an app should start on a
// machine without libpulse and stay silent there (docs/bridge.md "Linking").
//
// libpulse's simple API gives a stream to write samples into and nothing more, so the part mixes
// in software (`super::mixer`). One thread owns one stereo stream. Every clip and every loop
// sounding is a voice in the mix; the thread asks the mix for the next 10 ms of all of them
// summed, each at its own volume and pan, and writes that. The write blocks while the stream is
// full, which paces the thread. A loop is a voice that wraps from its last sample to its first,
// so it repeats sample-exactly, and a new volume is eased in as the samples are mixed.
//
// The stream is open only while something sounds, and for a moment after: a quiet app holds no
// connection, and the sound server can let the device sleep.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::{CString, c_char, c_int, c_void};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use super::RATE;
use super::mixer::{Mix, gains};

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

unsafe extern "C" {
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

const RTLD_LAZY: c_int = 1;

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

/// The thread mixes and writes this many frames at a time (10 ms), and the stream holds five of
/// them (50 ms). A clip is heard within about that of being played, a stopped loop is silent
/// within about that, and a thread that runs a little late does not run the stream dry.
const CHUNK: usize = RATE as usize / 100;
/// Stereo, 16-bit: four bytes a frame.
const STREAM_BYTES: u32 = (RATE / 20) * 4;

/// With nothing left to play the thread writes silence for this many chunks (two seconds) before
/// closing the stream, so a run of taps shares one connection rather than opening one each.
const LINGER: usize = 200;

/// How long to wait before looking for the server again, when it did not answer and a loop is
/// still wanted.
const RETRY: Duration = Duration::from_secs(3);

/// libpulse-simple's entry points, resolved from the loaded library. Function pointers only, so
/// the mixing thread can hold a copy.
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
        // `pulse/simple.h` declares for it, and a missing one abandons the whole engine.
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
}

/// A `pa_simple` playback stream: stereo, 16-bit, at [`RATE`]. Made, written and closed by the
/// mixing thread alone.
struct Stream {
    pulse: Pulse,
    raw: *mut c_void,
}

impl Stream {
    /// Connect to the server, or `None` when none answers.
    fn open(pulse: Pulse, app: &CString) -> Option<Stream> {
        let spec = SampleSpec {
            format: SAMPLE_S16LE,
            rate: RATE,
            channels: 2,
        };
        let attr = BufferAttr {
            maxlength: u32::MAX,
            tlength: STREAM_BYTES,
            prebuf: u32::MAX,
            minreq: u32::MAX,
            fragsize: u32::MAX,
        };
        let mut error: c_int = 0;
        // SAFETY: every pointer is to a live local or a NUL-terminated string, and null is the
        // documented "default" for the server, the device and the channel map.
        let raw = unsafe {
            (pulse.new)(
                std::ptr::null(),
                app.as_ptr(),
                STREAM_PLAYBACK,
                std::ptr::null(),
                c"sound effects".as_ptr(),
                &spec,
                std::ptr::null(),
                &attr,
                &mut error,
            )
        };
        if raw.is_null() {
            // Not `then_some`: that would build a `Stream` around the null and free it.
            log::debug!(target: "day_part_sound", "no sound server answered (libpulse error {error})");
            return None;
        }
        Some(Stream { pulse, raw })
    }

    /// Hand the server `frames` of interleaved stereo, waiting while the stream is full. False
    /// when the server has gone away.
    fn write(&self, frames: &[i16]) -> bool {
        // SAFETY: the stream is live and the slice is that many bytes of initialized samples.
        let sent = unsafe {
            (self.pulse.write)(
                self.raw,
                frames.as_ptr().cast(),
                std::mem::size_of_val(frames),
                std::ptr::null_mut(),
            )
        };
        sent >= 0
    }

    /// Drop what the stream holds and has not played yet.
    fn flush(&self) {
        // SAFETY: flushing a live stream.
        unsafe { (self.pulse.flush)(self.raw, std::ptr::null_mut()) };
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        // SAFETY: the stream is live until here, and nothing uses it afterwards.
        unsafe { (self.pulse.free)(self.raw) };
    }
}

/// What the UI thread and the mixing thread share.
struct Mixer {
    pulse: Pulse,
    /// The name the sound server lists the stream under: the executable's.
    app: CString,
    state: Mutex<State>,
    wake: Condvar,
}

#[derive(Default)]
struct State {
    mix: Mix,
    /// Drop what the stream has buffered, because everything in it was just unloaded.
    flush: bool,
}

impl Mixer {
    fn state(&self) -> MutexGuard<'_, State> {
        // Nothing under the lock can leave the mix half-written, so a poisoned lock is usable.
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The mixing thread. It sleeps while there is nothing to play and no stream open; otherwise
    /// it renders 10 ms and writes it, opening the stream when a sound arrives and closing it
    /// once the mix has been empty for a while.
    fn run(&self) {
        let mut stream: Option<Stream> = None;
        let mut quiet = 0;
        let mut chunk = [0i16; CHUNK * 2];
        loop {
            let flush = {
                let mut state = self.state();
                while state.mix.voices.is_empty() && stream.is_none() {
                    state = self.wake.wait(state).unwrap_or_else(|e| e.into_inner());
                }
                quiet = if state.mix.voices.is_empty() {
                    quiet + 1
                } else {
                    0
                };
                state.mix.render(&mut chunk);
                std::mem::take(&mut state.flush)
            };
            if stream.is_none() {
                stream = Stream::open(self.pulse, &self.app);
            }
            let Some(open) = &stream else {
                // No server answered. A clip that could not sound is dropped; a loop is still
                // wanted, so look for the server again in a while, or when something else plays.
                let mut state = self.state();
                state.mix.voices.retain(|v| v.looping);
                if !state.mix.voices.is_empty() {
                    drop(self.wake.wait_timeout(state, RETRY));
                }
                continue;
            };
            if flush {
                open.flush();
            }
            if quiet > LINGER || !open.write(&chunk) {
                stream = None;
            }
        }
    }
}

struct Engine {
    mixer: Arc<Mixer>,
    /// Each clip's samples, decoded once: mono at [`RATE`].
    clips: HashMap<String, Arc<Vec<f32>>>,
}

thread_local! {
    static ENGINE: RefCell<Option<Engine>> = const { RefCell::new(None) };
    static LOOKED: Cell<bool> = const { Cell::new(false) };
}

impl Engine {
    /// Load libpulse and start the mixing thread, which waits for a first sound. `None` when
    /// the library is not installed.
    fn new() -> Option<Engine> {
        let pulse = Pulse::load()?;
        let app = std::env::current_exe()
            .ok()
            .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
            .and_then(|s| CString::new(s).ok())
            .unwrap_or_else(|| c"day".to_owned());
        let mixer = Arc::new(Mixer {
            pulse,
            app,
            state: Mutex::new(State::default()),
            wake: Condvar::new(),
        });
        let theirs = mixer.clone();
        std::thread::Builder::new()
            .name("day-part-sound".into())
            .spawn(move || theirs.run())
            .ok()?;
        Some(Engine {
            mixer,
            clips: HashMap::new(),
        })
    }

    /// Decode `path` the first time it is asked for.
    fn load(&mut self, path: &str) -> Option<Arc<Vec<f32>>> {
        if let Some(c) = self.clips.get(path) {
            return Some(c.clone());
        }
        let clip = Arc::new(super::load(path)?);
        self.clips.insert(path.to_owned(), clip.clone());
        Some(clip)
    }

    fn play(&mut self, path: &str, volume: f32, pan: f32, priority: i32) {
        let Some(clip) = self.load(path) else {
            return;
        };
        if self
            .mixer
            .state()
            .mix
            .play(clip, gains(volume, pan), priority)
        {
            self.mixer.wake.notify_one();
        }
    }

    fn loop_start(&mut self, id: i32, path: &str, volume: f32, pan: f32) {
        let Some(clip) = self.load(path) else {
            return;
        };
        self.mixer
            .state()
            .mix
            .loop_start(id, clip, gains(volume, pan));
        self.mixer.wake.notify_one();
    }

    fn unload_all(&mut self) {
        let mut state = self.mixer.state();
        state.mix.voices.clear();
        state.flush = true;
        drop(state);
        self.clips.clear();
    }
}

fn with_engine(f: impl FnOnce(&mut Engine)) {
    ENGINE.with(|e| {
        let mut e = e.borrow_mut();
        if e.is_none() && !LOOKED.with(|l| l.replace(true)) {
            *e = Engine::new();
            if e.is_none() {
                log::info!(target: "day_part_sound", "no libpulse; sound is off");
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
    with_engine(|e| {
        e.load(path);
    });
}

pub fn play(path: &str, volume: f32, pan: f32, priority: i32) {
    with_engine(|e| e.play(path, volume, pan, priority));
}

pub fn loop_start(id: i32, path: &str, volume: f32, pan: f32) {
    with_engine(|e| e.loop_start(id, path, volume, pan));
}

pub fn loop_set(id: i32, volume: f32, pan: f32) {
    with_engine(|e| e.mixer.state().mix.loop_set(id, gains(volume, pan)));
}

pub fn loop_stop(id: i32) {
    // Reached from `Loop`'s destructor, which may run while the thread and this thread-local are
    // being torn down: `try_with`, and never look for libpulse just to stop something.
    let _ = ENGINE.try_with(|e| {
        if let Ok(e) = e.try_borrow()
            && let Some(engine) = e.as_ref()
        {
            engine.mixer.state().mix.loop_stop(id);
        }
    });
}

pub fn unload_all() {
    // No engine yet means nothing loaded; do not look for libpulse just to empty it.
    ENGINE.with(|e| {
        if let Some(engine) = e.borrow_mut().as_mut() {
            engine.unload_all();
        }
    });
}
