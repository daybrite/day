// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// iOS and macOS: AVAudioEngine with a pool of AVAudioPlayerNodes, each connected to the main mixer
// in the canonical format (mono, 44.1 kHz, float). Every clip is decoded once into an
// AVAudioPCMBuffer in that same format, so a node never meets a buffer it cannot play (a format
// mismatch raises an Objective-C exception, which Rust cannot catch). A play takes a free node or
// stops the oldest lowest-priority one, sets its volume and pan, schedules the buffer and plays.
//
// On iOS the session goes into the `.ambient` category: game sound mixes with whatever else is
// playing, and the Silent switch and screen lock silence it. macOS has no session.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::time::{Duration, Instant};

use objc2::AllocAnyThread;
use objc2::rc::Retained;
use objc2_avf_audio::{
    AVAudioEngine, AVAudioFormat, AVAudioMixing, AVAudioPCMBuffer, AVAudioPlayerNode,
    AVAudioStereoMixing,
};

use super::{RATE, VOICES};

struct Voice {
    node: Retained<AVAudioPlayerNode>,
    started: Instant,
    /// When the clip it was given runs out.
    until: Instant,
    priority: i32,
}

struct Clip {
    buffer: Retained<AVAudioPCMBuffer>,
    length: Duration,
}

struct Engine {
    engine: Retained<AVAudioEngine>,
    format: Retained<AVAudioFormat>,
    voices: Vec<Voice>,
    clips: HashMap<String, Clip>,
}

thread_local! {
    static ENGINE: RefCell<Option<Engine>> = const { RefCell::new(None) };
    /// The engine could not start (no output device): stay silent rather than retry every play.
    static FAILED: Cell<bool> = const { Cell::new(false) };
}

impl Engine {
    fn new() -> Option<Engine> {
        #[cfg(target_os = "ios")]
        {
            use objc2_avf_audio::{AVAudioSession, AVAudioSessionCategoryAmbient};
            // SAFETY: the shared session is a process singleton; the category is a framework
            // constant, read once.
            unsafe {
                let session = AVAudioSession::sharedInstance();
                if let Some(ambient) = AVAudioSessionCategoryAmbient {
                    let _ = session.setCategory_error(ambient);
                }
                let _ = session.setActive_error(true);
            }
        }
        // SAFETY: plain AVFAudio object construction and graph wiring. Every node is attached
        // before it is connected, and the format is the standard (deinterleaved float) one, which
        // the main mixer accepts.
        unsafe {
            let engine = AVAudioEngine::new();
            let format = AVAudioFormat::initStandardFormatWithSampleRate_channels(
                AVAudioFormat::alloc(),
                RATE as f64,
                1,
            )?;
            let mixer = engine.mainMixerNode();
            let now = Instant::now();
            let mut voices = Vec::with_capacity(VOICES);
            for _ in 0..VOICES {
                let node = AVAudioPlayerNode::new();
                engine.attachNode(&node);
                engine.connect_to_format(&node, &mixer, Some(&format));
                voices.push(Voice {
                    node,
                    started: now,
                    until: now,
                    priority: i32::MIN,
                });
            }
            engine.prepare();
            if let Err(e) = engine.startAndReturnError() {
                log::warn!(target: "day_part_sound", "the audio engine did not start: {e:?}");
                return None;
            }
            Some(Engine {
                engine,
                format,
                voices,
                clips: HashMap::new(),
            })
        }
    }

    /// Decode `path` into a buffer the first time it is asked for.
    fn load(&mut self, path: &str) -> Option<(Retained<AVAudioPCMBuffer>, Duration)> {
        if let Some(c) = self.clips.get(path) {
            return Some((c.buffer.clone(), c.length));
        }
        let samples = super::load(path)?;
        let frames = u32::try_from(samples.len()).ok()?;
        // SAFETY: the buffer is created in the engine's own format with room for every frame; the
        // copy writes exactly `frames` floats into channel 0, the only channel of a mono format.
        let buffer = unsafe {
            let buffer = AVAudioPCMBuffer::initWithPCMFormat_frameCapacity(
                AVAudioPCMBuffer::alloc(),
                &self.format,
                frames,
            )?;
            buffer.setFrameLength(frames);
            let channels = buffer.floatChannelData();
            if channels.is_null() {
                return None;
            }
            std::ptr::copy_nonoverlapping(samples.as_ptr(), (*channels).as_ptr(), samples.len());
            buffer
        };
        let length = Duration::from_secs_f64(samples.len() as f64 / RATE as f64);
        self.clips.insert(
            path.to_owned(),
            Clip {
                buffer: buffer.clone(),
                length,
            },
        );
        Some((buffer, length))
    }

    fn play(&mut self, path: &str, volume: f32, pan: f32, priority: i32) {
        // A route or configuration change (headphones, a new output device) stops the engine;
        // start it again on the next play.
        // SAFETY: querying and starting the engine this module owns.
        unsafe {
            if !self.engine.isRunning() {
                self.engine.prepare();
                if self.engine.startAndReturnError().is_err() {
                    return;
                }
            }
        }
        let Some((buffer, length)) = self.load(path) else {
            return;
        };
        let now = Instant::now();
        let slot = match self.voices.iter().position(|v| v.until <= now) {
            Some(i) => i,
            None => {
                let Some((i, busiest)) = self
                    .voices
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, v)| (v.priority, v.started))
                else {
                    return;
                };
                if busiest.priority > priority {
                    return;
                }
                i
            }
        };
        let voice = &mut self.voices[slot];
        // SAFETY: the node belongs to this engine and the buffer is in the node's connection
        // format; a null completion handler is allowed.
        unsafe {
            voice.node.stop();
            voice.node.setVolume(volume);
            voice.node.setPan(pan);
            voice
                .node
                .scheduleBuffer_completionHandler(&buffer, std::ptr::null_mut());
            voice.node.play();
        }
        voice.started = now;
        voice.until = now + length;
        voice.priority = priority;
    }

    fn stop_all(&mut self) {
        let now = Instant::now();
        for v in &mut self.voices {
            // SAFETY: stopping a node this engine owns.
            unsafe { v.node.stop() };
            v.until = now;
        }
    }
}

fn with_engine(f: impl FnOnce(&mut Engine)) {
    if FAILED.with(|f| f.get()) {
        return;
    }
    ENGINE.with(|e| {
        let mut e = e.borrow_mut();
        if e.is_none() {
            *e = Engine::new();
            if e.is_none() {
                FAILED.with(|f| f.set(true));
                return;
            }
        }
        if let Some(engine) = e.as_mut() {
            f(engine);
        }
    });
}

pub fn supported() -> bool {
    true
}

pub fn preload(path: &str) {
    with_engine(|e| {
        e.load(path);
    });
}

pub fn play(path: &str, volume: f32, pan: f32, priority: i32) {
    with_engine(|e| e.play(path, volume, pan, priority));
}

pub fn unload_all() {
    // No engine yet means nothing loaded; do not start one just to empty it.
    ENGINE.with(|e| {
        if let Some(engine) = e.borrow_mut().as_mut() {
            engine.stop_all();
            engine.clips.clear();
        }
    });
}
