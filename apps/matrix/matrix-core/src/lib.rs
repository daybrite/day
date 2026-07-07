//! matrix-core — the matrix-rust-sdk ↔ Day bridge.
//!
//! Owns a tokio multi-thread runtime + the `matrix_sdk::Client`, drives sync via matrix-sdk-ui, and
//! publishes app-facing state (rooms, timeline, sync status) as Day reactive `Signal`s. UI actions
//! (login/open_room/send) are plain sync-looking fns that spawn tokio work; results marshal back to
//! the main thread with `day_reactive::on_main`.
//!
//! CRITICAL bridge rule (see ../DESIGN.md): `on_main` needs `FnOnce + Send`, but Day `Signal`s are
//! `!Send`. So a tokio task NEVER captures a `Signal`. It captures only Send DATA and looks up
//! [`state()`] *inside* the `on_main` closure (which runs on the main thread). Never call `state()`
//! off the main thread.

use std::cell::OnceCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, OnceLock};

use day_reactive::{on_main, Scope, Signal};
use futures_util::{pin_mut, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::runtime::Runtime;

use matrix_sdk::authentication::matrix::MatrixSession;
use matrix_sdk::media::MediaFormat;
use matrix_sdk::ruma::events::room::message::{MessageType, RoomMessageEventContent};
use matrix_sdk::ruma::{MilliSecondsSinceUnixEpoch, RoomId};
use matrix_sdk::Client;
use matrix_sdk_ui::room_list_service::filters::new_filter_non_left;
use matrix_sdk_ui::sync_service::SyncService;
use matrix_sdk_ui::timeline::{RoomExt, Timeline, VirtualTimelineItem};

type R<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

// ─────────────────────────────── app-facing data model (all Send + Clone) ───────────────────────

/// High-level sync/session state, shown by the shell to pick login vs. main UI.
#[derive(Clone, Debug, PartialEq)]
pub enum SyncStatus {
    /// Trying to restore a saved session at startup.
    Restoring,
    /// No session — show the login screen.
    LoggedOut,
    /// A login is in flight.
    LoggingIn,
    /// Logged in and syncing — show the main UI.
    Syncing,
    /// A login/restore error to surface to the user.
    Error(String),
}

/// One room in the room list (a flat, UI-ready snapshot; no SDK types leak out).
#[derive(Clone, Debug, PartialEq)]
pub struct RoomSummary {
    pub id: String,
    pub name: String,
    /// A short preview of the latest message (may be empty until loaded).
    pub preview: String,
    pub unread: u64,
    /// Encoded (PNG/JPEG) avatar bytes once fetched; `None` until loaded.
    pub avatar: Option<Arc<Vec<u8>>>,
}

/// One row in a room's timeline. Date dividers and messages share the list so the native `list`
/// keys them uniformly.
#[derive(Clone, Debug, PartialEq)]
pub enum TimelineRow {
    DateDivider {
        key: String,
        label: String,
    },
    Message {
        key: String,
        sender: String,
        sender_id: String,
        body: String,
        mine: bool,
        time: String,
        /// First message of a consecutive same-sender run — show the avatar + sender name; when
        /// `false` the message continues the group above it (avatar/name hidden for a tight cluster).
        head: bool,
        /// True when the message body is an image (bytes fetched lazily into `image`).
        is_image: bool,
        image: Option<Arc<Vec<u8>>>,
        avatar: Option<Arc<Vec<u8>>>,
    },
    /// A membership/state change or otherwise non-message event, rendered as a faint centered note.
    Notice {
        key: String,
        text: String,
    },
}

impl TimelineRow {
    pub fn key(&self) -> &str {
        match self {
            TimelineRow::DateDivider { key, .. }
            | TimelineRow::Message { key, .. }
            | TimelineRow::Notice { key, .. } => key,
        }
    }
}

// ─────────────────────────────── reactive state (main-thread only) ──────────────────────────────

/// The app's reactive view-model. All fields are `Copy` `Signal`s; read them from the UI on the main
/// thread. Updated only via [`state()`] inside `on_main` closures.
#[derive(Clone, Copy)]
pub struct MatrixState {
    pub status: Signal<SyncStatus>,
    pub rooms: Signal<Vec<RoomSummary>>,
    pub current_room: Signal<Option<String>>,
    pub timeline: Signal<Vec<TimelineRow>>,
    pub can_back_paginate: Signal<bool>,
    pub me: Signal<Option<String>>,
}

thread_local! {
    static STATE: OnceCell<MatrixState> = const { OnceCell::new() };
}

/// The app's reactive state. Lazily created (once) in a detached scope on the MAIN THREAD. Read from
/// the UI; write only inside `on_main`.
pub fn state() -> MatrixState {
    STATE.with(|c| {
        *c.get_or_init(|| {
            // A detached scope keeps these signals alive for the whole app (never disposed by UI
            // re-renders).
            Scope::detached().enter(|| MatrixState {
                status: Signal::new(SyncStatus::Restoring),
                rooms: Signal::new(Vec::new()),
                current_room: Signal::new(None),
                timeline: Signal::new(Vec::new()),
                can_back_paginate: Signal::new(true),
                me: Signal::new(None),
            })
        })
    })
}

fn set_status(s: SyncStatus) {
    on_main(move || state().status.set(s));
}

/// Append a diagnostic line to `<store>/diag.log` (day-cli does not forward the launched app's
/// stderr, so file logging is how we observe the bridge). A no-op unless the `DAY_MATRIX_DIAG`
/// environment variable is set — off in normal runs, opt-in for debugging.
pub fn diag(msg: &str) {
    use std::io::Write;
    static ON: OnceLock<bool> = OnceLock::new();
    if !ON.get_or_init(|| std::env::var_os("DAY_MATRIX_DIAG").is_some()) {
        return;
    }
    if let Ok(dir) = store_dir() {
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("diag.log"))
        {
            let _ = writeln!(f, "{msg}");
        }
    }
}

fn push_rooms(rooms: Vec<RoomSummary>) {
    diag(&format!(
        "push_rooms: {} rooms -> scheduling on_main",
        rooms.len()
    ));
    on_main(move || {
        diag(&format!(
            "on_main FIRED: setting rooms signal = {}",
            rooms.len()
        ));
        state().rooms.set(rooms);
    });
}

/// Push a timeline snapshot, but only if the room is still the open one (drop stale updates from a
/// room we've since navigated away from).
fn push_timeline(room_id: String, rows: Vec<TimelineRow>) {
    let msg = rows
        .iter()
        .filter(|r| matches!(r, TimelineRow::Message { .. }))
        .count();
    let div = rows
        .iter()
        .filter(|r| matches!(r, TimelineRow::DateDivider { .. }))
        .count();
    let notice = rows
        .iter()
        .filter(|r| matches!(r, TimelineRow::Notice { .. }))
        .count();
    diag(&format!(
        "push_timeline: {} rows ({msg} msg, {div} div, {notice} notice)",
        rows.len()
    ));
    on_main(move || {
        let open = state().current_room.get_untracked();
        if open.as_deref() == Some(room_id.as_str()) {
            state().timeline.set(rows);
        }
    });
}

// ─────────────────────────────── tokio runtime + SDK handles ────────────────────────────────────

static RT: OnceLock<Runtime> = OnceLock::new();
fn runtime() -> &'static Runtime {
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime")
    })
}

static CLIENT: Mutex<Option<Client>> = Mutex::new(None);
static SYNC: Mutex<Option<SyncService>> = Mutex::new(None);
static TIMELINE: Mutex<Option<(String, Arc<Timeline>)>> = Mutex::new(None);

fn set_client(c: Client) {
    *CLIENT.lock().unwrap() = Some(c);
}
fn client() -> Option<Client> {
    CLIENT.lock().unwrap().clone()
}

// ─────────────────────────────── public actions (call on the main thread) ───────────────────────

/// Start the runtime and try to restore a saved session. Call once at startup (after `day::launch`
/// has installed the main poster). Ends in `Syncing` (restored) or `LoggedOut`.
pub fn init() {
    // Resolve the Android app files dir HERE, on the main/UI thread, and cache it. `store_dir()` runs
    // on tokio threads (do_login/do_restore), and a JNI `FindClass` from a spawned native thread uses
    // the *system* classloader (no app classes) → ClassNotFoundException. The UI thread carries the
    // app classloader, so resolving once here and reading the cache off-thread sidesteps that.
    #[cfg(target_os = "android")]
    {
        let _ = ANDROID_STORE.get_or_init(android_files_dir);
    }
    // DIAGNOSTIC: surface matrix-sdk / matrix-sdk-ui logs to stderr (RUST_LOG controls level).
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .try_init();
    set_status(SyncStatus::Restoring);
    runtime().spawn(async {
        match do_restore().await {
            Ok(true) => {} // start_sync set Syncing
            Ok(false) => set_status(SyncStatus::LoggedOut),
            Err(e) => {
                tracing::warn!("session restore failed: {e}");
                set_status(SyncStatus::LoggedOut);
            }
        }
    });
}

/// Log in with password, persist the session, and start syncing.
pub fn login(homeserver: String, username: String, password: String) {
    set_status(SyncStatus::LoggingIn);
    runtime().spawn(async move {
        if let Err(e) = do_login(homeserver, username, password).await {
            set_status(SyncStatus::Error(e.to_string()));
        }
    });
}

/// Forget the saved session and return to the login screen.
pub fn logout() {
    *CLIENT.lock().unwrap() = None;
    *SYNC.lock().unwrap() = None;
    *TIMELINE.lock().unwrap() = None;
    if let Ok(dir) = store_dir() {
        let _ = std::fs::remove_file(dir.join("session.json"));
    }
    on_main(|| {
        state().rooms.set(Vec::new());
        state().current_room.set(None);
        state().timeline.set(Vec::new());
        state().status.set(SyncStatus::LoggedOut);
    });
}

/// Open a room: set it current, clear the timeline, and start streaming its events.
pub fn open_room(room_id: String) {
    let rid = room_id.clone();
    on_main(move || {
        state().current_room.set(Some(rid));
        state().timeline.set(Vec::new());
        state().can_back_paginate.set(true);
    });
    runtime().spawn(async move {
        if let Err(e) = run_timeline(room_id).await {
            tracing::warn!("timeline error: {e}");
        }
    });
}

/// Leave the current room view (back to the room list on mobile).
pub fn close_room() {
    *TIMELINE.lock().unwrap() = None;
    on_main(|| {
        state().current_room.set(None);
        state().timeline.set(Vec::new());
    });
}

/// Send a plain-text message to a room (auto-encrypted in encrypted rooms).
pub fn send_message(room_id: String, body: String) {
    if body.trim().is_empty() {
        return;
    }
    runtime().spawn(async move {
        if let Err(e) = do_send(room_id, body).await {
            tracing::warn!("send failed: {e}");
        }
    });
}

/// Load older messages for the open room.
pub fn paginate_back(room_id: String) {
    runtime().spawn(async move {
        let tl = {
            let guard = TIMELINE.lock().unwrap();
            guard
                .as_ref()
                .filter(|(id, _)| *id == room_id)
                .map(|(_, t)| t.clone())
        };
        if let Some(tl) = tl {
            match tl.paginate_backwards(50).await {
                Ok(hit_start) => on_main(move || state().can_back_paginate.set(!hit_start)),
                Err(e) => tracing::warn!("paginate failed: {e}"),
            }
        }
    });
}

// ─────────────────────────────── async implementations (tokio) ──────────────────────────────────

async fn do_login(homeserver: String, username: String, password: String) -> R<()> {
    let store = store_dir()?;
    let client = Client::builder()
        .homeserver_url(&homeserver)
        .sqlite_store(store.join("db"), Some(STORE_PASSPHRASE))
        .build()
        .await?;
    client
        .matrix_auth()
        .login_username(&username, &password)
        .initial_device_display_name("Day Matrix")
        .await?;
    if let Some(session) = client.matrix_auth().session() {
        save_session(&homeserver, &session, &store)?;
    }
    set_client(client.clone());
    start_sync(client).await
}

async fn do_restore() -> R<bool> {
    let store = store_dir()?;
    let Some((homeserver, session)) = load_session(&store)? else {
        return Ok(false);
    };
    let client = Client::builder()
        .homeserver_url(&homeserver)
        .sqlite_store(store.join("db"), Some(STORE_PASSPHRASE))
        .build()
        .await?;
    client.restore_session(session).await?;
    set_client(client.clone());
    start_sync(client).await?;
    Ok(true)
}

async fn start_sync(client: Client) -> R<()> {
    let me = client.user_id().map(|u| u.to_string());
    on_main(move || state().me.set(me));

    let sync = SyncService::builder(client.clone()).build().await?;
    sync.start().await;
    // DIAGNOSTIC: log sync-service state transitions.
    let mut states = sync.state();
    tokio::spawn(async move {
        while let Some(s) = states.next().await {
            diag(&format!("sync-service state: {s:?}"));
        }
    });
    let rls = sync.room_list_service();
    *SYNC.lock().unwrap() = Some(sync);

    set_status(SyncStatus::Syncing);
    diag("start_sync: SyncService started, running room list");
    let r = run_room_list(rls).await;
    diag(&format!(
        "run_room_list returned: {:?}",
        r.as_ref().map(|_| ()).map_err(|e| e.to_string())
    ));
    r
}

async fn run_room_list(rls: Arc<matrix_sdk_ui::RoomListService>) -> R<()> {
    use eyeball_im::Vector;
    let room_list = rls.all_rooms().await?;
    diag("all_rooms() ready; consuming entries");
    let (stream, controller) = room_list.entries_with_dynamic_adapters(200);
    controller.set_filter(Box::new(new_filter_non_left()));
    controller.add_one_page();
    pin_mut!(stream);

    let mut rooms: Vector<matrix_sdk::Room> = Vector::new();
    // The list stream only yields on *structural* changes (rooms added/removed/reordered). A room can
    // surface before its `m.room.name` / heroes state has synced, and won't re-emit when only that
    // resolves. Re-summarize on a slow interval to catch those late names. Crucially we DEFER the very
    // first render until every room has a real name (or a short grace period elapses): the native list
    // builds a row once per stable key (room id) and does not rebuild it when only the name changes, so
    // a row must be born with its final name + name-derived script id. Change-detection then avoids
    // redundant churn once things settle.
    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(1));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_pushed: Vec<RoomSummary> = Vec::new();
    let mut settled = false;
    let mut ticks = 0u32;
    loop {
        tokio::select! {
            maybe = stream.next() => match maybe {
                Some(diffs) => {
                    for diff in diffs {
                        diff.apply(&mut rooms);
                    }
                    diag(&format!("room-list diff -> {} rooms", rooms.len()));
                }
                None => break,
            },
            _ = ticker.tick() => { ticks += 1; }
        }
        if rooms.is_empty() {
            continue;
        }
        let mut summaries = Vec::with_capacity(rooms.len());
        let mut all_named = true;
        for room in rooms.iter() {
            let name = match resolved_name(room).await {
                Some(n) => n,
                None => {
                    all_named = false;
                    name_fallback(room)
                }
            };
            summaries.push(room_summary(room, name).await);
        }
        // Hold the first render until names resolve, but no longer than ~6s so a genuinely nameless
        // room still appears (with its id fallback). After settling, push on every real change.
        if !settled && !all_named && ticks < 6 {
            continue;
        }
        settled = true;
        if summaries != last_pushed {
            last_pushed = summaries.clone();
            push_rooms(summaries);
        }
    }
    Ok(())
}

async fn room_summary(room: &matrix_sdk::Room, name: String) -> RoomSummary {
    RoomSummary {
        id: room.room_id().to_string(),
        name,
        preview: String::new(),
        unread: room.num_unread_messages(),
        avatar: room_avatar(room).await,
    }
}

/// The resolved display name, or `None` when neither the explicit `m.room.name` nor a genuine computed
/// name has synced yet (the SDK renders the unresolved `RoomDisplayName::Empty` as the literal "Empty
/// Room", which we never want to show). Caller decides whether to wait or use [`name_fallback`].
async fn resolved_name(room: &matrix_sdk::Room) -> Option<String> {
    use matrix_sdk::RoomDisplayName::{Aliased, Calculated, Named};
    // 1) The explicit m.room.name once its state event has synced.
    if let Some(n) = room.name() {
        let n = n.trim();
        if !n.is_empty() {
            return Some(n.to_string());
        }
    }
    // 2) A computed display name, but only a genuine one (Named/Aliased/Calculated) — the Empty /
    //    EmptyWas variants are exactly the "Empty Room" placeholder we must not snapshot.
    let computed = match room.cached_display_name() {
        Some(d) => Some(d),
        None => room.display_name().await.ok(),
    };
    if let Some(Named(s) | Aliased(s) | Calculated(s)) = computed {
        let s = s.trim();
        if !s.is_empty() {
            return Some(s.to_string());
        }
    }
    None
}

/// A stable, never-"Empty Room" fallback for a room whose name hasn't resolved: the room-id localpart.
/// Handles both classic "!local:server" ids and the colon-less "!hash" ids newer room versions use.
fn name_fallback(room: &matrix_sdk::Room) -> String {
    let id = room.room_id().as_str();
    let local = id.strip_prefix('!').unwrap_or(id);
    local.split(':').next().unwrap_or(local).to_string()
}

/// Decoded room-avatar bytes, keyed by room id, so the room-list diff loop fetches each avatar at
/// most once (successful fetches only). Populated off the main thread inside [`room_avatar`].
static ROOM_AVATARS: LazyLock<Mutex<HashMap<String, Arc<Vec<u8>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Fetch a room's avatar as encoded (PNG/JPEG) bytes for [`RoomSummary::avatar`], or `None` when the
/// room has no avatar. Cheap when there is no `avatar_url` (a sync check, no I/O); otherwise the
/// full-size media is fetched once and cached. The UI falls back to an initial disc when this is
/// `None`.
async fn room_avatar(room: &matrix_sdk::Room) -> Option<Arc<Vec<u8>>> {
    room.avatar_url()?; // no avatar set → initial-disc fallback in the UI
    let id = room.room_id().to_string();
    if let Some(bytes) = ROOM_AVATARS.lock().unwrap().get(&id).cloned() {
        return Some(bytes);
    }
    let bytes = match room.avatar(MediaFormat::File).await {
        Ok(Some(b)) => Arc::new(b),
        _ => return None,
    };
    ROOM_AVATARS.lock().unwrap().insert(id, bytes.clone());
    Some(bytes)
}

async fn run_timeline(room_id: String) -> R<()> {
    use eyeball_im::Vector;
    let client = client().ok_or("no client")?;
    let rid = RoomId::parse(&room_id)?;
    let room = client.get_room(&rid).ok_or("room not found")?;
    let timeline = Arc::new(room.timeline().await?);
    *TIMELINE.lock().unwrap() = Some((room_id.clone(), timeline.clone()));

    let me = client.user_id().map(|u| u.to_string()).unwrap_or_default();
    let (initial, mut stream) = timeline.subscribe().await;
    let mut items: Vector<Arc<matrix_sdk_ui::timeline::TimelineItem>> = initial;
    diag(&format!(
        "timeline subscribed: {} initial items",
        items.len()
    ));
    push_timeline(room_id.clone(), map_timeline(&items, &me));

    // Load older history — messages sent before we joined predate the live window.
    let tl_bg = timeline.clone();
    tokio::spawn(async move {
        match tl_bg.paginate_backwards(50).await {
            Ok(hit) => diag(&format!("paginate_backwards done (hit_start={hit})")),
            Err(e) => diag(&format!("paginate_backwards error: {e}")),
        }
    });

    while let Some(diffs) = stream.next().await {
        for diff in diffs {
            diff.apply(&mut items);
        }
        push_timeline(room_id.clone(), map_timeline(&items, &me));
    }
    Ok(())
}

fn map_timeline(
    items: &eyeball_im::Vector<Arc<matrix_sdk_ui::timeline::TimelineItem>>,
    _me: &str,
) -> Vec<TimelineRow> {
    let mut out = Vec::with_capacity(items.len());
    // Track the previous message's sender to collapse consecutive same-sender runs into one visual
    // group (only the first message of a run shows an avatar + name). Reset on any non-message row.
    let mut prev_sender: Option<String> = None;
    for item in items.iter() {
        if let Some(ev) = item.as_event() {
            if let Some(msg) = ev.content().as_message() {
                let is_image = matches!(msg.msgtype(), MessageType::Image(_));
                let key = ev
                    .event_id()
                    .map(|e| e.to_string())
                    .unwrap_or_else(|| format!("pending-{}", out.len()));
                let sender_id = ev.sender().to_string();
                let head = prev_sender.as_deref() != Some(sender_id.as_str());
                prev_sender = Some(sender_id.clone());
                out.push(TimelineRow::Message {
                    key,
                    sender: display_sender(ev),
                    sender_id,
                    body: msg.body().to_string(),
                    mine: ev.is_own(),
                    time: fmt_time(ev.timestamp()),
                    head,
                    is_image,
                    image: None,
                    avatar: None,
                });
            } else if ev.content().is_redacted() {
                prev_sender = None;
                out.push(TimelineRow::Notice {
                    key: ev
                        .event_id()
                        .map(|e| e.to_string())
                        .unwrap_or_else(|| format!("red-{}", out.len())),
                    text: "message deleted".into(),
                });
            }
        } else if let Some(VirtualTimelineItem::DateDivider(ts)) = item.as_virtual() {
            prev_sender = None;
            out.push(TimelineRow::DateDivider {
                key: format!("date-{}", ms(*ts)),
                label: fmt_date(*ts),
            });
        }
    }
    out
}

/// A friendly sender name — the profile display name if resolved, else the mxid localpart.
fn display_sender(ev: &matrix_sdk_ui::timeline::EventTimelineItem) -> String {
    if let matrix_sdk_ui::timeline::TimelineDetails::Ready(profile) = ev.sender_profile() {
        if let Some(name) = &profile.display_name {
            return name.clone();
        }
    }
    ev.sender().localpart().to_string()
}

async fn do_send(room_id: String, body: String) -> R<()> {
    let client = client().ok_or("no client")?;
    let rid = RoomId::parse(&room_id)?;
    let room = client.get_room(&rid).ok_or("room not found")?;
    room.send(RoomMessageEventContent::text_plain(body)).await?;
    Ok(())
}

// ─────────────────────────────── session persistence + store dir ────────────────────────────────

const STORE_PASSPHRASE: &str = "day-matrix-store";

#[derive(Serialize, Deserialize)]
struct Saved {
    homeserver: String,
    session: MatrixSession,
}

/// The directory that holds the SQLite/crypto store (`db/`), the saved session, and the optional
/// diag log. Per platform (see DESIGN.md "Platform status"):
/// - **Desktop**: a hidden per-user dir under `$HOME`.
/// - **iOS**: `$HOME/Library/Application Support/daybrite-matrix` — `$HOME` is the app sandbox, but
///   the container root itself isn't writable; Application Support is the conventional app-data dir.
/// - **Android**: the app's files dir (`/data/data/<pkg>/files`), obtained from the Android `Context`
///   via day-android's JNI bridge — there is no usable `$HOME`.
fn store_dir() -> R<PathBuf> {
    let dir = base_store_dir()?;
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

#[cfg(not(any(target_os = "ios", target_os = "android")))]
fn base_store_dir() -> R<PathBuf> {
    let base = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    Ok(base.join(".daybrite-matrix"))
}

#[cfg(target_os = "ios")]
fn base_store_dir() -> R<PathBuf> {
    // `$HOME` is the app's sandbox container. Apps may not write to the container root, so use the
    // standard Library/Application Support subtree (created on demand) for the persistent store.
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or("HOME unset (iOS sandbox)")?;
    Ok(home.join("Library/Application Support/daybrite-matrix"))
}

/// The Android app files dir, resolved ONCE on the main thread in [`init`] and cached — see the
/// classloader note there and in [`base_store_dir`].
#[cfg(target_os = "android")]
static ANDROID_STORE: OnceLock<Option<PathBuf>> = OnceLock::new();

#[cfg(target_os = "android")]
fn base_store_dir() -> R<PathBuf> {
    // Read the cache resolved on the main thread — never call the JNI here. `store_dir()` runs on
    // tokio threads (do_login/do_restore), where a JNI `FindClass` would hit the system classloader
    // (no app classes) and crash. Fall back to the OS temp dir if resolution failed at startup.
    let dir = ANDROID_STORE
        .get()
        .and_then(|o| o.clone())
        .unwrap_or_else(std::env::temp_dir);
    Ok(dir.join("daybrite-matrix"))
}

/// `Context.getFilesDir().getAbsolutePath()` via day-android's JNI bridge (DayBridge.filesDirPath).
/// MUST be called on the main/UI thread (see [`init`]): a `FindClass` off a spawned native thread
/// resolves against the system classloader, which has no app classes.
#[cfg(target_os = "android")]
fn android_files_dir() -> Option<PathBuf> {
    use day_android::jni::objects::JString;
    use day_android::with_env;
    const BRIDGE: &str = "dev/daybrite/day/bridge/DayBridge";
    with_env(|env| {
        let obj = env
            .call_static_method(BRIDGE, "filesDirPath", "()Ljava/lang/String;", &[])
            .ok()?
            .l()
            .ok()?;
        if obj.is_null() {
            return None;
        }
        let path: String = env.get_string(&JString::from(obj)).ok()?.into();
        if path.is_empty() {
            None
        } else {
            Some(PathBuf::from(path))
        }
    })
}

fn save_session(homeserver: &str, session: &MatrixSession, dir: &Path) -> R<()> {
    let saved = Saved {
        homeserver: homeserver.to_string(),
        session: session.clone(),
    };
    std::fs::write(dir.join("session.json"), serde_json::to_vec(&saved)?)?;
    Ok(())
}

fn load_session(dir: &Path) -> R<Option<(String, MatrixSession)>> {
    let path = dir.join("session.json");
    if !path.exists() {
        return Ok(None);
    }
    let saved: Saved = serde_json::from_slice(&std::fs::read(path)?)?;
    Ok(Some((saved.homeserver, saved.session)))
}

// ─────────────────────────────── time formatting (no chrono dep) ────────────────────────────────

fn ms(ts: MilliSecondsSinceUnixEpoch) -> i64 {
    u64::from(ts.0) as i64
}

/// `HH:MM` in UTC (v1; localization is a follow-up).
fn fmt_time(ts: MilliSecondsSinceUnixEpoch) -> String {
    let secs = ms(ts) / 1000;
    let tod = secs.rem_euclid(86_400);
    format!("{:02}:{:02}", tod / 3600, (tod % 3600) / 60)
}

/// `YYYY-MM-DD` (UTC) for the date-divider rows.
fn fmt_date(ts: MilliSecondsSinceUnixEpoch) -> String {
    let days = ms(ts).div_euclid(86_400_000);
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Howard Hinnant's days→civil algorithm (proleptic Gregorian), days since 1970-01-01.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}
