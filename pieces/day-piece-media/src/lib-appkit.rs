// ---------------------------------------------------------------------------
// AppKit: AVPlayerView (AVKit) fronting an AVPlayer (AVFoundation) — native transport chrome for
// free via `controlsStyle`. Looping has no AVPlayer flag, so a small NSObject observer watches
// AVPlayerItemDidPlayToEndTimeNotification and seeks back to zero; the observer is retained in a
// thread_local for the view's lifetime (the notification center does not retain observers).
// ---------------------------------------------------------------------------

use super::*;
use std::cell::RefCell;
use std::collections::HashMap;

use day_appkit::AppKit;
use day_spec::NodeId;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::NSView;
use objc2_av_foundation::{AVPlayer, AVPlayerItem, AVPlayerItemDidPlayToEndTimeNotification};
use objc2_av_kit::{AVPlayerView, AVPlayerViewControlsStyle};
use objc2_core_media::kCMTimeZero;
use objc2_foundation::{NSNotificationCenter, NSObject, NSString, NSURL};

struct LoopIvars {
    player: Retained<AVPlayer>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DayMediaLoop"]
    #[ivars = LoopIvars]
    struct MediaLoop;

    unsafe impl NSObjectProtocol for MediaLoop {}

    impl MediaLoop {
        // Fired when ANY player item plays to its end (registered with object: nil so `.load()`
        // swaps stay covered) — loop only when it is OUR player's current item.
        #[unsafe(method(itemDidPlayToEnd:))]
        fn item_did_play_to_end(&self, note: *mut AnyObject) {
            let player = &self.ivars().player;
            let Some(current) = (unsafe { player.currentItem() }) else {
                return;
            };
            let ended: *mut AnyObject = unsafe { msg_send![&*note, object] };
            if ended != Retained::as_ptr(&current).cast_mut().cast() {
                return;
            }
            unsafe {
                player.seekToTime(kCMTimeZero);
                player.play();
            }
        }
    }
);

impl MediaLoop {
    fn new(mtm: MainThreadMarker, player: Retained<AVPlayer>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(LoopIvars { player });
        let this: Retained<Self> = unsafe { msg_send![super(this), init] };
        unsafe {
            NSNotificationCenter::defaultCenter().addObserver_selector_name_object(
                &this,
                sel!(itemDidPlayToEnd:),
                Some(AVPlayerItemDidPlayToEndTimeNotification),
                None,
            );
        }
        this
    }
}

thread_local! {
    // Keep each loop observer alive as long as its player view (the center holds it weakly).
    static OBSERVERS: RefCell<HashMap<usize, Retained<MediaLoop>>> = RefCell::new(HashMap::new());
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

fn load_url(player: &AVPlayer, source: &str, mtm: MainThreadMarker) {
    let Some(url) = media_url(source) else {
        return;
    };
    let item = unsafe { AVPlayerItem::playerItemWithURL(&url, mtm) };
    unsafe { player.replaceCurrentItemWithPlayerItem(Some(&item)) };
}

fn make(backend: &mut AppKit, p: &MediaProps, _id: NodeId) -> Retained<NSView> {
    let mtm = backend.mtm();
    // SAFETY: creates an AVPlayerView + AVPlayer on the main thread.
    let view: Retained<AVPlayerView> = unsafe { msg_send![AVPlayerView::alloc(mtm), init] };
    let player: Retained<AVPlayer> = unsafe { msg_send![AVPlayer::alloc(mtm), init] };
    unsafe {
        player.setMuted(p.muted);
        view.setPlayer(Some(&player));
        view.setControlsStyle(if p.controls {
            AVPlayerViewControlsStyle::Inline
        } else {
            AVPlayerViewControlsStyle::None
        });
    }
    if !p.url.is_empty() {
        load_url(&player, &p.url, mtm);
    }
    if p.autoplay {
        unsafe { player.play() };
    }
    let ns: Retained<NSView> = Retained::from(<AVPlayerView as AsRef<NSView>>::as_ref(&view));
    if p.looping {
        let observer = MediaLoop::new(mtm, player);
        OBSERVERS.with(|m| {
            m.borrow_mut()
                .insert((ns.as_ref() as *const NSView) as usize, observer)
        });
    }
    ns
}

fn update(backend: &mut AppKit, h: &Retained<NSView>, patch: &MediaPatch) {
    let Some(view) = h.downcast_ref::<AVPlayerView>() else {
        return;
    };
    let Some(player) = (unsafe { view.player() }) else {
        return;
    };
    match patch {
        MediaPatch::Load(url) => {
            load_url(&player, url, backend.mtm());
            unsafe { player.play() };
        }
        MediaPatch::Play => unsafe { player.play() },
        MediaPatch::Pause => unsafe { player.pause() },
    }
}

day_pieces::renderer!(day_appkit::RENDERERS, AppKit,
    kind: KIND, props: MediaProps, patch: MediaPatch,
    make: make, update: update, measure: day_pieces::fill_measure);
