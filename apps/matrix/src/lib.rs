//! Day Matrix client — a full-featured Matrix chat client on the `day` native-UI framework,
//! backed by matrix-rust-sdk (via the `matrix-core` bridge). See DESIGN.md.
//!
//! This is the polished v1 UI: a styled login card, a room list with avatars + unread badges, a
//! message-bubble timeline (own vs other, grouped by sender, auto-scroll-to-bottom), a multi-line
//! composer, and a desktop sidebar+detail split (single-pane push/back on mobile). All native, built
//! from the current Day primitives against `matrix_core`'s reactive state.

use std::sync::Arc;

use day::prelude::*;
use day_piece_remote_image::remote_image;
use matrix_core::{RoomSummary, SyncStatus, TimelineRow};

// ─────────────────────────────── palette + metrics ──────────────────────────────────────────────

const BG_APP: Color = Color::hex(0xF2F3F5); // window / timeline background
const BG_SIDEBAR: Color = Color::WHITE; // room-list pane
const BG_CARD: Color = Color::WHITE; // login card
const BG_HEADER: Color = Color::hex(0xF7F8FA); // header / composer bars
#[cfg(not(any(target_os = "ios", target_os = "android")))]
const RULE: Color = Color::hex(0xE2E3E7); // hairline separators (desktop sidebar/detail rule)
const OWN_BUBBLE: Color = Color::hex(0x2F6FDE); // my messages (blue)
const OTHER_BUBBLE: Color = Color::hex(0xE9E9EB); // their messages (gray)
const TEXT_PRIMARY: Color = Color::hex(0x1C1C1E);
const TEXT_MUTED: Color = Color::hex(0x8A8A8E);
const ACCENT: Color = Color::hex(0x2F6FDE);
const ERROR: Color = Color::hex(0xC0392B);
const ON_OWN: Color = Color::WHITE;
const ON_OWN_MUTED: Color = Color::rgba(1.0, 1.0, 1.0, 0.75);

/// Deterministic, pleasant avatar-disc colors, indexed by a name hash.
const AVATAR_PALETTE: [u32; 8] = [
    0x2F6FDE, 0x8E44AD, 0x27AE60, 0xE67E22, 0xC0392B, 0x16A085, 0x2980B9, 0xD35400,
];

#[cfg(not(any(target_os = "ios", target_os = "android")))]
const SIDEBAR_W: f64 = 320.0; // desktop room-list sidebar width
const AVATAR_MSG: f64 = 34.0;
const AVATAR_ROOM: f64 = 44.0;

// ─────────────────────────────── shell ──────────────────────────────────────────────────────────

/// Kick off session restore exactly once (after launch, so the main poster is installed).
fn ensure_init() {
    thread_local! { static DONE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
    if !DONE.with(|c| c.replace(true)) {
        matrix_core::init();
    }
}

pub fn root() -> AnyPiece {
    ensure_init();
    let st = matrix_core::state();
    column((
        when(
            move || matches!(st.status.get(), SyncStatus::Restoring),
            loading_view,
        ),
        when(
            move || {
                matches!(
                    st.status.get(),
                    SyncStatus::LoggedOut | SyncStatus::LoggingIn | SyncStatus::Error(_)
                )
            },
            login_view,
        ),
        when(
            move || matches!(st.status.get(), SyncStatus::Syncing),
            main_view,
        ),
    ))
    .background(BG_APP)
    .grow()
}

fn loading_view() -> AnyPiece {
    let block = column((
        label("Matrix")
            .font(Font::LargeTitle)
            .weight(FontWeight::Bold),
        label("Connecting…")
            .font(Font::Subheadline)
            .color(TEXT_MUTED),
        spinner(),
    ))
    .spacing(14.0)
    .align(HAlign::Center);
    column((spacer(), row((spacer(), block, spacer())), spacer()))
        .background(BG_APP)
        .grow()
}

// ─────────────────────────────── login ──────────────────────────────────────────────────────────

fn login_view() -> AnyPiece {
    // Default homeserver differs per platform: the Android emulator reaches the host machine at
    // 10.0.2.2 (its own `localhost` is the emulated device); everyone else uses `localhost`.
    #[cfg(target_os = "android")]
    let default_homeserver = "http://10.0.2.2:6167";
    #[cfg(not(target_os = "android"))]
    let default_homeserver = "http://localhost:6167";
    let homeserver = Signal::new(default_homeserver.to_string());
    let username = Signal::new(String::new());
    let password = Signal::new(String::new());
    let submit = move || {
        matrix_core::login(
            homeserver.get_untracked(),
            username.get_untracked(),
            password.get_untracked(),
        );
    };
    let card = column((
        label("Matrix")
            .font(Font::LargeTitle)
            .weight(FontWeight::Bold)
            .id("login-title"),
        label("Sign in to your homeserver")
            .font(Font::Subheadline)
            .color(TEXT_MUTED),
        text_field(homeserver)
            .placeholder("Homeserver URL")
            .id("login-homeserver"),
        text_field(username)
            .placeholder("Username  (e.g. @alice:localhost)")
            .id("login-username"),
        text_field(password)
            .placeholder("Password")
            .id("login-password"),
        button("Sign in").action(submit).id("login-submit"),
        login_status(),
    ))
    .spacing(14.0)
    .align(HAlign::Leading)
    .padding(28.0)
    .background(BG_CARD)
    .corner_radius(18.0)
    .width(360.0);

    // Center the card both ways: horizontal spacers (row) inside vertical spacers (column). Spacers
    // grow to fill, so the centering panes span the window regardless of the card's fixed width.
    column((spacer(), row((spacer(), card, spacer())), spacer()))
        .background(BG_APP)
        .grow()
}

/// The status/error line under the Sign-in button (spinner while connecting, red error otherwise).
fn login_status() -> AnyPiece {
    let st = matrix_core::state();
    column((
        when(
            move || matches!(st.status.get(), SyncStatus::LoggingIn),
            || {
                row((
                    spinner(),
                    label("Signing in…").font(Font::Footnote).color(TEXT_MUTED),
                ))
                .spacing(8.0)
                .align(VAlign::Center)
            },
        ),
        when(
            move || matches!(st.status.get(), SyncStatus::Error(_)),
            move || {
                label(move || match st.status.get() {
                    SyncStatus::Error(e) => e,
                    _ => String::new(),
                })
                .font(Font::Footnote)
                .color(ERROR)
                .id("login-error")
            },
        ),
    ))
    .spacing(6.0)
    .align(HAlign::Leading)
    .any()
}

// ─────────────────────────────── main shell (rooms + timeline) ───────────────────────────────────

/// Desktop: room list (fixed-width sidebar) always visible on the left; timeline on the right.
#[cfg(not(any(target_os = "ios", target_os = "android")))]
fn main_view() -> AnyPiece {
    row((rooms_pane().width(SIDEBAR_W), vrule(), detail_pane()))
        .align(VAlign::Top)
        .grow()
}

/// Mobile: a single pane that swaps between the room list and the open room (back button pops).
#[cfg(any(target_os = "ios", target_os = "android"))]
fn main_view() -> AnyPiece {
    let st = matrix_core::state();
    column((
        when(move || st.current_room.get().is_none(), rooms_pane),
        when(move || st.current_room.get().is_some(), timeline_scaffold),
    ))
    .grow()
}

/// A 1-point full-height separator between the sidebar and the detail pane.
#[cfg(not(any(target_os = "ios", target_os = "android")))]
fn vrule() -> AnyPiece {
    column(()).width(1.0).grow_h().background(RULE)
}

fn rooms_pane() -> AnyPiece {
    let st = matrix_core::state();
    column((
        row((
            label("Chats").font(Font::Title2).weight(FontWeight::Bold),
            spacer(),
            button("Sign out")
                .action(matrix_core::logout)
                .id("rooms-signout"),
        ))
        .align(VAlign::Center)
        .padding(Insets::symmetric(16.0, 14.0)),
        divider(),
        list(
            move || st.rooms.get(),
            |r: &RoomSummary| r.id.clone(),
            |slot| room_row(slot.get()),
        )
        .row_height(RowHeight::Automatic)
        .on_select(|id: String| matrix_core::open_room(id))
        .grow(),
    ))
    .id("rooms-pane")
    .background(BG_SIDEBAR)
    .grow()
}

fn room_row(r: RoomSummary) -> AnyPiece {
    let open_id = r.id.clone();
    let row_id = script_id(&r.name);
    let unread = r.unread;
    // The name, plus a muted last-message preview line when one is available (preview is optional;
    // omit the second line rather than show a placeholder that could be inaccurate).
    let name = label(r.name.clone())
        .font(Font::Body)
        .weight(FontWeight::Semibold)
        .color(TEXT_PRIMARY);
    let title: AnyPiece = if r.preview.trim().is_empty() {
        name.any()
    } else {
        column((
            name,
            label(r.preview.clone())
                .font(Font::Footnote)
                .color(TEXT_MUTED),
        ))
        .spacing(2.0)
        .align(HAlign::Leading)
        .any()
    };
    row((
        avatar_view(&r.name, r.avatar.clone(), AVATAR_ROOM),
        title.grow(),
        when(move || unread > 0, move || unread_badge(unread)),
    ))
    .spacing(12.0)
    .align(VAlign::Center)
    .padding(Insets::symmetric(14.0, 10.0))
    .on_tap(move || matrix_core::open_room(open_id.clone()))
    .id(row_id)
}

fn unread_badge(count: u64) -> AnyPiece {
    label(format!("{count}"))
        .font(Font::Caption2)
        .weight(FontWeight::Semibold)
        .color(Color::WHITE)
        .padding(Insets::symmetric(7.0, 2.0))
        .background(ACCENT)
        .corner_radius(11.0)
}

// ─────────────────────────────── timeline + composer ─────────────────────────────────────────────

/// Desktop detail pane: an empty state until a room is opened, then the timeline scaffold.
#[cfg(not(any(target_os = "ios", target_os = "android")))]
fn detail_pane() -> AnyPiece {
    let st = matrix_core::state();
    column((
        when(move || st.current_room.get().is_none(), empty_detail),
        when(move || st.current_room.get().is_some(), timeline_scaffold),
    ))
    .background(BG_APP)
    .grow()
}

#[cfg(not(any(target_os = "ios", target_os = "android")))]
fn empty_detail() -> AnyPiece {
    let block = column((
        label("Select a chat")
            .font(Font::Title2)
            .weight(FontWeight::Semibold)
            .color(TEXT_MUTED),
        label("Choose a room from the list to start messaging.")
            .font(Font::Subheadline)
            .color(TEXT_MUTED),
    ))
    .spacing(8.0)
    .align(HAlign::Center);
    column((spacer(), row((spacer(), block, spacer())), spacer()))
        .background(BG_APP)
        .grow()
}

/// Header + message list + composer for the open room. Reads reactive state, so it stays correct
/// when the open room changes without a rebuild (desktop room switching).
fn timeline_scaffold() -> AnyPiece {
    let st = matrix_core::state();
    let draft = Signal::new(String::new());
    let send = move || {
        let body = draft.get_untracked();
        if !body.trim().is_empty() {
            if let Some(id) = st.current_room.get_untracked() {
                matrix_core::send_message(id, body);
                draft.set(String::new());
            }
        }
    };
    // Auto scroll-to-bottom: notify the list's trigger whenever the timeline length changes.
    let scroll = Trigger::new();
    bind(
        move || st.timeline.with(|t| t.len()),
        move |_: &usize| scroll.notify(),
    );

    column((
        timeline_header(),
        divider(),
        list(
            move || st.timeline.get(),
            |r: &TimelineRow| r.key().to_string(),
            |slot| timeline_row(slot.get()),
        )
        .row_height(RowHeight::Automatic)
        .scroll_to_end(scroll)
        .stick_to_bottom(true)
        .grow(),
        divider(),
        row((
            text_area(draft)
                .placeholder("Message…")
                .min_lines(1)
                .max_lines(5)
                .id("composer-input"),
            button("Send").action(send).id("composer-send"),
        ))
        .spacing(8.0)
        .align(VAlign::Bottom)
        .padding(Insets::all(12.0))
        .background(BG_HEADER),
    ))
    .background(BG_APP)
    .grow()
}

/// Desktop header: just the open room's name (the list stays visible on the left).
#[cfg(not(any(target_os = "ios", target_os = "android")))]
fn timeline_header() -> AnyPiece {
    row((
        label(current_room_name())
            .font(Font::Headline)
            .weight(FontWeight::Semibold)
            .color(TEXT_PRIMARY),
        spacer(),
    ))
    .align(VAlign::Center)
    .padding(Insets::symmetric(18.0, 14.0))
    .background(BG_HEADER)
}

/// Mobile header: a back button (pops to the room list) + the room name.
#[cfg(any(target_os = "ios", target_os = "android"))]
fn timeline_header() -> AnyPiece {
    row((
        button("‹ Chats")
            .action(matrix_core::close_room)
            .id("timeline-back"),
        label(current_room_name())
            .font(Font::Headline)
            .weight(FontWeight::Semibold)
            .color(TEXT_PRIMARY),
        spacer(),
    ))
    .spacing(10.0)
    .align(VAlign::Center)
    .padding(Insets::symmetric(12.0, 10.0))
    .background(BG_HEADER)
}

/// A reactive closure yielding the open room's display name (looked up in the room list).
fn current_room_name() -> impl Fn() -> String {
    let st = matrix_core::state();
    move || match st.current_room.get() {
        Some(id) => st
            .rooms
            .with(|rs| rs.iter().find(|r| r.id == id).map(|r| r.name.clone()))
            .unwrap_or_default(),
        None => String::new(),
    }
}

fn timeline_row(r: TimelineRow) -> AnyPiece {
    match r {
        TimelineRow::DateDivider { label: text, .. } => row((
            divider().grow(),
            label(text)
                .font(Font::Caption2)
                .weight(FontWeight::Semibold)
                .color(TEXT_MUTED)
                .padding(Insets::symmetric(10.0, 0.0)),
            divider().grow(),
        ))
        .align(VAlign::Center)
        .padding(Insets::symmetric(16.0, 8.0)),
        TimelineRow::Notice { text, .. } => label(text)
            .font(Font::Caption)
            .color(TEXT_MUTED)
            .padding(Insets::symmetric(16.0, 4.0)),
        TimelineRow::Message {
            sender,
            body,
            mine,
            time,
            head,
            avatar,
            ..
        } => message_row(sender, body, time, mine, head, avatar),
    }
}

/// One message: a blue right-aligned bubble for mine, a gray left-aligned bubble (with avatar +
/// sender name on the first message of a run) for others.
fn message_row(
    sender: String,
    body: String,
    time: String,
    mine: bool,
    head: bool,
    avatar: Option<Arc<Vec<u8>>>,
) -> AnyPiece {
    // Vertical breathing room: a little more above the first message of a group, tight within.
    let vpad = if head { 6.0 } else { 1.0 };
    if mine {
        let inner: AnyPiece = if head {
            column((
                label(body).font(Font::Body).color(ON_OWN),
                label(time).font(Font::Caption2).color(ON_OWN_MUTED),
            ))
            .spacing(3.0)
            .align(HAlign::Trailing)
            .any()
        } else {
            label(body).font(Font::Body).color(ON_OWN).any()
        };
        let bubble = inner
            .padding(Insets::symmetric(13.0, 8.0))
            .background(OWN_BUBBLE)
            .corner_radius(15.0);
        row((spacer(), bubble))
            .padding(Insets::symmetric(14.0, vpad))
            .any()
    } else {
        let inner: AnyPiece = if head {
            column((
                row((
                    label(sender.clone())
                        .font(Font::Caption)
                        .weight(FontWeight::Semibold)
                        .color(disc_color(&sender)),
                    spacer(),
                    label(time).font(Font::Caption2).color(TEXT_MUTED),
                ))
                .spacing(10.0),
                label(body).font(Font::Body).color(TEXT_PRIMARY),
            ))
            .spacing(3.0)
            .align(HAlign::Leading)
            .any()
        } else {
            label(body).font(Font::Body).color(TEXT_PRIMARY).any()
        };
        let bubble = inner
            .padding(Insets::symmetric(13.0, 8.0))
            .background(OTHER_BUBBLE)
            .corner_radius(15.0);
        let leading: AnyPiece = if head {
            avatar_view(&sender, avatar, AVATAR_MSG)
        } else {
            hgap(AVATAR_MSG)
        };
        row((leading, bubble, spacer()))
            .spacing(8.0)
            .align(VAlign::Top)
            .padding(Insets::symmetric(14.0, vpad))
            .any()
    }
}

// ─────────────────────────────── avatars + helpers ──────────────────────────────────────────────

/// A round avatar: the real image when bytes are present, else a colored initial disc.
fn avatar_view(name: &str, bytes: Option<Arc<Vec<u8>>>, d: f64) -> AnyPiece {
    if bytes.is_some() {
        let sig = Signal::new(bytes);
        remote_image(sig)
            .circle()
            .placeholder_color(disc_color(name))
            .frame(d, d)
    } else {
        initial_disc(name, d)
    }
}

/// A colored disc with the name's centered initial (the avatar fallback).
fn initial_disc(name: &str, d: f64) -> AnyPiece {
    let font = if d >= 40.0 {
        Font::Title3
    } else {
        Font::Subheadline
    };
    column((
        spacer(),
        label(initial_of(name))
            .font(font)
            .weight(FontWeight::Semibold)
            .color(Color::WHITE),
        spacer(),
    ))
    .align(HAlign::Center)
    .frame(d, d)
    .background(disc_color(name))
    .corner_radius(d / 2.0)
}

/// An invisible fixed-width box to indent grouped (non-head) messages under the avatar column.
fn hgap(w: f64) -> AnyPiece {
    column(()).width(w)
}

/// The uppercase first alphanumeric character of a name (a "#" fallback).
fn initial_of(name: &str) -> String {
    name.chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "#".to_string())
}

/// A deterministic, pleasant color derived from a name (for avatar discs + sender names).
fn disc_color(name: &str) -> Color {
    let h = name
        .bytes()
        .fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    Color::hex(AVATAR_PALETTE[(h as usize) % AVATAR_PALETTE.len()])
}

/// A stable, script-friendly row id from a room name (e.g. "Day Test Room" → "room-day-test-room").
fn script_id(name: &str) -> String {
    let mut s = String::from("room-");
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            s.push(c.to_ascii_lowercase());
        } else {
            s.push('-');
        }
    }
    s
}

// Mobile entry points (no-ops off iOS/Android). The iOS Runner's main.swift calls `day_main`; the
// Android DayActivity resolves the `nativeStart`/`nativeOnEvent`/`nativeRunPosted` JNI exports — both
// are generated here, wired to `root`. The desktop binary uses `src/main.rs` instead.
day::ios_main!("Matrix", root);
day::android_main!(root);
