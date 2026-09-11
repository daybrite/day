// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! day-part-sound — HEADLESS sound effects: short clips bundled with the app, played through each
//! platform's own low-latency engine.
//!
//! ```no_run
//! use day_part_sound::AssetName;
//!
//! const TAP: AssetName = AssetName::from_static("sounds/tap.wav");
//! day_part_sound::preload(&[TAP]);
//! day_part_sound::play(&TAP);
//! ```
//!
//! A clip is an ordinary data asset under the app's `resource/assets/` (docs/resources.md), named
//! by its generated `res::assets::…` constant or by path. The format is WAV. The canonical clip,
//! 16-bit PCM, mono, 44.1 kHz, plays everywhere as it is; other PCM layouts (stereo, 8-, 24- and
//! 32-bit, float, another rate) are converted on load where an engine takes raw samples.
//!
//! | Platform | Engine |
//! |---|---|
//! | iOS, macOS | AVAudioEngine and a pool of AVAudioPlayerNodes (iOS in the `.ambient` session) |
//! | Android | SoundPool with game audio attributes |
//! | web | Web Audio |
//! | Windows | XAudio2 |
//! | Linux | libcanberra, loaded at run time |
//! | HarmonyOS | SoundPool with the game stream usage |
//!
//! Like haptics, every call is fire-and-forget and best effort: none returns an error or panics. A
//! clip that is missing or unreadable is logged once and stays silent, and so does a platform with
//! no engine ([`is_supported`] says which). Sound mixes with other apps' audio rather than
//! interrupting it, and on iOS the Silent switch silences it.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Mutex, MutexGuard};

pub use day_spec::AssetName;

/// At most this many clips sound at once. Past it, the oldest voice with the lowest priority stops
/// for the new clip.
pub const VOICES: usize = 8;

/// A clip asked for again sooner than this after it last started is skipped, so a burst of the
/// same sound (a streak of bricks, a deal of cards) stays crisp instead of piling up into one loud
/// smear.
pub const MIN_REPEAT_MS: u64 = 40;

/// The sample rate the engines that take raw samples run at: the canonical clip's own.
#[cfg_attr(
    not(any(target_os = "ios", target_os = "macos", target_os = "windows")),
    allow(dead_code)
)]
const RATE: u32 = 44_100;

/// How many of the latest plays [`recent`] remembers.
const RECENT: usize = 64;

/// How to play one clip ([`play_with`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Play {
    /// 0 (silent) to 1 (as recorded), scaled by the master [`volume`].
    pub volume: f32,
    /// −1 (left) to 1 (right). Best effort: Windows and Linux play every clip centered.
    pub pan: f32,
    /// When every voice is busy, the oldest voice with the lowest priority stops for this clip;
    /// a clip below the priority of everything playing is skipped instead.
    pub priority: i32,
}

impl Default for Play {
    fn default() -> Self {
        Play {
            volume: 1.0,
            pan: 0.0,
            priority: 0,
        }
    }
}

impl Play {
    /// Centered, at the default priority, at `volume`.
    pub fn at(volume: f32) -> Play {
        Play {
            volume,
            ..Play::default()
        }
    }
}

/// The switch, the volume and the bookkeeping every play passes through. One process-wide lock
/// rather than thread-locals: each `thread_local!` costs Android a pthread key, and bionic has only
/// 128 of them for the whole process.
struct State {
    enabled: bool,
    volume: f32,
    /// When each clip last started, for [`MIN_REPEAT_MS`].
    last: BTreeMap<String, u64>,
    played: VecDeque<String>,
    /// Clips already reported missing or unreadable, so each is logged once.
    warned: BTreeSet<String>,
}

static STATE: Mutex<State> = Mutex::new(State {
    enabled: true,
    volume: 1.0,
    last: BTreeMap::new(),
    played: VecDeque::new(),
    warned: BTreeSet::new(),
});

fn state() -> MutexGuard<'static, State> {
    // Nothing under the lock can leave it half-written, so a poisoned lock is still usable.
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Whether this platform has a sound engine. It reports whether the engine can be reached (on
/// Linux, whether libcanberra is installed), not whether a speaker is plugged in.
pub fn is_supported() -> bool {
    imp::supported()
}

/// Load `clips` now, so their first play starts at once. Engines that load in the background
/// (Android, HarmonyOS, the web) keep loading after this returns; a clip played before it is ready
/// is skipped, because a late sound effect is worse than none.
pub fn preload(clips: &[AssetName]) {
    for clip in clips {
        imp::preload(clip.as_str());
    }
}

/// Play `clip` at full volume, centered.
pub fn play(clip: &AssetName) {
    play_with(clip, Play::default());
}

/// Play `clip` as `opts` says. Nothing happens while sound is off ([`set_enabled`]), at zero
/// volume, or within [`MIN_REPEAT_MS`] of the same clip's last start.
pub fn play_with(clip: &AssetName, opts: Play) {
    let path = clip.as_str();
    let volume = (opts.volume * volume()).min(1.0);
    if !is_enabled() || volume.is_nan() || volume <= 0.0 {
        return;
    }
    {
        let mut s = state();
        if let Some(now) = now_ms() {
            match s.last.get(path) {
                Some(&t) if now.saturating_sub(t) < MIN_REPEAT_MS => return,
                _ => {
                    s.last.insert(path.to_owned(), now);
                }
            }
        }
        if s.played.len() == RECENT {
            s.played.pop_front();
        }
        s.played.push_back(path.to_owned());
    }
    log::debug!(target: "day_part_sound", "play {path}");
    let pan = if opts.pan.is_finite() {
        opts.pan.clamp(-1.0, 1.0)
    } else {
        0.0
    };
    imp::play(path, volume, pan, opts.priority);
}

/// The master volume, 0 to 1, that every play is scaled by.
pub fn set_volume(volume: f32) {
    if volume.is_finite() {
        state().volume = volume.clamp(0.0, 1.0);
    }
}

pub fn volume() -> f32 {
    state().volume
}

/// The app's sound switch: while off, [`play`] does nothing. On by default.
pub fn set_enabled(on: bool) {
    state().enabled = on;
}

pub fn is_enabled() -> bool {
    state().enabled
}

/// Stop everything playing and release every loaded clip (a game's page closing).
pub fn unload_all() {
    imp::unload_all();
    state().last.clear();
}

/// The clips most recently played, oldest first: what a test or a walkthrough log checks. A clip
/// counts here once it passed the switch, the volume and the repeat check, whether or not the
/// platform could sound it.
pub fn recent() -> Vec<String> {
    state().played.iter().cloned().collect()
}

/// Log a clip's problem the first time it comes up.
#[cfg_attr(
    not(any(
        target_os = "ios",
        target_os = "macos",
        target_os = "windows",
        all(target_os = "linux", not(target_env = "ohos")),
        test
    )),
    allow(dead_code)
)]
fn warn_once(path: &str, why: &str) {
    let first = state().warned.insert(path.to_owned());
    if first {
        log::warn!(target: "day_part_sound", "{path}: {why}");
    }
}

/// A clip's samples, mono at [`RATE`]: what the engines that take raw samples play.
#[cfg_attr(
    not(any(target_os = "ios", target_os = "macos", target_os = "windows")),
    allow(dead_code)
)]
fn load(path: &str) -> Option<Vec<f32>> {
    let Some(res) = day_spec::resource(AssetName::dynamic(path)) else {
        warn_once(path, "no such bundled asset");
        return None;
    };
    match wav::decode(res.as_slice()) {
        Ok(pcm) => Some(wav::resample(pcm, RATE)),
        Err(e) => {
            warn_once(path, e);
            None
        }
    }
}

/// Milliseconds on a clock that only moves forward, for the repeat check. The web has no clock in
/// wasm's `std` (`Instant::now` aborts there), so its arm keeps the same rule in JavaScript.
#[cfg(not(target_arch = "wasm32"))]
fn now_ms() -> Option<u64> {
    static EPOCH: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    Some(
        EPOCH
            .get_or_init(std::time::Instant::now)
            .elapsed()
            .as_millis() as u64,
    )
}

#[cfg(target_arch = "wasm32")]
fn now_ms() -> Option<u64> {
    None
}

// ---------------------------------------------------------------------------
// Per-platform engines. Each exposes `supported`, `preload`, `play` and `unload_all`.
// ---------------------------------------------------------------------------

#[cfg(any(target_os = "ios", target_os = "macos"))]
#[path = "apple.rs"]
mod imp;

#[cfg(target_os = "windows")]
#[path = "windows.rs"]
mod imp;

#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
#[path = "linux.rs"]
mod imp;

// Android, HarmonyOS and the web reach their engines through the bridge below; any other target
// gets its `other` arm, which is silent.
#[cfg(not(any(
    target_os = "ios",
    target_os = "macos",
    target_os = "windows",
    all(target_os = "linux", not(target_env = "ohos"))
)))]
mod imp {
    use day_bridge::Support;

    pub fn supported() -> bool {
        super::ready_native_support() != Support::Unsupported
            && super::ready_native().unwrap_or(false)
    }

    pub fn preload(path: &str) {
        super::preload_native(path);
    }

    pub fn play(path: &str, volume: f32, pan: f32, priority: i32) {
        super::play_native(path, volume, pan, priority);
    }

    pub fn unload_all() {
        super::unload_native();
    }
}

day_bridge::bridge! {
    // The contract the foreign arms implement. `path` is the clip's asset path ("sounds/tap.wav");
    // each arm finds the asset in its own platform's store.
    #[day_bridge::declare]
    extern "day" {
        fn preload_native(path: &str);
        fn play_native(path: &str, volume: f32, pan: f32, priority: i32);
        fn unload_native();
        /// Whether the engine can be reached at all.
        fn ready_native() -> Result<bool, day_bridge::Error>;
    }

    // Android: SoundPool, the platform's engine for short, low-latency clips. It decodes each clip
    // into memory once and mixes up to `maxStreams` of them, stopping the lowest priority first.
    // Clips come straight from the APK's assets (data assets are its `assets/` root), which aapt
    // stores uncompressed for `.wav`, as `openFd` needs. Nothing here is reachable from Rust.
    #[day_bridge::impl(java, platforms = [android])]
    java!(
        prelude = r#"
            import android.content.Context;
            import android.content.res.AssetFileDescriptor;
            import android.media.AudioAttributes;
            import android.media.SoundPool;
            import android.util.Log;
            import dev.daybrite.day.bridge.DayBridge;
            import java.util.HashMap;
            import java.util.HashSet;
        "#,
        body = r#"
            private static SoundPool pool = null;
            private static final HashMap<String, Integer> ids = new HashMap<>();
            private static final HashSet<Integer> loaded = new HashSet<>();

            private static SoundPool pool() {
                if (pool == null) {
                    AudioAttributes attrs = new AudioAttributes.Builder()
                            .setUsage(AudioAttributes.USAGE_GAME)
                            .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION)
                            .build();
                    pool = new SoundPool.Builder().setMaxStreams(8).setAudioAttributes(attrs).build();
                    // Loading is asynchronous; a clip plays only once SoundPool says it is ready.
                    pool.setOnLoadCompleteListener((p, id, status) -> {
                        if (status == 0) {
                            synchronized (loaded) {
                                loaded.add(id);
                            }
                        }
                    });
                }
                return pool;
            }

            public static void preload_native(String path) {
                Context ctx = DayBridge.ctx;
                if (ctx == null || ids.containsKey(path)) {
                    return;
                }
                try (AssetFileDescriptor fd = ctx.getAssets().openFd(path)) {
                    ids.put(path, pool().load(fd, 1));
                } catch (Exception e) {
                    // Remember the miss so a missing clip is reported once, not on every play.
                    ids.put(path, 0);
                    Log.w("day-part-sound", path + ": " + e);
                }
            }

            public static void play_native(String path, float volume, float pan, int priority) {
                Integer id = ids.get(path);
                if (id == null) {
                    preload_native(path);
                    return;
                }
                boolean ready;
                synchronized (loaded) {
                    ready = loaded.contains(id);
                }
                if (!ready) {
                    return;
                }
                float left = volume * Math.min(1f, 1f - pan);
                float right = volume * Math.min(1f, 1f + pan);
                pool.play(id, left, right, priority, 0, 1f);
            }

            public static void unload_native() {
                if (pool != null) {
                    for (int id : ids.values()) {
                        if (id != 0) {
                            pool.unload(id);
                        }
                    }
                }
                ids.clear();
                synchronized (loaded) {
                    loaded.clear();
                }
            }

            public static boolean ready_native() {
                return DayBridge.ctx != null;
            }
        "#,
    );

    // HarmonyOS: SoundPool from the Media Kit, the system's engine for short sounds, with the game
    // stream usage. Clips come from the rawfile store, where `day build` stages data assets under
    // `day/`. SoundPool exists only in ArkTS.
    #[day_bridge::impl(arkts, platforms = [ohos])]
    arkts!(
        prelude = r#"
            import { media } from '@kit.MediaKit';
            import { audio } from '@kit.AudioKit';
            import { common } from '@kit.AbilityKit';
        "#,
        body = r#"
            let dayPool: media.SoundPool | undefined = undefined;
            let dayPoolMaking: Promise<media.SoundPool> | undefined = undefined;
            const dayIds: Map<string, number> = new Map<string, number>();
            const dayLoading: Set<string> = new Set<string>();

            function dayMakePool(): Promise<media.SoundPool> {
                if (!dayPoolMaking) {
                    const info: audio.AudioRendererInfo = {
                        usage: audio.StreamUsage.STREAM_USAGE_GAME,
                        rendererFlags: 1,
                    };
                    dayPoolMaking = media.createSoundPool(8, info).then((pool: media.SoundPool) => {
                        dayPool = pool;
                        return pool;
                    });
                }
                return dayPoolMaking;
            }

            export function preload_native(path: string): void {
                if (dayIds.has(path) || dayLoading.has(path)) {
                    return;
                }
                dayLoading.add(path);
                dayMakePool().then(async (pool: media.SoundPool) => {
                    const ctx = getContext() as common.UIAbilityContext;
                    const fd = await ctx.resourceManager.getRawFd('day/' + path);
                    const id: number = await pool.load(fd.fd, fd.offset, fd.length);
                    dayIds.set(path, id);
                }).catch((e: Error) => {
                    console.warn(`day-part-sound: ${path}: ${e.message}`);
                }).finally(() => {
                    dayLoading.delete(path);
                });
            }

            export function play_native(path: string, volume: number, pan: number, priority: number): void {
                const id = dayIds.get(path);
                if (id === undefined || !dayPool) {
                    preload_native(path);
                    return;
                }
                const params: media.PlayParameters = {
                    loop: 0,
                    rate: 0,
                    leftVolume: volume * Math.min(1, 1 - pan),
                    rightVolume: volume * Math.min(1, 1 + pan),
                    priority: priority,
                };
                dayPool.play(id, params).catch((e: Error) => {
                    console.warn(`day-part-sound: ${path}: ${e.message}`);
                });
            }

            export function unload_native(): void {
                const pool = dayPool;
                if (pool) {
                    dayIds.forEach((id: number) => {
                        pool.unload(id);
                    });
                }
                dayIds.clear();
            }

            export function ready_native(): boolean {
                return true;
            }
        "#,
    );

    // The web: one AudioContext, each clip fetched from the dist's `assets/data/` and decoded once,
    // each play a new AudioBufferSourceNode through a gain and a panner. Browsers start a context
    // suspended until the page is touched, so the first gesture resumes it; a clip due before then
    // is skipped. wasm's `std` has no clock, so the repeat check lives here.
    #[day_bridge::impl(js, platforms = [web])]
    js!(r#"
        const dayBuffers = new Map();
        const dayPending = new Set();
        const dayLast = new Map();
        const dayVoices = [];
        let dayCtx = null;

        function dayContext() {
            if (dayCtx === null) {
                const Context = window.AudioContext || window.webkitAudioContext;
                if (!Context) {
                    return null;
                }
                dayCtx = new Context();
                const unlock = () => {
                    if (dayCtx.state !== "running") {
                        dayCtx.resume();
                    }
                };
                for (const type of ["pointerdown", "keydown", "touchend"]) {
                    window.addEventListener(type, unlock, { capture: true });
                }
            }
            return dayCtx;
        }

        export function preload_native(path) {
            const ctx = dayContext();
            if (!ctx || dayBuffers.has(path) || dayPending.has(path)) {
                return;
            }
            dayPending.add(path);
            fetch("assets/data/" + encodeURI(path))
                .then((r) => (r.ok ? r.arrayBuffer() : Promise.reject(new Error("HTTP " + r.status))))
                .then((bytes) => ctx.decodeAudioData(bytes))
                .then((buffer) => dayBuffers.set(path, buffer))
                .catch((e) => console.warn("day-part-sound: " + path + ": " + e))
                .finally(() => dayPending.delete(path));
        }

        export function play_native(path, volume, pan, priority) {
            const ctx = dayContext();
            if (!ctx) {
                return;
            }
            const now = performance.now();
            const last = dayLast.get(path);
            if (last !== undefined && now - last < 40) {
                return;
            }
            dayLast.set(path, now);
            const buffer = dayBuffers.get(path);
            if (!buffer) {
                preload_native(path);
                return;
            }
            if (ctx.state !== "running") {
                ctx.resume();
            }
            if (dayVoices.length >= 8) {
                let steal = 0;
                for (let i = 1; i < dayVoices.length; i++) {
                    if (dayVoices[i].priority < dayVoices[steal].priority) {
                        steal = i;
                    }
                }
                if (dayVoices[steal].priority > priority) {
                    return;
                }
                dayVoices[steal].source.stop();
                dayVoices.splice(steal, 1);
            }
            const source = ctx.createBufferSource();
            source.buffer = buffer;
            const gain = ctx.createGain();
            gain.gain.value = volume;
            source.connect(gain);
            if (ctx.createStereoPanner) {
                const panner = ctx.createStereoPanner();
                panner.pan.value = pan;
                gain.connect(panner);
                panner.connect(ctx.destination);
            } else {
                gain.connect(ctx.destination);
            }
            const voice = { source, priority };
            dayVoices.push(voice);
            source.onended = () => {
                const i = dayVoices.indexOf(voice);
                if (i >= 0) {
                    dayVoices.splice(i, 1);
                }
            };
            source.start();
        }

        export function unload_native() {
            for (const voice of dayVoices) {
                voice.source.stop();
            }
            dayVoices.length = 0;
            dayBuffers.clear();
        }

        export function ready_native() {
            return Boolean(window.AudioContext || window.webkitAudioContext);
        }
    "#);

    // Everywhere else, and the arm `cargo test` and day-mock compile against: silence.
    #[day_bridge::impl(rust, platforms = [other])]
    fn preload_native(_path: &str) {}

    #[day_bridge::impl(rust, platforms = [other])]
    fn play_native(_path: &str, _volume: f32, _pan: f32, _priority: i32) {}

    #[day_bridge::impl(rust, platforms = [other])]
    fn unload_native() {}

    #[day_bridge::impl(rust, platforms = [other])]
    fn ready_native() -> Result<bool, day_bridge::Error> {
        Ok(false)
    }
}

/// Reading WAV: RIFF chunks, the `fmt ` description and the `data` samples, mixed down to mono.
mod wav {
    /// Decoded audio: mono samples in −1…1 at `rate` Hz.
    pub struct Pcm {
        pub rate: u32,
        pub samples: Vec<f32>,
    }

    /// The format tags a `fmt ` chunk may carry that this reader understands.
    const PCM: u16 = 1;
    const FLOAT: u16 = 3;
    const EXTENSIBLE: u16 = 0xFFFE;

    fn le16(b: &[u8], at: usize) -> u16 {
        u16::from_le_bytes([b[at], b[at + 1]])
    }

    fn le32(b: &[u8], at: usize) -> u32 {
        u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
    }

    /// Integer PCM at 8, 16, 24 or 32 bits and 32-bit float, any channel count, including
    /// `WAVE_FORMAT_EXTENSIBLE`. A `data` chunk cut short (a truncated file) plays as far as it
    /// goes.
    pub fn decode(bytes: &[u8]) -> Result<Pcm, &'static str> {
        if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
            return Err("not a WAV file");
        }
        let mut fmt = None;
        let mut data = None;
        let mut at = 12;
        while at + 8 <= bytes.len() {
            let len = le32(bytes, at + 4) as usize;
            let start = at + 8;
            let body = &bytes[start..start.saturating_add(len).min(bytes.len())];
            match &bytes[at..at + 4] {
                b"fmt " => {
                    if body.len() < 16 {
                        return Err("fmt chunk too short");
                    }
                    let mut tag = le16(body, 0);
                    if tag == EXTENSIBLE {
                        // The real format is the first two bytes of the SubFormat GUID.
                        if body.len() < 26 {
                            return Err("fmt chunk too short");
                        }
                        tag = le16(body, 24);
                    }
                    fmt = Some((tag, le16(body, 2), le32(body, 4), le16(body, 14)));
                }
                b"data" => data = Some(body),
                _ => {}
            }
            // Chunks are padded to an even length.
            at = start.saturating_add(len).saturating_add(len & 1);
        }
        let (tag, channels, rate, bits) = fmt.ok_or("no fmt chunk")?;
        let data = data.ok_or("no data chunk")?;
        if channels == 0 || rate == 0 {
            return Err("no channels or no sample rate");
        }
        let width = match (tag, bits) {
            (PCM, 8) => 1,
            (PCM, 16) => 2,
            (PCM, 24) => 3,
            (PCM, 32) | (FLOAT, 32) => 4,
            _ => return Err("not 8-, 16-, 24- or 32-bit PCM or 32-bit float"),
        };
        let channels = channels as usize;
        let frame = width * channels;
        let frames = data.len() / frame;
        let mut samples = Vec::with_capacity(frames);
        for f in data.chunks_exact(frame) {
            let mut sum = 0.0f32;
            for s in f.chunks_exact(width) {
                sum += match (tag, width) {
                    (PCM, 1) => (s[0] as f32 - 128.0) / 128.0,
                    (PCM, 2) => i16::from_le_bytes([s[0], s[1]]) as f32 / 32_768.0,
                    (PCM, 3) => {
                        (i32::from_le_bytes([0, s[0], s[1], s[2]]) >> 8) as f32 / 8_388_608.0
                    }
                    (PCM, _) => {
                        i32::from_le_bytes([s[0], s[1], s[2], s[3]]) as f32 / 2_147_483_648.0
                    }
                    _ => f32::from_le_bytes([s[0], s[1], s[2], s[3]]),
                };
            }
            let mono = sum / channels as f32;
            samples.push(if mono.is_finite() {
                mono.clamp(-1.0, 1.0)
            } else {
                0.0
            });
        }
        Ok(Pcm { rate, samples })
    }

    /// Resample to `rate` by linear interpolation: plenty for short effects, and nothing at all for
    /// a clip already at that rate.
    pub fn resample(pcm: Pcm, rate: u32) -> Vec<f32> {
        let len = pcm.samples.len();
        if pcm.rate == rate || len == 0 {
            return pcm.samples;
        }
        let n = ((len as u64 * rate as u64) / pcm.rate as u64).max(1) as usize;
        let step = pcm.rate as f64 / rate as f64;
        (0..n)
            .map(|i| {
                let x = i as f64 * step;
                let j = (x as usize).min(len - 1);
                let k = (j + 1).min(len - 1);
                let t = (x - j as f64) as f32;
                pcm.samples[j] + (pcm.samples[k] - pcm.samples[j]) * t
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A WAV file around `data`: a `fmt ` chunk (EXTENSIBLE when asked), an odd-length `LIST`
    /// chunk to exercise the padding rule, then `data`.
    fn wav_file(tag: u16, channels: u16, rate: u32, bits: u16, data: &[u8], ext: bool) -> Vec<u8> {
        let mut fmt = Vec::new();
        fmt.extend((if ext { 0xFFFE } else { tag }).to_le_bytes());
        fmt.extend(channels.to_le_bytes());
        fmt.extend(rate.to_le_bytes());
        let block = channels as u32 * (bits as u32 / 8);
        fmt.extend((rate * block).to_le_bytes());
        fmt.extend((block as u16).to_le_bytes());
        fmt.extend(bits.to_le_bytes());
        if ext {
            fmt.extend(22u16.to_le_bytes());
            fmt.extend(bits.to_le_bytes());
            fmt.extend(0u32.to_le_bytes());
            fmt.extend(tag.to_le_bytes());
            fmt.extend([0u8; 14]);
        }
        let mut out = b"RIFF\0\0\0\0WAVE".to_vec();
        out.extend(b"fmt ");
        out.extend((fmt.len() as u32).to_le_bytes());
        out.extend(fmt);
        out.extend(b"LIST");
        out.extend(3u32.to_le_bytes());
        out.extend([1, 2, 3, 0]);
        out.extend(b"data");
        out.extend((data.len() as u32).to_le_bytes());
        out.extend(data);
        let riff = (out.len() - 8) as u32;
        out[4..8].copy_from_slice(&riff.to_le_bytes());
        out
    }

    fn close(a: &[f32], b: &[f32]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-3)
    }

    #[test]
    fn reads_the_canonical_clip() {
        let data: Vec<u8> = [0i16, 16_384, -16_384, 32_767]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        let pcm = wav::decode(&wav_file(1, 1, 44_100, 16, &data, false)).unwrap();
        assert_eq!(pcm.rate, 44_100);
        assert!(
            close(&pcm.samples, &[0.0, 0.5, -0.5, 1.0]),
            "{:?}",
            pcm.samples
        );
    }

    #[test]
    fn mixes_stereo_down_and_reads_every_width() {
        let stereo: Vec<u8> = [16_384i16, -16_384, 8_192, 8_192]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        let pcm = wav::decode(&wav_file(1, 2, 22_050, 16, &stereo, false)).unwrap();
        assert!(close(&pcm.samples, &[0.0, 0.25]));
        let eight = wav::decode(&wav_file(1, 1, 8_000, 8, &[128, 192, 64], false)).unwrap();
        assert!(close(&eight.samples, &[0.0, 0.5, -0.5]));
        let s24 = [0x00, 0x00, 0x40, 0x00, 0x00, 0xC0]; // 0.5, −0.5
        let p24 = wav::decode(&wav_file(1, 1, 48_000, 24, &s24, false)).unwrap();
        assert!(close(&p24.samples, &[0.5, -0.5]));
        let f: Vec<u8> = [0.25f32, -0.75]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        let pf = wav::decode(&wav_file(3, 1, 44_100, 32, &f, false)).unwrap();
        assert!(close(&pf.samples, &[0.25, -0.75]));
        let ext = wav::decode(&wav_file(3, 1, 44_100, 32, &f, true)).unwrap();
        assert!(
            close(&ext.samples, &[0.25, -0.75]),
            "WAVE_FORMAT_EXTENSIBLE"
        );
    }

    #[test]
    fn a_truncated_file_plays_what_it_has_and_junk_is_refused() {
        let data: Vec<u8> = [1_000i16, 2_000, 3_000]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        let mut file = wav_file(1, 1, 44_100, 16, &data, false);
        file.truncate(file.len() - 3);
        assert_eq!(wav::decode(&file).unwrap().samples.len(), 1);
        assert!(wav::decode(b"not a wav file at all").is_err());
        assert!(wav::decode(&wav_file(1, 1, 44_100, 12, &data, false)).is_err());
        let mut no_data = wav_file(1, 1, 44_100, 16, &data, false);
        no_data.truncate(12 + 8 + 16);
        assert_eq!(wav::decode(&no_data).err(), Some("no data chunk"));
    }

    #[test]
    fn resampling_keeps_the_shape() {
        let up = wav::resample(
            wav::Pcm {
                rate: 22_050,
                samples: vec![0.0, 1.0],
            },
            44_100,
        );
        assert!(close(&up, &[0.0, 0.5, 1.0, 1.0]), "{up:?}");
        let same = wav::resample(
            wav::Pcm {
                rate: 44_100,
                samples: vec![0.3],
            },
            44_100,
        );
        assert_eq!(same, vec![0.3]);
    }

    /// The plays of this test's own clips among [`recent`], which every test shares.
    fn mine(names: &[&AssetName]) -> usize {
        recent()
            .iter()
            .filter(|p| names.iter().any(|n| n.as_str() == p.as_str()))
            .count()
    }

    #[test]
    fn plays_pass_the_switch_the_volume_and_the_repeat_check() {
        let clip = AssetName::from_static("sounds/test-repeat.wav");
        let other = AssetName::from_static("sounds/test-silent.wav");
        // Start this platform's engine first: its first start can outlast the repeat window.
        preload(&[AssetName::from_static("sounds/test-warm.wav")]);
        play(&clip);
        play(&clip);
        assert_eq!(mine(&[&clip]), 1, "a repeat is skipped");
        play_with(&other, Play::at(0.0));
        set_enabled(false);
        play(&other);
        set_enabled(true);
        set_volume(0.0);
        play(&other);
        set_volume(f32::NAN);
        assert_eq!(volume(), 0.0, "a NaN volume is ignored");
        set_volume(1.0);
        assert_eq!(mine(&[&other]), 0, "silent plays are not counted");
        play_with(
            &other,
            Play {
                volume: 0.5,
                pan: f32::NAN,
                priority: 3,
            },
        );
        assert_eq!(mine(&[&other]), 1);
    }

    // Every entry point must be safe to call on any host, with or without an engine.
    #[test]
    fn nothing_panics() {
        let _ = is_supported();
        preload(&[AssetName::from_static("sounds/missing.wav")]);
        play(&AssetName::from_static("sounds/missing.wav"));
        unload_all();
    }
}
