// ---------------------------------------------------------------------------
// UIKit: UIActivityIndicatorView — the native iOS activity indicator. `.large` selects the Large
// style (vs Medium); `startAnimating`/`stopAnimating` run/stop it. `hidesWhenStopped = false` keeps
// a stopped indicator visible (a frozen indicator), mirroring the AppKit `displayedWhenStopped`
// choice. objc2-ui-kit binds the whole control, so — unlike the media piece's hand-rolled
// AVPlayerViewController — no `extern_class!` shim is needed.
// ---------------------------------------------------------------------------

use super::*;
use day_spec::NodeId;
use day_uikit::Uikit;
use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_ui_kit::{UIActivityIndicatorView, UIActivityIndicatorViewStyle, UIView};

fn style(large: bool) -> UIActivityIndicatorViewStyle {
    if large {
        UIActivityIndicatorViewStyle::Large
    } else {
        UIActivityIndicatorViewStyle::Medium
    }
}

fn set_animating(spinner: &UIActivityIndicatorView, on: bool) {
    if on {
        spinner.startAnimating();
    } else {
        spinner.stopAnimating();
    }
}

fn make(_backend: &mut Uikit, p: &ActivityProps, _id: NodeId) -> Retained<UIView> {
    let mtm = MainThreadMarker::new().unwrap();
    let spinner = UIActivityIndicatorView::initWithActivityIndicatorStyle(
        UIActivityIndicatorView::alloc(mtm),
        style(p.large),
    );
    spinner.setHidesWhenStopped(false);
    set_animating(&spinner, p.animating);
    Retained::from(<UIActivityIndicatorView as AsRef<UIView>>::as_ref(&spinner))
}

fn update(_backend: &mut Uikit, h: &Retained<UIView>, patch: &ActivityPatch) {
    let Some(spinner) = h.downcast_ref::<UIActivityIndicatorView>() else {
        return;
    };
    match patch {
        ActivityPatch::Animating(on) => set_animating(spinner, *on),
    }
}

day_pieces::renderer!(day_uikit::RENDERERS, Uikit,
    kind: KIND, props: ActivityProps, patch: ActivityPatch,
    make: make, update: update);
