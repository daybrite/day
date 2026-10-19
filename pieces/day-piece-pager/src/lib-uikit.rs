use super::*;
use day_spec::NodeId;
use day_uikit::Uikit;
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_foundation::NSArray;
use objc2_ui_kit::*;
use std::cell::{Cell, RefCell};
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
}
impl State {
    fn neighbor(
        &self,
        page: &UIViewController,
        forward: bool,
    ) -> Option<Retained<UIViewController>> {
        let pages = self.pages.borrow();
        let slot = pages.iter().position(|p| std::ptr::eq(&**p, page))?;
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
                day_uikit::emit(state.node, Event::custom("pager", format!("begin {}",state.config.borrow().epoch)));
            }
        }
        #[unsafe(method(pageViewController:didFinishAnimating:previousViewControllers:transitionCompleted:))]
        fn finish(&self, controller: &UIPageViewController, _: bool, _: &NSArray<UIViewController>, completed: bool) {
            if let Some(state) = self.ivars().state.upgrade() {
                if completed && let Some(pages) = controller.viewControllers() && let Some(page) = pages.firstObject()
                    && let Some(slot) = state.pages.borrow().iter().position(|p| std::ptr::eq(&**p, &*page)) {
                    state.current.set(slot);
                }
                state.finished.set(state.serial.get());
                // UIKit is still unwinding its scroll transition. Dispatch next runloop turn.
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
                day_uikit::emit(state.node, Event::custom("pager",format!("settle {} {}",state.config.borrow().epoch,state.current.get())));
                let pending = state.pending.borrow_mut().take();
                if let Some(pending) = pending { state.apply(pending); }
            }
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
            if state.controller.view().is_some_and(|v| std::ptr::eq(&*v,child)) { return; }
            if state.pages.borrow().len() >= state.capacity { return; }
            let controller = UIViewController::new(self.mtm());
            child.removeFromSuperview(); controller.setView(Some(child));
            state.pages.borrow_mut().push(controller);
        }
        #[unsafe(method(layoutSubviews))]
        fn layout(&self) {
            unsafe { let _: () = msg_send![super(self), layoutSubviews]; }
            if let Some(view) = self.ivars().state.controller.view() { view.setFrame(self.bounds()); }
        }
        #[unsafe(method(didMoveToWindow))]
        fn moved(&self) {
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
    });
    let delegate = Delegate::alloc(mtm).set_ivars(DelegateIvars {
        state: Rc::downgrade(&state),
    });
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
    let host: Retained<Host> = unsafe { msg_send![super(host), init] };
    host.setClipsToBounds(true);
    if let Some(view) = state.controller.view() {
        host.addSubview(&view);
    }
    Retained::into_super(host)
}
fn update(_: &mut Uikit, h: &Retained<UIView>, p: &PagerPatch) {
    if let Some(host) = h.downcast_ref::<Host>() {
        host.ivars().state.apply(p.0.clone());
    }
}
fn release(_: &mut Uikit, h: &Retained<UIView>) {
    if let Some(host) = h.downcast_ref::<Host>() {
        let controller = &host.ivars().state.controller;
        controller.setDelegate(None);
        controller.setDataSource(None);
        controller.willMoveToParentViewController(None);
        controller.removeFromParentViewController();
    }
}
day_pieces::renderer!(day_uikit::RENDERERS, Uikit, kind:KIND, props:PagerProps, patch:PagerPatch, make:make, update:update, measure:day_pieces::fill_measure, release:release);
