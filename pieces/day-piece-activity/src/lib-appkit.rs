// ---------------------------------------------------------------------------
// AppKit: NSProgressIndicator in the Spinning style — the native macOS activity indicator.
// `setIndeterminate(true)` + `startAnimation:`/`stopAnimation:` run/stop the spin; `.large` maps to
// `controlSize` (Large vs Regular). `setDisplayedWhenStopped(true)` keeps a stopped indicator on
// screen (a frozen indicator, matching UIKit's `hidesWhenStopped = false`) rather than vanishing.
// ---------------------------------------------------------------------------

use super::*;
use day_appkit::AppKit;
use day_spec::NodeId;
use objc2::rc::Retained;
use objc2::{MainThreadOnly, msg_send};
use objc2_app_kit::{NSControlSize, NSProgressIndicator, NSProgressIndicatorStyle, NSView};

fn control_size(large: bool) -> NSControlSize {
    if large {
        NSControlSize::Large
    } else {
        NSControlSize::Regular
    }
}

fn set_animating(spinner: &NSProgressIndicator, on: bool) {
    // startAnimation:/stopAnimation: are `unsafe` in objc2 (they take a sender selector object).
    unsafe {
        if on {
            spinner.startAnimation(None);
        } else {
            spinner.stopAnimation(None);
        }
    }
}

fn make(backend: &mut AppKit, p: &ActivityProps, _id: NodeId) -> Retained<NSView> {
    let mtm = backend.mtm();
    // SAFETY: creates an NSProgressIndicator on the main thread.
    let spinner: Retained<NSProgressIndicator> =
        unsafe { msg_send![NSProgressIndicator::alloc(mtm), init] };
    spinner.setStyle(NSProgressIndicatorStyle::Spinning);
    spinner.setIndeterminate(true);
    spinner.setDisplayedWhenStopped(true);
    spinner.setControlSize(control_size(p.large));
    set_animating(&spinner, p.animating);
    Retained::from(<NSProgressIndicator as AsRef<NSView>>::as_ref(&spinner))
}

fn update(_backend: &mut AppKit, h: &Retained<NSView>, patch: &ActivityPatch) {
    let Some(spinner) = h.downcast_ref::<NSProgressIndicator>() else {
        return;
    };
    match patch {
        ActivityPatch::Animating(on) => set_animating(spinner, *on),
    }
}

day_pieces::renderer!(day_appkit::RENDERERS, AppKit,
    kind: KIND, props: ActivityProps, patch: ActivityPatch,
    make: make, update: update);
