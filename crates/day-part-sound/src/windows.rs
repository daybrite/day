// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// Windows: XAudio2 2.9, the low-latency engine Windows provides for games. One mastering voice and
// a pool of source voices in the canonical format (mono, 44.1 kHz, 16-bit). A play takes an idle
// voice (nothing queued) or stops the oldest lowest-priority one, submits the clip's samples and
// starts it. XAudio2 reads a submitted buffer from its own thread, so every clip a voice may still
// be reading stays alive: a voice keeps its clip, and `unload_all` retires clips until every voice
// has drained. Pan is not applied here; every clip plays centered.
//
// A loop has a source voice of its own from a second, smaller pool. Its buffer is submitted with
// `LoopCount = XAUDIO2_LOOP_INFINITE`, so XAudio2 repeats it sample-exactly until the voice is
// stopped and flushed.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

use windows::Win32::Media::Audio::XAudio2::{
    IXAudio2, IXAudio2MasteringVoice, IXAudio2SourceVoice, IXAudio2VoiceCallback, XAUDIO2_BUFFER,
    XAUDIO2_DEFAULT_FREQ_RATIO, XAUDIO2_DEFAULT_PROCESSOR, XAUDIO2_END_OF_STREAM,
    XAUDIO2_LOOP_INFINITE, XAUDIO2_VOICE_NOSAMPLESPLAYED, XAUDIO2_VOICE_STATE,
    XAudio2CreateWithVersionInfo,
};
use windows::Win32::Media::Audio::{AudioCategory_GameEffects, WAVE_FORMAT_PCM, WAVEFORMATEX};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
use windows::core::PCWSTR;

use super::{LOOPS, RATE, VOICES};

/// `NTDDI_WIN10`: the XAudio2 behavior Windows 10 shipped with.
const NTDDI_WIN10: u32 = 0x0A00_0000;

struct Voice {
    voice: IXAudio2SourceVoice,
    /// The clip it was last given, alive while the voice may still read it.
    clip: Option<Rc<Vec<i16>>>,
    started: Instant,
    priority: i32,
}

/// Whether `voice` has nothing queued: it has played everything it was given, or was flushed.
fn drained(voice: &IXAudio2SourceVoice) -> bool {
    let mut state = XAUDIO2_VOICE_STATE::default();
    // SAFETY: querying a voice this module owns into a local.
    unsafe { voice.GetState(&mut state, XAUDIO2_VOICE_NOSAMPLESPLAYED) };
    state.BuffersQueued == 0
}

impl Voice {
    fn idle(&self) -> bool {
        drained(&self.voice)
    }
}

/// A voice kept for loops, and the loop it is repeating: its id and the samples, alive while
/// the voice reads them.
struct LoopVoice {
    voice: IXAudio2SourceVoice,
    playing: Option<(i32, Rc<Vec<i16>>)>,
}

struct Engine {
    // Declared before the voices' owner so it outlives them; field order is drop order.
    voices: Vec<Voice>,
    loops: Vec<LoopVoice>,
    _master: IXAudio2MasteringVoice,
    _xaudio: IXAudio2,
    clips: HashMap<String, Rc<Vec<i16>>>,
    /// Clips unloaded while a voice might still have been reading them.
    retired: Vec<Rc<Vec<i16>>>,
}

thread_local! {
    static ENGINE: RefCell<Option<Engine>> = const { RefCell::new(None) };
    static FAILED: Cell<bool> = const { Cell::new(false) };
}

impl Engine {
    fn new() -> Option<Engine> {
        // SAFETY: COM and XAudio2 set-up on this thread. `CoInitializeEx` failing because the host
        // already chose another apartment is fine; XAudio2 works in either.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let mut xaudio: Option<IXAudio2> = None;
            XAudio2CreateWithVersionInfo(&mut xaudio, 0, XAUDIO2_DEFAULT_PROCESSOR, NTDDI_WIN10)
                .ok()?;
            let xaudio = xaudio?;
            let mut master: Option<IXAudio2MasteringVoice> = None;
            xaudio
                .CreateMasteringVoice(
                    &mut master,
                    0,
                    0,
                    0,
                    PCWSTR::null(),
                    None,
                    AudioCategory_GameEffects,
                )
                .ok()?;
            let master = master?;
            let format = WAVEFORMATEX {
                wFormatTag: WAVE_FORMAT_PCM as u16,
                nChannels: 1,
                nSamplesPerSec: RATE,
                nAvgBytesPerSec: RATE * 2,
                nBlockAlign: 2,
                wBitsPerSample: 16,
                cbSize: 0,
            };
            let now = Instant::now();
            let mut voices = Vec::with_capacity(VOICES);
            for _ in 0..VOICES {
                let mut voice: Option<IXAudio2SourceVoice> = None;
                xaudio
                    .CreateSourceVoice(
                        &mut voice,
                        &format,
                        0,
                        XAUDIO2_DEFAULT_FREQ_RATIO,
                        None::<&IXAudio2VoiceCallback>,
                        None,
                        None,
                    )
                    .ok()?;
                voices.push(Voice {
                    voice: voice?,
                    clip: None,
                    started: now,
                    priority: i32::MIN,
                });
            }
            let mut loops = Vec::with_capacity(LOOPS);
            for _ in 0..LOOPS {
                let mut voice: Option<IXAudio2SourceVoice> = None;
                xaudio
                    .CreateSourceVoice(
                        &mut voice,
                        &format,
                        0,
                        XAUDIO2_DEFAULT_FREQ_RATIO,
                        None::<&IXAudio2VoiceCallback>,
                        None,
                        None,
                    )
                    .ok()?;
                loops.push(LoopVoice {
                    voice: voice?,
                    playing: None,
                });
            }
            Some(Engine {
                voices,
                loops,
                _master: master,
                _xaudio: xaudio,
                clips: HashMap::new(),
                retired: Vec::new(),
            })
        }
    }

    fn load(&mut self, path: &str) -> Option<Rc<Vec<i16>>> {
        if let Some(c) = self.clips.get(path) {
            return Some(c.clone());
        }
        let samples: Vec<i16> = super::load(path)?
            .iter()
            .map(|s| (s * 32_767.0) as i16)
            .collect();
        let clip = Rc::new(samples);
        self.clips.insert(path.to_owned(), clip.clone());
        Some(clip)
    }

    /// Free the clips nothing can be reading any more: every voice, loops included, has drained.
    fn sweep(&mut self) {
        if !self.retired.is_empty()
            && self.voices.iter().all(Voice::idle)
            && self.loops.iter().all(|v| drained(&v.voice))
        {
            self.retired.clear();
        }
    }

    fn play(&mut self, path: &str, volume: f32, priority: i32) {
        self.sweep();
        let Some(clip) = self.load(path) else {
            return;
        };
        let Ok(bytes) = u32::try_from(clip.len() * 2) else {
            return;
        };
        let slot = match self.voices.iter().position(Voice::idle) {
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
        let buffer = XAUDIO2_BUFFER {
            Flags: XAUDIO2_END_OF_STREAM,
            AudioBytes: bytes,
            pAudioData: clip.as_ptr().cast(),
            ..Default::default()
        };
        // SAFETY: the voice is this module's; the samples stay alive in `voice.clip` (and in
        // `clips` or `retired`) for as long as XAudio2 may read them.
        unsafe {
            let _ = voice.voice.Stop(0, 0);
            let _ = voice.voice.FlushSourceBuffers();
            if voice.voice.SubmitSourceBuffer(&buffer, None).is_err() {
                return;
            }
            let _ = voice.voice.SetVolume(volume, 0);
            let _ = voice.voice.Start(0, 0);
        }
        voice.clip = Some(clip);
        voice.started = Instant::now();
        voice.priority = priority;
    }

    fn loop_start(&mut self, id: i32, path: &str, volume: f32) {
        self.sweep();
        let Some(clip) = self.load(path) else {
            return;
        };
        let Ok(bytes) = u32::try_from(clip.len() * 2) else {
            return;
        };
        // `play_loop` holds at most `LOOPS` loops, so a free voice is always here.
        let Some(v) = self.loops.iter_mut().find(|v| v.playing.is_none()) else {
            return;
        };
        let buffer = XAUDIO2_BUFFER {
            Flags: XAUDIO2_END_OF_STREAM,
            AudioBytes: bytes,
            pAudioData: clip.as_ptr().cast(),
            // The whole buffer, again and again.
            LoopCount: XAUDIO2_LOOP_INFINITE,
            ..Default::default()
        };
        // SAFETY: the voice is this module's; the samples stay alive in `v.playing` (and in
        // `clips` or `retired`) for as long as XAudio2 may read them.
        unsafe {
            let _ = v.voice.Stop(0, 0);
            let _ = v.voice.FlushSourceBuffers();
            if v.voice.SubmitSourceBuffer(&buffer, None).is_err() {
                return;
            }
            let _ = v.voice.SetVolume(volume, 0);
            let _ = v.voice.Start(0, 0);
        }
        v.playing = Some((id, clip));
    }

    fn loop_voice(&mut self, id: i32) -> Option<&mut LoopVoice> {
        self.loops
            .iter_mut()
            .find(|v| v.playing.as_ref().is_some_and(|(i, _)| *i == id))
    }

    fn loop_set(&mut self, id: i32, volume: f32) {
        if let Some(v) = self.loop_voice(id) {
            // SAFETY: setting the volume of a voice this module owns.
            unsafe {
                let _ = v.voice.SetVolume(volume, 0);
            }
        }
    }

    fn loop_stop(&mut self, id: i32) {
        let Some(v) = self.loop_voice(id) else {
            return;
        };
        // SAFETY: stopping and flushing a voice this module owns.
        unsafe {
            let _ = v.voice.Stop(0, 0);
            let _ = v.voice.FlushSourceBuffers();
        }
        // The flush lands on XAudio2's own thread, which may read the samples until then.
        if let Some((_, clip)) = v.playing.take() {
            self.retired.push(clip);
        }
    }

    fn unload_all(&mut self) {
        for v in &mut self.voices {
            // SAFETY: stopping and flushing a voice this module owns.
            unsafe {
                let _ = v.voice.Stop(0, 0);
                let _ = v.voice.FlushSourceBuffers();
            }
            if let Some(clip) = v.clip.take() {
                self.retired.push(clip);
            }
        }
        for v in &mut self.loops {
            // SAFETY: stopping and flushing a voice this module owns.
            unsafe {
                let _ = v.voice.Stop(0, 0);
                let _ = v.voice.FlushSourceBuffers();
            }
            if let Some((_, clip)) = v.playing.take() {
                self.retired.push(clip);
            }
        }
        self.retired.extend(self.clips.drain().map(|(_, c)| c));
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        for voice in self
            .voices
            .iter()
            .map(|v| &v.voice)
            .chain(self.loops.iter().map(|v| &v.voice))
        {
            // SAFETY: DestroyVoice blocks until the audio thread has let go of the voice, after
            // which its buffers (still owned here) may be freed.
            unsafe { voice.DestroyVoice() };
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
                log::warn!(target: "day_part_sound", "XAudio2 did not start");
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

pub fn play(path: &str, volume: f32, _pan: f32, priority: i32) {
    with_engine(|e| e.play(path, volume, priority));
}

pub fn loop_start(id: i32, path: &str, volume: f32, _pan: f32) {
    with_engine(|e| e.loop_start(id, path, volume));
}

pub fn loop_set(id: i32, volume: f32, _pan: f32) {
    with_engine(|e| e.loop_set(id, volume));
}

pub fn loop_stop(id: i32) {
    // Reached from `Loop`'s destructor, which may run while the thread and this thread-local are
    // being torn down: `try_with`, and never start an engine just to stop something in it.
    let _ = ENGINE.try_with(|e| {
        if let Ok(mut e) = e.try_borrow_mut()
            && let Some(engine) = e.as_mut()
        {
            engine.loop_stop(id);
        }
    });
}

pub fn unload_all() {
    ENGINE.with(|e| {
        if let Some(engine) = e.borrow_mut().as_mut() {
            engine.unload_all();
        }
    });
}
