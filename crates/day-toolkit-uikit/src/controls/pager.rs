use crate::Uikit;
use day_spec::Event;
use day_spec::NodeId;
use day_spec::controls::pager::*;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_foundation::{NSArray, NSString};
use objc2_ui_kit::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::rc::Weak;

struct State {
    node: NodeId,
    capacity: usize,
    controller: Retained<UIPageViewController>,
    pages: RefCell<Vec<Retained<UIViewController>>>,
    config: RefCell<PagerState>,
    pending: RefCell<Option<PagerState>>,
    busy: Cell<bool>,
    current: Cell<usize>,
    serial: Cell<u64>,
    finished: Cell<u64>,
    /// `PagerProps::track_progress` (docs/pager.md "Progress").
    track: bool,
    /// The page view controller's own scroll view and the observer of its offset, once
    /// found; registered for as long as this state lives.
    progress: RefCell<Option<(Retained<UIScrollView>, Retained<Progress>)>>,
    /// The last position reported, so an offset change that moves nothing sends nothing.
    reported: Cell<Option<f64>>,
}
impl Drop for State {
    fn drop(&mut self) {
        self.detach_progress();
    }
}
impl State {
    /// Observes the offset of the scroll view `UIPageViewController` pages with (its
    /// transition style is `.scroll`), the one continuous signal of where the pages are.
    /// Idempotent; the scroll view exists once the controller's view has loaded.
    fn attach_progress(state: &Rc<State>, mtm: MainThreadMarker) {
        if !state.track || state.progress.borrow().is_some() {
            return;
        }
        let Some(view) = state.controller.view() else {
            return;
        };
        let Some(scroll) = view
            .subviews()
            .iter()
            .find_map(|child| child.downcast::<UIScrollView>().ok())
        else {
            return;
        };
        let observer = Progress::alloc(mtm).set_ivars(ProgressIvars {
            state: Rc::downgrade(state),
        });
        // SAFETY: `observer` is a fresh DayPagerProgress with ivars set; NSObject's `init` takes no
        // arguments and returns the initialized object.
        let observer: Retained<Progress> = unsafe { msg_send![super(observer), init] };
        let key = NSString::from_str("contentOffset");
        // SAFETY: KVO does not retain the observer; `state.progress` keeps it and `scroll` until
        // `detach_progress` (also run on drop) removes it. Main thread; null context is allowed.
        unsafe {
            let _: () = msg_send![
                &*scroll,
                addObserver: &*observer,
                forKeyPath: &*key,
                options: 0usize,
                context: std::ptr::null_mut::<std::ffi::c_void>()
            ];
        }
        *state.progress.borrow_mut() = Some((scroll, observer));
    }
    fn detach_progress(&self) {
        if let Some((scroll, observer)) = self.progress.borrow_mut().take() {
            let key = NSString::from_str("contentOffset");
            // SAFETY: `attach_progress` registered `observer` on `scroll` for this key path; both
            // are still retained here. Main thread.
            unsafe {
                let _: () = msg_send![&*scroll, removeObserver: &*observer, forKeyPath: &*key];
            }
        }
    }
    /// The position in reading-order pages while the content moves: the current page's place
    /// plus how far the content has traveled from that page's origin, in page widths. The
    /// page's own frame in the scroll view is the reference, not the scroll view's resting
    /// offset: UIKit re-centers the content on the new page at the end of a turn, and moves
    /// the pages with it, so the position stays continuous through that jump.
    fn report_progress(&self) {
        let Some(scroll) = self.progress.borrow().as_ref().map(|(s, _)| s.clone()) else {
            return;
        };
        let (epoch, position) = {
            let config = self.config.borrow();
            let pages = self.pages.borrow();
            let current = self.current.get();
            let Some(place) = config.order.iter().position(|s| *s == current) else {
                return;
            };
            let origin_of = |slot: usize| -> Option<f64> {
                let view = pages.get(slot)?.viewIfLoaded()?;
                if !view.isDescendantOfView(&scroll) {
                    return None;
                }
                Some(
                    view.convertRect_toView(view.bounds(), Some(&scroll))
                        .origin
                        .x,
                )
            };
            let Some(origin) = origin_of(current) else {
                return;
            };
            // Which way the next page lies: where UIKit put a neighbor, when it has one in
            // place, and the layout direction otherwise.
            let forward = config
                .order
                .get(place + 1)
                .and_then(|s| origin_of(*s))
                .map(|x| if x < origin { -1.0 } else { 1.0 })
                .or_else(|| {
                    let before = config.order.get(place.checked_sub(1)?)?;
                    origin_of(*before).map(|x| if x > origin { -1.0 } else { 1.0 })
                })
                .unwrap_or_else(|| {
                    if scroll.effectiveUserInterfaceLayoutDirection()
                        == UIUserInterfaceLayoutDirection::RightToLeft
                    {
                        -1.0
                    } else {
                        1.0
                    }
                });
            let position = crate::scrolling::pager_position(
                place,
                scroll.contentOffset().x,
                origin,
                UICoordinateSpace::bounds(&*scroll).size.width,
                forward,
            );
            (config.epoch, position)
        };
        // UIKit re-centers its content at the end of a turn in two steps (the pages, then the
        // offset), and the offset change between them reads a whole page off (seen on iOS
        // 18.6: 0.99, 1.00, 2.00, 1.00 for one swipe). No drag or animation moves the content
        // three quarters of a page in one change, so such a reading is that seam; the rest
        // report (`send_rest`) does not come through here.
        if self
            .reported
            .get()
            .is_some_and(|last| (position - last).abs() >= 0.75)
        {
            return;
        }
        self.send_progress(epoch, position);
    }
    /// `progress <epoch> <position>` on the pager's custom channel, once per distinct value.
    fn send_progress(&self, epoch: u64, position: f64) {
        if !self.track
            || !position.is_finite()
            || self.reported.replace(Some(position)) == Some(position)
        {
            return;
        }
        crate::emit(
            self.node,
            Event::custom("pager", format!("progress {epoch} {position}")),
        );
    }
    /// The integral position of the page shown at rest.
    fn send_rest(&self) {
        let (epoch, place) = {
            let config = self.config.borrow();
            let current = self.current.get();
            (
                config.epoch,
                config.order.iter().position(|s| *s == current),
            )
        };
        if let Some(place) = place {
            self.send_progress(epoch, place as f64);
        }
    }
    fn neighbor(
        &self,
        page: &UIViewController,
        forward: bool,
    ) -> Option<Retained<UIViewController>> {
        let pages = self.pages.borrow();
        let slot = pages
            .iter()
            .position(|p| std::ptr::eq(&raw const **p, page))?;
        let config = self.config.borrow();
        let pos = config.order.iter().position(|s| *s == slot)?;
        let pos = if forward {
            pos.checked_add(1)?
        } else {
            pos.checked_sub(1)?
        };
        pages.get(*config.order.get(pos)?).cloned()
    }
    fn apply(&self, config: PagerState) {
        if self.busy.get() {
            *self.pending.borrow_mut() = Some(config);
            return;
        }
        let page = self.pages.borrow().get(config.selected).cloned();
        self.current.set(config.selected);
        *self.config.borrow_mut() = config;
        if let Some(page) = page {
            // Refresh UIKit's cached neighbors only at rest; never in its completion stack.
            self.controller
                .setViewControllers_direction_animated_completion(
                    Some(&NSArray::from_retained_slice(&[page])),
                    UIPageViewControllerNavigationDirection::Forward,
                    false,
                    None,
                );
        }
        self.send_rest();
    }
}
struct DelegateIvars {
    state: Weak<State>,
}
define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DayPagerDelegate"]
    #[ivars = DelegateIvars]
    struct Delegate;
    unsafe impl NSObjectProtocol for Delegate {}
    unsafe impl UIPageViewControllerDataSource for Delegate {
        #[unsafe(method_id(pageViewController:viewControllerBeforeViewController:))]
        fn before(&self, _: &UIPageViewController, page: &UIViewController) -> Option<Retained<UIViewController>> {
            self.ivars().state.upgrade().and_then(|state| state.neighbor(page, false))
        }
        #[unsafe(method_id(pageViewController:viewControllerAfterViewController:))]
        fn after(&self, _: &UIPageViewController, page: &UIViewController) -> Option<Retained<UIViewController>> {
            self.ivars().state.upgrade().and_then(|state| state.neighbor(page, true))
        }
    }
    unsafe impl UIPageViewControllerDelegate for Delegate {
        #[unsafe(method(pageViewController:willTransitionToViewControllers:))]
        fn begin(&self, _: &UIPageViewController, _: &NSArray<UIViewController>) {
            if let Some(state) = self.ivars().state.upgrade() {
                state.busy.set(true);
                state.serial.set(state.serial.get().wrapping_add(1));
                crate::emit(state.node, Event::custom("pager", format!("begin {}",state.config.borrow().epoch)));
            }
        }
        #[unsafe(method(pageViewController:didFinishAnimating:previousViewControllers:transitionCompleted:))]
        fn finish(&self, controller: &UIPageViewController, _: bool, _: &NSArray<UIViewController>, completed: bool) {
            if let Some(state) = self.ivars().state.upgrade() {
                if completed && let Some(pages) = controller.viewControllers() && let Some(page) = pages.firstObject()
                    && let Some(slot) = state.pages.borrow().iter().position(|p| std::ptr::eq(&raw const **p, &raw const *page)) {
                    state.current.set(slot);
                }
                state.finished.set(state.serial.get());
                // UIKit is still unwinding its scroll transition. Dispatch next runloop turn.
                // SAFETY: NSObject implements `performSelector:withObject:afterDelay:` (void); the
                // run loop retains `self` until `settled` (no argument) runs. Main thread.
                unsafe { let _: () = msg_send![self, performSelector:sel!(settled), withObject:std::ptr::null::<NSObject>(), afterDelay:0.0f64]; }
            }
        }
    }
    impl Delegate {
        #[unsafe(method(settled))]
        fn settled(&self) {
            if let Some(state) = self.ivars().state.upgrade() {
                if !state.busy.get() || state.finished.get() != state.serial.get() { return; }
                state.busy.set(false);
                crate::emit(state.node, Event::custom("pager",format!("settle {} {}",state.config.borrow().epoch,state.current.get())));
                state.send_rest();
                let pending = state.pending.borrow_mut().take();
                if let Some(pending) = pending { state.apply(pending); }
            }
        }
    }
);
struct ProgressIvars {
    state: Weak<State>,
}
define_class!(
    // Key-value observer of the paging scroll view's `contentOffset` (docs/pager.md
    // "Progress"). The page view controller is that scroll view's delegate and keeps it, so
    // the offset is observed rather than delegated.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DayPagerProgress"]
    #[ivars = ProgressIvars]
    struct Progress;
    impl Progress {
        #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
        fn observe(&self, _key: Option<&NSString>, _object: Option<&AnyObject>, _change: Option<&AnyObject>, _context: *mut std::ffi::c_void) {
            day_spec::ffi_guard::contain((), || {
                if let Some(state) = self.ivars().state.upgrade() {
                    state.report_progress();
                }
            });
        }
    }
);
struct HostIvars {
    state: Rc<State>,
    _delegate: Retained<Delegate>,
}
define_class!(
    #[unsafe(super(UIView))]
    #[thread_kind = MainThreadOnly]
    #[name = "DayPagerHost"]
    #[ivars = HostIvars]
    struct Host;
    impl Host {
        #[unsafe(method(didAddSubview:))]
        fn added(&self, child: &UIView) {
            let state = &self.ivars().state;
            if state.controller.view().is_some_and(|v| std::ptr::eq(&raw const *v,child)) { return; }
            if state.pages.borrow().len() >= state.capacity { return; }
            let controller = UIViewController::new(self.mtm());
            child.removeFromSuperview(); controller.setView(Some(child));
            state.pages.borrow_mut().push(controller);
        }
        #[unsafe(method(layoutSubviews))]
        fn layout(&self) {
            // SAFETY: UIView implements `layoutSubviews` (no arguments, void); this is its
            // override.
            unsafe { let _: () = msg_send![super(self), layoutSubviews]; }
            if let Some(view) = self.ivars().state.controller.view() { view.setFrame(self.bounds()); }
        }
        #[unsafe(method(didMoveToWindow))]
        fn moved(&self) {
            State::attach_progress(&self.ivars().state, self.mtm());
            let controller = &self.ivars().state.controller;
            if self.window().is_none() || controller.parentViewController().is_some() { return; }
            let mut responder = self.nextResponder();
            while let Some(r) = responder {
                if let Some(parent) = r.downcast_ref::<UIViewController>() {
                    parent.addChildViewController(controller);
                    controller.didMoveToParentViewController(Some(parent));
                    // Preserve the navigation controller's interactive edge-back gesture.
                    if let Some(nav) = parent.navigationController() && let Some(back) = nav.interactivePopGestureRecognizer()
                        && let Some(view) = controller.view() {
                        for child in view.subviews() {
                            if let Some(scroll) = child.downcast_ref::<UIScrollView>() {
                                scroll.panGestureRecognizer().requireGestureRecognizerToFail(&back);
                                scroll.setDirectionalLockEnabled(true);
                            }
                        }
                    }
                    break;
                }
                responder = r.nextResponder();
            }
        }
    }
);
fn make(_: &mut Uikit, p: &PagerProps, id: NodeId) -> Retained<UIView> {
    let mtm = MainThreadMarker::new().unwrap();
    // SAFETY: unaudited Apple API; live operands on the main thread (crate note "Unsafe calls").
    let controller = unsafe {
        UIPageViewController::initWithTransitionStyle_navigationOrientation_options(
            UIPageViewController::alloc(mtm),
            UIPageViewControllerTransitionStyle::Scroll,
            UIPageViewControllerNavigationOrientation::Horizontal,
            None,
        )
    };
    let state = Rc::new(State {
        node: id,
        capacity: p.slots,
        controller,
        pages: RefCell::new(vec![]),
        config: RefCell::new(PagerState::default()),
        pending: RefCell::new(None),
        busy: Cell::new(false),
        current: Cell::new(0),
        serial: Cell::new(0),
        finished: Cell::new(0),
        track: p.track_progress,
        progress: RefCell::new(None),
        reported: Cell::new(None),
    });
    let delegate = Delegate::alloc(mtm).set_ivars(DelegateIvars {
        state: Rc::downgrade(&state),
    });
    // SAFETY: `delegate` is a fresh DayPagerDelegate with ivars set; NSObject's `init` takes no
    // arguments and returns the initialized object.
    let delegate: Retained<Delegate> = unsafe { msg_send![super(delegate), init] };
    state
        .controller
        .setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    state
        .controller
        .setDataSource(Some(ProtocolObject::from_ref(&*delegate)));
    let host = Host::alloc(mtm).set_ivars(HostIvars {
        state: state.clone(),
        _delegate: delegate,
    });
    // SAFETY: `host` is a fresh DayPagerHost with ivars set; UIView's `init` takes no arguments and
    // returns the initialized view.
    let host: Retained<Host> = unsafe { msg_send![super(host), init] };
    host.setClipsToBounds(true);
    if let Some(view) = state.controller.view() {
        host.addSubview(&view);
    }
    State::attach_progress(&state, mtm);
    Retained::into_super(host)
}
fn update(_: &mut Uikit, h: &Retained<UIView>, p: &PagerPatch) {
    if let Some(host) = h.downcast_ref::<Host>() {
        host.ivars().state.apply(p.0.clone());
    }
}
fn release(_: &mut Uikit, h: &Retained<UIView>) {
    if let Some(host) = h.downcast_ref::<Host>() {
        host.ivars().state.detach_progress();
        let controller = &host.ivars().state.controller;
        controller.setDelegate(None);
        controller.setDataSource(None);
        controller.willMoveToParentViewController(None);
        controller.removeFromParentViewController();
    }
}
day_spec::renderer!(crate::RENDERERS, Uikit, kind:KIND, props:PagerProps, patch:PagerPatch, make:make, update:update, measure:day_spec::fill_measure, release:release);
