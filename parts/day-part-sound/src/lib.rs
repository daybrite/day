// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! day-part-sound provides headless sound effects: short clips bundled with the app, played through
//! each platform's low-latency engine.
//!
//! ```no_run
//! use day_part_sound::AssetName;
//!
//! const TAP: AssetName = AssetName::from_static("sounds/tap.wav");
//! day_part_sound::preload(&[TAP]);
//! day_part_sound::play(&TAP);
//! ```
//!
//! A clip can also repeat until it is told to stop: an engine hum, a siren, rain. [`play_loop`]
//! returns a [`Loop`], and the clip sounds for as long as that handle lives.
//!
//! ```no_run
//! use day_part_sound::{AssetName, Play};
//!
//! const HUM: AssetName = AssetName::from_static("sounds/hum.wav");
//! let hum = day_part_sound::play_loop(&HUM, Play::at(0.5));
//! hum.set_volume(0.2);
//! drop(hum); // the hum stops
//! ```
//!
//! A clip is an ordinary data asset under the app's `resource/assets/` (docs/resources.md), named
//! by its generated `res::assets::…` constant or by path. The format is WAV. The canonical clip,
//! 16-bit PCM, mono, 44.1 kHz, plays everywhere as it is; other PCM layouts (stereo, 8-, 24- and
//! 32-bit, float, another rate) are converted on load where an engine takes raw samples.
//!
//! | Platform | Engine | Loops |
//! |---|---|---|
//! | iOS, macOS | AVAudioEngine and a pool of AVAudioPlayerNodes (iOS in the `.ambient` session) | the node repeats its buffer |
//! | Android | SoundPool with game audio attributes | the stream repeats |
//! | web | Web Audio | the source node repeats its buffer |
//! | Windows | XAudio2 | the voice repeats its buffer |
//! | Linux | libcanberra, loaded at run time | emulated: a libpulse stream fed the clip round and round, or a long file of repeats |
//! | HarmonyOS | SoundPool with the game stream usage | the stream repeats |
//!
//! Like haptics, every call is fire-and-forget and best effort: none returns an error or panics. A
//! clip that is missing or unreadable is logged once and stays silent, and so does a platform with
//! no engine ([`is_supported`] says which). Sound mixes with other apps' audio rather than
//! interrupting it, and on iOS the Silent switch silences it.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::marker::PhantomData;
use std::sync::{Mutex, MutexGuard};

pub use day_spec::AssetName;

/// At most this many clips sound at once. Past it, the oldest voice with the lowest priority stops
/// for the new clip.
pub const VOICES: usize = 8;

/// At most this many loops sound at once ([`play_loop`]). Starting another stops the oldest. Loops
/// have voices of their own, so they neither take one of the [`VOICES`] nor lose theirs to a clip.
pub const LOOPS: usize = 4;

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
    /// The loops asked for and not yet stopped, oldest first.
    loops: Vec<Looping>,
    /// The id the last loop was given; ids are never reused while the process lives.
    next_loop: i32,
}

/// One loop: what the app asked for, and what the engine was last told.
struct Looping {
    id: i32,
    path: String,
    volume: f32,
    pan: f32,
    /// The volume and pan the engine is sounding it at, or `None` while it is silent (sound is
    /// off, or the volume came to zero). [`retune`] moves this toward what the app asked for.
    told: Option<(f32, f32)>,
}

static STATE: Mutex<State> = Mutex::new(State {
    enabled: true,
    volume: 1.0,
    last: BTreeMap::new(),
    played: VecDeque::new(),
    warned: BTreeSet::new(),
    loops: Vec::new(),
    next_loop: 0,
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

/// The master volume, 0 to 1, that every play is scaled by. A loop already sounding follows it.
pub fn set_volume(volume: f32) {
    if volume.is_finite() {
        state().volume = volume.clamp(0.0, 1.0);
        retune();
    }
}

pub fn volume() -> f32 {
    state().volume
}

/// The app's sound switch: while off, [`play`] does nothing and every loop is silent. A loop
/// started or still held while sound is off begins when it comes back on. On by default.
pub fn set_enabled(on: bool) {
    state().enabled = on;
    retune();
}

pub fn is_enabled() -> bool {
    state().enabled
}

/// Stop everything playing, loops included, and release every loaded clip (a game's page
/// closing). A [`Loop`] still held afterwards is spent: it reports that it is not playing, and
/// dropping it does nothing.
pub fn unload_all() {
    imp::unload_all();
    let mut s = state();
    s.last.clear();
    s.loops.clear();
}

/// The clips most recently played, oldest first: what a test or a walkthrough log checks. A clip
/// counts here once it passed the switch, the volume and the repeat check, whether or not the
/// platform could sound it.
pub fn recent() -> Vec<String> {
    state().played.iter().cloned().collect()
}

/// A clip repeating until it is stopped ([`play_loop`]). Dropping the handle stops it, so a loop
/// lasts exactly as long as whatever owns it: a field of a game's state, a value in an `Option`
/// that a pause sets to `None`.
///
/// The handle belongs to the UI thread, where the engines live, so it is neither `Send` nor
/// `Sync`.
#[must_use = "a loop stops as soon as its handle is dropped"]
#[derive(Debug)]
pub struct Loop {
    id: i32,
    _ui_thread: PhantomData<*const ()>,
}

impl Loop {
    /// Change the loop's volume, 0 to 1, scaled by the master [`volume`]. At zero the loop is
    /// silent but still held, and a later volume starts it again from the top of the clip.
    pub fn set_volume(&self, volume: f32) {
        if !volume.is_finite() {
            return;
        }
        if let Some(l) = state().loops.iter_mut().find(|l| l.id == self.id) {
            l.volume = volume.clamp(0.0, 1.0);
        }
        retune();
    }

    /// Move the loop across the stereo field, −1 (left) to 1 (right). Best effort, like
    /// [`Play::pan`]: Windows and Linux play every loop centered.
    pub fn set_pan(&self, pan: f32) {
        if !pan.is_finite() {
            return;
        }
        if let Some(l) = state().loops.iter_mut().find(|l| l.id == self.id) {
            l.pan = pan.clamp(-1.0, 1.0);
        }
        retune();
    }

    /// Whether the loop is still held: not stopped by [`unload_all`], and not displaced by a
    /// newer loop past [`LOOPS`]. It says nothing about whether a speaker is sounding it.
    pub fn is_playing(&self) -> bool {
        state().loops.iter().any(|l| l.id == self.id)
    }

    /// Stop the loop now. The same as dropping the handle, for a call site that reads better
    /// saying so.
    pub fn stop(self) {}
}

impl Drop for Loop {
    fn drop(&mut self) {
        let removed = {
            let mut s = state();
            s.loops
                .iter()
                .position(|l| l.id == self.id)
                .map(|i| s.loops.remove(i))
        };
        if let Some(l) = removed
            && l.told.is_some()
        {
            imp::loop_stop(l.id);
        }
    }
}

/// Play `clip` over and over until the returned [`Loop`] is dropped. `opts` gives the volume
/// and the pan; a loop has no priority, because loops have voices of their own ([`LOOPS`]) that
/// no other clip can take.
///
/// The clip repeats from its last sample straight to its first, so author it to close: whole
/// cycles of a tone, or a recording cut where the level and the slope match. A loop obeys the
/// master [`volume`] and the [`set_enabled`] switch as they change. Where clips load in the
/// background (Android, HarmonyOS, the web) a loop asked for early starts once its clip is
/// ready; a one-shot would have been skipped, but a loop is still wanted then.
pub fn play_loop(clip: &AssetName, opts: Play) -> Loop {
    let path = clip.as_str();
    let volume = if opts.volume.is_finite() {
        opts.volume.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let pan = if opts.pan.is_finite() {
        opts.pan.clamp(-1.0, 1.0)
    } else {
        0.0
    };
    let (id, displaced) = {
        let mut s = state();
        s.next_loop = s.next_loop.checked_add(1).unwrap_or(1);
        let id = s.next_loop;
        let displaced = (s.loops.len() >= LOOPS).then(|| s.loops.remove(0));
        s.loops.push(Looping {
            id,
            path: path.to_owned(),
            volume,
            pan,
            told: None,
        });
        (id, displaced)
    };
    if let Some(old) = displaced
        && old.told.is_some()
    {
        imp::loop_stop(old.id);
    }
    log::debug!(target: "day_part_sound", "loop {path}");
    retune();
    Loop {
        id,
        _ui_thread: PhantomData,
    }
}

/// The clips looping now, oldest first: what a test or a walkthrough checks. A loop counts here
/// from [`play_loop`] until it stops, whether or not the platform could sound it.
pub fn loops() -> Vec<String> {
    state().loops.iter().map(|l| l.path.clone()).collect()
}

/// What an engine is asked to do to one loop.
enum Tune {
    Start(i32, String, f32, f32),
    Set(i32, f32, f32),
    Stop(i32),
}

/// Bring every loop's engine voice in line with what is wanted now: the app's volume for it
/// times the master volume, or silence while sound is off. A loop whose volume comes to zero is
/// stopped in the engine rather than played at nothing, and started again when it has a volume.
fn retune() {
    let tunes: Vec<Tune> = {
        let mut s = state();
        let (enabled, master) = (s.enabled, s.volume);
        let mut tunes = Vec::new();
        for l in &mut s.loops {
            let volume = if enabled {
                (l.volume * master).min(1.0)
            } else {
                0.0
            };
            let want = (volume > 0.0).then_some((volume, l.pan));
            match (l.told, want) {
                (None, Some((v, p))) => tunes.push(Tune::Start(l.id, l.path.clone(), v, p)),
                (Some(_), None) => tunes.push(Tune::Stop(l.id)),
                (Some(was), Some((v, p))) if was != (v, p) => tunes.push(Tune::Set(l.id, v, p)),
                _ => {}
            }
            l.told = want;
        }
        tunes
    };
    // The engines are called with the lock released, as `play_with` calls them.
    for tune in tunes {
        match tune {
            Tune::Start(id, path, volume, pan) => imp::loop_start(id, &path, volume, pan),
            Tune::Set(id, volume, pan) => imp::loop_set(id, volume, pan),
            Tune::Stop(id) => imp::loop_stop(id),
        }
    }
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

/// A loop for an engine that can only play a file from its start to its end (libcanberra, where
/// Linux has no libpulse to stream through): the clip laid end to end in one long file, so every
/// repeat inside it is sample-exact, and the engine is asked for the file again only when it
/// runs out.
#[cfg(any(all(target_os = "linux", not(target_env = "ohos")), test))]
mod unrolled {
    /// The file runs at least this long, in whole repeats of the clip. Longer means the engine
    /// is asked again less often; shorter means a smaller file in the cache.
    pub const SECS: usize = 20;

    /// The file fades in over this long and out over this long, and the next playing of it
    /// starts this long before the last one ends, so the two cross over without a click however
    /// late the engine starts the new one.
    pub const EDGE_MS: usize = 80;

    /// `samples` (mono, at `rate`) as that file: canonical 16-bit PCM WAV bytes, with how many
    /// frames it holds and how many of them each fade covers. `None` for an empty clip.
    pub fn file(samples: &[f32], rate: u32) -> Option<(Vec<u8>, usize, usize)> {
        if samples.is_empty() || rate == 0 {
            return None;
        }
        let repeats = (SECS * rate as usize).div_ceil(samples.len()).max(1);
        let frames = samples.len() * repeats;
        // A clip too short to hold both fades gets shorter ones.
        let edge = (EDGE_MS * rate as usize / 1000).min(frames / 4);
        let bytes = u32::try_from(frames * 2).ok()?;
        let mut out = Vec::with_capacity(44 + frames * 2);
        out.extend(b"RIFF");
        out.extend((36 + bytes).to_le_bytes());
        out.extend(b"WAVEfmt ");
        out.extend(16u32.to_le_bytes());
        out.extend(1u16.to_le_bytes()); // PCM
        out.extend(1u16.to_le_bytes()); // mono
        out.extend(rate.to_le_bytes());
        out.extend((rate * 2).to_le_bytes());
        out.extend(2u16.to_le_bytes());
        out.extend(16u16.to_le_bytes());
        out.extend(b"data");
        out.extend(bytes.to_le_bytes());
        for i in 0..frames {
            let gain = if edge == 0 {
                1.0
            } else if i < edge {
                i as f32 / edge as f32
            } else if i >= frames - edge {
                (frames - 1 - i) as f32 / edge as f32
            } else {
                1.0
            };
            let sample = samples[i % samples.len()] * gain;
            out.extend(((sample * 32_767.0) as i16).to_le_bytes());
        }
        Some((out, frames, edge))
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
// Per-platform engines. Each exposes `supported`, `preload`, `play`, `unload_all`, and the three
// loop calls `loop_start`, `loop_set` and `loop_stop`, which name a loop by the id `play_loop`
// gave it. `loop_stop` runs from `Loop`'s destructor, so it must be safe during thread teardown.
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

    pub fn loop_start(id: i32, path: &str, volume: f32, pan: f32) {
        super::loop_start_native(id, path, volume, pan);
    }

    pub fn loop_set(id: i32, volume: f32, pan: f32) {
        super::loop_set_native(id, volume, pan);
    }

    pub fn loop_stop(id: i32) {
        super::loop_stop_native(id);
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
        /// Repeat the clip as loop `id` until `loop_stop_native(id)`, starting once it is loaded.
        fn loop_start_native(id: i32, path: &str, volume: f32, pan: f32);
        fn loop_set_native(id: i32, volume: f32, pan: f32);
        fn loop_stop_native(id: i32);
        /// Whether the engine can be reached at all.
        fn ready_native() -> Result<bool, day_bridge::Error>;
    }

    // Android: SoundPool, the platform's engine for short, low-latency clips. It decodes each clip
    // into memory once and mixes up to `maxStreams` of them, stopping the lowest priority first.
    // Clips come straight from the APK's assets (data assets are its `assets/` root), which aapt
    // stores uncompressed for `.wav`, as `openFd` needs. Nothing here is reachable from Rust.
    //
    // A loop is a stream played with `loop = -1` at the highest priority, so no one-shot takes its
    // stream; the pool has `VOICES + LOOPS` streams for the same reason. SoundPool keeps playing
    // while the app is in the background, which a one-shot outlives by a moment and a loop by
    // however long the app stays there, so loop streams pause with the activity and resume with it.
    #[day_bridge::impl(java, platforms = [android])]
    java!(
        prelude = r#"
            import android.app.Activity;
            import android.app.Application;
            import android.content.Context;
            import android.content.res.AssetFileDescriptor;
            import android.media.AudioAttributes;
            import android.media.SoundPool;
            import android.os.Bundle;
            import android.util.Log;
            import dev.daybrite.day.bridge.DayBridge;
            import java.util.HashMap;
            import java.util.HashSet;
        "#,
        body = r#"
            private static SoundPool pool = null;
            private static final HashMap<String, Integer> ids = new HashMap<>();
            private static final HashSet<Integer> loaded = new HashSet<>();

            /** One loop: what was asked for, and the stream sounding it (0 until its clip loads). */
            private static final class Looping {
                String path;
                float volume;
                float pan;
                int stream;
            }

            private static final HashMap<Integer, Looping> loops = new HashMap<>();
            private static boolean watching = false;

            private static SoundPool pool() {
                if (pool == null) {
                    AudioAttributes attrs = new AudioAttributes.Builder()
                            .setUsage(AudioAttributes.USAGE_GAME)
                            .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION)
                            .build();
                    // Eight streams for clips and four for loops (VOICES + LOOPS).
                    pool = new SoundPool.Builder().setMaxStreams(12).setAudioAttributes(attrs).build();
                    // Loading is asynchronous; a clip plays only once SoundPool says it is ready.
                    pool.setOnLoadCompleteListener((p, id, status) -> {
                        if (status == 0) {
                            synchronized (loaded) {
                                loaded.add(id);
                            }
                            // A loop asked for before its clip was ready starts now.
                            synchronized (loops) {
                                for (Looping l : loops.values()) {
                                    Integer sound = ids.get(l.path);
                                    if (sound != null && sound == id) {
                                        start(l);
                                    }
                                }
                            }
                        }
                    });
                    watch();
                }
                return pool;
            }

            /** Pause the loops while no activity is in front, and resume them when one is. */
            private static void watch() {
                Context ctx = DayBridge.ctx;
                if (watching || ctx == null || !(ctx.getApplicationContext() instanceof Application)) {
                    return;
                }
                watching = true;
                ((Application) ctx.getApplicationContext()).registerActivityLifecycleCallbacks(
                        new Application.ActivityLifecycleCallbacks() {
                            @Override public void onActivityCreated(Activity a, Bundle b) {}
                            @Override public void onActivityStarted(Activity a) {}
                            @Override public void onActivityStopped(Activity a) {}
                            @Override public void onActivitySaveInstanceState(Activity a, Bundle b) {}
                            @Override public void onActivityDestroyed(Activity a) {}

                            @Override public void onActivityResumed(Activity a) {
                                synchronized (loops) {
                                    for (Looping l : loops.values()) {
                                        if (l.stream != 0 && pool != null) {
                                            pool.resume(l.stream);
                                        }
                                    }
                                }
                            }

                            @Override public void onActivityPaused(Activity a) {
                                synchronized (loops) {
                                    for (Looping l : loops.values()) {
                                        if (l.stream != 0 && pool != null) {
                                            pool.pause(l.stream);
                                        }
                                    }
                                }
                            }
                        });
            }

            /** Start `l`'s stream if its clip has loaded and it has none yet. Call with `loops` held. */
            private static void start(Looping l) {
                Integer sound = ids.get(l.path);
                if (pool == null || sound == null || sound == 0 || l.stream != 0) {
                    return;
                }
                boolean ready;
                synchronized (loaded) {
                    ready = loaded.contains(sound);
                }
                if (!ready) {
                    return;
                }
                float left = l.volume * Math.min(1f, 1f - l.pan);
                float right = l.volume * Math.min(1f, 1f + l.pan);
                l.stream = pool.play(sound, left, right, Integer.MAX_VALUE, -1, 1f);
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

            public static void loop_start_native(int id, String path, float volume, float pan) {
                Looping l = new Looping();
                l.path = path;
                l.volume = volume;
                l.pan = pan;
                // Loading it (when nothing has yet) ends in the listener above, which starts it.
                preload_native(path);
                synchronized (loops) {
                    loops.put(id, l);
                    start(l);
                }
            }

            public static void loop_set_native(int id, float volume, float pan) {
                synchronized (loops) {
                    Looping l = loops.get(id);
                    if (l == null) {
                        return;
                    }
                    l.volume = volume;
                    l.pan = pan;
                    if (l.stream != 0 && pool != null) {
                        pool.setVolume(l.stream, volume * Math.min(1f, 1f - pan), volume * Math.min(1f, 1f + pan));
                    }
                }
            }

            public static void loop_stop_native(int id) {
                synchronized (loops) {
                    Looping l = loops.remove(id);
                    if (l != null && l.stream != 0 && pool != null) {
                        pool.stop(l.stream);
                    }
                }
            }

            public static void unload_native() {
                synchronized (loops) {
                    for (Looping l : loops.values()) {
                        if (l.stream != 0 && pool != null) {
                            pool.stop(l.stream);
                        }
                    }
                    loops.clear();
                }
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
    //
    // A loop is a stream played with `loop: -1`. `play` answers with the stream id later, so a loop
    // stopped or retuned before the answer arrives is settled when it does.
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

            // One loop: what was asked for, and the stream sounding it (0 until `play` answers).
            class DayLooping {
                path: string = '';
                volume: number = 0;
                pan: number = 0;
                stream: number = 0;
                starting: boolean = false;
            }

            const dayLoops: Map<number, DayLooping> = new Map<number, DayLooping>();

            // Start loop `id` if its clip has loaded and it has no stream yet.
            function dayStartLoop(id: number): void {
                const l = dayLoops.get(id);
                const pool = dayPool;
                if (!l || !pool || l.stream !== 0 || l.starting) {
                    return;
                }
                const sound = dayIds.get(l.path);
                if (sound === undefined) {
                    return;
                }
                const params: media.PlayParameters = {
                    loop: -1,
                    rate: 0,
                    leftVolume: l.volume * Math.min(1, 1 - l.pan),
                    rightVolume: l.volume * Math.min(1, 1 + l.pan),
                    priority: 100,
                };
                l.starting = true;
                pool.play(sound, params).then((stream: number) => {
                    l.starting = false;
                    if (dayLoops.get(id) !== l) {
                        // Stopped while `play` was on its way.
                        pool.stop(stream);
                        return;
                    }
                    l.stream = stream;
                    // The volume may have moved since `params` was read.
                    pool.setVolume(stream, l.volume * Math.min(1, 1 - l.pan), l.volume * Math.min(1, 1 + l.pan));
                }).catch((e: Error) => {
                    l.starting = false;
                    console.warn(`day-part-sound: ${l.path}: ${e.message}`);
                });
            }

            function dayMakePool(): Promise<media.SoundPool> {
                if (!dayPoolMaking) {
                    const info: audio.AudioRendererInfo = {
                        usage: audio.StreamUsage.STREAM_USAGE_GAME,
                        rendererFlags: 1,
                    };
                    // Eight streams for clips and four for loops (VOICES + LOOPS).
                    dayPoolMaking = media.createSoundPool(12, info).then((pool: media.SoundPool) => {
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
                    // A loop asked for before its clip was ready starts now.
                    dayLoops.forEach((l: DayLooping, loop: number) => {
                        if (l.path === path) {
                            dayStartLoop(loop);
                        }
                    });
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

            export function loop_start_native(id: number, path: string, volume: number, pan: number): void {
                const l = new DayLooping();
                l.path = path;
                l.volume = volume;
                l.pan = pan;
                dayLoops.set(id, l);
                if (dayIds.has(path) && dayPool) {
                    dayStartLoop(id);
                } else {
                    // Loading it ends in `preload_native`'s continuation, which starts it.
                    preload_native(path);
                }
            }

            export function loop_set_native(id: number, volume: number, pan: number): void {
                const l = dayLoops.get(id);
                if (!l) {
                    return;
                }
                l.volume = volume;
                l.pan = pan;
                if (l.stream !== 0 && dayPool) {
                    dayPool.setVolume(l.stream, volume * Math.min(1, 1 - pan), volume * Math.min(1, 1 + pan));
                }
            }

            export function loop_stop_native(id: number): void {
                const l = dayLoops.get(id);
                dayLoops.delete(id);
                if (l && l.stream !== 0 && dayPool) {
                    dayPool.stop(l.stream);
                }
            }

            export function unload_native(): void {
                const pool = dayPool;
                if (pool) {
                    dayLoops.forEach((l: DayLooping) => {
                        if (l.stream !== 0) {
                            pool.stop(l.stream);
                        }
                    });
                    dayIds.forEach((id: number) => {
                        pool.unload(id);
                    });
                }
                dayLoops.clear();
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
    //
    // A loop is a source node with `loop` set, kept with its gain and panner so its volume and pan
    // can move. A page keeps its audio when its tab is hidden, so while a loop is sounding the
    // context is suspended with the tab and resumed with it.
    #[day_bridge::impl(js, platforms = [web])]
    js!(r#"
        const dayBuffers = new Map();
        const dayPending = new Set();
        const dayLast = new Map();
        const dayVoices = [];
        const dayLoops = new Map();
        let dayCtx = null;
        let dayHidden = false;

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
                document.addEventListener("visibilitychange", () => {
                    if (document.hidden && dayLoops.size > 0 && dayCtx.state === "running") {
                        dayHidden = true;
                        dayCtx.suspend();
                    } else if (!document.hidden && dayHidden) {
                        dayHidden = false;
                        dayCtx.resume();
                    }
                });
            }
            return dayCtx;
        }

        // Route `source` to the speakers through a gain and, where the browser has one, a panner.
        function dayRoute(ctx, source, volume, pan) {
            const gain = ctx.createGain();
            gain.gain.value = volume;
            source.connect(gain);
            let panner = null;
            if (ctx.createStereoPanner) {
                panner = ctx.createStereoPanner();
                panner.pan.value = pan;
                gain.connect(panner);
                panner.connect(ctx.destination);
            } else {
                gain.connect(ctx.destination);
            }
            return { gain, panner };
        }

        // Start loop `id` if its clip has been decoded and it has no source yet.
        function dayStartLoop(id) {
            const ctx = dayContext();
            const l = dayLoops.get(id);
            if (!ctx || !l || l.source) {
                return;
            }
            const buffer = dayBuffers.get(l.path);
            if (!buffer) {
                return;
            }
            if (ctx.state !== "running" && !dayHidden) {
                ctx.resume();
            }
            const source = ctx.createBufferSource();
            source.buffer = buffer;
            source.loop = true;
            const route = dayRoute(ctx, source, l.volume, l.pan);
            l.source = source;
            l.gain = route.gain;
            l.panner = route.panner;
            source.start();
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
                .then((buffer) => {
                    dayBuffers.set(path, buffer);
                    // A loop asked for before its clip was decoded starts now.
                    for (const [id, l] of dayLoops) {
                        if (l.path === path) {
                            dayStartLoop(id);
                        }
                    }
                })
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
            dayRoute(ctx, source, volume, pan);
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

        export function loop_start_native(id, path, volume, pan) {
            dayLoops.set(id, { path, volume, pan, source: null, gain: null, panner: null });
            if (dayBuffers.has(path)) {
                dayStartLoop(id);
            } else {
                // Decoding it ends in `preload_native`'s continuation, which starts it.
                preload_native(path);
            }
        }

        export function loop_set_native(id, volume, pan) {
            const l = dayLoops.get(id);
            if (!l) {
                return;
            }
            l.volume = volume;
            l.pan = pan;
            if (l.gain) {
                l.gain.gain.value = volume;
            }
            if (l.panner) {
                l.panner.pan.value = pan;
            }
        }

        export function loop_stop_native(id) {
            const l = dayLoops.get(id);
            dayLoops.delete(id);
            if (l && l.source) {
                l.source.stop();
            }
        }

        export function unload_native() {
            for (const voice of dayVoices) {
                voice.source.stop();
            }
            dayVoices.length = 0;
            for (const l of dayLoops.values()) {
                if (l.source) {
                    l.source.stop();
                }
            }
            dayLoops.clear();
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
    fn loop_start_native(_id: i32, _path: &str, _volume: f32, _pan: f32) {}

    #[day_bridge::impl(rust, platforms = [other])]
    fn loop_set_native(_id: i32, _volume: f32, _pan: f32) {}

    #[day_bridge::impl(rust, platforms = [other])]
    fn loop_stop_native(_id: i32) {}

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

    /// A WAV file around `data`: a `fmt ` chunk (`EXTENSIBLE` when asked), an odd-length `LIST`
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

    #[test]
    fn an_unrolled_loop_repeats_the_clip_exactly_and_fades_only_at_its_ends() {
        // A 4-sample clip at 1 kHz: 20 s of it is 5,000 repeats, and each fade is 80 frames.
        let clip = [0.5f32, 1.0, -0.5, -1.0];
        let (bytes, frames, edge) = unrolled::file(&clip, 1_000).unwrap();
        assert_eq!((frames, edge), (20_000, 80));
        let pcm = wav::decode(&bytes).unwrap();
        assert_eq!((pcm.rate, pcm.samples.len()), (1_000, 20_000));
        // Past the fade-in, every repeat is the clip itself.
        assert!(
            close(&pcm.samples[80..84], &clip),
            "{:?}",
            &pcm.samples[80..84]
        );
        assert!(close(&pcm.samples[10_000..10_004], &clip));
        // The ends are silent and the fades are monotonic in level.
        assert_eq!(pcm.samples[0], 0.0);
        assert!(pcm.samples[frames - 1].abs() < 1e-3);
        assert!(pcm.samples[41].abs() < pcm.samples[81].abs());
        // A clip longer than the target is played once; an empty one has no file.
        let long = vec![0.25f32; 30_000];
        assert_eq!(unrolled::file(&long, 1_000).unwrap().1, 30_000);
        assert!(unrolled::file(&[], 44_100).is_none());
        // A clip too short for both fades still gets a file, with shorter fades.
        let (_, frames, edge) = unrolled::file(&[0.1; 3], 1).unwrap();
        assert_eq!((frames, edge), (21, 0));
    }

    /// The plays of this test's own clips among [`recent`], which every test shares.
    fn mine(names: &[&AssetName]) -> usize {
        recent()
            .iter()
            .filter(|p| names.iter().any(|n| n.as_str() == p.as_str()))
            .count()
    }

    /// The two tests below drive the one process-wide [`State`], and `cargo test` runs them on
    /// separate threads at once. [`unload_all`] clears the repeat bookkeeping, so landing between
    /// the two plays of the repeat check it lets the second play count, and the check reads a
    /// skipped repeat as played, which is how CI's Linux leg failed on 2026-09-11. Both hold this
    /// lock, so neither sees the other's edits to that state.
    static SHARED: Mutex<()> = Mutex::new(());

    /// Hold [`SHARED`] for the rest of the test. Poisoning is ignored, as it is for [`STATE`]: a
    /// failing test should report itself rather than turn every later one into a poison error.
    fn exclusive() -> MutexGuard<'static, ()> {
        SHARED.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn plays_pass_the_switch_the_volume_and_the_repeat_check() {
        let _shared = exclusive();
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

    #[test]
    fn loops_last_as_long_as_their_handles_and_follow_the_switch_and_the_volume() {
        let _shared = exclusive();
        let hum = AssetName::from_static("sounds/test-hum.wav");
        let siren = AssetName::from_static("sounds/test-siren.wav");
        let told = |path: &str| {
            state()
                .loops
                .iter()
                .find(|l| l.path == path)
                .and_then(|l| l.told)
        };
        assert!(loops().is_empty());
        let a = play_loop(&hum, Play::at(0.5));
        assert_eq!(loops(), vec!["sounds/test-hum.wav"]);
        assert!(a.is_playing());
        assert_eq!(told(hum.as_str()), Some((0.5, 0.0)));
        assert_eq!(mine(&[&hum]), 0, "a loop is not a play");

        // The handle's volume and pan, then the master volume, reach the engine.
        a.set_volume(0.8);
        a.set_pan(-2.0);
        assert_eq!(told(hum.as_str()), Some((0.8, -1.0)));
        a.set_volume(f32::NAN);
        assert_eq!(
            told(hum.as_str()),
            Some((0.8, -1.0)),
            "a NaN volume is ignored"
        );
        set_volume(0.5);
        assert_eq!(told(hum.as_str()), Some((0.4, -1.0)));
        set_volume(1.0);

        // Silence (the switch, or a zero volume) stops the engine's voice and keeps the loop.
        set_enabled(false);
        assert_eq!(told(hum.as_str()), None);
        let b = play_loop(&siren, Play::default());
        assert_eq!(
            told(siren.as_str()),
            None,
            "started silent while sound is off"
        );
        assert_eq!(loops().len(), 2);
        set_enabled(true);
        assert_eq!(told(hum.as_str()), Some((0.8, -1.0)));
        assert_eq!(told(siren.as_str()), Some((1.0, 0.0)));
        a.set_volume(0.0);
        assert_eq!(told(hum.as_str()), None);
        assert!(a.is_playing(), "silent, and still held");

        // Dropping a handle stops its loop and no other.
        drop(a);
        assert_eq!(loops(), vec!["sounds/test-siren.wav"]);
        b.stop();
        assert!(loops().is_empty());
    }

    #[test]
    fn a_loop_past_the_limit_displaces_the_oldest_and_unloading_spends_every_handle() {
        let _shared = exclusive();
        let names = ["a", "b", "c", "d", "e"].map(|n| format!("sounds/test-loop-{n}.wav"));
        let held: Vec<Loop> = names
            .iter()
            .map(|n| play_loop(&AssetName::dynamic(n.clone()), Play::default()))
            .collect();
        assert_eq!(loops(), names[1..].to_vec(), "the first of five made room");
        assert!(!held[0].is_playing());
        assert!(held[4].is_playing());
        // Ids are not reused, so a spent handle never stops a later loop.
        drop(held);
        assert!(loops().is_empty());

        let kept = play_loop(&AssetName::dynamic(names[0].clone()), Play::default());
        unload_all();
        assert!(loops().is_empty());
        assert!(!kept.is_playing());
        let next = play_loop(&AssetName::dynamic(names[1].clone()), Play::default());
        drop(kept);
        assert_eq!(
            loops(),
            vec![names[1].clone()],
            "the spent handle stopped nothing"
        );
        drop(next);
    }

    // Every entry point must be safe to call on any host, with or without an engine.
    #[test]
    fn nothing_panics() {
        let _shared = exclusive();
        let _ = is_supported();
        preload(&[AssetName::from_static("sounds/missing.wav")]);
        play(&AssetName::from_static("sounds/missing.wav"));
        let missing = play_loop(&AssetName::from_static("sounds/missing.wav"), Play::at(0.3));
        missing.set_volume(0.6);
        missing.set_pan(0.5);
        unload_all();
        drop(missing);
    }
}
