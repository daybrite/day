//! day-part-permissions — a HEADLESS portable API for the OS PERMISSION system. No UI; any Rust
//! code can depend on this crate to ask what the OS will do, and to ask the OS itself.
//!
//! ```no_run
//! use day_part_permissions::{Permission, Status, request, status};
//! if status(Permission::Camera) != Status::Granted {
//!     request(Permission::Camera, |s| println!("camera: {s}"));
//! }
//! ```
//!
//! Platform selection is purely `#[cfg(target_os)]`/`#[cfg(target_env)]` (consent is an OS concern,
//! not a widget-toolkit one): Apple platforms use the per-framework authorization APIs
//! (`CLLocationManager`, `AVCaptureDevice`, `PHPhotoLibrary`, `UNUserNotificationCenter`,
//! `CMMotionActivityManager`), Android `Context.checkSelfPermission` plus `requestPermissions` from
//! a headless `Fragment`, HarmonyOS `OH_AT_CheckSelfPermission`, and the web
//! `navigator.permissions` with the per-API request calls. Desktop Linux and Windows have no
//! consent database, so every gated capability there answers [`Gate::Ungated`] / [`Status::Granted`].
//!
//! # Two questions, two vocabularies
//!
//! [`gate`] answers *"does this target gate the capability at all?"* and [`status`] answers *"what
//! will the OS do if I use it now?"*. Keeping them apart is what lets desktop Linux answer
//! [`Status::Granted`] for the camera — nothing stands in your way, so proceed, and let the real
//! failure surface at `open("/dev/video0")` — without losing the structural fact that Linux has no
//! permission system ([`Gate::Ungated`]).
//!
//! # Declare before you ask
//!
//! Every mobile OS also requires a BUILD-TIME declaration, and Day generates those from the
//! `[permissions]` table in `Day.toml` (docs/permissions.md). This matters more than it sounds:
//! an undeclared permission reports [`Status::Restricted`] on Android, and on iOS/macOS touching a
//! gated API without its `NS…UsageDescription` key **terminates the process**.
//!
//! # Reasons are not a runtime parameter
//!
//! No platform accepts one. iOS and macOS read `NS…UsageDescription` from `Info.plist`;
//! `requestPermissions(String[], int)` and `requestPermissionsFromUser(context, string[])` take no
//! text, and neither does `getUserMedia` or `Notification.requestPermission`. So the reason lives in
//! the declaration, where it reaches the OS, and this crate hands you [`should_show_rationale`] and
//! [`can_prompt`] so your app can draw its own explanation first, in its own localized copy.
//!
//! # Threading and cancellation
//!
//! [`request`]'s completion runs on an unspecified thread — possibly the UI thread (Android's
//! `onRequestPermissionsResult`, the browser's only thread) — so deliver results with a
//! `day_reactive::Setter`, the way `day-part-http` does. There is no blocking `request`: the OS
//! prompt is drawn by the very thread a blocking call would park, so it would deadlock by
//! construction. And dropping a [`StatusFuture`] does NOT take the prompt off the screen — no
//! platform can do that. See [`StatusFuture`].

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard};

/// A permission to query or request.
///
/// The seven portable variants exist on iOS, Android and HarmonyOS; the per-platform tables in
/// docs/permissions.md say what each means everywhere else. For anything outside this set, use the
/// `Permission`-typed constants in the [`android`], [`ohos`], [`apple`] and [`web`] modules — each
/// is `#[cfg]`-gated to its platform, so a non-portable name is a compile error off it.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Permission {
    /// The device's location while the app is in the foreground. Apple
    /// `requestWhenInUseAuthorization` (`NSLocationWhenInUseUsageDescription`); Android
    /// `ACCESS_FINE_LOCATION` + `ACCESS_COARSE_LOCATION` requested together, which is what produces
    /// the precise/approximate dialog; HarmonyOS `APPROXIMATELY_LOCATION` + `LOCATION` (the platform
    /// rejects `LOCATION` alone); the web's `navigator.geolocation`.
    Location,
    /// Location while the app is BACKGROUNDED. Never request it first — every platform requires
    /// foreground location to be granted already, and on Android 11+ it cannot be granted from an
    /// in-app dialog at all, so [`can_prompt`] is `false` there and [`open_settings`] is the only
    /// path. Apple `requestAlwaysAuthorization`; Android `ACCESS_BACKGROUND_LOCATION`; HarmonyOS
    /// `LOCATION_IN_BACKGROUND`. The web has no such concept ([`Gate::Absent`]).
    LocationAlways,
    /// The camera. Apple `AVCaptureDevice` with `AVMediaTypeVideo`; Android `CAMERA`; HarmonyOS
    /// `ohos.permission.CAMERA`; the web's `getUserMedia({video:true})`.
    Camera,
    /// The microphone. Apple `AVCaptureDevice` with `AVMediaTypeAudio`; Android `RECORD_AUDIO`;
    /// HarmonyOS `ohos.permission.MICROPHONE`; the web's `getUserMedia({audio:true})`.
    Microphone,
    /// Posting user-visible notifications. Apple `UNUserNotificationCenter` (which needs no
    /// `Info.plist` key); Android `POST_NOTIFICATIONS` on API 33+, and `areNotificationsEnabled()`
    /// below it; the web's `Notification.requestPermission()`.
    Notifications,
    /// The photo library. Apple `PHPhotoLibrary` (its `Limited` tier reports [`Status::Granted`] —
    /// the user chose *some* photos, which is access); Android `READ_MEDIA_IMAGES` +
    /// `READ_MEDIA_VIDEO` on API 33+, else `READ_EXTERNAL_STORAGE`. HarmonyOS reaches photos
    /// through a picker that needs no permission ([`Gate::Ungated`]), and the web has no library
    /// concept at all ([`Gate::Absent`]) — use a file picker.
    Photos,
    /// Motion and fitness activity. This is NOT raw accelerometer/gyroscope access, which needs no
    /// permission on iOS or Android (docs/sensors.md) — it gates step counts and activity
    /// classification. iOS `CMMotionActivityManager` (`NSMotionUsageDescription`); Android
    /// `ACTIVITY_RECOGNITION` on API 29+; HarmonyOS `ACTIVITY_MOTION`. On the web this is the one
    /// that gates raw motion: iOS Safari's `DeviceMotionEvent.requestPermission()`. macOS has no
    /// CoreMotion ([`Gate::Absent`]).
    Motion,
    /// A permission named the way its platform names it — the escape hatch for anything the
    /// portable set doesn't cover. The string is the native id on Android
    /// (`"android.permission.BLUETOOTH_CONNECT"`) and HarmonyOS, a Permissions-API name on the web
    /// (`"clipboard-read"`), and a crate-defined `"apple.*"` id on Apple platforms, which have no
    /// permission strings of their own. Prefer the typed constants in [`android`], [`ohos`],
    /// [`apple`] and [`web`] over writing these literals.
    Raw(&'static str),
}

impl Permission {
    /// A stable ASCII id — `"location"`, `"camera"`, … or the raw string. Locale-independent, so it
    /// is safe to assert on in dayscript and to write to logs.
    pub fn as_str(self) -> &'static str {
        match self {
            Permission::Location => "location",
            Permission::LocationAlways => "location-always",
            Permission::Camera => "camera",
            Permission::Microphone => "microphone",
            Permission::Notifications => "notifications",
            Permission::Photos => "photos",
            Permission::Motion => "motion",
            Permission::Raw(s) => s,
        }
    }
}

impl std::fmt::Display for Permission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Whether the compiled target gates a capability at all — the structural question, asked
/// separately from [`status`] so that "no gate here" never has to masquerade as a denial.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    /// The OS keeps a consent record and can put a prompt on screen. [`status`] may report any of
    /// [`Status::Granted`], [`Status::Denied`], [`Status::Prompt`] or [`Status::Restricted`].
    Prompts,
    /// The capability exists and nothing gates it — desktop Linux and Windows have no consent
    /// database. [`status`] always reports [`Status::Granted`] and [`request`] resolves without a
    /// prompt.
    Ungated,
    /// The capability does not exist on this target at all. [`status`] always reports
    /// [`Status::Unsupported`].
    Absent,
}

impl Gate {
    /// A short display label (`"prompts"` / `"ungated"` / `"absent"`).
    pub fn label(self) -> &'static str {
        match self {
            Gate::Prompts => "prompts",
            Gate::Ungated => "ungated",
            Gate::Absent => "absent",
        }
    }
}

impl std::fmt::Display for Gate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// What the OS will do if the capability is used now.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Go ahead. NOT a promise that the hardware exists — a laptop with no camera still answers
    /// `Granted`, because no permission stands in the way. Ask the capability's own part (e.g.
    /// `day_part_sensors::is_available`) about hardware.
    Granted,
    /// The user said no. Whether asking again can help is [`can_prompt`]'s answer: on Apple it never
    /// can (send them to [`open_settings`]); on Android it depends on whether they checked
    /// "don't ask again".
    Denied,
    /// Nobody has decided yet — [`request`] will put a prompt on screen. The platforms spell this
    /// `notDetermined` (Apple), `'prompt'` (web), or simply "not granted and never asked".
    Prompt,
    /// Policy forbids it and the user cannot change that: iOS `restricted` (Screen Time, MDM,
    /// supervised devices), or a permission missing from the app's merged manifest on Android —
    /// a request there returns denied in the same frame and Settings has nothing to offer.
    Restricted,
    /// This target has no such capability ([`Gate::Absent`]).
    Unsupported,
    /// Not known yet. Only two situations produce it, and only from the synchronous [`status`]:
    /// the web, where `navigator.permissions.query()` is asynchronous, and Apple `Notifications`,
    /// whose settings have no synchronous accessor. Both prime a cache on first use, so this is
    /// a first-call state rather than a lasting one — and [`status_future`] never returns it.
    Unknown,
}

impl Status {
    /// Whether the capability may be used now.
    pub fn is_granted(self) -> bool {
        self == Status::Granted
    }

    /// A short display label (`"granted"`, `"denied"`, `"prompt"`, `"restricted"`,
    /// `"unsupported"`, `"unknown"`). Locale-independent — safe to assert on in dayscript.
    pub fn label(self) -> &'static str {
        match self {
            Status::Granted => "granted",
            Status::Denied => "denied",
            Status::Prompt => "prompt",
            Status::Restricted => "restricted",
            Status::Unsupported => "unsupported",
            Status::Unknown => "unknown",
        }
    }
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

// ---------------------------------------------------------------------------
// Query
// ---------------------------------------------------------------------------

/// Whether this target gates `perm` at all (fixed at compile time for most permissions; a few are
/// decided by the OS version, e.g. Android motion below API 29).
pub fn gate(perm: Permission) -> Gate {
    imp::gate(perm)
}

/// What the OS will do if `perm` is used now. Never blocks.
///
/// On the web, and for `Notifications` on Apple platforms, the platform can only answer
/// asynchronously: this returns the last value this process observed, or [`Status::Unknown`] before
/// the first observation. Use [`status_future`] when the answer must be authoritative.
pub fn status(perm: Permission) -> Status {
    imp::status(perm)
}

/// [`status`], but always authoritative — it waits for the platform's own answer where that is
/// asynchronous, so it never yields [`Status::Unknown`].
///
/// `on_done` may run BEFORE this returns: on the platforms with a synchronous answer there is
/// nothing to wait for, and the web has no other thread to defer to.
pub fn status_async(perm: Permission, on_done: impl FnOnce(Status) + Send + 'static) {
    imp::status_async(perm, Box::new(on_done));
}

/// [`status_async`] as a `Future`. Plain oneshot plumbing over the same completion — any executor
/// can await it, including a test's `block_on`.
pub fn status_future(perm: Permission) -> StatusFuture {
    let shared = new_state();
    let sink = shared.clone();
    imp::status_async(perm, Box::new(move |s| settle(&sink, s)));
    StatusFuture {
        shared,
        done: false,
    }
}

/// Whether calling [`request`] would actually put a prompt on screen right now. When this is
/// `false` the answer is already final for this launch — draw an "open Settings" path
/// ([`open_settings`]) rather than a "grant access" button.
pub fn can_prompt(perm: Permission) -> bool {
    imp::can_prompt(perm)
}

/// Whether the user has already refused once AND a further prompt is still possible — the
/// platform's signal that an explanation should come first. This is Android's
/// `shouldShowRequestPermissionRationale`; every other platform answers `false`, because Apple
/// never re-prompts and the web has no equivalent.
///
/// This crate cannot draw the explanation itself (a part must not depend on `day-pieces`), which is
/// the right split anyway: the copy is yours and belongs in your own `res::str` keys.
pub fn should_show_rationale(perm: Permission) -> bool {
    imp::should_show_rationale(perm)
}

// ---------------------------------------------------------------------------
// Request
// ---------------------------------------------------------------------------

/// Ask the OS for `perm`, showing the system prompt when [`can_prompt`] says one would appear and
/// resolving immediately with the current status when it would not.
///
/// `on_done` runs on an unspecified thread — possibly the UI thread — so capture a
/// `day_reactive::Setter` to deliver into UI state. There is deliberately no blocking form: every
/// platform draws the prompt on the thread a blocking call would park.
///
/// Concurrent requests for the same permission coalesce: the second caller joins the first, one
/// prompt appears, and both callbacks receive the same answer.
pub fn request(perm: Permission, on_done: impl FnOnce(Status) + Send + 'static) {
    request_boxed(perm, Box::new(on_done));
}

/// [`request`] as a `Future`.
///
/// Dropping it stops you listening; it does **not** take the dialog off the screen, because no
/// platform can dismiss its own permission prompt programmatically. The user's answer is still
/// recorded, so the next [`status`] is correct. (This is the one place this crate's futures differ
/// from `day-part-http`'s, whose `Drop` really does cancel the request.)
pub fn request_future(perm: Permission) -> StatusFuture {
    let shared = new_state();
    let sink = shared.clone();
    request_boxed(perm, Box::new(move |s| settle(&sink, s)));
    StatusFuture {
        shared,
        done: false,
    }
}

/// Ask for several permissions in ONE prompt sequence where the platform batches them (Android
/// submits a single array; elsewhere they are chained). Answers are positional — `out[i]` is the
/// status of `perms[i]`.
pub fn request_many(perms: &[Permission], on_done: impl FnOnce(Vec<Status>) + Send + 'static) {
    let perms = perms.to_vec();
    let n = perms.len();
    if n == 0 {
        on_done(Vec::new());
        return;
    }
    // Collect into a slot-indexed buffer; the last completion hands the whole vector over. Every
    // per-permission request already coalesces, so a duplicate in `perms` costs no extra prompt.
    let state = Arc::new(Mutex::new((
        vec![Status::Unknown; n],
        n,
        Some(Box::new(on_done) as ManyCb),
    )));
    for (i, p) in perms.into_iter().enumerate() {
        let state = state.clone();
        request_boxed(
            p,
            Box::new(move |s| {
                let finished = {
                    let mut st = lock(&state);
                    st.0[i] = s;
                    st.1 -= 1;
                    if st.1 == 0 { st.2.take() } else { None }
                };
                // Outside the lock: the callback is user code and must never run under it.
                if let Some(cb) = finished {
                    let out = std::mem::take(&mut lock(&state).0);
                    cb(out);
                }
            }),
        );
    }
}

/// [`request_many`] as a `Future`.
pub fn request_many_future(perms: &[Permission]) -> StatusesFuture {
    let shared = Arc::new(Mutex::new(AnswerState::<Vec<Status>>::default()));
    let sink = shared.clone();
    request_many(perms, move |v| settle(&sink, v));
    StatusesFuture {
        shared,
        done: false,
    }
}

/// Open the OS page where the user can change this app's permissions. `true` if a page was opened.
///
/// iOS uses `UIApplication.openSettingsURLString`, macOS the per-permission
/// `x-apple.systempreferences:` privacy anchor, Android `ACTION_APPLICATION_DETAILS_SETTINGS`, and
/// HarmonyOS the application-info ability. The web and desktop Linux/Windows have no such
/// destination and answer `false`.
pub fn open_settings(perm: Permission) -> bool {
    imp::open_settings(perm)
}

// ---------------------------------------------------------------------------
// Coalescing table
// ---------------------------------------------------------------------------

type Cb = Box<dyn FnOnce(Status) + Send>;
type ManyCb = Box<dyn FnOnce(Vec<Status>) + Send>;

/// In-flight requests, keyed by permission: a second caller joins the first rather than putting a
/// second prompt on screen (which Android answers with an empty grant array and the web answers
/// with a duplicate dialog). The crate owns this table itself, the way `day-part-http` owns its
/// cancel-token table — a part cannot use day-core's `present()` rail.
///
/// LEAF LOCK: no platform call and no user callback ever runs while it is held.
fn inflight() -> &'static Mutex<HashMap<Permission, Vec<Cb>>> {
    static INFLIGHT: std::sync::OnceLock<Mutex<HashMap<Permission, Vec<Cb>>>> =
        std::sync::OnceLock::new();
    INFLIGHT.get_or_init(|| Mutex::new(HashMap::new()))
}

fn request_boxed(perm: Permission, on_done: Cb) {
    // Fast paths that need no prompt and no table entry.
    if gate(perm) == Gate::Absent {
        on_done(Status::Unsupported);
        return;
    }
    let now = status(perm);
    if matches!(now, Status::Granted | Status::Restricted)
        || (now == Status::Denied && !can_prompt(perm))
    {
        on_done(now);
        return;
    }

    let start = {
        let mut table = lock(inflight());
        match table.get_mut(&perm) {
            Some(waiters) => {
                waiters.push(on_done);
                false
            }
            None => {
                table.insert(perm, vec![on_done]);
                true
            }
        }
    };
    if start {
        imp::request(perm, Box::new(move |s| resolve(perm, s)));
    }
}

/// Deliver `s` to everyone waiting on `perm`. Called from whatever thread the platform completes
/// on; takes the waiters out under the lock and invokes them after releasing it.
fn resolve(perm: Permission, s: Status) {
    let waiters = lock(inflight()).remove(&perm).unwrap_or_default();
    for cb in waiters {
        cb(s);
    }
}

// ---------------------------------------------------------------------------
// Futures
// ---------------------------------------------------------------------------

/// Shared state between a future and the completion that resolves it.
///
/// Locking protocol (the mutex is a LEAF lock — no platform or user code runs under it):
/// - `poll` checks the answer and stores the waker under ONE acquisition, closing the lost-wakeup
///   race a check-then-store would open.
/// - the completion stores the answer, takes the waker, UNLOCKS, then wakes — an inline waker
///   (as in tests) re-polls synchronously, which re-takes the lock.
struct AnswerState<T> {
    answer: Option<T>,
    waker: Option<std::task::Waker>,
}

impl<T> Default for AnswerState<T> {
    fn default() -> Self {
        AnswerState {
            answer: None,
            waker: None,
        }
    }
}

fn new_state() -> Arc<Mutex<AnswerState<Status>>> {
    Arc::new(Mutex::new(AnswerState::default()))
}

fn settle<T>(shared: &Arc<Mutex<AnswerState<T>>>, value: T) {
    let waker = {
        let mut st = lock(shared);
        st.answer = Some(value);
        st.waker.take()
    };
    if let Some(w) = waker {
        w.wake();
    }
}

/// Lock, riding out poisoning: a panic while holding one of this crate's locks leaves plain data
/// behind (no broken invariants), so the poisoned value is still the truth.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    match m.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// A pending permission answer, from [`status_future`] or [`request_future`].
///
/// Dropping one stops you listening. It does NOT dismiss a prompt that is already on screen — no
/// platform offers that — so the user's answer is still recorded and the next [`status`] reflects
/// it. Aborting a `day::task` that awaits one therefore leaves the dialog up.
pub struct StatusFuture {
    shared: Arc<Mutex<AnswerState<Status>>>,
    done: bool,
}

impl Future for StatusFuture {
    type Output = Status;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Status> {
        let mut st = lock(&self.shared);
        if let Some(a) = st.answer.take() {
            drop(st);
            self.done = true;
            return std::task::Poll::Ready(a);
        }
        st.waker = Some(cx.waker().clone());
        std::task::Poll::Pending
    }
}

impl Drop for StatusFuture {
    fn drop(&mut self) {
        if !self.done {
            lock(&self.shared).waker = None;
        }
    }
}

/// The [`request_many_future`] counterpart of [`StatusFuture`]; same drop semantics.
pub struct StatusesFuture {
    shared: Arc<Mutex<AnswerState<Vec<Status>>>>,
    done: bool,
}

impl Future for StatusesFuture {
    type Output = Vec<Status>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Vec<Status>> {
        let mut st = lock(&self.shared);
        if let Some(a) = st.answer.take() {
            drop(st);
            self.done = true;
            return std::task::Poll::Ready(a);
        }
        st.waker = Some(cx.waker().clone());
        std::task::Poll::Pending
    }
}

impl Drop for StatusesFuture {
    fn drop(&mut self) {
        if !self.done {
            lock(&self.shared).waker = None;
        }
    }
}

// ---------------------------------------------------------------------------
// Shared logic — compiled on EVERY target and unit-tested on any host, so the decisions most
// likely to be wrong are covered even though parts are not in the workspace's default-members.
//
// `allow(dead_code)`: each helper is called by one platform's arm (`classify_android` by Android,
// `from_web_state` by the web, `merge` by the two with one-to-many permission fan-out), so it looks
// unused on every other target. Keeping them here — rather than in the arms — is what makes them
// testable on a host that can't compile those arms at all.
// ---------------------------------------------------------------------------

/// Fold the statuses of several native permissions that one portable permission maps onto (Android
/// `Location` → `{FINE, COARSE}`). Precedence, most to least permissive:
/// `Granted > Prompt > Denied > Restricted > Unsupported > Unknown`.
///
/// `Granted` wins because coarse-only location is still location; `Prompt` outranks `Denied` because
/// a prompt that can still be shown is the actionable state.
#[allow(dead_code)]
pub(crate) fn merge(a: Status, b: Status) -> Status {
    fn rank(s: Status) -> u8 {
        match s {
            Status::Granted => 5,
            Status::Prompt => 4,
            Status::Denied => 3,
            Status::Restricted => 2,
            Status::Unsupported => 1,
            Status::Unknown => 0,
        }
    }
    if rank(a) >= rank(b) { a } else { b }
}

/// Turn Android's three probes into a [`Status`].
///
/// `declared` is whether the permission survived into the app's MERGED manifest; without it a
/// request returns denied in the same frame and Settings cannot help, which is exactly
/// [`Status::Restricted`].
///
/// Day keeps no "already asked" state, so a denied-but-declared permission with no rationale flag
/// is reported as [`Status::Prompt`] whether the user has never been asked or has permanently
/// refused. That is safe — asking after a permanent refusal shows no dialog and resolves `Denied`
/// immediately — and an app that needs the distinction should record it when it calls
/// [`request`] (docs/permissions.md shows the three-line `day-part-prefs` recipe).
#[allow(dead_code)]
pub(crate) fn classify_android(granted: bool, declared: bool, rationale: bool) -> Status {
    match (granted, declared, rationale) {
        (true, _, _) => Status::Granted,
        (false, false, _) => Status::Restricted,
        (false, true, true) => Status::Denied,
        (false, true, false) => Status::Prompt,
    }
}

/// Map a browser `PermissionStatus.state` string. Unknown values are `Prompt`, not `Unsupported`:
/// a browser that doesn't recognize `{name:'camera'}` in `permissions.query` can still show the
/// prompt through `getUserMedia`, so the actionable answer is "ask".
#[allow(dead_code)]
pub(crate) fn from_web_state(state: &str) -> Status {
    match state {
        "granted" => Status::Granted,
        "denied" => Status::Denied,
        _ => Status::Prompt,
    }
}

/// Map an Apple framework authorization constant. The four frameworks agree on the low values
/// (0 notDetermined, 1 restricted, 2 denied, 3 authorized) and differ above them: `PHPhotoLibrary`
/// uses 4 for `limited` and `UNUserNotificationCenter` 4 for `provisional`, both of which mean the
/// app may proceed, and `CLLocationManager` uses 4 for `authorizedWhenInUse`.
#[allow(dead_code)]
pub(crate) fn from_apple_status(raw: i64) -> Status {
    match raw {
        0 => Status::Prompt,
        1 => Status::Restricted,
        2 => Status::Denied,
        3 | 4 => Status::Granted,
        _ => Status::Unknown,
    }
}

// ---------------------------------------------------------------------------
// Per-OS implementations. Each exposes `gate`, `status`, `status_async`, `can_prompt`,
// `should_show_rationale`, `request` and `open_settings`.
// ---------------------------------------------------------------------------

#[cfg(any(target_os = "macos", target_os = "ios"))]
#[path = "apple.rs"]
mod imp;

#[cfg(target_os = "android")]
#[path = "android.rs"]
mod imp;

// Desktop/embedded Linux has no consent database; HarmonyOS (also `target_os = "linux"`) has one.
#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
#[path = "defaults.rs"]
mod imp;

#[cfg(all(target_os = "linux", target_env = "ohos"))]
#[path = "ohos.rs"]
mod imp;

#[cfg(target_os = "windows")]
#[path = "defaults.rs"]
mod imp;

#[cfg(target_arch = "wasm32")]
#[path = "web.rs"]
mod imp;

// Any other platform: no permission system, and no capability either — the honest answer is that
// nothing here is known to work.
#[cfg(not(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "windows",
    target_os = "linux",
    target_os = "android",
    target_arch = "wasm32"
)))]
mod imp {
    use super::{Gate, Permission, Status};

    pub fn gate(_perm: Permission) -> Gate {
        Gate::Absent
    }
    pub fn status(_perm: Permission) -> Status {
        Status::Unsupported
    }
    pub fn status_async(_perm: Permission, on_done: Box<dyn FnOnce(Status) + Send>) {
        on_done(Status::Unsupported);
    }
    pub fn can_prompt(_perm: Permission) -> bool {
        false
    }
    pub fn should_show_rationale(_perm: Permission) -> bool {
        false
    }
    pub fn request(_perm: Permission, on_done: Box<dyn FnOnce(Status) + Send>) {
        on_done(Status::Unsupported);
    }
    pub fn open_settings(_perm: Permission) -> bool {
        false
    }
}

// ---------------------------------------------------------------------------
// Platform-specific permission constants. Each module is `#[cfg]`-gated, so naming a permission
// that doesn't exist on the compiled target is a compile error rather than a silent `Unsupported`.
// ---------------------------------------------------------------------------

/// Android permission ids beyond the portable set.
#[cfg(target_os = "android")]
pub mod android {
    use super::Permission;

    pub const ACTIVITY_RECOGNITION: Permission =
        Permission::Raw("android.permission.ACTIVITY_RECOGNITION");
    pub const BLUETOOTH_CONNECT: Permission =
        Permission::Raw("android.permission.BLUETOOTH_CONNECT");
    pub const BLUETOOTH_SCAN: Permission = Permission::Raw("android.permission.BLUETOOTH_SCAN");
    pub const READ_CONTACTS: Permission = Permission::Raw("android.permission.READ_CONTACTS");
    pub const READ_CALENDAR: Permission = Permission::Raw("android.permission.READ_CALENDAR");
    pub const CALL_PHONE: Permission = Permission::Raw("android.permission.CALL_PHONE");
    pub const READ_MEDIA_VISUAL_USER_SELECTED: Permission =
        Permission::Raw("android.permission.READ_MEDIA_VISUAL_USER_SELECTED");

    /// Any other Android permission id.
    pub const fn raw(name: &'static str) -> Permission {
        Permission::Raw(name)
    }
}

/// HarmonyOS permission names beyond the portable set.
#[cfg(all(target_os = "linux", target_env = "ohos"))]
pub mod ohos {
    use super::Permission;

    pub const APPROXIMATELY_LOCATION: Permission =
        Permission::Raw("ohos.permission.APPROXIMATELY_LOCATION");
    pub const READ_CONTACTS: Permission = Permission::Raw("ohos.permission.READ_CONTACTS");
    pub const READ_CALENDAR: Permission = Permission::Raw("ohos.permission.READ_CALENDAR");
    pub const DISTRIBUTED_DATASYNC: Permission =
        Permission::Raw("ohos.permission.DISTRIBUTED_DATASYNC");

    /// Any other HarmonyOS permission name.
    pub const fn raw(name: &'static str) -> Permission {
        Permission::Raw(name)
    }
}

/// Apple authorizations beyond the portable set.
///
/// Apple has no permission *strings* — each of these is a per-framework authorization API — so
/// these ids are this crate's own namespace, recognized by its Apple implementation.
#[cfg(any(target_os = "macos", target_os = "ios"))]
pub mod apple {
    use super::Permission;

    pub const SPEECH_RECOGNITION: Permission = Permission::Raw("apple.speech");
    pub const CONTACTS: Permission = Permission::Raw("apple.contacts");
    pub const CALENDAR: Permission = Permission::Raw("apple.calendar");
    pub const REMINDERS: Permission = Permission::Raw("apple.reminders");
    pub const MEDIA_LIBRARY: Permission = Permission::Raw("apple.media-library");
    pub const APP_TRACKING: Permission = Permission::Raw("apple.tracking");
    pub const LOCAL_NETWORK: Permission = Permission::Raw("apple.local-network");
}

/// Browser Permissions-API names beyond the portable set. The string is passed straight to
/// `navigator.permissions.query`.
#[cfg(target_arch = "wasm32")]
pub mod web {
    use super::Permission;

    pub const MIDI: Permission = Permission::Raw("midi");
    pub const CLIPBOARD_READ: Permission = Permission::Raw("clipboard-read");
    pub const CLIPBOARD_WRITE: Permission = Permission::Raw("clipboard-write");
    pub const SCREEN_WAKE_LOCK: Permission = Permission::Raw("screen-wake-lock");
    pub const PERSISTENT_STORAGE: Permission = Permission::Raw("persistent-storage");

    /// Any other Permissions-API name.
    pub const fn raw(name: &'static str) -> Permission {
        Permission::Raw(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PORTABLE: [Permission; 7] = [
        Permission::Location,
        Permission::LocationAlways,
        Permission::Camera,
        Permission::Microphone,
        Permission::Notifications,
        Permission::Photos,
        Permission::Motion,
    ];

    /// Probing must never panic, whatever the host is — CI runners have no sensors, no camera
    /// entitlement, and often no window server.
    #[test]
    fn probing_never_panics() {
        for p in PORTABLE
            .into_iter()
            .chain([Permission::Raw("nonsense.permission")])
        {
            let _ = gate(p);
            let _ = status(p);
            let _ = can_prompt(p);
            let _ = should_show_rationale(p);
            let _ = p.as_str();
        }
    }

    /// The two vocabularies must agree: a target that doesn't gate a capability reports it as
    /// usable, and one that lacks the capability reports it as absent.
    #[test]
    fn gate_and_status_agree() {
        for p in PORTABLE {
            match gate(p) {
                Gate::Ungated => assert_eq!(status(p), Status::Granted, "{p} is ungated"),
                Gate::Absent => assert_eq!(status(p), Status::Unsupported, "{p} is absent"),
                Gate::Prompts => assert_ne!(status(p), Status::Unsupported, "{p} prompts"),
            }
        }
    }

    #[test]
    fn merge_precedence() {
        assert_eq!(merge(Status::Denied, Status::Granted), Status::Granted);
        assert_eq!(merge(Status::Prompt, Status::Denied), Status::Prompt);
        assert_eq!(merge(Status::Restricted, Status::Denied), Status::Denied);
        assert_eq!(
            merge(Status::Unknown, Status::Unsupported),
            Status::Unsupported
        );
        assert_eq!(merge(Status::Granted, Status::Granted), Status::Granted);
    }

    /// The whole Android truth table, including the deliberate never-asked/permanently-denied
    /// conflation Day accepts by keeping no state of its own.
    #[test]
    fn android_truth_table() {
        assert_eq!(classify_android(true, true, false), Status::Granted);
        assert_eq!(classify_android(true, false, false), Status::Granted);
        assert_eq!(classify_android(false, false, false), Status::Restricted);
        assert_eq!(classify_android(false, true, true), Status::Denied);
        assert_eq!(classify_android(false, true, false), Status::Prompt);
    }

    #[test]
    fn apple_and_web_mappings() {
        assert_eq!(from_apple_status(0), Status::Prompt);
        assert_eq!(from_apple_status(1), Status::Restricted);
        assert_eq!(from_apple_status(2), Status::Denied);
        assert_eq!(from_apple_status(3), Status::Granted);
        // limited (Photos) / provisional (notifications) / whenInUse (location) all mean "proceed".
        assert_eq!(from_apple_status(4), Status::Granted);
        assert_eq!(from_web_state("granted"), Status::Granted);
        assert_eq!(from_web_state("denied"), Status::Denied);
        // An unrecognized name still has a working request path, so "ask" is the useful answer.
        assert_eq!(from_web_state("weird"), Status::Prompt);
    }

    /// Labels are a wire format: dayscript asserts on them and they must not drift with the locale.
    #[test]
    fn labels_are_stable() {
        assert_eq!(Status::Granted.label(), "granted");
        assert_eq!(Status::Prompt.label(), "prompt");
        assert_eq!(Gate::Ungated.label(), "ungated");
        assert_eq!(Permission::LocationAlways.as_str(), "location-always");
        assert_eq!(Permission::Raw("x.y").as_str(), "x.y");
    }

    /// An empty batch must still call back (and not deadlock on the join counter).
    #[test]
    fn request_many_empty_completes() {
        let seen = Arc::new(Mutex::new(None));
        let sink = seen.clone();
        request_many(&[], move |v| *lock(&sink) = Some(v));
        assert_eq!(*lock(&seen), Some(Vec::new()));
    }
}
