// ---------------------------------------------------------------------------
// GTK: gtk4::Video — a core GTK widget (compiles everywhere) backed by GtkMediaFile, which needs a
// gstreamer media backend in the gtk4 BUILD for actual playback. Linux distro packages ship one
// (-Dmedia-gstreamer=enabled); Homebrew's gtk4 ships none, so on macos-gtk GtkVideo shows its own
// "no media backend" error UI (the same caveat class as webkitgtk — see docs/media.md). GtkVideo's
// overlay controls are always on; `.controls(false)` is a no-op here.
// ---------------------------------------------------------------------------

use super::*;
use day_gtk::Gtk;
use day_spec::NodeId;
use gtk4::gio;
use gtk4::prelude::*;

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

fn make(_backend: &mut Gtk, p: &MediaProps, _id: NodeId) -> gtk4::Widget {
    let video = gtk4::Video::new();
    video.set_autoplay(p.autoplay);
    video.set_loop(p.looping);
    if !p.url.is_empty() {
        let media = media_file(&p.url);
        media.set_muted(p.muted);
        video.set_media_stream(Some(&media));
    }
    video.upcast()
}

fn update(_backend: &mut Gtk, h: &gtk4::Widget, patch: &MediaPatch) {
    let Some(video) = h.downcast_ref::<gtk4::Video>() else {
        return;
    };
    match patch {
        MediaPatch::Load(url) => {
            // Preserve the current mute state across the swap (muted lives on the stream).
            let muted = video.media_stream().is_some_and(|s| s.is_muted());
            let media = media_file(url);
            media.set_muted(muted);
            video.set_media_stream(Some(&media));
            media.play();
        }
        MediaPatch::Play => {
            if let Some(stream) = video.media_stream() {
                stream.play();
            }
        }
        MediaPatch::Pause => {
            if let Some(stream) = video.media_stream() {
                stream.pause();
            }
        }
    }
}

day_pieces::renderer!(day_gtk::RENDERERS, Gtk,
    kind: KIND, props: MediaProps, patch: MediaPatch,
    make: make, update: update, measure: day_pieces::fill_measure);
