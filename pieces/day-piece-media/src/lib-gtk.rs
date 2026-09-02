// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// GTK: gtk4::Video — a core GTK widget (compiles everywhere) backed by GtkMediaFile, which needs a
// gstreamer media backend in the gtk4 BUILD for actual playback. Linux distro packages ship one
// (-Dmedia-gstreamer=enabled); Homebrew's gtk4 ships none, so on macos-gtk GtkVideo shows its own
// "no media backend" error UI (the same caveat class as webkitgtk — see docs/media.md). GtkVideo's
// overlay controls are always on; `.controls(false)` is a no-op here.
//
// A sound-only player needs no Video widget at all: GtkMediaStream plays on its own, so the leaf
// is an empty, invisible GtkBox of no size that carries the stream as widget data. Playback state
// comes from the stream's own notify signals (`playing`, `error`, `prepared`, `ended`).
// ---------------------------------------------------------------------------

use super::*;
use day_gtk::Gtk;
use day_spec::{NodeId, Proposal, Size};
use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;

/// The stream a sound-only leaf carries, keyed as widget data (a `Video` owns its stream itself).
const STREAM_KEY: &str = "day-media-stream";
/// Marks a sound-only leaf, which measures zero.
const AUDIO_ONLY_KEY: &str = "day-media-audio-only";

/// `GtkMediaFile` from the one source string: an explicit scheme parses as a URI, anything else is
/// a local file path.
fn media_file(source: &str) -> gtk4::MediaFile {
    let file = if source.contains("://") {
        gio::File::for_uri(source)
    } else {
        gio::File::for_path(source)
    };
    gtk4::MediaFile::for_file(&file)
}

fn report(id: NodeId, code: i32, text: String) {
    day_gtk::emit(
        id,
        Event::Custom {
            tag: report::TAG,
            num: code as f64,
            text,
        },
    );
}

/// Watch one stream: every notify becomes a report. A `Load` swaps streams, so this is called
/// per stream rather than per widget.
fn observe(stream: &gtk4::MediaStream, id: NodeId) {
    let state = move |s: &gtk4::MediaStream| {
        if let Some(err) = s.error() {
            (report::ERROR, err.message().to_string())
        } else if s.is_ended() {
            (report::ENDED, String::new())
        } else if s.is_playing() {
            if s.is_prepared() {
                (report::PLAYING, String::new())
            } else {
                (report::LOADING, String::new())
            }
        } else if s.is_prepared() {
            (report::PAUSED, String::new())
        } else {
            (report::LOADING, String::new())
        }
    };
    let on = move |s: &gtk4::MediaStream| {
        let (code, text) = state(s);
        report(id, code, text);
    };
    stream.connect_playing_notify(on);
    stream.connect_prepared_notify(on);
    stream.connect_ended_notify(on);
    stream.connect_error_notify(on);
}

fn new_stream(url: &str, p: &MediaProps, id: NodeId) -> gtk4::MediaFile {
    let media = media_file(url);
    media.set_muted(p.muted);
    media.set_volume(p.volume.clamp(0.0, 1.0));
    media.set_loop(p.looping);
    observe(media.upcast_ref(), id);
    media
}

/// The node a leaf reports to, carried on the widget so a `Load` swap knows whom to tell.
const NODE_KEY: &str = "day-media-node";

fn remember_node(h: &gtk4::Widget, id: NodeId) {
    // SAFETY: a `u64` under a key private to this arm.
    unsafe { h.set_data(NODE_KEY, id.0) };
}

fn node_of(h: &gtk4::Widget) -> Option<NodeId> {
    // SAFETY: only ever a `u64` (see `remember_node`).
    unsafe { h.data::<u64>(NODE_KEY).map(|p| NodeId(*p.as_ref())) }
}

/// The stream behind a leaf, whichever shape it is.
fn stream_of(h: &gtk4::Widget) -> Option<gtk4::MediaStream> {
    if let Some(video) = h.downcast_ref::<gtk4::Video>() {
        return video.media_stream();
    }
    // SAFETY: the only value ever stored under this key is a `MediaStream` (see `set_stream`).
    unsafe { h.data::<gtk4::MediaStream>(STREAM_KEY).map(|p| p.as_ref().clone()) }
}

fn set_stream(h: &gtk4::Widget, stream: Option<gtk4::MediaStream>) {
    if let Some(video) = h.downcast_ref::<gtk4::Video>() {
        video.set_media_stream(stream.as_ref());
        return;
    }
    // SAFETY: replaces (and drops) whatever `MediaStream` the key held; the key is private to
    // this arm and only ever holds that type.
    unsafe {
        match stream {
            Some(s) => h.set_data(STREAM_KEY, s),
            None => {
                let _ = h.steal_data::<gtk4::MediaStream>(STREAM_KEY);
            }
        }
    }
}

fn make(_backend: &mut Gtk, p: &MediaProps, id: NodeId) -> gtk4::Widget {
    if p.audio_only {
        let host = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        host.set_visible(false);
        // SAFETY: a plain marker under a key private to this arm.
        unsafe { host.set_data(AUDIO_ONLY_KEY, true) };
        let widget: gtk4::Widget = host.upcast();
        remember_node(&widget, id);
        if !p.url.is_empty() {
            let media = new_stream(&p.url, p, id);
            if p.autoplay {
                media.play();
            }
            set_stream(&widget, Some(media.upcast()));
        }
        return widget;
    }
    let video = gtk4::Video::new();
    video.set_autoplay(p.autoplay);
    video.set_loop(p.looping);
    if !p.url.is_empty() {
        let media = new_stream(&p.url, p, id);
        video.set_media_stream(Some(&media));
    }
    let widget: gtk4::Widget = video.upcast();
    remember_node(&widget, id);
    widget
}

fn update(_backend: &mut Gtk, h: &gtk4::Widget, patch: &MediaPatch) {
    match patch {
        MediaPatch::Load(url) => {
            // Preserve the current mute state and volume across the swap (both live on the
            // stream).
            let (muted, volume) = match stream_of(h) {
                Some(s) => (s.is_muted(), s.volume()),
                None => (false, 1.0),
            };
            let media = media_file(url);
            media.set_muted(muted);
            media.set_volume(volume);
            media.set_loop(h.downcast_ref::<gtk4::Video>().is_some_and(|v| v.is_loop()));
            if let Some(id) = node_of(h) {
                observe(media.upcast_ref(), id);
            }
            set_stream(h, Some(media.clone().upcast()));
            media.play();
        }
        MediaPatch::Play => {
            if let Some(stream) = stream_of(h) {
                stream.play();
            }
        }
        MediaPatch::Pause => {
            if let Some(stream) = stream_of(h) {
                stream.pause();
            }
        }
        MediaPatch::Stop => {
            if let Some(stream) = stream_of(h) {
                stream.pause();
                set_stream(h, None);
                if let Some(id) = node_of(h) {
                    report(id, report::IDLE, String::new());
                }
            }
        }
        MediaPatch::Volume(v) => {
            if let Some(stream) = stream_of(h) {
                stream.set_volume(v.clamp(0.0, 1.0));
            }
        }
    }
}

/// A sound-only leaf takes no room; a video fills what it is offered.
fn measure(backend: &mut Gtk, h: &gtk4::Widget, proposal: Proposal) -> Size {
    // SAFETY: only ever a `bool` (see `make`).
    if unsafe { h.data::<bool>(AUDIO_ONLY_KEY) }.is_some() {
        return Size::ZERO;
    }
    day_pieces::fill_measure(backend, h, proposal)
}

/// Silence the stream when the widget goes: a detached GtkMediaStream keeps playing.
fn release(_backend: &mut Gtk, h: &gtk4::Widget) {
    if let Some(stream) = stream_of(h) {
        stream.pause();
    }
    set_stream(h, None);
}

// glib is what the widget-data helpers above are generic over; naming it keeps the import honest
// for a reader tracing `set_data`/`data` to their trait.
#[allow(unused_imports)]
use glib::object::ObjectExt as _;

day_pieces::renderer!(day_gtk::RENDERERS, Gtk,
    kind: KIND, props: MediaProps, patch: MediaPatch,
    make: make, update: update, measure: measure, release: release);
