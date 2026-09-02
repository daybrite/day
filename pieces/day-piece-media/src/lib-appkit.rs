// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// AppKit: AVPlayerView (AVKit) fronting an AVPlayer (AVFoundation) — native transport chrome for
// free via `controlsStyle`. A sound-only player skips the AVPlayerView: the leaf is an empty,
// hidden NSView of no size, and the AVPlayer behind it lives in this arm's own table.
//
// One NSObject per player observes what the player does — `timeControlStatus` and the current
// item's `status` over KVO, plus the played-to-end and failed-to-play notifications — and reports
// it on the node's Custom channel as a `PlaybackState`; the same observer seeks a looping player
// back to zero. It is retained here for the view's lifetime (neither KVO nor the notification
// center retains observers) and deregistered in `release`.
// ---------------------------------------------------------------------------

use super::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::ptr::null_mut;

use day_appkit::AppKit;
use day_spec::{NodeId, Proposal, Size};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::NSView;
use objc2::AnyThread;
use objc2::runtime::ProtocolObject;
use objc2_av_foundation::{
    AVMetadataCommonIdentifierAlbumName, AVMetadataCommonIdentifierArtist,
    AVMetadataCommonIdentifierTitle, AVMetadataIdentifierIcyMetadataStreamTitle,
    AVPlayerItemMetadataOutput, AVPlayerItemMetadataOutputPushDelegate,
    AVPlayerItemOutputPushDelegate, AVPlayerItemTrack, AVTimedMetadataGroup,
    AVPlayer, AVPlayerItem, AVPlayerItemDidPlayToEndTimeNotification,
    AVPlayerItemFailedToPlayToEndTimeNotification, AVPlayerItemStatus, AVPlayerTimeControlStatus,
};
use objc2_av_kit::{AVPlayerView, AVPlayerViewControlsStyle};
use objc2_core_media::kCMTimeZero;
use objc2_foundation::{
    NSArray,
    NSKeyValueObservingOptions, NSNotificationCenter, NSObject, NSObjectNSKeyValueObserverRegistration,
    NSString, NSURL,
};

/// The KVO key paths the observer registers for. `currentItem.status` reaches through to
/// whatever item is current, so a `.load()` swap stays covered without re-registering.
const KEY_PATHS: [&str; 3] = [
    "timeControlStatus",
    "currentItem.status",
    // Observing it is what makes AVFoundation PROCESS timed metadata at all ("AVPlayerItem may
    // omit the processing of timed metadata when no observer of this property is
    // registered"); the ICY block of an Icecast stream lands here.
    "currentItem.timedMetadata",
];

struct ObserverIvars {
    player: Retained<AVPlayer>,
    node: NodeId,
    looping: bool,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DayMediaObserver"]
    #[ivars = ObserverIvars]
    struct MediaObserver;

    unsafe impl NSObjectProtocol for MediaObserver {}

    impl MediaObserver {
        // Fired when ANY player item plays to its end (registered with object: nil so `.load()`
        // swaps stay covered) — act only when it is OUR player's current item.
        #[unsafe(method(itemDidPlayToEnd:))]
        fn item_did_play_to_end(&self, note: *mut AnyObject) {
            if !self.is_ours(note) {
                return;
            }
            let player = &self.ivars().player;
            if self.ivars().looping {
                unsafe {
                    player.seekToTime(kCMTimeZero);
                    player.play();
                }
            } else {
                self.report(report::ENDED, String::new());
            }
        }

        // The item stopped mid-way (a dropped stream, an unreadable file). The error rides in the
        // notification's userInfo; the item's own `error` is often still nil at this point.
        #[unsafe(method(itemFailedToPlayToEnd:))]
        fn item_failed_to_play_to_end(&self, note: *mut AnyObject) {
            if !self.is_ours(note) {
                return;
            }
            self.report(report::ERROR, failure_message(note));
        }

        // KVO: either key path moved — re-derive the whole state rather than reading the change
        // dictionary, since the answer depends on both.
        #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
        fn observe_value(
            &self,
            key_path: *mut AnyObject,
            _object: *mut AnyObject,
            _change: *mut AnyObject,
            _context: *mut std::ffi::c_void,
        ) {
            // SAFETY: KVO hands the key path as the NSString it was registered with.
            let is_metadata = !key_path.is_null()
                && unsafe { &*(key_path as *const NSString) }.to_string()
                    == "currentItem.timedMetadata";
            if is_metadata {
                let items = unsafe { self.ivars().player.currentItem() }.and_then(|item| {
                    #[allow(deprecated)]
                    unsafe {
                        item.timedMetadata()
                    }
                });
                if let Some(text) = items.as_deref().and_then(items_text) {
                    self.report(report::METADATA, text);
                }
                return;
            }
            let (code, text) = state_of(&self.ivars().player);
            self.report(code, text);
        }
    }
);

impl MediaObserver {
    fn new(mtm: MainThreadMarker, player: Retained<AVPlayer>, node: NodeId, looping: bool) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ObserverIvars {
            player,
            node,
            looping,
        });
        let this: Retained<Self> = unsafe { msg_send![super(this), init] };
        let center = NSNotificationCenter::defaultCenter();
        unsafe {
            center.addObserver_selector_name_object(
                &this,
                sel!(itemDidPlayToEnd:),
                Some(AVPlayerItemDidPlayToEndTimeNotification),
                None,
            );
            center.addObserver_selector_name_object(
                &this,
                sel!(itemFailedToPlayToEnd:),
                Some(AVPlayerItemFailedToPlayToEndTimeNotification),
                None,
            );
            for path in KEY_PATHS {
                this.ivars().player.addObserver_forKeyPath_options_context(
                    &this,
                    &NSString::from_str(path),
                    NSKeyValueObservingOptions::New,
                    null_mut(),
                );
            }
        }
        this
    }

    /// Deregister everything BEFORE the observer drops: a center or a KVO registration left
    /// pointing at a freed object messages garbage on the next change.
    fn detach(&self) {
        unsafe {
            NSNotificationCenter::defaultCenter().removeObserver(self);
            for path in KEY_PATHS {
                self.ivars()
                    .player
                    .removeObserver_forKeyPath(self, &NSString::from_str(path));
            }
        }
    }

    /// Whether a notification's object is our player's current item.
    fn is_ours(&self, note: *mut AnyObject) -> bool {
        let Some(current) = (unsafe { self.ivars().player.currentItem() }) else {
            return false;
        };
        let object: *mut AnyObject = unsafe { msg_send![&*note, object] };
        object == Retained::as_ptr(&current).cast_mut().cast()
    }

    fn report(&self, code: i32, text: String) {
        day_appkit::emit(
            self.ivars().node,
            Event::Custom {
                tag: report::TAG,
                num: code as f64,
                text,
            },
        );
    }
}

// ---------------------------------------------------------------------------
// The stream's own "now playing" (docs/media.md): an AVPlayerItemMetadataOutput on every item,
// delivering timed metadata to this delegate on the main queue — the ICY `StreamTitle` an
// Icecast server interleaves into an MP3/AAC stream (AVFoundation asks for it and decodes it),
// and the ID3 frames of an HLS stream — reported as one `report::METADATA` event.
// ---------------------------------------------------------------------------

struct MetadataIvars {
    node: NodeId,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "DayMediaMetadataAppKit"]
    #[ivars = MetadataIvars]
    struct MetadataDelegate;

    unsafe impl NSObjectProtocol for MetadataDelegate {}
    unsafe impl AVPlayerItemOutputPushDelegate for MetadataDelegate {}

    unsafe impl AVPlayerItemMetadataOutputPushDelegate for MetadataDelegate {
        #[unsafe(method(metadataOutput:didOutputTimedMetadataGroups:fromPlayerItemTrack:))]
        fn did_output(
            &self,
            _output: &AVPlayerItemMetadataOutput,
            groups: &NSArray<AVTimedMetadataGroup>,
            _track: Option<&AVPlayerItemTrack>,
        ) {
            log::debug!(
                "day-piece-media: {} timed metadata group(s)",
                groups.len()
            );
            if let Some(text) = metadata_text(groups) {
                day_appkit::emit(
                    self.ivars().node,
                    Event::Custom {
                        tag: report::TAG,
                        num: report::METADATA as f64,
                        text,
                    },
                );
            }
        }
    }
);

// The delegate protocol demands `Send + Sync`; the ivars are one plain id, and every callback
// arrives on the main queue (`setDelegate_queue` below).
unsafe impl Send for MetadataDelegate {}
unsafe impl Sync for MetadataDelegate {}

impl MetadataDelegate {
    fn new(node: NodeId) -> Retained<MetadataDelegate> {
        let this = MetadataDelegate::alloc().set_ivars(MetadataIvars { node });
        unsafe { msg_send![super(this), init] }
    }
}

/// The `report::METADATA` payload for a batch of timed groups: an ICY `StreamTitle` as it
/// came, else the ID3/common title, artist, and album packed for [`StreamMetadata::from_report`].
fn metadata_text(groups: &NSArray<AVTimedMetadataGroup>) -> Option<String> {
    groups
        .iter()
        .find_map(|group| {
            let items = unsafe { group.items() };
            items_text(&items)
        })
}

/// The same, for one batch of items (a group's, or the item's `timedMetadata`).
fn items_text(items: &NSArray<objc2_av_foundation::AVMetadataItem>) -> Option<String> {
    let ident = |s: Option<&'static objc2_foundation::NSString>| s.map(|s| s.to_string());
    let icy = unsafe { ident(AVMetadataIdentifierIcyMetadataStreamTitle) };
    let title_id = unsafe { ident(AVMetadataCommonIdentifierTitle) };
    let artist_id = unsafe { ident(AVMetadataCommonIdentifierArtist) };
    let album_id = unsafe { ident(AVMetadataCommonIdentifierAlbumName) };
    let (mut title, mut artist, mut album) = (String::new(), String::new(), String::new());
    for item in items.iter() {
        let id = unsafe { item.identifier() }.map(|i| i.to_string());
        let Some(value) = (unsafe { item.stringValue() }).map(|v| v.to_string()) else {
            continue;
        };
        if id.is_some() && id == icy {
            return Some(value);
        } else if id.is_some() && id == title_id {
            title = value;
        } else if id.is_some() && id == artist_id {
            artist = value;
        } else if id.is_some() && id == album_id {
            album = value;
        }
    }
    if title.is_empty() && artist.is_empty() && album.is_empty() {
        return None;
    }
    Some(format!("{title}\u{1f}{artist}\u{1f}{album}"))
}

/// Attach a metadata output for `delegate` to `item`, before the item goes to the player.
fn attach_metadata(item: &AVPlayerItem, delegate: &MetadataDelegate) {
    unsafe {
        let output = AVPlayerItemMetadataOutput::initWithIdentifiers(
            AVPlayerItemMetadataOutput::alloc(),
            None,
        );
        output.setDelegate_queue(
            Some(ProtocolObject::from_ref(delegate)),
            Some(dispatch2::DispatchQueue::main()),
        );
        item.addOutput(&output);
    }
    log::debug!("day-piece-media: metadata output attached");
}

/// The failure a `…FailedToPlayToEndTime` notification carries, as text.
fn failure_message(note: *mut AnyObject) -> String {
    unsafe {
        let info: *mut AnyObject = msg_send![&*note, userInfo];
        if info.is_null() {
            return "playback failed".into();
        }
        let key = NSString::from_str("AVPlayerItemFailedToPlayToEndTimeErrorKey");
        let err: *mut AnyObject = msg_send![&*info, objectForKey: &*key];
        if err.is_null() {
            return "playback failed".into();
        }
        let desc: Retained<NSString> = msg_send![&*err, localizedDescription];
        desc.to_string()
    }
}

/// What the player is doing right now, as a report code and detail.
fn state_of(player: &AVPlayer) -> (i32, String) {
    let Some(item) = (unsafe { player.currentItem() }) else {
        return (report::IDLE, String::new());
    };
    if unsafe { item.status() } == AVPlayerItemStatus::Failed {
        let text = unsafe { item.error() }
            .map(|e| e.localizedDescription().to_string())
            .unwrap_or_else(|| "playback failed".into());
        return (report::ERROR, text);
    }
    match unsafe { player.timeControlStatus() } {
        AVPlayerTimeControlStatus::Playing => (report::PLAYING, String::new()),
        AVPlayerTimeControlStatus::WaitingToPlayAtSpecifiedRate => (report::LOADING, String::new()),
        _ => (report::PAUSED, String::new()),
    }
}

/// What this arm keeps per realized view: the player (a sound-only view has no other route to
/// it), its observer, and whether it measures zero.
struct Live {
    player: Retained<AVPlayer>,
    observer: Retained<MediaObserver>,
    /// The timed-metadata delegate, one per player, re-attached to every item it loads.
    metadata: Retained<MetadataDelegate>,
    audio_only: bool,
}

day_core::tls_group! {
    static LIVE: RefCell<HashMap<usize, Live>> = RefCell::new(HashMap::new());
}

fn key_of(h: &NSView) -> usize {
    (h as *const NSView) as usize
}

/// `NSURL` from the one source string: an explicit scheme parses as a URL, anything else is a
/// local file path.
fn media_url(source: &str) -> Option<Retained<NSURL>> {
    let ns = NSString::from_str(source);
    if source.contains("://") {
        NSURL::URLWithString(&ns)
    } else {
        Some(NSURL::fileURLWithPath(&ns))
    }
}

fn load_url(
    player: &AVPlayer,
    source: &str,
    metadata: &MetadataDelegate,
    mtm: MainThreadMarker,
) {
    let Some(url) = media_url(source) else {
        return;
    };
    let item = unsafe { AVPlayerItem::playerItemWithURL(&url, mtm) };
    attach_metadata(&item, metadata);
    unsafe { player.replaceCurrentItemWithPlayerItem(Some(&item)) };
}

fn make(backend: &mut AppKit, p: &MediaProps, id: NodeId) -> Retained<NSView> {
    let mtm = backend.mtm();
    // SAFETY: creates the player (and its view) on the main thread.
    let player: Retained<AVPlayer> = unsafe { msg_send![AVPlayer::alloc(mtm), init] };
    unsafe {
        player.setMuted(p.muted);
        player.setVolume(p.volume.clamp(0.0, 1.0) as f32);
    }
    let ns: Retained<NSView> = if p.audio_only {
        // Nothing to draw: an empty, hidden view that measures zero (see `measure`).
        let view: Retained<NSView> = unsafe { msg_send![NSView::alloc(mtm), init] };
        view.setHidden(true);
        view
    } else {
        let view: Retained<AVPlayerView> = unsafe { msg_send![AVPlayerView::alloc(mtm), init] };
        unsafe {
            view.setPlayer(Some(&player));
            view.setControlsStyle(if p.controls {
                AVPlayerViewControlsStyle::Inline
            } else {
                AVPlayerViewControlsStyle::None
            });
        }
        Retained::from(<AVPlayerView as AsRef<NSView>>::as_ref(&view))
    };
    // Observe BEFORE loading, so the first item's status lands in the signal too.
    let observer = MediaObserver::new(mtm, player.clone(), id, p.looping);
    let metadata = MetadataDelegate::new(id);
    LIVE.with(|m| {
        m.borrow_mut().insert(
            key_of(&ns),
            Live {
                player: player.clone(),
                observer,
                metadata: metadata.clone(),
                audio_only: p.audio_only,
            },
        )
    });
    if !p.url.is_empty() {
        load_url(&player, &p.url, &metadata, mtm);
    }
    if p.autoplay {
        unsafe { player.play() };
    }
    ns
}

fn update(backend: &mut AppKit, h: &Retained<NSView>, patch: &MediaPatch) {
    let Some((player, metadata)) = LIVE.with(|m| {
        m.borrow()
            .get(&key_of(h))
            .map(|l| (l.player.clone(), l.metadata.clone()))
    }) else {
        return;
    };
    match patch {
        MediaPatch::Load(url) => {
            load_url(&player, url, &metadata, backend.mtm());
            unsafe { player.play() };
        }
        MediaPatch::Play => unsafe { player.play() },
        MediaPatch::Pause => unsafe { player.pause() },
        // Dropping the item is what lets a live stream's connection go; the KVO on
        // `currentItem.status` reports the resulting Idle.
        MediaPatch::Stop => unsafe {
            player.pause();
            player.replaceCurrentItemWithPlayerItem(None);
        },
        MediaPatch::Volume(v) => unsafe { player.setVolume(v.clamp(0.0, 1.0) as f32) },
    }
}

/// A sound-only player takes no room; a video fills what it is offered.
fn measure(backend: &mut AppKit, h: &Retained<NSView>, proposal: Proposal) -> Size {
    if LIVE.with(|m| m.borrow().get(&key_of(h)).is_some_and(|l| l.audio_only)) {
        return Size::ZERO;
    }
    day_pieces::fill_measure(backend, h, proposal)
}

/// Stop playback and drop the retained player + observer when the view goes away.
///
/// Without this the table grows by one entry per realized media view, and — worse — its key
/// is the view's ADDRESS, which the allocator reuses: a later view landing on a freed address
/// would inherit the dead entry's player and drive the wrong one.
fn release(_backend: &mut AppKit, h: &Retained<NSView>) {
    let Some(live) = LIVE.with(|m| m.borrow_mut().remove(&key_of(h))) else {
        return;
    };
    // Pause before the drop below releases the player — teardown, not deallocation order,
    // should be what silences it.
    unsafe { live.player.pause() };
    live.observer.detach();
}

day_pieces::renderer!(day_appkit::RENDERERS, AppKit,
    kind: KIND, props: MediaProps, patch: MediaPatch,
    make: make, update: update, measure: measure, release: release);
