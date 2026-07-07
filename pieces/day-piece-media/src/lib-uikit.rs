// ---------------------------------------------------------------------------
// UIKit: AVPlayerViewController (AVKit) fronting an AVPlayer (AVFoundation) — the same player as
// AppKit, but objc2-av-kit 0.3 only generates the macOS (AVPlayerView) binding, so here we
// hand-roll the view controller via `extern_class!` + `msg_send!` (exactly how the webview
// hand-rolls WKWebView on iOS). Its `view` is the leaf UIView day-uikit manages; the controller
// itself is retained in a thread_local (nothing else holds it once its view is embedded). Looping
// uses the same NSNotificationCenter observer as lib-appkit.rs.
// ---------------------------------------------------------------------------

use super::*;
use std::cell::RefCell;
use std::collections::HashMap;

use day_spec::NodeId;
use day_uikit::Uikit;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol};
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, extern_class, msg_send, sel,
};
use objc2_av_foundation::{AVPlayer, AVPlayerItem, AVPlayerItemDidPlayToEndTimeNotification};
use objc2_core_media::kCMTimeZero;
use objc2_foundation::{NSNotificationCenter, NSString, NSURL};
use objc2_ui_kit::{UIResponder, UIView, UIViewController};

// AVPlayerViewController lives in AVKit.framework, which must be LINKED or
// `objc_getClass("AVPlayerViewController")` returns nil and `alloc` aborts (SIGABRT) — declared
// via this crate's `[package.metadata.day.ios].frameworks = ["AVKit", "AVFoundation"]`, which the
// generated DayPieces SwiftPM package links into the app (the framework-contribution seam).

// The iOS AVPlayerViewController (a UIViewController subclass). We only need a handful of methods,
// called via msg_send!.
extern_class!(
    #[unsafe(super(UIViewController, UIResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    struct AVPlayerViewController;
);

struct LoopIvars {
    player: Retained<AVPlayer>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DayMediaLoopUIKit"]
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

/// What we retain per media view: the controller (and through it the player) plus the optional
/// loop observer.
type MediaRefs = (
    Retained<AVPlayerViewController>,
    Option<Retained<MediaLoop>>,
);

thread_local! {
    // Keep each (controller, loop observer) alive as long as its view — the update path finds the
    // controller (and through it the player) by the leaf view's pointer.
    static CONTROLLERS: RefCell<HashMap<usize, MediaRefs>> = RefCell::new(HashMap::new());
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

fn make(_backend: &mut Uikit, p: &MediaProps, _id: NodeId) -> Retained<UIView> {
    let mtm = MainThreadMarker::new().unwrap();
    let vc: Retained<AVPlayerViewController> =
        unsafe { msg_send![AVPlayerViewController::alloc(mtm), init] };
    let player: Retained<AVPlayer> = unsafe { msg_send![AVPlayer::alloc(mtm), init] };
    unsafe {
        player.setMuted(p.muted);
        let _: () = msg_send![&vc, setPlayer: &*player];
        let _: () = msg_send![&vc, setShowsPlaybackControls: p.controls];
    }
    if !p.url.is_empty() {
        load_url(&player, &p.url, mtm);
    }
    if p.autoplay {
        unsafe { player.play() };
    }
    let observer = p.looping.then(|| MediaLoop::new(mtm, player));
    let view: Retained<UIView> = unsafe { msg_send![&vc, view] };
    CONTROLLERS.with(|m| {
        m.borrow_mut()
            .insert((view.as_ref() as *const UIView) as usize, (vc, observer))
    });
    view
}

fn update(_backend: &mut Uikit, h: &Retained<UIView>, patch: &MediaPatch) {
    let key = (h.as_ref() as *const UIView) as usize;
    let Some(player) = CONTROLLERS.with(|m| {
        m.borrow().get(&key).map(|(vc, _)| {
            let p: Option<Retained<AVPlayer>> = unsafe { msg_send![&**vc, player] };
            p
        })
    }) else {
        return;
    };
    let Some(player) = player else {
        return;
    };
    match patch {
        MediaPatch::Load(url) => {
            let mtm = MainThreadMarker::new().unwrap();
            load_url(&player, url, mtm);
            unsafe { player.play() };
        }
        MediaPatch::Play => unsafe { player.play() },
        MediaPatch::Pause => unsafe { player.pause() },
    }
}

day_pieces::renderer!(day_uikit::RENDERERS, Uikit,
    kind: KIND, props: MediaProps, patch: MediaPatch,
    make: make, update: update, measure: day_pieces::fill_measure);
