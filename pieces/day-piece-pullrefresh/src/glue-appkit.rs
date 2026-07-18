// ---------------------------------------------------------------------------
// AppKit emulated-gesture glue: macOS has no native pull-to-refresh, but NSScrollView's ELASTIC
// scrolling (trackpad rubber-band) exposes exactly the gesture — during an overscroll past the top
// the clip view's bounds origin goes NEGATIVE (the document view is flipped, so top-of-content is
// y = 0). Observe NSViewBoundsDidChangeNotification on the clip view and fire once per pull when
// the overshoot crosses a threshold, re-arming when the scroll settles back to the top. Applied as
// a `Decorate::tweak` on the wrapped scrollable (docs/tweaks.md); inert when the child's realized
// view is not an NSScrollView.
//
// Safety: the observer does NOT run app logic directly — it routes a `pullrefresh:begin` Custom
// event through the backend's sink (`day_appkit::emit`), queued and pumped with the framework's
// panic containment, exactly like a built-in control's event. The piece's `cx.on` wire on the host
// node turns it into the begin.
// ---------------------------------------------------------------------------

use std::cell::Cell;

use day_core::RNode;
use day_spec::{Event, NodeId};
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSScrollView, NSViewBoundsDidChangeNotification};
use objc2_foundation::{NSNotification, NSNotificationCenter};

/// How far past the top (in points) the elastic overscroll must travel to count as a pull —
/// roughly UIRefreshControl's activation distance.
const PULL_THRESHOLD: f64 = 60.0;

pub(crate) fn attach(node: RNode, host: NodeId) {
    let _ = day_appkit::with_native(node, |view, _class, _mtm| {
        let Some(sv) = view.downcast_ref::<NSScrollView>() else {
            return; // not a scroll-backed child — gesture inert (spinner overlay still works)
        };
        let clip = sv.contentView();
        clip.setPostsBoundsChangedNotifications(true);

        // Fire once per gesture: armed while at/below the top, disarmed after firing until the
        // scroll returns to rest at the top.
        let armed = Cell::new(true);
        let clip_for_block = clip.clone();
        let block = block2::RcBlock::new(move |_: std::ptr::NonNull<NSNotification>| {
            let y = clip_for_block.bounds().origin.y;
            if y <= -PULL_THRESHOLD {
                if armed.get() {
                    armed.set(false);
                    day_appkit::emit(host, Event::custom("pullrefresh:begin", ""));
                }
            } else if y >= 0.0 {
                armed.set(true);
            }
        });
        let center = NSNotificationCenter::defaultCenter();
        let clip_obj: &AnyObject = &clip;
        let token = unsafe {
            center.addObserverForName_object_queue_usingBlock(
                Some(NSViewBoundsDidChangeNotification),
                Some(clip_obj),
                None,
                &block,
            )
        };
        // Unregister when the piece's scope is disposed (the native_ref cleanup pattern).
        day_reactive::Scope::current().on_cleanup(move || {
            let center = NSNotificationCenter::defaultCenter();
            // The observer token is a ProtocolObject (no Deref to AnyObject); every ObjC object
            // shares representation, so the pointer cast is the standard bridge.
            let ptr = objc2::rc::Retained::as_ptr(&token) as *const AnyObject;
            unsafe { center.removeObserver(&*ptr) };
        });
    });
}
