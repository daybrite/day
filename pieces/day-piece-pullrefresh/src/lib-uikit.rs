// ---------------------------------------------------------------------------
// UIKit: the real thing — UIRefreshControl. The piece's realized node is a passthrough host
// `UIView` subclass (DayRefreshHost): when day-core mounts the wrapped scrollable into it
// (generic `addSubview`), `didAddSubview:` sees the `UIScrollView` (a `UITableView` from `list()`
// IS one) and assigns the prepared `UIRefreshControl` to its `refreshControl` property — the
// attach is fully piece-internal, no framework child hook needed. The control's `valueChanged`
// fires on a user pull and reports back through `Event::custom` (§8.2); `RefreshPatch` drives
// `beginRefreshing`/`endRefreshing` for the programmatic path.
// ---------------------------------------------------------------------------

use super::*;

use day_spec::NodeId;
use day_uikit::Uikit;
use objc2::rc::Retained;
use objc2::runtime::NSObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_ui_kit::{UIControlEvents, UIRefreshControl, UIScrollView, UIView};

struct TargetIvars {
    node: NodeId,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DayPullRefreshTarget"]
    #[ivars = TargetIvars]
    struct RefreshTarget;

    impl RefreshTarget {
        /// UIControl target-action for `UIControlEventValueChanged` — the user pulled.
        #[unsafe(method(refreshPulled:))]
        fn refresh_pulled(&self, _sender: &UIRefreshControl) {
            day_uikit::emit(
                self.ivars().node,
                Event::custom("pullrefresh:begin", ""),
            );
        }
    }
);

impl RefreshTarget {
    fn new(mtm: MainThreadMarker, node: NodeId) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TargetIvars { node });
        unsafe { msg_send![super(this), init] }
    }
}

struct HostIvars {
    control: Retained<UIRefreshControl>,
    // UIControl targets are held weakly — the host retains the target for the control's lifetime.
    _target: Retained<RefreshTarget>,
}

define_class!(
    #[unsafe(super(UIView))]
    #[thread_kind = MainThreadOnly]
    #[name = "DayRefreshHost"]
    #[ivars = HostIvars]
    struct RefreshHost;

    impl RefreshHost {
        /// day-core mounts the wrapped scrollable as a direct subview; hand it the refresh control.
        #[unsafe(method(didAddSubview:))]
        fn did_add_subview(&self, subview: &UIView) {
            if let Some(sv) = subview.downcast_ref::<UIScrollView>() {
                sv.setRefreshControl(Some(&self.ivars().control));
            }
        }
    }
);

fn make(_backend: &mut Uikit, p: &RefreshProps, id: NodeId) -> Retained<UIView> {
    let mtm = MainThreadMarker::new().unwrap();
    let control: Retained<UIRefreshControl> =
        unsafe { msg_send![UIRefreshControl::alloc(mtm), init] };
    let target = RefreshTarget::new(mtm, id);
    let target_obj: &objc2::runtime::AnyObject = &target;
    unsafe {
        control.addTarget_action_forControlEvents(
            Some(target_obj),
            sel!(refreshPulled:),
            UIControlEvents::ValueChanged,
        );
    }
    if p.refreshing {
        control.beginRefreshing();
    }
    let host = RefreshHost::alloc(mtm).set_ivars(HostIvars {
        control,
        _target: target,
    });
    let host: Retained<RefreshHost> = unsafe { msg_send![super(host), init] };
    Retained::into_super(host)
}

fn update(_backend: &mut Uikit, h: &Retained<UIView>, patch: &RefreshPatch) {
    let Some(host) = (**h).downcast_ref::<RefreshHost>() else {
        return;
    };
    let control = &host.ivars().control;
    let RefreshPatch::SetRefreshing(on) = patch;
    if *on {
        if !control.isRefreshing() {
            control.beginRefreshing();
        }
    } else if control.isRefreshing() {
        control.endRefreshing();
    }
}

day_pieces::renderer!(day_uikit::RENDERERS, Uikit,
    kind: KIND, props: RefreshProps, patch: RefreshPatch,
    make: make, update: update, measure: day_pieces::fill_measure);
