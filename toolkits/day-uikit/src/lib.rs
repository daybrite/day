//! day-uikit — the ios-uikit backend (DESIGN.md §9). objc2, pure Rust, no shim.
//!
//! `Handle = Retained<UIView>`; UIKit is top-left/y-down so Day frames apply directly. The app
//! boots via `UIApplicationMain` + a `define_class!` app delegate (pane's proven pattern: the
//! delegate class is force-registered before `UIApplicationMain`, and exposes `window`/
//! `setWindow:` for the no-scene-manifest compat path). iOS-only (`cfg(target_os = "ios")`);
//! host builds see an empty crate.

#![allow(unused_unsafe)]

#[cfg(target_os = "ios")]
pub use imp::*;

#[cfg(target_os = "ios")]
mod picker;
#[cfg(target_os = "ios")]
mod textarea;

#[cfg(target_os = "ios")]
pub mod ext;
#[cfg(target_os = "ios")]
pub use ext::*;

#[cfg(target_os = "ios")]
mod imp {
    use std::any::Any;
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;
    use std::ffi::{c_char, c_int};
    use std::ptr::NonNull;
    use std::rc::Rc;

    use linkme::distributed_slice;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, NSObjectProtocol, ProtocolObject};
    use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
    use objc2_core_foundation::{CGAffineTransform, CGFloat, CGPoint, CGRect, CGSize};
    use objc2_core_graphics::CGContext;
    use objc2_foundation::{NSObject, NSString};
    use objc2_quartz_core::CADisplayLink;
    // UIApplicationMain is "deprecated" in objc2 only as a rename to the private
    // `UIApplication::__main` binding; the classic entry point is what we want.
    use objc2_ui_kit::NSIndexPathUIKitAdditions as _;
    #[allow(deprecated)]
    use objc2_ui_kit::UIApplicationMain;
    use objc2_ui_kit::UINavigationControllerDelegate;
    use objc2_ui_kit::{
        UIAction, UIContextMenuConfiguration, UIContextMenuInteraction,
        UIContextMenuInteractionDelegate, UIMenu, UIMenuElement, UIMenuElementAttributes,
        UIMenuOptions,
    };
    use objc2_ui_kit::{
        UIActivityIndicatorView, UIApplication, UIApplicationDelegate, UIButton, UIButtonType,
        UIColor, UIControl, UIControlEvents, UIControlState, UILabel, UIModalPresentationStyle,
        UIProgressView, UIRectEdge, UIScreen, UIScrollView, UISlider, UISwitch, UITextBorderStyle,
        UITextField, UIView, UIViewAnimationOptions, UIViewController, UIWindow,
    };
    use objc2_ui_kit::{
        UIGestureRecognizer, UIGestureRecognizerState, UIPanGestureRecognizer,
        UITapGestureRecognizer,
    };
    use objc2_ui_kit::{UIScrollViewDelegate, UITableViewDataSource, UITableViewDelegate};
    use objc2_ui_kit::{UITabBarController, UITabBarControllerDelegate};
    // `.import`/`.exportToService` modes (deprecated in favor of `initFor…ContentTypes:`, which
    // would pull in the UniformTypeIdentifiers crate) remain the simplest UTType-free path.
    #[allow(deprecated)]
    use objc2_ui_kit::UIDocumentPickerMode;
    use objc2_ui_kit::{UIDocumentPickerDelegate, UIDocumentPickerViewController};

    use day_spec::props::*;
    use day_spec::{
        A11yProps, AnimSpec, Cap, Curve, DrawOp, Edges, Event, EventSink, Font, ListSource, NodeId,
        PieceKind, Platform, Proposal, RawHandle, Rect, Registry, Renderer, Size, Support, Toolkit,
        Transform, WINDOW_NODE, WindowOptions, kinds,
    };

    pub type Handle = Retained<UIView>;

    /// The day-core event sink (node-id keyed).
    type Sink = Rc<dyn Fn(NodeId, Event)>;

    thread_local! {
        static SINK: RefCell<Option<Sink>> = const { RefCell::new(None) };
        static TARGETS: RefCell<HashMap<usize, Retained<DayTarget>>> = RefCell::new(HashMap::new());
        static WINDOW: RefCell<Option<Retained<UIWindow>>> = const { RefCell::new(None) };
        /// The Day content root + its keyboard-less frame (window coords) — keyboard avoidance
        /// (docs/focus.md) shrinks the root to the keyboard top and restores this on dismiss.
        static ROOT_VIEW: RefCell<Option<Retained<UIView>>> = const { RefCell::new(None) };
        static ROOT_BASE_FRAME: Cell<CGRect> = const {
            Cell::new(CGRect {
                origin: CGPoint { x: 0.0, y: 0.0 },
                size: CGSize {
                    width: 0.0,
                    height: 0.0,
                },
            })
        };
        /// The UITextField that currently owns the keyboard (editBegan/editEnded), so the
        /// keyboard handler can reveal it inside its enclosing UIScrollView.
        static FOCUSED_FIELD: RefCell<Option<Retained<UIView>>> = const { RefCell::new(None) };
        #[allow(clippy::type_complexity)]
        static PENDING: RefCell<Option<(Uikit, WindowOptions, Box<dyn FnOnce(Uikit, Handle, Size)>)>> =
            RefCell::new(None);
        /// The frame clock (§8.4): a single persistent CADisplayLink, paused when idle, plus the
        /// one pending vsync callback day-core asked for. `request_frame` stores the cb + un-pauses;
        /// `step:` takes the cb, calls it with the frame timestamp, and re-pauses if none was queued.
        #[allow(clippy::type_complexity)]
        static FRAME: RefCell<(Option<Retained<CADisplayLink>>, Option<Box<dyn FnOnce(f64)>>)> =
            RefCell::new((None, None));
    }

    /// Scroll the focused field's nearest enclosing UIScrollView so the field is visible
    /// (keyboard avoidance, docs/focus.md). Runs a turn AFTER the keyboard-driven root resize
    /// so Day's relayout has settled the frames it converts.
    fn reveal_focused_field() {
        // Next main-queue turn: Day's relayout for the resized root has run by then, so the
        // frames this converts are settled. (Same queue the backend's poster uses.)
        dispatch2::DispatchQueue::main().exec_async(|| {
            let Some(field) = FOCUSED_FIELD.with(|f| f.borrow().clone()) else {
                return;
            };
            let mut sup = field.superview();
            while let Some(v) = sup {
                sup = v.superview();
                if let Ok(sv) = v.downcast::<UIScrollView>() {
                    // Convert into the scroll's coordinate space (== content space for
                    // UIScrollView, whose bounds origin is the content offset), with a little
                    // breathing room below the field.
                    let mut r = field.convertRect_toView(field.bounds(), Some(&sv));
                    r.size.height += 12.0;
                    unsafe { sv.scrollRectToVisible_animated(r, true) };
                    return;
                }
            }
        });
    }

    pub fn emit(id: NodeId, ev: Event) {
        let sink = SINK.with(|s| s.borrow().clone());
        if let Some(sink) = sink {
            sink(id, ev);
        }
    }

    fn ptr_of(v: &UIView) -> usize {
        (v as *const UIView).cast::<()>() as usize
    }
    fn view_of<T: AsRef<UIView>>(x: Retained<T>) -> Handle {
        Retained::from(x.as_ref())
    }

    // -----------------------------------------------------------------------
    // DayTarget — target/action trampoline, node-id keyed
    // -----------------------------------------------------------------------

    struct TargetIvars {
        node: NodeId,
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "DayUIKitTarget"]
        #[ivars = TargetIvars]
        struct DayTarget;

        unsafe impl NSObjectProtocol for DayTarget {}

        impl DayTarget {
            #[unsafe(method(fire:))]
            fn fire(&self, sender: &UIControl) {
                let node = self.ivars().node;
                let obj: &AnyObject = sender.as_ref();
                if let Some(sw) = obj.downcast_ref::<UISwitch>() {
                    emit(node, Event::ToggleChanged(unsafe { sw.isOn() }));
                } else if let Some(sl) = obj.downcast_ref::<UISlider>() {
                    emit(node, Event::ValueChanged(unsafe { sl.value() } as f64));
                } else if let Some(tf) = obj.downcast_ref::<UITextField>() {
                    let s = unsafe { tf.text() }.map(|s| s.to_string()).unwrap_or_default();
                    emit(node, Event::TextChanged(s));
                } else {
                    emit(node, Event::Pressed);
                }
            }

            /// EditingDidBegin — the keyboard is up and this field owns it (docs/focus.md).
            #[unsafe(method(editBegan:))]
            fn edit_began(&self, sender: &UIControl) {
                FOCUSED_FIELD.with(|f| *f.borrow_mut() = Some(Retained::from(sender as &UIView)));
                // The keyboard may already be up (focus moved between fields): reveal now too,
                // not only from the keyboard-frame notification.
                reveal_focused_field();
                emit(self.ivars().node, Event::FocusChanged(true));
            }

            /// EditingDidEnd — the field resigned (keyboard dismissed or focus moved on).
            #[unsafe(method(editEnded:))]
            fn edit_ended(&self, _sender: &UIControl) {
                FOCUSED_FIELD.with(|f| *f.borrow_mut() = None);
                emit(self.ivars().node, Event::FocusChanged(false));
            }

            /// EditingDidEndOnExit — the Return key. Registering this handler is also what
            /// makes Return dismiss the keyboard (the UIKit convention); an `on_submit` that
            /// moves focus re-raises it on the next field.
            #[unsafe(method(editExit:))]
            fn edit_exit(&self, _sender: &UIControl) {
                emit(self.ivars().node, Event::Submitted);
            }
        }
    );

    impl DayTarget {
        fn new(mtm: MainThreadMarker, node: NodeId) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(TargetIvars { node });
            unsafe { msg_send![super(this), init] }
        }
    }

    // -----------------------------------------------------------------------
    // DayFrameTarget — the CADisplayLink target for the frame clock (§8.4)
    // -----------------------------------------------------------------------

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "DayUIKitFrameTarget"]
        #[ivars = ()]
        struct DayFrameTarget;

        unsafe impl NSObjectProtocol for DayFrameTarget {}

        impl DayFrameTarget {
            /// One vsync tick. Deliver the pending callback (day-core re-arms it if it wants more),
            /// then pause the link if nothing was re-queued so an idle app stops waking the display.
            #[unsafe(method(step:))]
            fn step(&self, link: &CADisplayLink) {
                let ts = unsafe { link.timestamp() };
                let cb = FRAME.with(|f| f.borrow_mut().1.take());
                if let Some(cb) = cb {
                    cb(ts);
                }
                let idle = FRAME.with(|f| f.borrow().1.is_none());
                if idle {
                    unsafe { link.setPaused(true) };
                }
            }
        }
    );

    impl DayFrameTarget {
        fn new(mtm: MainThreadMarker) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(());
            unsafe { msg_send![super(this), init] }
        }
    }

    // -----------------------------------------------------------------------
    // DayGesture — tap/pan recognizer target, node-id keyed (docs/shapes.md)
    // -----------------------------------------------------------------------

    struct GestureIvars {
        node: NodeId,
        is_drag: bool,
    }

    thread_local! {
        /// Keeps each view's gesture targets alive + records which are attached (idempotent).
        static GESTURES: RefCell<HashMap<usize, Vec<Retained<DayGesture>>>> =
            RefCell::new(HashMap::new());
        /// Per-view context-menu interaction + its delegate (kept alive; replaced on reconfigure).
        #[allow(clippy::type_complexity)]
        static CTX_MENUS: RefCell<
            HashMap<usize, (Retained<UIContextMenuInteraction>, Retained<DayContextMenu>)>,
        > = RefCell::new(HashMap::new());
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "DayUIKitGesture"]
        #[ivars = GestureIvars]
        struct DayGesture;

        unsafe impl NSObjectProtocol for DayGesture {}

        impl DayGesture {
            #[unsafe(method(fire:))]
            fn fire(&self, g: &UIGestureRecognizer) {
                let node = self.ivars().node;
                let view = unsafe { g.view() };
                let loc = unsafe { g.locationInView(view.as_deref()) };
                let at = day_spec::Point::new(loc.x, loc.y);
                if self.ivars().is_drag {
                    let obj: &AnyObject = g.as_ref();
                    let (translation, phase) = if let Some(pan) =
                        obj.downcast_ref::<UIPanGestureRecognizer>()
                    {
                        let t = unsafe { pan.translationInView(view.as_deref()) };
                        let phase = match unsafe { g.state() } {
                            UIGestureRecognizerState::Began => day_spec::DragPhase::Began,
                            UIGestureRecognizerState::Ended
                            | UIGestureRecognizerState::Cancelled
                            | UIGestureRecognizerState::Failed => day_spec::DragPhase::Ended,
                            _ => day_spec::DragPhase::Changed,
                        };
                        (day_spec::Point::new(t.x, t.y), phase)
                    } else {
                        (day_spec::Point::ZERO, day_spec::DragPhase::Changed)
                    };
                    emit(
                        node,
                        Event::Drag {
                            phase,
                            location: at,
                            translation,
                        },
                    );
                } else {
                    emit(node, Event::Tap(at));
                }
            }
        }
    );

    impl DayGesture {
        fn new(mtm: MainThreadMarker, node: NodeId, is_drag: bool) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(GestureIvars { node, is_drag });
            unsafe { msg_send![super(this), init] }
        }
    }

    // -----------------------------------------------------------------------
    // Menus (docs/menus.md): the day-neutral MenuItem tree becomes a UIMenu of UIActions, shown
    // by a UIContextMenuInteraction on long-press. Custom actions emit MenuAction(id); standard
    // roles route their selector up the responder chain so Cut/Copy/Paste hit the focused field.
    // -----------------------------------------------------------------------

    struct CtxMenuIvars {
        menu: Retained<UIMenu>,
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "DayUIKitContextMenu"]
        #[ivars = CtxMenuIvars]
        struct DayContextMenu;

        unsafe impl NSObjectProtocol for DayContextMenu {}

        unsafe impl UIContextMenuInteractionDelegate for DayContextMenu {
            #[unsafe(method_id(contextMenuInteraction:configurationForMenuAtLocation:))]
            fn configuration_for_menu(
                &self,
                _interaction: &UIContextMenuInteraction,
                _location: CGPoint,
            ) -> Option<Retained<UIContextMenuConfiguration>> {
                let menu = self.ivars().menu.clone();
                let provider = block2::RcBlock::new(
                    move |_suggested: NonNull<objc2_foundation::NSArray<UIMenuElement>>| -> *mut UIMenu {
                        Retained::into_raw(menu.clone())
                    },
                );
                Some(unsafe {
                    UIContextMenuConfiguration::configurationWithIdentifier_previewProvider_actionProvider(
                        None,
                        std::ptr::null_mut(),
                        block2::RcBlock::as_ptr(&provider),
                        mtm(),
                    )
                })
            }
        }
    );

    impl DayContextMenu {
        fn new(mtm: MainThreadMarker, menu: Retained<UIMenu>) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(CtxMenuIvars { menu });
            unsafe { msg_send![super(this), init] }
        }
    }

    /// Default label for a standard role left unlabeled by the app.
    fn ui_role_label(role: day_spec::MenuRole) -> &'static str {
        use day_spec::MenuRole::*;
        match role {
            Cut => "Cut",
            Copy => "Copy",
            Paste => "Paste",
            SelectAll => "Select All",
            Undo => "Undo",
            Redo => "Redo",
            Delete => "Delete",
            About => "About",
            Quit => "Quit",
            Preferences => "Settings",
            Minimize => "Minimize",
            CloseWindow => "Close",
            Fullscreen => "Full Screen",
        }
    }

    /// The UIResponder standard-edit selector a role routes to (None → a no-op labelled action, since
    /// iOS has no responder equivalent — e.g. Quit/About/window management).
    fn ui_role_selector(role: day_spec::MenuRole) -> Option<objc2::runtime::Sel> {
        use day_spec::MenuRole::*;
        Some(match role {
            Cut => sel!(cut:),
            Copy => sel!(copy:),
            Paste => sel!(paste:),
            SelectAll => sel!(selectAll:),
            Delete => sel!(delete:),
            _ => return None,
        })
    }

    /// Build a single UIAction; `handler` runs on the main thread when chosen.
    fn ui_action(
        mtm: MainThreadMarker,
        title: &str,
        enabled: bool,
        handler: impl Fn() + 'static,
    ) -> Retained<UIMenuElement> {
        let block = block2::RcBlock::new(move |_a: NonNull<UIAction>| handler());
        let action = unsafe {
            UIAction::actionWithTitle_image_identifier_handler(
                &NSString::from_str(title),
                None,
                None,
                block2::RcBlock::as_ptr(&block),
                mtm,
            )
        };
        if !enabled {
            unsafe { action.setAttributes(UIMenuElementAttributes::Disabled) };
        }
        Retained::into_super(action)
    }

    /// Lower one run of items (already split on separators) into UIMenuElements.
    fn ui_menu_elements(
        mtm: MainThreadMarker,
        items: &[day_spec::MenuItem],
    ) -> Vec<Retained<UIMenuElement>> {
        let mut out: Vec<Retained<UIMenuElement>> = Vec::new();
        for item in items {
            match item {
                day_spec::MenuItem::Separator => {}
                day_spec::MenuItem::Submenu { label, items } => {
                    out.push(Retained::into_super(build_ui_menu(mtm, label, items)));
                }
                day_spec::MenuItem::Action {
                    id,
                    label,
                    shortcut: _,
                    enabled,
                    role,
                } => {
                    if let Some(role) = role {
                        let title = if label.is_empty() {
                            ui_role_label(*role).to_string()
                        } else {
                            label.clone()
                        };
                        let sel = ui_role_selector(*role);
                        out.push(ui_action(mtm, &title, *enabled, move || {
                            if let Some(sel) = sel {
                                let app = UIApplication::sharedApplication(mtm);
                                unsafe {
                                    app.sendAction_to_from_forEvent(sel, None, None, None);
                                }
                            }
                        }));
                    } else {
                        let id = *id;
                        out.push(ui_action(mtm, label, *enabled, move || {
                            emit(WINDOW_NODE, Event::MenuAction(id));
                        }));
                    }
                }
            }
        }
        out
    }

    /// Build a UIMenu whose children preserve separators as inline sections (the native iOS look).
    fn build_ui_menu(
        mtm: MainThreadMarker,
        title: &str,
        items: &[day_spec::MenuItem],
    ) -> Retained<UIMenu> {
        // Split on separators; each run becomes an inline submenu so dividers render natively.
        let groups: Vec<&[day_spec::MenuItem]> = items
            .split(|i| matches!(i, day_spec::MenuItem::Separator))
            .filter(|g| !g.is_empty())
            .collect();
        let children: Vec<Retained<UIMenuElement>> = if groups.len() <= 1 {
            ui_menu_elements(mtm, items)
        } else {
            groups
                .into_iter()
                .map(|g| {
                    let elems = ui_menu_elements(mtm, g);
                    let arr = objc2_foundation::NSArray::from_retained_slice(&elems);
                    let inline = unsafe {
                        UIMenu::menuWithTitle_image_identifier_options_children(
                            &NSString::from_str(""),
                            None,
                            None,
                            UIMenuOptions::DisplayInline,
                            &arr,
                            mtm,
                        )
                    };
                    Retained::into_super(inline)
                })
                .collect()
        };
        let arr = objc2_foundation::NSArray::from_retained_slice(&children);
        unsafe { UIMenu::menuWithTitle_children(&NSString::from_str(title), &arr, mtm) }
    }

    // -----------------------------------------------------------------------
    // Navigation (docs/navigation.md): UINavigationController child-contained in the
    // root VC. Each page = UIViewController whose view pins a content subview to the
    // safe area; the content view is Day's handle (its frame is native-owned).
    // -----------------------------------------------------------------------

    struct NavState {
        nav: Retained<objc2_ui_kit::UINavigationController>,
        host_node: NodeId,
        /// Our mirror of the intended VC stack (index 0 = root page).
        vcs: Vec<Retained<UIViewController>>,
        /// A day-initiated pop is in flight: the delegate must not re-emit NavBack.
        expect_pop: std::cell::Cell<bool>,
        /// Native user-back pops (swipe / back button) awaiting Day's answering `NavPatch::Popped`.
        /// The native stack already popped, so that answering patch must be ABSORBED (decrement)
        /// rather than popping again — a stale no-op pop would wedge `expect_pop` true, so the NEXT
        /// native back gets swallowed, `selection` never resets, and re-selecting the SAME item does
        /// nothing (docs/navigation.md). Mirrors Android's DayNavHost.nativePops.
        native_pops: std::cell::Cell<usize>,
        /// The native VC count at the LAST `didShow` — the pop detector's baseline. Comparing
        /// against the `vcs` mirror instead is wrong: a previous transition's pop-`didShow` can
        /// arrive arbitrarily late, after the next push has already appended to the mirror but
        /// before its `remove()` cleaned the popped entry — the fresh push's own `didShow` then
        /// reads `native < vcs.len()` and a phantom NavBack tears down the just-pushed page.
        /// Only an actual DECREASE in the observed native count is a pop.
        last_native: std::cell::Cell<usize>,
        _delegate: Retained<DayNavDelegate>,
    }

    thread_local! {
        /// Keyed by the nav host view ptr (the UINavigationController's view).
        static NAV_STATE: RefCell<HashMap<usize, NavState>> = RefCell::new(HashMap::new());
        /// Page CONTENT view ptr → its UIViewController.
        static PAGE_VCS: RefCell<HashMap<usize, Retained<UIViewController>>> =
            RefCell::new(HashMap::new());
        /// Handles whose frames are native-owned (page content views).
        static NAV_PAGES: RefCell<std::collections::HashSet<usize>> =
            RefCell::new(std::collections::HashSet::new());
    }

    struct NavPageIvars {
        node: NodeId,
    }

    define_class!(
        #[unsafe(super(UIView))]
        #[thread_kind = MainThreadOnly]
        #[name = "DayNavPageView"]
        #[ivars = NavPageIvars]
        struct DayNavPageView;

        impl DayNavPageView {
            #[unsafe(method(layoutSubviews))]
            fn layout_subviews(&self) {
                let _: () = unsafe { msg_send![super(self), layoutSubviews] };
                // Pin the content subview to the safe area (below the navigation bar)
                // and report its size so NavLayout re-lays the Day content (§8.3).
                let bounds = self.bounds();
                let insets = self.safeAreaInsets();
                let frame = CGRect::new(
                    CGPoint::new(insets.left, insets.top),
                    CGSize::new(
                        (bounds.size.width - insets.left - insets.right).max(0.0),
                        (bounds.size.height - insets.top - insets.bottom).max(0.0),
                    ),
                );
                if let Some(content) = unsafe { self.subviews() }.firstObject() {
                    unsafe { content.setFrame(frame) };
                }
                emit(
                    self.ivars().node,
                    Event::FrameChanged(Size::new(frame.size.width, frame.size.height)),
                );
            }
        }
    );

    impl DayNavPageView {
        fn new(mtm: MainThreadMarker, node: NodeId) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(NavPageIvars { node });
            let v: Retained<Self> = unsafe { msg_send![super(this), init] };
            unsafe { v.setBackgroundColor(Some(&UIColor::systemBackgroundColor())) };
            v
        }
    }

    struct NavDelegateIvars {
        host: std::cell::Cell<usize>,
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "DayNavDelegate"]
        #[ivars = NavDelegateIvars]
        struct DayNavDelegate;

        unsafe impl NSObjectProtocol for DayNavDelegate {}

        unsafe impl UINavigationControllerDelegate for DayNavDelegate {
            #[unsafe(method(navigationController:didShowViewController:animated:))]
            fn did_show(
                &self,
                nav: &objc2_ui_kit::UINavigationController,
                _vc: &UIViewController,
                _animated: bool,
            ) {
                // A user pop must satisfy BOTH baselines (each alone has a false positive):
                // the observed native count DECREASED since the last didShow (else a late
                // pop-didShow arriving after the next push's mirror append reads native <
                // mirror and phantom-pops the fresh page), AND the mirror still holds more
                // than native (else a day-initiated reset — setViewControllers shrinking the
                // stack with the mirror already cleaned — reads as a user pop). Day-initiated
                // pops set expect_pop; what passes both tests without it is the user's back
                // button / swipe.
                let host = self.ivars().host.get();
                let (emit_back, node) = NAV_STATE.with(|m| {
                    let mut m = m.borrow_mut();
                    let Some(state) = m.get_mut(&host) else {
                        return (false, NodeId(0));
                    };
                    let native = unsafe { nav.viewControllers() }.count();
                    let prev = state.last_native.replace(native);
                    if native < prev && native < state.vcs.len() {
                        if state.expect_pop.replace(false) {
                            (false, NodeId(0))
                        } else {
                            // A user back (swipe / back button): sync the mirror now (Day's remove()
                            // will find it gone) and record that Day's answering NavPatch::Popped
                            // must be ABSORBED — the native stack has already popped.
                            state.vcs.truncate(native);
                            state.native_pops.set(state.native_pops.get() + 1);
                            (true, state.host_node)
                        }
                    } else {
                        (false, NodeId(0))
                    }
                });
                if emit_back {
                    emit(
                        node,
                        Event::NavBack {
                            already_popped: true,
                        },
                    );
                }
            }
        }
    );

    impl DayNavDelegate {
        fn new(mtm: MainThreadMarker, host: usize) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(NavDelegateIvars {
                host: std::cell::Cell::new(host),
            });
            unsafe { msg_send![super(this), init] }
        }
    }

    // -------------------------------------------------------------------
    // Cover (docs/cover.md): a fullscreen modal DayCoverVC whose view is a
    // DayNavPageView (safe-area pinning + FrameChanged reports), presented and
    // dismissed through the modal FIFO like every other VC transition.
    // -------------------------------------------------------------------

    struct CoverState {
        vc: Retained<DayCoverVC>,
        node: NodeId,
    }

    thread_local! {
        /// Cover content view ptr → its presentation state.
        static COVER_STATE: RefCell<HashMap<usize, CoverState>> = RefCell::new(HashMap::new());
        /// The current `defers_system_gestures` union (day `Edges` bits) — read by the root
        /// and cover VCs' `preferredScreenEdgesDeferringSystemGestures` overrides.
        static DEFER_EDGES: Cell<u8> = const { Cell::new(0) };
    }

    /// Day `Edges` bits → `UIRectEdge` (leading/trailing map to left/right).
    fn rect_edges() -> UIRectEdge {
        let bits = DEFER_EDGES.with(|e| e.get());
        let mut edge = UIRectEdge::empty();
        if bits & Edges::TOP.0 != 0 {
            edge |= UIRectEdge::Top;
        }
        if bits & Edges::BOTTOM.0 != 0 {
            edge |= UIRectEdge::Bottom;
        }
        if bits & Edges::LEADING.0 != 0 {
            edge |= UIRectEdge::Left;
        }
        if bits & Edges::TRAILING.0 != 0 {
            edge |= UIRectEdge::Right;
        }
        edge
    }

    define_class!(
        #[unsafe(super(UIViewController))]
        #[thread_kind = MainThreadOnly]
        #[name = "DayCoverVC"]
        #[ivars = ()]
        struct DayCoverVC;

        /// The presented cover is the VC UIKit consults for system-gesture deferral, so the
        /// `defers_system_gestures` union applies while a game/cover is up.
        impl DayCoverVC {
            #[unsafe(method(preferredScreenEdgesDeferringSystemGestures))]
            fn preferred_edges(&self) -> UIRectEdge {
                rect_edges()
            }
        }
    );

    impl DayCoverVC {
        fn new(mtm: MainThreadMarker) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(());
            unsafe { msg_send![super(this), init] }
        }
    }

    define_class!(
        #[unsafe(super(UIViewController))]
        #[thread_kind = MainThreadOnly]
        #[name = "DayRootVC"]
        #[ivars = ()]
        struct DayRootVC;

        /// Same override on the window root, so the modifier also works outside a cover.
        impl DayRootVC {
            #[unsafe(method(preferredScreenEdgesDeferringSystemGestures))]
            fn preferred_edges(&self) -> UIRectEdge {
                rect_edges()
            }
        }
    );

    impl DayRootVC {
        fn new(mtm: MainThreadMarker) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(());
            unsafe { msg_send![super(this), init] }
        }
    }

    /// Queue the cover's presentation behind any in-flight modal transition (§dialogs FIFO).
    fn cover_present(vc: Retained<DayCoverVC>) {
        modal_enqueue(ModalOp::Run(Box::new(move || {
            if vc.presentingViewController().is_some() {
                return; // already up (a re-present while closing was cancelled)
            }
            let Some(top) = topmost_vc() else { return };
            modal_begin_transition();
            let completion = block2::RcBlock::new(modal_end_transition);
            unsafe {
                top.presentViewController_animated_completion(&vc, true, Some(&completion));
            }
        })));
    }

    /// Queue the cover's dismissal; the completion reports "cover-hidden" so the piece can
    /// dispose the content only after it left the screen.
    fn cover_dismiss(vc: Retained<DayCoverVC>, node: NodeId) {
        modal_enqueue(ModalOp::Run(Box::new(move || {
            let Some(presenting) = vc.presentingViewController() else {
                emit(node, Event::custom("cover-hidden", ""));
                return;
            };
            modal_begin_transition();
            // The completion is the normal "cover-hidden" source — but UIKit can drop a
            // transition completion outright (same failure the modal watchdog exists for),
            // and the piece would then never dispose the hidden content. The fallback
            // watchdog emits once the VC has actually left the hierarchy; the piece's
            // closing gate makes a duplicate report harmless.
            let fired = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let completion = {
                let fired = fired.clone();
                block2::RcBlock::new(move || {
                    fired.store(true, std::sync::atomic::Ordering::Relaxed);
                    emit(node, Event::custom("cover-hidden", ""));
                    modal_end_transition();
                })
            };
            unsafe {
                presenting.dismissViewControllerAnimated_completion(true, Some(&completion));
            }
            let mtm = objc2::MainThreadMarker::new().expect("cover ops run on main");
            let vc_probe = dispatch2::MainThreadBound::new(vc.clone(), mtm);
            let when = dispatch2::DispatchTime::try_from(std::time::Duration::from_millis(1500))
                .unwrap_or(dispatch2::DispatchTime::NOW);
            let _ = dispatch2::DispatchQueue::main().after(when, move || {
                let mtm = objc2::MainThreadMarker::new().expect("dispatched to main");
                if !fired.load(std::sync::atomic::Ordering::Relaxed)
                    && vc_probe.get(mtm).presentingViewController().is_none()
                {
                    eprintln!("day: cover dismissal completion lost — reporting cover-hidden");
                    emit(node, Event::custom("cover-hidden", ""));
                }
            });
        })));
    }

    // -------------------------------------------------------------------
    // Tabs (docs/tabs.md): UITabBarController child-contained in the root VC.
    // Each tab page is a UIViewController wrapping a DayNavPageView (safe-area
    // pinned content + FrameChanged), identical to a nav page.
    // -------------------------------------------------------------------

    struct TabsState {
        tabbar: Retained<UITabBarController>,
        /// Our mirror of the tab VC order.
        vcs: Vec<Retained<UIViewController>>,
        /// Tab to select once the VCs are installed.
        initial: usize,
        _delegate: Retained<DayTabDelegate>,
    }

    thread_local! {
        static TABS_STATE: RefCell<HashMap<usize, TabsState>> = RefCell::new(HashMap::new());
        /// TABS_PAGE content view ptr → its UIViewController.
        static TABS_PAGE_VCS: RefCell<HashMap<usize, Retained<UIViewController>>> =
            RefCell::new(HashMap::new());
    }

    struct TabDelegateIvars {
        node: NodeId,
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "DayTabDelegate"]
        #[ivars = TabDelegateIvars]
        struct DayTabDelegate;

        unsafe impl NSObjectProtocol for DayTabDelegate {}

        unsafe impl UITabBarControllerDelegate for DayTabDelegate {
            #[unsafe(method(tabBarController:didSelectViewController:))]
            fn did_select(&self, tabbar: &UITabBarController, _vc: &UIViewController) {
                // UIKit calls this only for user taps, not programmatic selection — no guard.
                let idx = unsafe { tabbar.selectedIndex() };
                emit(self.ivars().node, Event::SelectionChanged(idx as i64));
            }
        }
    );

    impl DayTabDelegate {
        fn new(mtm: MainThreadMarker, node: NodeId) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(TabDelegateIvars { node });
            unsafe { msg_send![super(this), init] }
        }
    }

    // -------------------------------------------------------------------
    // -----------------------------------------------------------------------
    // DayNavCell — a nav row whose icon reads as a natural iOS glyph: a small
    // (20pt) template image tinted with the neutral secondaryLabel colour (NOT
    // the accent), its baseline aligned to the row label's text baseline. The
    // stock UITableViewCell centres its imageView and accent-tints it, so we lay
    // the row out ourselves (docs/navigation.md).
    // -----------------------------------------------------------------------
    struct NavCellIvars {
        icon: Retained<objc2_ui_kit::UIImageView>,
        title: Retained<UILabel>,
    }

    define_class!(
        #[unsafe(super(objc2_ui_kit::UITableViewCell))]
        #[thread_kind = MainThreadOnly]
        #[name = "DayNavCell"]
        #[ivars = NavCellIvars]
        struct DayNavCell;

        impl DayNavCell {
            #[unsafe(method(layoutSubviews))]
            fn layout_subviews(&self) {
                let _: () = unsafe { msg_send![super(self), layoutSubviews] };
                let iv = self.ivars();
                let b = self.contentView().bounds();
                let (cw, ch) = (b.size.width, b.size.height);
                let Some(font) = (unsafe { iv.title.font() }) else {
                    return;
                };
                let line_h = unsafe { font.lineHeight() };
                let label_y = ((ch - line_h) / 2.0).max(0.0);
                let baseline = label_y + unsafe { font.ascender() }; // text baseline from top
                const LEADING: f64 = 16.0;
                const ICON: f64 = 20.0;
                const GAP: f64 = 12.0;
                let has_icon = unsafe { iv.icon.image() }.is_some();
                let text_x = if has_icon { LEADING + ICON + GAP } else { LEADING };
                unsafe {
                    iv.title.setFrame(CGRect::new(
                        CGPoint::new(text_x, label_y),
                        CGSize::new((cw - text_x - 6.0).max(0.0), line_h),
                    ));
                    iv.icon.setHidden(!has_icon);
                    if has_icon {
                        // The icon's bottom sits on the label's text baseline.
                        iv.icon.setFrame(CGRect::new(
                            CGPoint::new(LEADING, baseline - ICON),
                            CGSize::new(ICON, ICON),
                        ));
                    }
                }
            }
        }
    );

    impl DayNavCell {
        fn new(mtm: MainThreadMarker) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(NavCellIvars {
                icon: unsafe { objc2_ui_kit::UIImageView::new(mtm) },
                title: unsafe { UILabel::new(mtm) },
            });
            let none: Option<&NSString> = None;
            let cell: Retained<Self> = unsafe {
                msg_send![
                    super(this),
                    initWithStyle: objc2_ui_kit::UITableViewCellStyle::Default,
                    reuseIdentifier: none,
                ]
            };
            let iv = cell.ivars();
            unsafe {
                iv.title
                    .setFont(Some(&objc2_ui_kit::UIFont::preferredFontForTextStyle(
                        objc2_ui_kit::UIFontTextStyleBody,
                    )));
                iv.icon
                    .setContentMode(objc2_ui_kit::UIViewContentMode::ScaleAspectFit);
                iv.icon.setTintColor(Some(&UIColor::secondaryLabelColor()));
                cell.contentView().addSubview(&iv.title);
                cell.contentView().addSubview(&iv.icon);
                cell.setAccessoryType(
                    objc2_ui_kit::UITableViewCellAccessoryType::DisclosureIndicator,
                );
            }
            cell
        }

        fn configure(&self, title: &NSString, image: Option<&objc2_ui_kit::UIImage>) {
            let iv = self.ivars();
            unsafe {
                iv.title.setText(Some(title));
                iv.icon.setImage(image);
            }
            self.setNeedsLayout();
        }
    }

    // DayNavTableData — nav_menu() as inset-grouped rows with chevrons
    // -------------------------------------------------------------------

    struct NavTableIvars {
        node: NodeId,
        items: RefCell<Vec<Retained<NSString>>>,
        /// Pre-resolved template icons per row (docs/navigation.md), `None` where a row has none.
        /// Template mode tints them with the cell's tint colour (the iOS list idiom).
        icons: RefCell<Vec<Option<Retained<objc2_ui_kit::UIImage>>>>,
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "DayNavTableData"]
        #[ivars = NavTableIvars]
        struct DayNavTableData;

        unsafe impl NSObjectProtocol for DayNavTableData {}
        unsafe impl UIScrollViewDelegate for DayNavTableData {}

        unsafe impl UITableViewDataSource for DayNavTableData {
            #[unsafe(method(tableView:numberOfRowsInSection:))]
            fn rows_in_section(&self, _tv: &objc2_ui_kit::UITableView, _section: isize) -> isize {
                self.ivars().items.borrow().len() as isize
            }

            #[unsafe(method_id(tableView:cellForRowAtIndexPath:))]
            fn cell_for_row(
                &self,
                _tv: &objc2_ui_kit::UITableView,
                index_path: &objc2_foundation::NSIndexPath,
            ) -> Retained<objc2_ui_kit::UITableViewCell> {
                let mtm = self.mtm();
                let cell = DayNavCell::new(mtm);
                let row = unsafe { index_path.row() } as usize;
                let title = self
                    .ivars()
                    .items
                    .borrow()
                    .get(row)
                    .cloned()
                    .unwrap_or_else(|| NSString::from_str(""));
                let img = self.ivars().icons.borrow().get(row).and_then(|o| o.clone());
                cell.configure(&title, img.as_deref());
                objc2::rc::Retained::into_super(cell)
            }
        }

        unsafe impl UITableViewDelegate for DayNavTableData {
            #[unsafe(method(tableView:didSelectRowAtIndexPath:))]
            fn did_select(
                &self,
                tv: &objc2_ui_kit::UITableView,
                index_path: &objc2_foundation::NSIndexPath,
            ) {
                let row = unsafe { index_path.row() };
                unsafe { tv.deselectRowAtIndexPath_animated(index_path, true) };
                emit(self.ivars().node, Event::SelectionChanged(row as i64));
            }
        }
    );

    /// Load a bundled image by NAME for a nav/tab icon (docs/navigation.md): by-name from the
    /// DayPieces asset catalog first — the reliable iOS path, same as the `image()` piece — then a
    /// loose staged file (dev / assets). Callers apply `.alwaysTemplate` so it tints with the
    /// control's colour.
    fn load_bundled_uiimage(name: &str) -> Option<Retained<objc2_ui_kit::UIImage>> {
        let nsname = NSString::from_str(name);
        let main = unsafe { objc2_foundation::NSBundle::mainBundle() };
        let bname = NSString::from_str("DayPieces_DayPieces");
        let bext = NSString::from_str("bundle");
        if let Some(url) = unsafe { main.URLForResource_withExtension(Some(&bname), Some(&bext)) }
            && let Some(day_bundle) = unsafe { objc2_foundation::NSBundle::bundleWithURL(&url) }
            && let Some(img) = unsafe {
                objc2_ui_kit::UIImage::imageNamed_inBundle_compatibleWithTraitCollection(
                    &nsname,
                    Some(&day_bundle),
                    None,
                )
            }
        {
            return Some(img);
        }
        if let Some(path) = day_spec::resource::resolve_image_file(name)
            && let Some(img) = unsafe {
                objc2_ui_kit::UIImage::imageWithContentsOfFile(&NSString::from_str(
                    &path.to_string_lossy(),
                ))
            }
        {
            return Some(img);
        }
        None
    }

    impl DayNavTableData {
        fn new(
            mtm: MainThreadMarker,
            node: NodeId,
            items: &[String],
            icons: &[Option<String>],
        ) -> Retained<Self> {
            let resolved: Vec<Option<Retained<objc2_ui_kit::UIImage>>> = icons
                .iter()
                .map(|ic| {
                    let img = load_bundled_uiimage(ic.as_deref()?)?;
                    Some(unsafe {
                        img.imageWithRenderingMode(
                            objc2_ui_kit::UIImageRenderingMode::AlwaysTemplate,
                        )
                    })
                })
                .collect();
            let this = Self::alloc(mtm).set_ivars(NavTableIvars {
                node,
                items: RefCell::new(items.iter().map(|s| NSString::from_str(s)).collect()),
                icons: RefCell::new(resolved),
            });
            unsafe { msg_send![super(this), init] }
        }
    }

    thread_local! {
        /// NAV_MENU table ptr → (data source, row count).
        static NAV_MENUS: RefCell<HashMap<usize, (Retained<DayNavTableData>, usize)>> =
            RefCell::new(HashMap::new());
    }

    // -----------------------------------------------------------------------
    // DayListData — UITableView data source + delegate for the recycling list (docs/list.md, §10)
    // -----------------------------------------------------------------------

    struct ListIvars {
        node: NodeId,
        source: RefCell<Option<ListSource>>,
        row_height: std::cell::Cell<f64>,
        selectable: std::cell::Cell<bool>,
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "DayListData"]
        #[ivars = ListIvars]
        struct DayListData;

        unsafe impl NSObjectProtocol for DayListData {}
        unsafe impl UIScrollViewDelegate for DayListData {}

        unsafe impl UITableViewDataSource for DayListData {
            #[unsafe(method(tableView:numberOfRowsInSection:))]
            fn rows_in_section(&self, _tv: &objc2_ui_kit::UITableView, _section: isize) -> isize {
                // Snapshot-only read (no tree) — safe during reloadData inside a with_tree borrow.
                self.ivars()
                    .source
                    .borrow()
                    .as_ref()
                    .map(|s| (s.len)() as isize)
                    .unwrap_or(0)
            }

            #[unsafe(method_id(tableView:cellForRowAtIndexPath:))]
            fn cell_for_row(
                &self,
                tv: &objc2_ui_kit::UITableView,
                index_path: &objc2_foundation::NSIndexPath,
            ) -> Retained<objc2_ui_kit::UITableViewCell> {
                let mtm = self.mtm();
                let ident = NSString::from_str("day.cell");
                let cell = unsafe { tv.dequeueReusableCellWithIdentifier(&ident) }.unwrap_or_else(
                    || unsafe {
                        objc2_ui_kit::UITableViewCell::initWithStyle_reuseIdentifier(
                            objc2_ui_kit::UITableViewCell::alloc(mtm),
                            objc2_ui_kit::UITableViewCellStyle::Default,
                            Some(&ident),
                        )
                    },
                );
                // Day builds/rebinds its row content inside the cell's contentView.
                let content = cell.contentView();
                let row = unsafe { index_path.row() } as usize;
                if let Some(source) = self.ivars().source.borrow().as_ref() {
                    let raw = Retained::as_ptr(&content) as RawHandle;
                    (source.bind_row)(row, raw);
                }
                cell
            }
        }

        unsafe impl UITableViewDelegate for DayListData {
            #[unsafe(method(tableView:heightForRowAtIndexPath:))]
            fn height_for_row(
                &self,
                _tv: &objc2_ui_kit::UITableView,
                _index_path: &objc2_foundation::NSIndexPath,
            ) -> CGFloat {
                self.ivars().row_height.get()
            }

            #[unsafe(method(tableView:didSelectRowAtIndexPath:))]
            fn did_select(
                &self,
                tv: &objc2_ui_kit::UITableView,
                index_path: &objc2_foundation::NSIndexPath,
            ) {
                let row = unsafe { index_path.row() };
                unsafe { tv.deselectRowAtIndexPath_animated(index_path, true) };
                if self.ivars().selectable.get() {
                    emit(self.ivars().node, Event::SelectionChanged(row as i64));
                }
            }
        }
    );

    impl DayListData {
        fn new(
            mtm: MainThreadMarker,
            node: NodeId,
            selectable: bool,
            row_height: f64,
        ) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(ListIvars {
                node,
                source: RefCell::new(None),
                row_height: std::cell::Cell::new(row_height),
                selectable: std::cell::Cell::new(selectable),
            });
            unsafe { msg_send![super(this), init] }
        }
    }

    /// A realized LIST's (table view, its data source), keyed by table ptr.
    type ListEntry = (Retained<objc2_ui_kit::UITableView>, Retained<DayListData>);

    thread_local! {
        /// LIST table ptr → (table, data source).
        static LIST_STATE: RefCell<HashMap<usize, ListEntry>> = RefCell::new(HashMap::new());
    }

    // -----------------------------------------------------------------------
    // DayCanvasView — replay in drawRect (§11)
    // -----------------------------------------------------------------------

    thread_local! {
        static OPS: RefCell<HashMap<usize, Vec<day_spec::DrawOp>>> = RefCell::new(HashMap::new());
    }

    struct CanvasIvars;

    define_class!(
        #[unsafe(super(UIView))]
        #[thread_kind = MainThreadOnly]
        #[name = "DayCanvasView"]
        #[ivars = CanvasIvars]
        struct DayCanvasView;

        impl DayCanvasView {
            #[unsafe(method(drawRect:))]
            fn draw_rect(&self, _dirty: CGRect) {
                let ptr = (self as *const DayCanvasView).cast::<UIView>() as usize;
                let ops = OPS.with(|m| m.borrow().get(&ptr).cloned()).unwrap_or_default();
                for op in &ops {
                    draw_op(op);
                }
            }
        }
    );

    impl DayCanvasView {
        fn new(mtm: MainThreadMarker) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(CanvasIvars);
            let v: Retained<Self> = unsafe { msg_send![super(this), init] };
            unsafe {
                v.setBackgroundColor(Some(&UIColor::clearColor()));
                v.setOpaque(false);
            }
            v
        }
    }

    fn uicolor(c: day_spec::Color) -> Retained<UIColor> {
        unsafe { UIColor::colorWithRed_green_blue_alpha(c.r, c.g, c.b, c.a) }
    }

    /// Apply a `background`/`corner_radius` surface to a container view: UIView carries a native
    /// `backgroundColor`; the corner radius / rounded clip go on its CALayer (reached with
    /// `msg_send`, no QuartzCore dep). Idempotent — called at realize and on a background patch.
    fn apply_surface(v: &UIView, bg: Option<day_spec::Color>, corner_radius: f64, clips: bool) {
        unsafe {
            match bg {
                Some(c) => v.setBackgroundColor(Some(&uicolor(c))),
                None => v.setBackgroundColor(None),
            }
            let layer: *mut objc2::runtime::AnyObject = msg_send![v, layer];
            let _: () = msg_send![layer, setCornerRadius: corner_radius];
            let _: () = msg_send![layer, setMasksToBounds: clips || corner_radius > 0.0];
        }
    }

    fn cg(r: day_spec::Rect) -> CGRect {
        CGRect::new(
            CGPoint::new(r.origin.x, r.origin.y),
            CGSize::new(r.size.width, r.size.height),
        )
    }

    fn draw_op(op: &day_spec::DrawOp) {
        use day_spec::DrawOp;
        unsafe {
            match op {
                DrawOp::Fill(shape, paint) => match paint {
                    day_spec::Paint::Solid(color) => {
                        uicolor(*color).setFill();
                        if let Some(p) = bezier(shape) {
                            p.fill();
                        }
                    }
                    day_spec::Paint::Linear(g) => {
                        // Native linear gradient: clip to the shape's path, CGGradient along
                        // the line resolved from the unit points in the shape's bounds.
                        let ctx = objc2_ui_kit::UIGraphicsGetCurrentContext();
                        if let (Some(p), Some(ctx), Some(grad)) =
                            (bezier(shape), ctx, cggradient(&g.stops))
                        {
                            let b = shape.bounds();
                            let (s, e) = (g.start.resolve(b), g.end.resolve(b));
                            CGContext::save_g_state(Some(&ctx));
                            p.addClip();
                            CGContext::draw_linear_gradient(
                                Some(&ctx),
                                Some(&grad),
                                CGPoint::new(s.x, s.y),
                                CGPoint::new(e.x, e.y),
                                objc2_core_graphics::CGGradientDrawingOptions::DrawsBeforeStartLocation
                                    | objc2_core_graphics::CGGradientDrawingOptions::DrawsAfterEndLocation,
                            );
                            CGContext::restore_g_state(Some(&ctx));
                        }
                    }
                    day_spec::Paint::Radial(g) => {
                        // Native radial gradient: clip to the path, map unit space onto the
                        // bounds via the CTM (elliptical in non-square bounds), draw circular
                        // in unit coordinates.
                        let ctx = objc2_ui_kit::UIGraphicsGetCurrentContext();
                        if let (Some(p), Some(ctx), Some(grad)) =
                            (bezier(shape), ctx, cggradient(&g.stops))
                        {
                            let b = shape.bounds();
                            CGContext::save_g_state(Some(&ctx));
                            p.addClip();
                            CGContext::translate_ctm(Some(&ctx), b.origin.x, b.origin.y);
                            CGContext::scale_ctm(Some(&ctx), b.size.width, b.size.height);
                            let c = CGPoint::new(g.center.x, g.center.y);
                            CGContext::draw_radial_gradient(
                                Some(&ctx),
                                Some(&grad),
                                c,
                                0.0,
                                c,
                                g.radius,
                                objc2_core_graphics::CGGradientDrawingOptions::DrawsBeforeStartLocation
                                    | objc2_core_graphics::CGGradientDrawingOptions::DrawsAfterEndLocation,
                            );
                            CGContext::restore_g_state(Some(&ctx));
                        }
                    }
                },
                DrawOp::Stroke(shape, color, width) => {
                    uicolor(*color).setStroke();
                    if let Some(p) = bezier(shape) {
                        p.setLineWidth(*width);
                        p.stroke();
                    }
                }
                DrawOp::Text {
                    text,
                    at,
                    size,
                    color,
                    anchor,
                } => {
                    let font = objc2_ui_kit::UIFont::systemFontOfSize(*size);
                    let col = uicolor(*color);
                    let keys: [&NSString; 2] = [
                        objc2_ui_kit::NSFontAttributeName,
                        objc2_ui_kit::NSForegroundColorAttributeName,
                    ];
                    let objs: [&AnyObject; 2] =
                        [font.as_ref() as &AnyObject, col.as_ref() as &AnyObject];
                    let attrs =
                        objc2_foundation::NSDictionary::from_slices::<NSString>(&keys, &objs);
                    let ns = NSString::from_str(text);
                    let mut origin = CGPoint::new(at.x, at.y);
                    if *anchor == day_spec::TextAnchor::Centered {
                        let sz: CGSize = msg_send![&ns, sizeWithAttributes: &*attrs];
                        origin.x -= sz.width / 2.0;
                        origin.y -= sz.height / 2.0;
                    }
                    let _: () = msg_send![&ns, drawAtPoint: origin, withAttributes: &*attrs];
                }
                DrawOp::Save => {
                    let ctx = objc2_ui_kit::UIGraphicsGetCurrentContext();
                    CGContext::save_g_state(ctx.as_deref());
                }
                DrawOp::Restore => {
                    let ctx = objc2_ui_kit::UIGraphicsGetCurrentContext();
                    CGContext::restore_g_state(ctx.as_deref());
                }
                DrawOp::Concat(m) => {
                    let ctx = objc2_ui_kit::UIGraphicsGetCurrentContext();
                    // CGAffineTransform shares day_geometry::Affine's row-vector convention.
                    let t = CGAffineTransform {
                        a: m.a,
                        b: m.b,
                        c: m.c,
                        d: m.d,
                        tx: m.tx,
                        ty: m.ty,
                    };
                    CGContext::concat_ctm(ctx.as_deref(), t);
                }
            }
        }
    }

    /// A `CGGradient` from a display-list gradient's stops (device RGB, like every canvas color).
    fn cggradient(
        stops: &[(f64, day_spec::Color)],
    ) -> Option<objc2_core_foundation::CFRetained<objc2_core_graphics::CGGradient>> {
        if stops.is_empty() {
            return None;
        }
        let components: Vec<f64> = stops
            .iter()
            .flat_map(|(_, c)| [c.r, c.g, c.b, c.a])
            .collect();
        let locations: Vec<f64> = stops.iter().map(|(o, _)| *o).collect();
        let space = objc2_core_graphics::CGColorSpace::new_device_rgb();
        unsafe {
            objc2_core_graphics::CGGradient::with_color_components(
                space.as_deref(),
                components.as_ptr(),
                locations.as_ptr(),
                stops.len(),
            )
        }
    }

    fn bezier(shape: &day_spec::Shape) -> Option<Retained<objc2_ui_kit::UIBezierPath>> {
        use day_spec::Shape;
        use objc2_ui_kit::UIBezierPath;
        unsafe {
            Some(match shape {
                Shape::Rect(r) => UIBezierPath::bezierPathWithRect(cg(*r)),
                Shape::RoundedRect(r, rad) => {
                    UIBezierPath::bezierPathWithRoundedRect_cornerRadius(cg(*r), *rad)
                }
                Shape::Ellipse(r) => UIBezierPath::bezierPathWithOvalInRect(cg(*r)),
                Shape::Arc {
                    rect,
                    start_deg,
                    sweep_deg,
                } => {
                    let center = CGPoint::new(
                        rect.origin.x + rect.size.width / 2.0,
                        rect.origin.y + rect.size.height / 2.0,
                    );
                    let radius = rect.size.width.min(rect.size.height) / 2.0;
                    UIBezierPath::bezierPathWithArcCenter_radius_startAngle_endAngle_clockwise(
                        center,
                        radius,
                        start_deg.to_radians(),
                        (start_deg + sweep_deg).to_radians(),
                        true,
                    )
                }
                Shape::Line(a, b) => {
                    let p = UIBezierPath::bezierPath();
                    p.moveToPoint(CGPoint::new(a.x, a.y));
                    p.addLineToPoint(CGPoint::new(b.x, b.y));
                    p
                }
                Shape::Polygon(pts) => {
                    if pts.len() < 2 {
                        return None;
                    }
                    let p = UIBezierPath::bezierPath();
                    p.moveToPoint(CGPoint::new(pts[0].x, pts[0].y));
                    for pt in &pts[1..] {
                        p.addLineToPoint(CGPoint::new(pt.x, pt.y));
                    }
                    p.closePath();
                    p
                }
            })
        }
    }

    // -----------------------------------------------------------------------
    // The backend
    // -----------------------------------------------------------------------

    #[distributed_slice]
    pub static RENDERERS: [fn() -> Renderer<Uikit>];

    pub struct Uikit {
        registry: Registry<Uikit>,
    }

    impl Uikit {
        pub fn new() -> Self {
            let mut registry = Registry::default();
            for f in RENDERERS {
                registry.register(f());
            }
            Uikit { registry }
        }
    }

    impl Default for Uikit {
        fn default() -> Self {
            Self::new()
        }
    }

    fn mtm() -> MainThreadMarker {
        MainThreadMarker::new().expect("day-uikit: not on the main thread")
    }

    /// Run `body` (which mutates one or more animatable view properties) inside a UIKit animation
    /// matching `anim`, or immediately when `anim` is `None`. Backend-executed animation (§8.4):
    /// UIKit diffs the changes made in the block and animates them on the render server (off the
    /// main thread), so Day never ticks frames for native widgets.
    fn with_uikit_anim(anim: Option<&AnimSpec>, body: impl Fn() + 'static) {
        let Some(a) = anim else {
            body();
            return;
        };
        let animations = block2::RcBlock::new(body);
        let delay = a.delay_secs().max(0.0);
        unsafe {
            match a.curve {
                Curve::Spring { damping, .. } => {
                    // The specified duration is authoritative; `damping` still shapes the bounce.
                    UIView::animateWithDuration_delay_usingSpringWithDamping_initialSpringVelocity_options_animations_completion(
                        a.duration_secs().max(0.05),
                        delay,
                        damping.clamp(0.05, 1.0),
                        0.0,
                        UIViewAnimationOptions(0),
                        &animations,
                        None,
                        mtm(),
                    );
                }
                curve => {
                    UIView::animateWithDuration_delay_options_animations_completion(
                        a.duration_secs().max(0.01),
                        delay,
                        uiview_anim_options(curve),
                        &animations,
                        None,
                        mtm(),
                    );
                }
            }
        }
    }

    fn uiview_anim_options(curve: Curve) -> UIViewAnimationOptions {
        match curve {
            Curve::EaseIn => UIViewAnimationOptions::CurveEaseIn,
            Curve::EaseOut => UIViewAnimationOptions::CurveEaseOut,
            Curve::Linear => UIViewAnimationOptions::CurveLinear,
            // EaseInOut is the 0 default; springs never reach here.
            Curve::EaseInOut | Curve::Spring { .. } => UIViewAnimationOptions::CurveEaseInOut,
        }
    }

    /// Build UIKit's `CGAffineTransform` for a Day [`Transform`], composed scale → rotate →
    /// translate about the view's layer anchor (default center, matching `Transform`'s default
    /// anchor). Non-center anchors approximate: translation is exact, scale/rotation stay about
    /// center (§8.4 — arbitrary-anchor transforms need a layer anchorPoint change, a later refinement).
    fn cgaffine(t: Transform) -> CGAffineTransform {
        let th = t.rotate_deg.to_radians();
        let (s, c) = th.sin_cos();
        CGAffineTransform {
            a: t.sx * c,
            b: t.sx * s,
            c: -t.sy * s,
            d: t.sy * c,
            tx: t.tx,
            ty: t.ty,
        }
    }

    /// Day `Role` → the UIAccessibility trait bit to add (explicit canvas/custom roles only —
    /// native controls self-describe, §13). UIKit has no toggle/meter trait, so those are `None`.
    fn ui_traits(role: day_spec::Role) -> Option<objc2_ui_kit::UIAccessibilityTraits> {
        use day_spec::Role;
        use objc2_ui_kit::{
            UIAccessibilityTraitAdjustable, UIAccessibilityTraitButton, UIAccessibilityTraitHeader,
            UIAccessibilityTraitImage,
        };
        unsafe {
            Some(match role {
                Role::Button | Role::Toggle => UIAccessibilityTraitButton,
                Role::Slider => UIAccessibilityTraitAdjustable,
                Role::Heading(_) => UIAccessibilityTraitHeader,
                Role::Image => UIAccessibilityTraitImage,
                _ => return None,
            })
        }
    }

    /// Native UIAccessibility traits → Day `Role` (best-effort, for `read_a11y`/`a11y_audit`).
    fn day_role_from_traits(t: objc2_ui_kit::UIAccessibilityTraits) -> day_spec::Role {
        use day_spec::Role;
        use objc2_ui_kit::{
            UIAccessibilityTraitAdjustable, UIAccessibilityTraitButton, UIAccessibilityTraitHeader,
            UIAccessibilityTraitImage,
        };
        unsafe {
            if t & UIAccessibilityTraitAdjustable != 0 {
                Role::Slider
            } else if t & UIAccessibilityTraitHeader != 0 {
                Role::Heading(0)
            } else if t & UIAccessibilityTraitImage != 0 {
                Role::Image
            } else if t & UIAccessibilityTraitButton != 0 {
                Role::Button
            } else {
                Role::None
            }
        }
    }

    /// The iOS native semantic text style for a logical [`Font`] (`None` for a custom size).
    /// `UIFont.preferredFont(forTextStyle:)` IS Dynamic Type — it scales with the user's chosen text
    /// size in Settings ▸ Accessibility ▸ Display & Text Size ▸ Larger Text.
    fn ui_text_style(f: Font) -> Option<&'static objc2_ui_kit::UIFontTextStyle> {
        use objc2_ui_kit::*;
        unsafe {
            Some(match f {
                Font::LargeTitle => UIFontTextStyleLargeTitle,
                Font::Title => UIFontTextStyleTitle1,
                Font::Title2 => UIFontTextStyleTitle2,
                Font::Title3 => UIFontTextStyleTitle3,
                Font::Headline => UIFontTextStyleHeadline,
                Font::Subheadline => UIFontTextStyleSubheadline,
                Font::Body => UIFontTextStyleBody,
                Font::Callout => UIFontTextStyleCallout,
                Font::Footnote => UIFontTextStyleFootnote,
                Font::Caption => UIFontTextStyleCaption1,
                Font::Caption2 => UIFontTextStyleCaption2,
                Font::System(_) | Font::Custom(..) => return None,
            })
        }
    }

    fn ui_weight(w: day_spec::FontWeight) -> objc2_ui_kit::UIFontWeight {
        use day_spec::FontWeight as W;
        use objc2_ui_kit::*;
        unsafe {
            match w {
                W::UltraLight => UIFontWeightUltraLight,
                W::Thin => UIFontWeightThin,
                W::Light => UIFontWeightLight,
                W::Regular => UIFontWeightRegular,
                W::Medium => UIFontWeightMedium,
                W::Semibold => UIFontWeightSemibold,
                W::Bold => UIFontWeightBold,
                W::Heavy => UIFontWeightHeavy,
                W::Black => UIFontWeightBlack,
            }
        }
    }

    /// The iOS Dynamic Type DEFAULT (content size = Large) point size for a semantic style — the base
    /// that `UIFontMetrics` scales from. Used to build weighted fonts that still auto-scale.
    fn ui_default_size(f: Font) -> objc2_core_foundation::CGFloat {
        match f {
            Font::LargeTitle => 34.0,
            Font::Title => 28.0,
            Font::Title2 => 22.0,
            Font::Title3 => 20.0,
            Font::Headline => 17.0,
            Font::Subheadline => 15.0,
            Font::Body => 17.0,
            Font::Callout => 16.0,
            Font::Footnote => 13.0,
            Font::Caption => 12.0,
            Font::Caption2 => 11.0,
            Font::System(pt) => pt,
            Font::Custom(_, pt) => pt,
        }
    }

    fn apply_font(label: &UILabel, spec: day_spec::FontSpec) {
        use objc2_ui_kit::*;
        let base: Retained<UIFont> = match spec.style {
            Font::System(pt) => unsafe {
                // A custom size, weighted, then run through UIFontMetrics so it ALSO honors Dynamic
                // Type (accessibility text scale) instead of being a fixed pixel size.
                let w = spec.weight.map(ui_weight).unwrap_or(UIFontWeightRegular);
                let raw = UIFont::systemFontOfSize_weight(pt, w);
                UIFontMetrics::metricsForTextStyle(UIFontTextStyleBody).scaledFontForFont(&raw)
            },
            // A bundled family (§18.4): registered at launch from the DayPieces bundle (and
            // listed in UIAppFonts), then scaled through UIFontMetrics like Font::System so it
            // tracks Dynamic Type. Unknown families fall back to the system font, loudly. A
            // weight override maps to the bold trait below (the family decides what it has).
            Font::Custom(name, pt) => unsafe {
                let raw = match UIFont::fontWithName_size(&NSString::from_str(name), pt) {
                    Some(f) => f,
                    None => {
                        eprintln!(
                            "day: unknown font family {name:?} — falling back to the system \
                             font (is the file in the project's fonts/ directory?)"
                        );
                        let w = spec.weight.map(ui_weight).unwrap_or(UIFontWeightRegular);
                        UIFont::systemFontOfSize_weight(pt, w)
                    }
                };
                UIFontMetrics::metricsForTextStyle(UIFontTextStyleBody).scaledFontForFont(&raw)
            },
            style => unsafe {
                let ts = ui_text_style(style).expect("semantic style");
                match spec.weight {
                    // No weight override → preferredFont, which is Dynamic Type (auto-scales live).
                    None => UIFont::preferredFontForTextStyle(ts),
                    // A weight override: build the weighted system font at the style's DEFAULT size,
                    // then run it through the style's UIFontMetrics so it ALSO auto-scales with Dynamic
                    // Type (a bare `systemFont(ofSize:weight:)` is a fixed size and would NOT re-scale).
                    Some(w) => {
                        let raw =
                            UIFont::systemFontOfSize_weight(ui_default_size(style), ui_weight(w));
                        UIFontMetrics::metricsForTextStyle(ts).scaledFontForFont(&raw)
                    }
                }
            },
        };
        // Symbolic-trait tweaks on the resolved font: italic, plus synthesized bold for a custom
        // family with a heavy weight override (system fonts got their weight above).
        let mut extra = UIFontDescriptorSymbolicTraits::empty();
        if spec.italic {
            extra |= UIFontDescriptorSymbolicTraits::TraitItalic;
        }
        if matches!(spec.style, Font::Custom(..))
            && spec
                .weight
                .is_some_and(|w| w >= day_spec::FontWeight::Semibold)
        {
            extra |= UIFontDescriptorSymbolicTraits::TraitBold;
        }
        let font = if !extra.is_empty() {
            unsafe {
                let desc = base.fontDescriptor();
                let traits = desc.symbolicTraits() | extra;
                match desc.fontDescriptorWithSymbolicTraits(traits) {
                    Some(d2) => UIFont::fontWithDescriptor_size(&d2, base.pointSize()),
                    None => base,
                }
            }
        } else {
            base
        };
        unsafe {
            label.setFont(Some(&font));
            // Re-scale live when the user changes the accessibility text size (works for fonts derived
            // from preferredFont / UIFontMetrics).
            let _: () = objc2::msg_send![label, setAdjustsFontForContentSizeCategory: true];
        }
    }

    /// Warn ONCE per kind that this backend has no registered renderer for `kind`, before falling
    /// back to a visible placeholder. A missing renderer usually means the piece's `uikit` feature
    /// wasn't enabled (Tier A.2 derives it automatically under `day build`). Deduped per kind so a
    /// placeholder rendered every frame doesn't spam the log.
    fn warn_missing_renderer(kind: PieceKind) {
        static SEEN: std::sync::Mutex<Option<std::collections::HashSet<&'static str>>> =
            std::sync::Mutex::new(None);
        let Ok(mut guard) = SEEN.lock() else { return };
        if guard
            .get_or_insert_with(std::collections::HashSet::new)
            .insert(kind)
        {
            eprintln!(
                "day: no renderer for piece kind \"{kind}\" on uikit \
                 — is the piece's uikit feature enabled? (rendering a placeholder)"
            );
        }
    }

    impl Toolkit for Uikit {
        type Handle = Handle;

        fn capability(&self, cap: Cap) -> Support {
            match cap {
                Cap::Dialogs | Cap::FileDialogs | Cap::Animation | Cap::Cover => Support::Native,
                _ => Support::Unsupported,
            }
        }

        fn realize(&mut self, kind: PieceKind, props: &dyn Any, id: NodeId) -> Handle {
            let mtm = mtm();
            match kind {
                kinds::CONTAINER => {
                    let v = unsafe { UIView::new(mtm) };
                    if let Some(p) = props.downcast_ref::<ContainerProps>() {
                        if p.role == Some(day_spec::SurfaceRole::SectionCard) {
                            // tertiarySystemFill is a DYNAMIC UIColor: UIKit re-resolves it on
                            // trait-collection (light/dark) changes automatically.
                            unsafe {
                                v.setBackgroundColor(Some(&UIColor::tertiarySystemFillColor()));
                                let layer = v.layer();
                                layer.setCornerRadius(p.corner_radius);
                                layer.setMasksToBounds(true);
                            }
                        } else if p.background.is_some() || p.corner_radius > 0.0 || p.clips {
                            apply_surface(&v, p.background, p.corner_radius, p.clips);
                        }
                    }
                    view_of(v)
                }
                kinds::NAV => {
                    let p = props.downcast_ref::<NavProps>().unwrap();
                    let _ = p;
                    let nav = unsafe { objc2_ui_kit::UINavigationController::new(mtm) };
                    // Child-VC containment under the window's root VC (v1: app root).
                    let root_vc = WINDOW
                        .with(|w| w.borrow().clone())
                        .and_then(|w| w.rootViewController());
                    if let Some(root_vc) = root_vc {
                        unsafe {
                            root_vc.addChildViewController(&nav);
                            nav.didMoveToParentViewController(Some(&root_vc));
                        }
                    }
                    let host = view_of(unsafe { nav.view() }.expect("nav view"));
                    let delegate = DayNavDelegate::new(mtm, ptr_of(&host));
                    unsafe { nav.setDelegate(Some(ProtocolObject::from_ref(&*delegate))) };
                    NAV_STATE.with(|m| {
                        m.borrow_mut().insert(
                            ptr_of(&host),
                            NavState {
                                nav,
                                host_node: id,
                                vcs: Vec::new(),
                                expect_pop: std::cell::Cell::new(false),
                                native_pops: std::cell::Cell::new(0),
                                last_native: std::cell::Cell::new(0),
                                _delegate: delegate,
                            },
                        )
                    });
                    host
                }
                kinds::NAV_PAGE => {
                    let p = props.downcast_ref::<NavPageProps>().unwrap();
                    let outer = DayNavPageView::new(mtm, id);
                    let content = unsafe { UIView::new(mtm) };
                    unsafe { outer.addSubview(&content) };
                    let vc = unsafe { UIViewController::new(mtm) };
                    unsafe {
                        vc.setView(Some(&outer));
                        vc.setTitle(Some(&NSString::from_str(&p.title)));
                    }
                    let handle = view_of(content);
                    PAGE_VCS.with(|m| m.borrow_mut().insert(ptr_of(&handle), vc));
                    NAV_PAGES.with(|set| set.borrow_mut().insert(ptr_of(&handle)));
                    handle
                }
                // Fullscreen cover (docs/cover.md): a DayCoverVC over a DayNavPageView (safe-
                // area pinning + FrameChanged reports, like a nav page), created detached;
                // CoverPatch::Present shows it modally over the whole window.
                kinds::COVER => {
                    let outer = DayNavPageView::new(mtm, id);
                    let content = unsafe { UIView::new(mtm) };
                    unsafe { outer.addSubview(&content) };
                    let vc = DayCoverVC::new(mtm);
                    unsafe {
                        vc.setView(Some(&outer));
                        vc.setModalPresentationStyle(UIModalPresentationStyle::FullScreen);
                    }
                    let handle = view_of(content);
                    COVER_STATE.with(|m| {
                        m.borrow_mut()
                            .insert(ptr_of(&handle), CoverState { vc, node: id })
                    });
                    // The content view's frame is native-owned (the cover VC lays it out).
                    NAV_PAGES.with(|set| set.borrow_mut().insert(ptr_of(&handle)));
                    handle
                }
                kinds::TABS => {
                    let p = props.downcast_ref::<TabsProps>().unwrap();
                    let tabbar = unsafe { UITabBarController::new(mtm) };
                    let root_vc = WINDOW
                        .with(|w| w.borrow().clone())
                        .and_then(|w| w.rootViewController());
                    if let Some(root_vc) = root_vc {
                        unsafe {
                            root_vc.addChildViewController(&tabbar);
                            tabbar.didMoveToParentViewController(Some(&root_vc));
                        }
                    }
                    let host = view_of(unsafe { tabbar.view() }.expect("tabbar view"));
                    let delegate = DayTabDelegate::new(mtm, id);
                    unsafe { tabbar.setDelegate(Some(ProtocolObject::from_ref(&*delegate))) };
                    TABS_STATE.with(|m| {
                        m.borrow_mut().insert(
                            ptr_of(&host),
                            TabsState {
                                tabbar,
                                vcs: Vec::new(),
                                initial: p.selected,
                                _delegate: delegate,
                            },
                        )
                    });
                    host
                }
                kinds::TABS_PAGE => {
                    let p = props.downcast_ref::<TabsPageProps>().unwrap();
                    let outer = DayNavPageView::new(mtm, id);
                    let content = unsafe { UIView::new(mtm) };
                    unsafe { outer.addSubview(&content) };
                    let vc = unsafe { UIViewController::new(mtm) };
                    unsafe {
                        vc.setView(Some(&outer));
                        // The VC title becomes its tab bar item's label.
                        vc.setTitle(Some(&NSString::from_str(&p.title)));
                    }
                    // Optional tab icon (docs/tabs.md): a bundled template image on the tab item,
                    // the iOS-idiomatic tab bar (icon over label). Template mode tints with the tab
                    // bar's colour (unselected grey, selected accent).
                    if let Some(name) = p.icon.as_deref()
                        && let Some(img) = load_bundled_uiimage(name)
                    {
                        // Tab-bar icons are ~25pt; the shared 96px asset must be downscaled (a
                        // UITabBar shows an image at its full point size otherwise). Prepare a
                        // thumbnail, then template so it tints with the bar (grey/selected accent).
                        let sized =
                            unsafe { img.imageByPreparingThumbnailOfSize(CGSize::new(26.0, 26.0)) }
                                .unwrap_or(img);
                        let templ = unsafe {
                            sized.imageWithRenderingMode(
                                objc2_ui_kit::UIImageRenderingMode::AlwaysTemplate,
                            )
                        };
                        if let Some(item) = unsafe { vc.tabBarItem() } {
                            unsafe { item.setImage(Some(&templ)) };
                        }
                    }
                    let handle = view_of(content);
                    TABS_PAGE_VCS.with(|m| m.borrow_mut().insert(ptr_of(&handle), vc));
                    NAV_PAGES.with(|set| set.borrow_mut().insert(ptr_of(&handle)));
                    handle
                }
                kinds::NAV_MENU => {
                    let p = props.downcast_ref::<NavMenuProps>().unwrap();
                    let data = DayNavTableData::new(mtm, id, &p.items, &p.icons);
                    let table = unsafe {
                        objc2_ui_kit::UITableView::initWithFrame_style(
                            objc2_ui_kit::UITableView::alloc(mtm),
                            CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(320.0, 400.0)),
                            objc2_ui_kit::UITableViewStyle::InsetGrouped,
                        )
                    };
                    unsafe {
                        table.setDataSource(Some(ProtocolObject::from_ref(&*data)));
                        table.setDelegate(Some(ProtocolObject::from_ref(&*data)));
                        table.reloadData();
                    }
                    let view = view_of(table);
                    NAV_MENUS.with(|m| m.borrow_mut().insert(ptr_of(&view), (data, p.items.len())));
                    view
                }
                kinds::LIST => {
                    let p = props.downcast_ref::<ListProps>().unwrap();
                    let row_height = match p.row_height {
                        RowHeight::Uniform(h) => h,
                        RowHeight::Automatic => 44.0,
                    };
                    let table = unsafe {
                        objc2_ui_kit::UITableView::initWithFrame_style(
                            objc2_ui_kit::UITableView::alloc(mtm),
                            CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(0.0, 0.0)),
                            objc2_ui_kit::UITableViewStyle::Plain,
                        )
                    };
                    let data = DayListData::new(mtm, id, p.selectable, row_height);
                    unsafe {
                        table.setRowHeight(row_height);
                        table.setDataSource(Some(ProtocolObject::from_ref(&*data)));
                        table.setDelegate(Some(ProtocolObject::from_ref(&*data)));
                        if !p.selectable {
                            table.setAllowsSelection(false);
                        }
                    }
                    let view = view_of(table.clone());
                    LIST_STATE.with(|m| m.borrow_mut().insert(ptr_of(&view), (table, data)));
                    view
                }
                kinds::SCROLL => {
                    let sv = unsafe { UIScrollView::new(mtm) };
                    view_of(sv)
                }
                kinds::LABEL => {
                    let p = props.downcast_ref::<LabelProps>().unwrap();
                    let label = unsafe { UILabel::new(mtm) };
                    unsafe {
                        label.setText(Some(&NSString::from_str(&p.text)));
                        label.setNumberOfLines(0);
                    }
                    apply_font(&label, p.font);
                    if let Some(c) = p.color {
                        unsafe { label.setTextColor(Some(&uicolor(c))) };
                    }
                    view_of(label)
                }
                kinds::BUTTON => {
                    let p = props.downcast_ref::<ButtonProps>().unwrap();
                    let target = DayTarget::new(mtm, id);
                    let btn = unsafe { UIButton::buttonWithType(UIButtonType::System, mtm) };
                    unsafe {
                        // Bordered / Prominent map to UIButtonConfiguration tiers (iOS 15+) —
                        // the plain system button reads as a LINK, not a button. A configured
                        // button takes its title from the configuration, so set it there.
                        match p.style {
                            day_spec::props::ButtonStyleSpec::Automatic => {
                                btn.setTitle_forState(
                                    Some(&NSString::from_str(&p.title)),
                                    UIControlState::Normal,
                                );
                            }
                            style => {
                                let config = match style {
                                    day_spec::props::ButtonStyleSpec::Prominent => {
                                        objc2_ui_kit::UIButtonConfiguration::borderedProminentButtonConfiguration(mtm)
                                    }
                                    _ => objc2_ui_kit::UIButtonConfiguration::borderedButtonConfiguration(mtm),
                                };
                                config.setTitle(Some(&NSString::from_str(&p.title)));
                                btn.setConfiguration(Some(&config));
                            }
                        }
                        let tobj: &AnyObject = target.as_ref();
                        btn.addTarget_action_forControlEvents(
                            Some(tobj),
                            sel!(fire:),
                            UIControlEvents::TouchUpInside,
                        );
                    }
                    let view = view_of(btn);
                    TARGETS.with(|m| m.borrow_mut().insert(ptr_of(&view), target));
                    view
                }
                kinds::TOGGLE => {
                    let p = props.downcast_ref::<ToggleProps>().unwrap();
                    let target = DayTarget::new(mtm, id);
                    let sw = unsafe { UISwitch::new(mtm) };
                    unsafe {
                        sw.setOn(p.on);
                        let tobj: &AnyObject = target.as_ref();
                        sw.addTarget_action_forControlEvents(
                            Some(tobj),
                            sel!(fire:),
                            UIControlEvents::ValueChanged,
                        );
                    }
                    let view = view_of(sw);
                    TARGETS.with(|m| m.borrow_mut().insert(ptr_of(&view), target));
                    view
                }
                kinds::SLIDER => {
                    let p = props.downcast_ref::<SliderProps>().unwrap();
                    let target = DayTarget::new(mtm, id);
                    let sl = unsafe { UISlider::new(mtm) };
                    unsafe {
                        sl.setMinimumValue(p.min as f32);
                        sl.setMaximumValue(p.max as f32);
                        sl.setValue(p.value as f32);
                        let tobj: &AnyObject = target.as_ref();
                        sl.addTarget_action_forControlEvents(
                            Some(tobj),
                            sel!(fire:),
                            UIControlEvents::ValueChanged,
                        );
                    }
                    let view = view_of(sl);
                    TARGETS.with(|m| m.borrow_mut().insert(ptr_of(&view), target));
                    view
                }
                kinds::PICKER => crate::picker::realize_any(self, props, id),
                kinds::TEXT_AREA => crate::textarea::realize_any(self, props, id),
                kinds::TEXT_FIELD => {
                    let p = props.downcast_ref::<TextFieldProps>().unwrap();
                    let target = DayTarget::new(mtm, id);
                    let tf = unsafe { UITextField::new(mtm) };
                    unsafe {
                        tf.setText(Some(&NSString::from_str(&p.text)));
                        tf.setPlaceholder(Some(&NSString::from_str(&p.placeholder)));
                        tf.setBorderStyle(UITextBorderStyle::RoundedRect);
                        let tobj: &AnyObject = target.as_ref();
                        tf.addTarget_action_forControlEvents(
                            Some(tobj),
                            sel!(fire:),
                            UIControlEvents::EditingChanged,
                        );
                        // Focus + submit (docs/focus.md): begin/end report the focus pair;
                        // end-on-exit is the Return key (and makes Return dismiss the keyboard).
                        tf.addTarget_action_forControlEvents(
                            Some(tobj),
                            sel!(editBegan:),
                            UIControlEvents::EditingDidBegin,
                        );
                        tf.addTarget_action_forControlEvents(
                            Some(tobj),
                            sel!(editEnded:),
                            UIControlEvents::EditingDidEnd,
                        );
                        tf.addTarget_action_forControlEvents(
                            Some(tobj),
                            sel!(editExit:),
                            UIControlEvents::EditingDidEndOnExit,
                        );
                    }
                    let view = view_of(tf);
                    TARGETS.with(|m| m.borrow_mut().insert(ptr_of(&view), target));
                    view
                }
                kinds::DIVIDER => {
                    let v = unsafe { UIView::new(mtm) };
                    unsafe { v.setBackgroundColor(Some(&UIColor::separatorColor())) };
                    view_of(v)
                }
                kinds::PROGRESS => {
                    let p = props.downcast_ref::<ProgressProps>().unwrap();
                    match p.value {
                        Some(v) => {
                            let pv = unsafe { UIProgressView::new(mtm) };
                            unsafe { pv.setProgress(v as f32) };
                            view_of(pv)
                        }
                        None => {
                            let ai = unsafe { UIActivityIndicatorView::new(mtm) };
                            unsafe { ai.startAnimating() };
                            view_of(ai)
                        }
                    }
                }
                kinds::CANVAS => view_of(DayCanvasView::new(mtm)),
                kinds::IMAGE => {
                    let p = props.downcast_ref::<ImageProps>().unwrap();
                    let iv = unsafe { objc2_ui_kit::UIImageView::new(mtm) };
                    // Scaling (§18.3): AspectFit / AspectFill (crop, clipped) / ScaleToFill.
                    let mode = match p.content_mode {
                        ContentMode::Fit => objc2_ui_kit::UIViewContentMode::ScaleAspectFit,
                        ContentMode::Fill => objc2_ui_kit::UIViewContentMode::ScaleAspectFill,
                        ContentMode::Stretch => objc2_ui_kit::UIViewContentMode::ScaleToFill,
                    };
                    unsafe {
                        iv.setContentMode(mode);
                        iv.setClipsToBounds(true);
                    }
                    let name = NSString::from_str(&p.source);
                    let mut set = false;
                    // Processed image (§18.3): load by name from the DayPieces `Assets.car` — the
                    // SwiftPM `.process` catalog compiled by actool into DayPieces_DayPieces.bundle.
                    let main = unsafe { objc2_foundation::NSBundle::mainBundle() };
                    let bname = NSString::from_str("DayPieces_DayPieces");
                    let bext = NSString::from_str("bundle");
                    if let Some(url) =
                        unsafe { main.URLForResource_withExtension(Some(&bname), Some(&bext)) }
                        && let Some(day_bundle) =
                            unsafe { objc2_foundation::NSBundle::bundleWithURL(&url) }
                        && let Some(img) = unsafe {
                            objc2_ui_kit::UIImage::imageNamed_inBundle_compatibleWithTraitCollection(
                                &name,
                                Some(&day_bundle),
                                None,
                            )
                        }
                    {
                        unsafe { iv.setImage(Some(&img)) };
                        set = true;
                    }
                    // Fallback: a loose file staged in the bundle (assets/ or images/), or dev.
                    if !set
                        && let Some(path) = day_spec::resource::resolve_image_file(&p.source)
                        && let Some(img) = unsafe {
                            objc2_ui_kit::UIImage::imageWithContentsOfFile(&NSString::from_str(
                                &path.to_string_lossy(),
                            ))
                        }
                    {
                        unsafe { iv.setImage(Some(&img)) };
                    }
                    view_of(iv)
                }
                _ => {
                    if let Some(make) = self.registry.get(kind).map(|r| r.make) {
                        return make(self, props, id);
                    }
                    warn_missing_renderer(kind);
                    let label = unsafe { UILabel::new(mtm) };
                    unsafe { label.setText(Some(&NSString::from_str(&format!("⟨{kind}⟩")))) };
                    view_of(label)
                }
            }
        }

        fn update(
            &mut self,
            h: &Handle,
            kind: PieceKind,
            patch: &dyn Any,
            anim: Option<&AnimSpec>,
        ) {
            match kind {
                kinds::CONTAINER => {
                    if let Some(ContainerPatch::Background(c)) =
                        patch.downcast_ref::<ContainerPatch>()
                    {
                        let v = h.clone();
                        let c = *c;
                        with_uikit_anim(anim, move || unsafe {
                            match c {
                                Some(c) => v.setBackgroundColor(Some(&uicolor(c))),
                                None => v.setBackgroundColor(None),
                            }
                        });
                    }
                }
                kinds::TABS => {
                    if let Some(TabsPatch::Selected(i)) = patch.downcast_ref::<TabsPatch>() {
                        let tabbar = TABS_STATE.with(|m| {
                            m.borrow()
                                .get(&ptr_of(h))
                                .and_then(|s| (*i < s.vcs.len()).then(|| s.tabbar.clone()))
                        });
                        if let Some(tabbar) = tabbar {
                            unsafe { tabbar.setSelectedIndex(*i) };
                        }
                    }
                }
                kinds::COVER => {
                    if let Some(p) = patch.downcast_ref::<CoverPatch>() {
                        let state = COVER_STATE
                            .with(|m| m.borrow().get(&ptr_of(h)).map(|s| (s.vc.clone(), s.node)));
                        let Some((vc, node)) = state else { return };
                        match p {
                            CoverPatch::Present {
                                background,
                                dismiss_disabled,
                            } => {
                                if let (Some(c), Some(view)) = (background, vc.view()) {
                                    unsafe { view.setBackgroundColor(Some(&uicolor(*c))) };
                                }
                                // Inert under .fullScreen, but honored if the presentation
                                // style ever becomes a sheet.
                                unsafe { vc.setModalInPresentation(*dismiss_disabled) };
                                cover_present(vc);
                            }
                            CoverPatch::DismissDisabled(d) => unsafe {
                                vc.setModalInPresentation(*d);
                            },
                            CoverPatch::Dismiss => cover_dismiss(vc, node),
                        }
                    }
                }
                kinds::NAV => {
                    if let Some(p) = patch.downcast_ref::<NavPatch>() {
                        // Copy out of NAV_STATE BEFORE touching UIKit: push/pop can invoke
                        // the delegate synchronously, which re-borrows NAV_STATE.
                        enum Act {
                            Push(
                                Retained<UIViewController>,
                                Retained<objc2_ui_kit::UINavigationController>,
                            ),
                            Pop(Retained<objc2_ui_kit::UINavigationController>),
                            None,
                        }
                        let act = NAV_STATE.with(|m| {
                            let m = m.borrow();
                            let Some(state) = m.get(&ptr_of(h)) else {
                                return Act::None;
                            };
                            match p {
                                NavPatch::Pushed { .. } => state
                                    .vcs
                                    .last()
                                    .map(|vc| Act::Push(vc.clone(), state.nav.clone()))
                                    .unwrap_or(Act::None),
                                NavPatch::Popped => {
                                    // Answering a native user-back? The stack already popped, so
                                    // absorb it (don't pop again — that stale pop would wedge
                                    // expect_pop). Otherwise it's a day-initiated pop: perform it.
                                    if state.native_pops.get() > 0 {
                                        state.native_pops.set(state.native_pops.get() - 1);
                                        Act::None
                                    } else {
                                        state.expect_pop.set(true);
                                        Act::Pop(state.nav.clone())
                                    }
                                }
                                NavPatch::Title(_) => Act::None,
                            }
                        });
                        // Defer past any in-flight modal transition: a push/pop issued the
                        // instant a (scripted) dialog dismissal starts races the dismissal
                        // transition and wedges the navigation controller.
                        match act {
                            Act::Push(vc, nav) => modal_after_idle(move || unsafe {
                                nav.pushViewController_animated(&vc, true)
                            }),
                            Act::Pop(nav) => modal_after_idle(move || {
                                let _ = unsafe { nav.popViewControllerAnimated(true) };
                            }),
                            Act::None => {}
                        }
                    }
                }
                kinds::LABEL => {
                    if let (Some(p), Some(label)) = (
                        patch.downcast_ref::<LabelPatch>(),
                        (**h).downcast_ref::<UILabel>(),
                    ) {
                        match p {
                            LabelPatch::Text(t) => unsafe {
                                label.setText(Some(&NSString::from_str(t)))
                            },
                            LabelPatch::Font(f) => apply_font(label, *f),
                            // `None` restores the adaptive default (labelColor tracks dark mode).
                            LabelPatch::Color(c) => unsafe {
                                match c {
                                    Some(c) => label.setTextColor(Some(&uicolor(*c))),
                                    None => label.setTextColor(Some(&UIColor::labelColor())),
                                }
                            },
                        }
                    }
                }
                kinds::BUTTON => {
                    if let (Some(p), Some(btn)) = (
                        patch.downcast_ref::<ButtonPatch>(),
                        (**h).downcast_ref::<UIButton>(),
                    ) {
                        match p {
                            ButtonPatch::Title(t) => unsafe {
                                // A configured (bordered/prominent) button titles via its
                                // configuration; a plain one via the state title.
                                if let Some(config) = btn.configuration() {
                                    config.setTitle(Some(&NSString::from_str(t)));
                                    btn.setConfiguration(Some(&config));
                                } else {
                                    btn.setTitle_forState(
                                        Some(&NSString::from_str(t)),
                                        UIControlState::Normal,
                                    )
                                }
                            },
                            ButtonPatch::Enabled(e) => unsafe { btn.setEnabled(*e) },
                        }
                    }
                }
                kinds::TOGGLE => {
                    if let (Some(p), Some(sw)) = (
                        patch.downcast_ref::<TogglePatch>(),
                        (**h).downcast_ref::<UISwitch>(),
                    ) {
                        match p {
                            TogglePatch::On(on) => {
                                if unsafe { sw.isOn() } != *on {
                                    unsafe { sw.setOn(*on) };
                                }
                            }
                            TogglePatch::Enabled(e) => unsafe { sw.setEnabled(*e) },
                        }
                    }
                }
                kinds::SLIDER => {
                    if let (Some(p), Some(sl)) = (
                        patch.downcast_ref::<SliderPatch>(),
                        (**h).downcast_ref::<UISlider>(),
                    ) {
                        match p {
                            SliderPatch::Value(v) => {
                                if (unsafe { sl.value() } as f64 - v).abs() > 0.001 {
                                    unsafe { sl.setValue(*v as f32) };
                                }
                            }
                            SliderPatch::Enabled(e) => unsafe { sl.setEnabled(*e) },
                        }
                    }
                }
                kinds::PROGRESS => {
                    if let Some(ProgressPatch::Value(Some(val))) =
                        patch.downcast_ref::<ProgressPatch>()
                        && let Some(pv) = (**h).downcast_ref::<UIProgressView>()
                        && (unsafe { pv.progress() } as f64 - val).abs() > 0.0001
                    {
                        unsafe { pv.setProgress(*val as f32) };
                    }
                }
                kinds::PICKER => crate::picker::update_any(self, h, patch),
                kinds::TEXT_AREA => crate::textarea::update_any(self, h, patch),
                kinds::TEXT_FIELD => {
                    if let (Some(p), Some(tf)) = (
                        patch.downcast_ref::<TextFieldPatch>(),
                        (**h).downcast_ref::<UITextField>(),
                    ) {
                        match p {
                            TextFieldPatch::Text { text, from_native } => {
                                let cur = unsafe { tf.text() }
                                    .map(|s| s.to_string())
                                    .unwrap_or_default();
                                if !*from_native && cur != *text {
                                    unsafe { tf.setText(Some(&NSString::from_str(text))) };
                                }
                            }
                            TextFieldPatch::Placeholder(t) => unsafe {
                                tf.setPlaceholder(Some(&NSString::from_str(t)))
                            },
                            TextFieldPatch::Enabled(e) => unsafe { tf.setEnabled(*e) },
                        }
                    }
                }
                kinds::LIST => match patch.downcast_ref::<ListPatch>() {
                    Some(ListPatch::Reload) => {
                        LIST_STATE.with(|m| {
                            if let Some((table, _)) = m.borrow().get(&ptr_of(h)) {
                                // reloadData: numberOfRows reads the snapshot only, cellForRow is
                                // deferred — safe inside a with_tree borrow.
                                unsafe { table.reloadData() };
                            }
                        });
                    }
                    Some(ListPatch::ScrollToEnd) => {
                        LIST_STATE.with(|m| {
                            if let Some((table, data)) = m.borrow().get(&ptr_of(h)) {
                                // Row count from the snapshot (no tree). Empty list → no-op.
                                let n = data
                                    .ivars()
                                    .source
                                    .borrow()
                                    .as_ref()
                                    .map(|s| (s.len)())
                                    .unwrap_or(0);
                                if n > 0 {
                                    let ip =
                                        objc2_foundation::NSIndexPath::indexPathForRow_inSection(
                                            (n - 1) as isize,
                                            0,
                                        );
                                    unsafe {
                                        table.scrollToRowAtIndexPath_atScrollPosition_animated(
                                            &ip,
                                            objc2_ui_kit::UITableViewScrollPosition::Bottom,
                                            true,
                                        )
                                    };
                                }
                            }
                        });
                    }
                    _ => {}
                },
                _ => {
                    if let Some(update) = self.registry.get(kind).map(|r| r.update) {
                        update(self, h, patch);
                    }
                }
            }
        }

        fn release(&mut self, h: Handle) {
            TARGETS.with(|m| {
                m.borrow_mut().remove(&ptr_of(&h));
            });
            LIST_STATE.with(|m| {
                m.borrow_mut().remove(&ptr_of(&h));
            });
            NAV_STATE.with(|m| {
                m.borrow_mut().remove(&ptr_of(&h));
            });
            NAV_PAGES.with(|set| {
                set.borrow_mut().remove(&ptr_of(&h));
            });
            PAGE_VCS.with(|m| {
                m.borrow_mut().remove(&ptr_of(&h));
            });
            COVER_STATE.with(|m| {
                m.borrow_mut().remove(&ptr_of(&h));
            });
            NAV_MENUS.with(|m| {
                m.borrow_mut().remove(&ptr_of(&h));
            });
            TABS_STATE.with(|m| {
                m.borrow_mut().remove(&ptr_of(&h));
            });
            TABS_PAGE_VCS.with(|m| {
                m.borrow_mut().remove(&ptr_of(&h));
            });
            GESTURES.with(|m| {
                m.borrow_mut().remove(&ptr_of(&h));
            });
            unsafe { h.removeFromSuperview() };
        }

        fn insert(&mut self, parent: &Handle, child: &Handle, index: usize) {
            // Tabs host: the page's VC joins the tab bar controller. All tabs are resident, so
            // rebuild the VC array on each insert and select the requested initial tab.
            let tabs_install = TABS_STATE.with(|m| {
                let mut m = m.borrow_mut();
                let state = m.get_mut(&ptr_of(parent))?;
                let vc = TABS_PAGE_VCS.with(|p| p.borrow().get(&ptr_of(child)).cloned())?;
                let at = index.min(state.vcs.len());
                state.vcs.insert(at, vc);
                Some((state.tabbar.clone(), state.vcs.clone(), state.initial))
            });
            if let Some((tabbar, vcs, initial)) = tabs_install {
                let arr = objc2_foundation::NSArray::from_retained_slice(&vcs);
                unsafe { tabbar.setViewControllers(Some(&arr)) };
                let sel = initial.min(vcs.len().saturating_sub(1));
                unsafe { tabbar.setSelectedIndex(sel) };
                return;
            }
            // Nav host: pages join the VC stack; index 0 becomes the root VC now, later
            // pages are presented by the Pushed patch.
            // Copy out of NAV_STATE before setViewControllers (same re-entrancy rule).
            let set_root = NAV_STATE.with(|m| {
                let mut m = m.borrow_mut();
                let state = m.get_mut(&ptr_of(parent))?;
                let vc = PAGE_VCS.with(|p| p.borrow().get(&ptr_of(child)).cloned())?;
                state.vcs.push(vc.clone());
                Some((index == 0).then_some((state.nav.clone(), vc)))
            });
            match set_root {
                Some(Some((nav, vc))) => {
                    let arr = objc2_foundation::NSArray::from_retained_slice(&[vc]);
                    unsafe { nav.setViewControllers(&arr) };
                }
                Some(None) => {}
                None => {
                    // A cover's content view already lives inside its DayCoverVC's view —
                    // reparenting it into the tree slot (addSubview MOVES a view) would
                    // strand the presented cover empty (docs/cover.md).
                    if COVER_STATE.with(|m| m.borrow().contains_key(&ptr_of(child))) {
                        return;
                    }
                    unsafe { parent.addSubview(child) }
                }
            }
        }

        fn remove(&mut self, parent: &Handle, child: &Handle) {
            let nav_child = NAV_STATE.with(|m| {
                let mut m = m.borrow_mut();
                let Some(state) = m.get_mut(&ptr_of(parent)) else {
                    return false;
                };
                if let Some(vc) = PAGE_VCS.with(|p| p.borrow().get(&ptr_of(child)).cloned()) {
                    state.vcs.retain(|v| !std::ptr::eq(&**v, &*vc));
                }
                true
            });
            if !nav_child {
                unsafe { child.removeFromSuperview() };
            }
        }

        fn move_child(&mut self, parent: &Handle, child: &Handle, _to: usize) {
            unsafe { parent.addSubview(child) };
        }

        fn measure(&mut self, h: &Handle, kind: PieceKind, p: Proposal) -> Size {
            let fit = |w: f64, hh: f64| {
                let s = unsafe { h.sizeThatFits(CGSize::new(w, hh)) };
                Size::new(s.width.ceil(), s.height.ceil())
            };
            match kind {
                kinds::NAV_MENU => {
                    let rows = NAV_MENUS
                        .with(|m| m.borrow().get(&ptr_of(h)).map(|(_, n)| *n).unwrap_or(0));
                    Size::new(
                        p.width.unwrap_or(320.0),
                        p.height.unwrap_or(rows as f64 * 44.0 + 40.0),
                    )
                }
                kinds::LABEL => {
                    let w = p.width.unwrap_or(1.0e6);
                    let s = fit(w, 1.0e6);
                    Size::new(s.width.min(w), s.height)
                }
                kinds::BUTTON | kinds::TOGGLE => fit(1.0e6, 1.0e6),
                kinds::SLIDER => {
                    Size::new(p.width.unwrap_or(180.0), fit(1.0e6, 1.0e6).height.max(31.0))
                }
                kinds::PICKER => crate::picker::measure_any(self, h, p),
                kinds::TEXT_AREA => crate::textarea::measure_any(self, h, p),
                kinds::TEXT_FIELD => {
                    Size::new(p.width.unwrap_or(180.0), fit(1.0e6, 1.0e6).height.max(34.0))
                }
                kinds::DIVIDER => Size::new(p.width.unwrap_or(0.0), 1.0),
                kinds::PROGRESS => {
                    if (**h).downcast_ref::<UIActivityIndicatorView>().is_some() {
                        Size::new(20.0, 20.0)
                    } else {
                        Size::new(p.width.unwrap_or(180.0), 4.0)
                    }
                }
                kinds::LIST => Size::new(p.width.unwrap_or(0.0), p.height.unwrap_or(0.0)),
                _ => {
                    if let Some(measure) = self.registry.get(kind).and_then(|r| r.measure) {
                        measure(self, h, p)
                    } else {
                        let s = fit(1.0e6, 1.0e6);
                        Size::new(p.width.unwrap_or(s.width), p.height.unwrap_or(s.height))
                    }
                }
            }
        }

        fn set_frame(&mut self, h: &Handle, frame: Rect, anim: Option<&AnimSpec>) {
            // Nav page content: the page view pins it to the safe area (native-owned).
            if NAV_PAGES.with(|set| set.borrow().contains(&ptr_of(h))) {
                return;
            }
            let f = CGRect::new(
                CGPoint::new(frame.origin.x, frame.origin.y),
                CGSize::new(frame.size.width, frame.size.height),
            );
            let v = h.clone();
            with_uikit_anim(anim, move || unsafe { v.setFrame(f) });
        }

        fn set_opacity(&mut self, h: &Handle, opacity: f64, anim: Option<&AnimSpec>) {
            let v = h.clone();
            with_uikit_anim(anim, move || unsafe { v.setAlpha(opacity as CGFloat) });
        }

        fn set_transform(
            &mut self,
            h: &Handle,
            t: Transform,
            _size: Size,
            anim: Option<&AnimSpec>,
        ) {
            let v = h.clone();
            let tf = cgaffine(t);
            with_uikit_anim(anim, move || unsafe { v.setTransform(tf) });
        }

        fn set_scroll_content(&mut self, h: &Handle, content: Size) {
            if let Some(sv) = (**h).downcast_ref::<UIScrollView>() {
                unsafe { sv.setContentSize(CGSize::new(content.width, content.height)) };
            }
        }

        fn scroll_to(&mut self, h: &Handle, target: Rect, animated: bool) {
            if let Some(sv) = (**h).downcast_ref::<UIScrollView>() {
                unsafe {
                    sv.scrollRectToVisible_animated(
                        CGRect::new(
                            CGPoint::new(target.origin.x, target.origin.y),
                            CGSize::new(target.size.width, target.size.height),
                        ),
                        animated,
                    )
                };
            }
        }

        fn focus(&mut self, h: &Handle, _node: NodeId, focused: bool) {
            // Focus IS the keyboard on iOS: becoming first responder raises it, resigning
            // dismisses it. Resign only while this view still owns it, so a stale release
            // can't drop a sibling's keyboard.
            unsafe {
                if focused {
                    h.becomeFirstResponder();
                } else if h.isFirstResponder() {
                    h.resignFirstResponder();
                }
            }
        }

        fn set_event_sink(&mut self, sink: EventSink) {
            SINK.with(|s| *s.borrow_mut() = Some(Rc::from(sink)));
        }

        fn enable_gesture(&mut self, h: &Handle, node: NodeId, kind: day_spec::GestureKind) {
            let key = ptr_of(h);
            let is_drag = matches!(kind, day_spec::GestureKind::Drag);
            let already = GESTURES.with(|m| {
                m.borrow()
                    .get(&key)
                    .is_some_and(|v| v.iter().any(|t| t.ivars().is_drag == is_drag))
            });
            if already {
                return;
            }
            let mtm = mtm();
            let target = DayGesture::new(mtm, node, is_drag);
            unsafe {
                let recognizer: Retained<UIGestureRecognizer> = if is_drag {
                    let pan = UIPanGestureRecognizer::initWithTarget_action(
                        UIPanGestureRecognizer::alloc(mtm),
                        Some(&target),
                        Some(sel!(fire:)),
                    );
                    Retained::into_super(pan)
                } else {
                    let tap = UITapGestureRecognizer::initWithTarget_action(
                        UITapGestureRecognizer::alloc(mtm),
                        Some(&target),
                        Some(sel!(fire:)),
                    );
                    Retained::into_super(tap)
                };
                h.setUserInteractionEnabled(true);
                h.addGestureRecognizer(&recognizer);
            }
            GESTURES.with(|m| m.borrow_mut().entry(key).or_default().push(target));
        }

        fn set_context_menu(&mut self, h: &Handle, _node: NodeId, items: &[day_spec::MenuItem]) {
            let key = ptr_of(h);
            // Remove any prior interaction (replace-on-reconfigure).
            if let Some((interaction, _)) = CTX_MENUS.with(|m| m.borrow_mut().remove(&key)) {
                let proto = ProtocolObject::from_ref(&*interaction);
                unsafe { h.removeInteraction(proto) };
            }
            if items.is_empty() {
                return;
            }
            let mtm = mtm();
            let menu = build_ui_menu(mtm, "", items);
            let delegate = DayContextMenu::new(mtm, menu);
            let proto = ProtocolObject::from_ref(&*delegate);
            let interaction = unsafe {
                UIContextMenuInteraction::initWithDelegate(
                    UIContextMenuInteraction::alloc(mtm),
                    proto,
                )
            };
            unsafe {
                h.setUserInteractionEnabled(true);
                h.addInteraction(ProtocolObject::from_ref(&*interaction));
            }
            CTX_MENUS.with(|m| m.borrow_mut().insert(key, (interaction, delegate)));
        }

        fn set_app_menu(&mut self, _items: &[day_spec::MenuItem]) {
            // iOS has no persistent global menu bar (that is a Mac Catalyst / iPad-with-keyboard
            // concern handled via UIMenuBuilder in `buildMenuWithBuilder:`). On iPhone the native
            // affordances are the per-view context menu (`set_context_menu`) and the system edit
            // menu; a global bar is intentionally a no-op here. See docs/menus.md.
        }

        fn supports_lifecycle(&self, phase: day_spec::Lifecycle) -> bool {
            lifecycle_supported(phase)
        }

        fn attach_list(&mut self, host: &Handle, source: ListSource) {
            LIST_STATE.with(|m| {
                if let Some((table, data)) = m.borrow().get(&ptr_of(host)) {
                    data.ivars().source.replace(Some(source));
                    unsafe { table.reloadData() };
                }
            });
        }

        fn adopt(&mut self, raw: RawHandle) -> Handle {
            // A recycling UITableViewCell's contentView — Day fills/rebinds its row content there.
            let ptr = raw as *mut UIView;
            unsafe { Retained::retain(ptr) }.expect("adopt: null list cell content")
        }

        fn set_a11y(&mut self, h: &Handle, a11y: &A11yProps) {
            unsafe {
                if let Some(id) = &a11y.identifier {
                    let ns = NSString::from_str(id);
                    let _: () = msg_send![&**h, setAccessibilityIdentifier: &*ns];
                }
                if let Some(label) = &a11y.label {
                    let ns = NSString::from_str(label);
                    let _: () = msg_send![&**h, setAccessibilityLabel: &*ns];
                }
                if let Some(hint) = &a11y.hint {
                    let ns = NSString::from_str(hint);
                    let _: () = msg_send![&**h, setAccessibilityHint: &*ns];
                }
                if let Some(value) = &a11y.value {
                    let ns = NSString::from_str(value);
                    let _: () = msg_send![&**h, setAccessibilityValue: &*ns];
                }
                // Explicit role → traits (canvas/custom; native controls self-describe, §13).
                if let Some(traits) = ui_traits(a11y.role) {
                    let _: () = msg_send![&**h, setAccessibilityTraits: traits];
                }
                if a11y.hidden {
                    let _: () = msg_send![&**h, setAccessibilityElementsHidden: true];
                }
            }
        }

        fn read_a11y(&self, h: &Handle) -> day_spec::A11ySnapshot {
            unsafe {
                let traits: objc2_ui_kit::UIAccessibilityTraits =
                    msg_send![&**h, accessibilityTraits];
                let label: Option<Retained<NSString>> = msg_send![&**h, accessibilityLabel];
                let value: Option<Retained<NSString>> = msg_send![&**h, accessibilityValue];
                let ident: Option<Retained<NSString>> = msg_send![&**h, accessibilityIdentifier];
                day_spec::A11ySnapshot {
                    found: true,
                    role: day_role_from_traits(traits),
                    label: label.map(|s| s.to_string()),
                    value: value.map(|s| s.to_string()),
                    identifier: ident.map(|s| s.to_string()).filter(|s| !s.is_empty()),
                }
            }
        }

        fn replay(&mut self, h: &Handle, ops: &[DrawOp], _size: Size) {
            OPS.with(|m| m.borrow_mut().insert(ptr_of(h), ops.to_vec()));
            unsafe { h.setNeedsDisplay() };
        }

        fn snapshot_window(&mut self) -> Result<Vec<u8>, String> {
            Err("use `simctl io booted screenshot` (device-level capture) on ios-uikit".into())
        }

        fn present(&mut self, req: u64, spec: &day_spec::present::PresentSpec) {
            use day_spec::present::{ButtonRole, PresentResult, PresentSpec};
            use objc2_ui_kit::{
                UIAlertAction, UIAlertActionStyle, UIAlertController, UIAlertControllerStyle,
            };
            let m = mtm();
            let (title, message) = (
                NSString::from_str(spec.title()),
                spec.message().map(NSString::from_str),
            );
            match spec {
                PresentSpec::Dialog { buttons, sheet, .. } => {
                    let style = if *sheet {
                        UIAlertControllerStyle::ActionSheet
                    } else {
                        UIAlertControllerStyle::Alert
                    };
                    let ac = unsafe {
                        UIAlertController::alertControllerWithTitle_message_preferredStyle(
                            Some(&title),
                            message.as_deref(),
                            style,
                            m,
                        )
                    };
                    for (i, b) in buttons.iter().enumerate() {
                        let astyle = match b.role {
                            ButtonRole::Cancel => UIAlertActionStyle::Cancel,
                            ButtonRole::Destructive => UIAlertActionStyle::Destructive,
                            ButtonRole::Default => UIAlertActionStyle::Default,
                        };
                        let idx = i as i64;
                        let handler = block2::RcBlock::new(move |_: NonNull<UIAlertAction>| {
                            emit(
                                WINDOW_NODE,
                                Event::PresentResult {
                                    req,
                                    result: PresentResult::Button(idx),
                                },
                            );
                            present_forget(req);
                        });
                        let action = unsafe {
                            UIAlertAction::actionWithTitle_style_handler(
                                Some(&NSString::from_str(&b.label)),
                                astyle,
                                Some(&handler),
                                m,
                            )
                        };
                        unsafe { ac.addAction(&action) };
                    }
                    PRESENT_VCS.with(|p| p.borrow_mut().insert(req, ac.clone()));
                    modal_enqueue(ModalOp::Present(req, ac.into_super()));
                }
                PresentSpec::Prompt {
                    placeholder,
                    initial,
                    ok,
                    cancel,
                    ..
                } => {
                    let ac = unsafe {
                        UIAlertController::alertControllerWithTitle_message_preferredStyle(
                            Some(&title),
                            message.as_deref(),
                            UIAlertControllerStyle::Alert,
                            m,
                        )
                    };
                    let (ph, init) = (NSString::from_str(placeholder), NSString::from_str(initial));
                    let cfg =
                        block2::RcBlock::new(move |tf: NonNull<objc2_ui_kit::UITextField>| {
                            let tf = unsafe { tf.as_ref() };
                            unsafe {
                                tf.setPlaceholder(Some(&ph));
                                tf.setText(Some(&init));
                            }
                        });
                    unsafe { ac.addTextFieldWithConfigurationHandler(Some(&cfg)) };
                    let ac_ok = ac.clone();
                    let ok_handler = block2::RcBlock::new(move |_: NonNull<UIAlertAction>| {
                        let text = unsafe { ac_ok.textFields() }
                            .and_then(|fs| fs.firstObject())
                            .and_then(|f| unsafe { f.text() })
                            .map(|s| s.to_string())
                            .unwrap_or_default();
                        emit(
                            WINDOW_NODE,
                            Event::PresentResult {
                                req,
                                result: PresentResult::Text(text),
                            },
                        );
                        present_forget(req);
                    });
                    let cancel_handler = block2::RcBlock::new(move |_: NonNull<UIAlertAction>| {
                        emit(
                            WINDOW_NODE,
                            Event::PresentResult {
                                req,
                                result: PresentResult::Dismissed,
                            },
                        );
                        present_forget(req);
                    });
                    unsafe {
                        ac.addAction(&UIAlertAction::actionWithTitle_style_handler(
                            Some(&NSString::from_str(ok)),
                            UIAlertActionStyle::Default,
                            Some(&ok_handler),
                            m,
                        ));
                        ac.addAction(&UIAlertAction::actionWithTitle_style_handler(
                            Some(&NSString::from_str(cancel)),
                            UIAlertActionStyle::Cancel,
                            Some(&cancel_handler),
                            m,
                        ));
                    }
                    PRESENT_VCS.with(|p| p.borrow_mut().insert(req, ac.clone()));
                    modal_enqueue(ModalOp::Present(req, ac.into_super()));
                }
                // Native file pickers: UIDocumentPickerViewController with a delegate. Open uses
                // `.import` mode (the system hands back an app-local copy, readable via std::fs);
                // save exports the Day-staged temp file to the chosen destination.
                PresentSpec::OpenFile { .. } => {
                    if dayscript_driven() {
                        return; // pending request resolved by the scripted `respond`
                    }
                    let types =
                        objc2_foundation::NSArray::from_retained_slice(&[NSString::from_str(
                            "public.item",
                        )]);
                    #[allow(deprecated)]
                    let picker = unsafe {
                        UIDocumentPickerViewController::initWithDocumentTypes_inMode(
                            UIDocumentPickerViewController::alloc(m),
                            &types,
                            UIDocumentPickerMode::Import,
                        )
                    };
                    present_doc_picker(req, m, picker);
                }
                PresentSpec::SaveFile { src_path, .. } => {
                    if dayscript_driven() {
                        return; // pending request resolved by the scripted `respond`
                    }
                    let url = unsafe {
                        objc2_foundation::NSURL::fileURLWithPath(&NSString::from_str(src_path))
                    };
                    #[allow(deprecated)]
                    let picker = unsafe {
                        UIDocumentPickerViewController::initWithURL_inMode(
                            UIDocumentPickerViewController::alloc(m),
                            &url,
                            UIDocumentPickerMode::ExportToService,
                        )
                    };
                    present_doc_picker(req, m, picker);
                }
            }
        }

        fn dismiss(&mut self, req: u64) {
            modal_enqueue(ModalOp::Dismiss(req, 0));
        }

        fn open_url(&mut self, url: &str) {
            let Some(nsurl) =
                (unsafe { objc2_foundation::NSURL::URLWithString(&NSString::from_str(url)) })
            else {
                return;
            };
            // `openURL:` is deprecated in favour of the options/completion form, which only adds an
            // options-key type and a result block a fire-and-forget link ignores. This still hands
            // the URL to the system (Safari for http(s), Mail for mailto:, …).
            #[allow(deprecated)]
            unsafe {
                let _: bool = UIApplication::sharedApplication(mtm()).openURL(&nsurl);
            }
        }

        fn defer_system_gestures(&mut self, edges: Edges) {
            DEFER_EDGES.with(|e| e.set(edges.0));
            // Re-query the override on the root VC and every cover VC (UIKit consults the
            // topmost presented VC, which is the cover while one is up).
            let root_vc = WINDOW
                .with(|w| w.borrow().clone())
                .and_then(|w| w.rootViewController());
            if let Some(vc) = root_vc {
                unsafe { vc.setNeedsUpdateOfScreenEdgesDeferringSystemGestures() };
            }
            let covers: Vec<Retained<DayCoverVC>> =
                COVER_STATE.with(|m| m.borrow().values().map(|s| s.vc.clone()).collect());
            for vc in covers {
                unsafe { vc.setNeedsUpdateOfScreenEdgesDeferringSystemGestures() };
            }
        }

        fn dark_mode(&mut self) -> bool {
            // A DAY_THEME launch override wins (themed capture runs); else the current
            // trait collection's interface style.
            match std::env::var("DAY_THEME").ok().as_deref() {
                Some("dark") => return true,
                Some("light") => return false,
                _ => {}
            }
            let style = unsafe {
                objc2_ui_kit::UITraitCollection::currentTraitCollection().userInterfaceStyle()
            };
            style == objc2_ui_kit::UIUserInterfaceStyle::Dark
        }

        fn ui_idle(&mut self) -> bool {
            let active = MODAL_BUSY.get()
                || MODAL_QUEUE.with(|q| !q.borrow().is_empty())
                || topmost_vc().is_some_and(|top| top.transitionCoordinator().is_some());
            if active {
                UI_LAST_ACTIVE.with(|t| t.set(Some(std::time::Instant::now())));
                return false;
            }
            // One settle margin past the last observed transition: the coordinator clears a
            // frame before the final composite, and a capture in that gap still shows a
            // sliver of the outgoing page.
            UI_LAST_ACTIVE
                .with(|t| t.get())
                .is_none_or(|t| t.elapsed() > std::time::Duration::from_millis(250))
        }
    }

    /// One queued modal transition. UIKit view-controller presentation is transactional: a
    /// present or dismiss issued while another transition is in flight is silently dropped (or
    /// lands stacked on a half-presented alert, where a later `dismiss` hits the child instead
    /// of the alert) — exactly how scripted respond → present bursts left dialogs stuck on
    /// screen in CI. Every present/dismiss therefore goes through a FIFO pumped from each
    /// transition's completion block, so transitions never overlap.
    enum ModalOp {
        Present(u64, Retained<UIViewController>),
        /// Dismiss request + how many 50ms defer-retries it has already made.
        Dismiss(u64, u32),
        /// A deferred UI mutation (nav push/pop) that must not overlap a modal transition.
        Run(Box<dyn FnOnce()>),
    }

    /// Whether a dayscript engine is driving this app (docs/testing): scripted sessions
    /// answer file pickers programmatically via `respond`, so the NATIVE picker UI is never
    /// touched — and the document picker is a REMOTE view controller whose hosted view can
    /// survive programmatic dismissal on the simulator, photobombing every later screenshot.
    /// Skip presenting it; the pending request still resolves through the normal channel.
    /// Alerts / prompts / sheets are in-process and still present natively.
    fn dayscript_driven() -> bool {
        std::env::var_os("DAYSCRIPT_PORT").is_some()
    }

    fn modal_enqueue(op: ModalOp) {
        MODAL_QUEUE.with(|q| q.borrow_mut().push_back(op));
        modal_pump();
    }

    /// Mark a transition in flight and arm a watchdog: if UIKit ever drops a transition's
    /// completion (observed with remote view controllers under scripted bursts), the queue
    /// would jam forever behind the stuck busy flag — after 2s the watchdog clears it and
    /// pumps, so one lost completion can't freeze every later dialog and deferred nav op.
    fn modal_begin_transition() {
        MODAL_BUSY.set(true);
        let generation = MODAL_GEN.get().wrapping_add(1);
        MODAL_GEN.set(generation);
        let when = dispatch2::DispatchTime::try_from(std::time::Duration::from_secs(4))
            .unwrap_or(dispatch2::DispatchTime::NOW);
        let _ = dispatch2::DispatchQueue::main().after(when, move || {
            if MODAL_BUSY.get() && MODAL_GEN.get() == generation {
                eprintln!("day: modal transition completion lost — unjamming the queue");
                MODAL_BUSY.set(false);
                modal_pump();
            }
        });
    }

    /// Normal end of a transition: clear busy, invalidate the watchdog, run the next op.
    fn modal_end_transition() {
        MODAL_GEN.set(MODAL_GEN.get().wrapping_add(1));
        MODAL_BUSY.set(false);
        modal_pump();
    }

    /// Put `op` back at the queue's head and retry shortly: some other UIKit transition (a
    /// nav push/pop) is animating, and modal work issued across it is silently dropped.
    fn modal_defer_retry(op: ModalOp) {
        MODAL_QUEUE.with(|q| q.borrow_mut().push_front(op));
        MODAL_BUSY.set(true); // hold the queue while we wait
        MODAL_GEN.set(MODAL_GEN.get().wrapping_add(1));
        let when = dispatch2::DispatchTime::try_from(std::time::Duration::from_millis(50))
            .unwrap_or(dispatch2::DispatchTime::NOW);
        let _ = dispatch2::DispatchQueue::main().after(when, || {
            MODAL_BUSY.set(false);
            modal_pump();
        });
    }

    /// Run `f` now if no modal transition is in flight or queued, else queue it behind them.
    fn modal_after_idle(f: impl FnOnce() + 'static) {
        let idle = !MODAL_BUSY.get() && MODAL_QUEUE.with(|q| q.borrow().is_empty());
        if idle {
            f();
        } else {
            modal_enqueue(ModalOp::Run(Box::new(f)));
        }
    }

    /// Run the next queued modal op if no transition is in flight. Each op's completion clears
    /// the busy flag and pumps again.
    fn modal_pump() {
        if MODAL_BUSY.get() {
            return;
        }
        let Some(op) = MODAL_QUEUE.with(|q| q.borrow_mut().pop_front()) else {
            return;
        };
        match op {
            ModalOp::Present(req, vc) => {
                // Presenting while ANOTHER transition animates (a nav push the script just
                // triggered, an appearance change) is refused by UIKit without ever calling
                // the completion — the original stuck-dialog bug. Wait it out.
                if topmost_vc().is_some_and(|top| top.transitionCoordinator().is_some()) {
                    modal_defer_retry(ModalOp::Present(req, vc));
                    return;
                }
                let Some(top) = topmost_vc() else {
                    // No window to present on: resolve as dismissed so the app future settles.
                    present_forget(req);
                    emit(
                        WINDOW_NODE,
                        Event::PresentResult {
                            req,
                            result: day_spec::present::PresentResult::Dismissed,
                        },
                    );
                    modal_pump();
                    return;
                };
                modal_begin_transition();
                let completion = block2::RcBlock::new(modal_end_transition);
                unsafe {
                    top.presentViewController_animated_completion(&vc, true, Some(&completion))
                };
            }
            ModalOp::Dismiss(req, tries) => {
                // If this request's Present is still queued it never reached the screen — drop
                // it (the result was already resolved; there is nothing to dismiss).
                let dropped_queued = MODAL_QUEUE.with(|q| {
                    let mut q = q.borrow_mut();
                    let before = q.len();
                    q.retain(|op| !matches!(op, ModalOp::Present(r, _) if *r == req));
                    before != q.len()
                });
                if dropped_queued {
                    present_forget(req);
                    modal_pump();
                    return;
                }
                let vc: Option<Retained<UIViewController>> = PRESENT_VCS
                    .with(|p| p.borrow().get(&req).map(|ac| ac.clone().into_super()))
                    .or_else(|| {
                        PRESENT_PICKERS.with(|p| {
                            p.borrow()
                                .get(&req)
                                .map(|(picker, _)| picker.clone().into_super())
                        })
                    });
                let Some(vc) = vc else {
                    // Already gone (the user answered natively, or a stale request).
                    present_forget(req);
                    modal_pump();
                    return;
                };
                // Not attached yet (its presentation transition is still in flight — e.g. the
                // watchdog unjammed the queue mid-present) or some other transition is still
                // animating: retry shortly, bounded. Skipping here would strand the dialog on
                // screen (the original CI bug); the bound keeps a never-presented controller
                // from wedging the queue forever.
                let attached = vc.presentingViewController().is_some();
                let animating = vc
                    .presentingViewController()
                    .is_some_and(|p| p.transitionCoordinator().is_some());
                if !attached || animating {
                    if tries < 100 {
                        modal_defer_retry(ModalOp::Dismiss(req, tries + 1));
                    } else {
                        present_forget(req);
                        modal_pump();
                    }
                    return;
                }
                present_forget(req);
                // Dismiss from the PRESENTING side: `dismiss` on the controller itself would
                // target any child IT presents (remote document pickers host internal view
                // controllers), reporting completion while the picker stays on screen. The
                // presenter tears down its whole presented stack. Animated: an UNANIMATED
                // dismissal of a remote view controller reports completion while the remote
                // layer stays visible on the simulator — the animated handshake is the path
                // that actually removes it (the queue serializes transitions either way).
                let presenting = vc
                    .presentingViewController()
                    .expect("attached checked above");
                modal_begin_transition();
                let completion = block2::RcBlock::new(modal_end_transition);
                unsafe {
                    presenting.dismissViewControllerAnimated_completion(true, Some(&completion))
                };
            }
            ModalOp::Run(f) => {
                f();
                modal_pump();
            }
        }
    }

    /// Drop the retained controller for `req` — on programmatic dismissal, or from the action
    /// handlers when the user answered natively (UIKit dismisses the alert itself on a tap).
    fn present_forget(req: u64) {
        PRESENT_VCS.with(|p| {
            p.borrow_mut().remove(&req);
        });
        PRESENT_PICKERS.with(|p| {
            p.borrow_mut().remove(&req);
        });
    }

    /// Wire a document picker's delegate, retain both, and queue its presentation.
    fn present_doc_picker(
        req: u64,
        m: MainThreadMarker,
        picker: Retained<UIDocumentPickerViewController>,
    ) {
        unsafe { picker.setAllowsMultipleSelection(false) };
        let delegate = DayDocPicker::new(m, req);
        unsafe { picker.setDelegate(Some(ProtocolObject::from_ref(&*delegate))) };
        PRESENT_PICKERS.with(|p| p.borrow_mut().insert(req, (picker.clone(), delegate)));
        modal_enqueue(ModalOp::Present(req, picker.into_super()));
    }

    thread_local! {
        /// Live alert controllers keyed by request id (for programmatic dismissal).
        static PRESENT_VCS: RefCell<HashMap<u64, Retained<objc2_ui_kit::UIAlertController>>> =
            RefCell::new(HashMap::new());
        /// Live document pickers + their retained delegates, keyed by request id.
        #[allow(clippy::type_complexity)]
        static PRESENT_PICKERS: RefCell<
            HashMap<
                u64,
                (
                    Retained<UIDocumentPickerViewController>,
                    Retained<DayDocPicker>,
                ),
            >,
        > = RefCell::new(HashMap::new());
        /// FIFO of modal transitions (see [`ModalOp`]) — ops run one at a time, pumped from
        /// each transition's completion.
        static MODAL_QUEUE: RefCell<std::collections::VecDeque<ModalOp>> =
            const { RefCell::new(std::collections::VecDeque::new()) };
        /// Whether a present/dismiss transition is currently in flight.
        static MODAL_BUSY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
        /// Transition generation — invalidates the watchdog of a normally-completed transition.
        static MODAL_GEN: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
        /// When a transition was last seen in flight (`ui_idle`'s settle margin).
        static UI_LAST_ACTIVE: std::cell::Cell<Option<std::time::Instant>> =
            const { std::cell::Cell::new(None) };
    }

    /// The frontmost view controller (walk past any already-presented modal, but stop short of
    /// one that is mid-dismissal — presenting on it would be dropped by UIKit).
    fn topmost_vc() -> Option<Retained<UIViewController>> {
        let mut vc = WINDOW.with(|w| w.borrow().clone())?.rootViewController()?;
        while let Some(p) = vc.presentedViewController() {
            if p.isBeingDismissed() {
                break;
            }
            vc = p;
        }
        Some(vc)
    }

    struct DocPickerIvars {
        req: u64,
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "DayUIKitDocPicker"]
        #[ivars = DocPickerIvars]
        struct DayDocPicker;

        unsafe impl NSObjectProtocol for DayDocPicker {}

        unsafe impl UIDocumentPickerDelegate for DayDocPicker {
            #[unsafe(method(documentPicker:didPickDocumentsAtURLs:))]
            fn did_pick(
                &self,
                _picker: &UIDocumentPickerViewController,
                urls: &objc2_foundation::NSArray<objc2_foundation::NSURL>,
            ) {
                let req = self.ivars().req;
                let mut paths = Vec::new();
                for i in 0..urls.count() {
                    let url = urls.objectAtIndex(i);
                    if let Some(p) = unsafe { url.path() } {
                        paths.push(p.to_string());
                    }
                }
                let result = if paths.is_empty() {
                    day_spec::present::PresentResult::Dismissed
                } else {
                    day_spec::present::PresentResult::Files(paths)
                };
                emit(WINDOW_NODE, Event::PresentResult { req, result });
                PRESENT_PICKERS.with(|m| {
                    m.borrow_mut().remove(&req);
                });
                present_forget(req);
            }

            #[unsafe(method(documentPickerWasCancelled:))]
            fn was_cancelled(&self, _picker: &UIDocumentPickerViewController) {
                let req = self.ivars().req;
                emit(
                    WINDOW_NODE,
                    Event::PresentResult {
                        req,
                        result: day_spec::present::PresentResult::Dismissed,
                    },
                );
                PRESENT_PICKERS.with(|m| {
                    m.borrow_mut().remove(&req);
                });
                present_forget(req);
            }
        }
    );

    impl DayDocPicker {
        fn new(mtm: MainThreadMarker, req: u64) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(DocPickerIvars { req });
            unsafe { msg_send![super(this), init] }
        }
    }

    // -----------------------------------------------------------------------
    // App delegate + Platform (UIApplicationMain)
    // -----------------------------------------------------------------------

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "DayAppDelegate"]
        struct AppDelegate;

        unsafe impl NSObjectProtocol for AppDelegate {}

        unsafe impl UIApplicationDelegate for AppDelegate {
            // The no-scene-manifest compat path reads `delegate.window` (pane's hard-won lesson).
            #[unsafe(method(window))]
            fn window(&self) -> *mut UIWindow {
                WINDOW.with(|w| {
                    w.borrow()
                        .as_ref()
                        .map(|r| &**r as *const UIWindow as *mut UIWindow)
                        .unwrap_or(std::ptr::null_mut())
                })
            }
            #[unsafe(method(setWindow:))]
            fn set_window(&self, window: *mut UIWindow) {
                let retained = unsafe { window.as_ref() }.map(Retained::from);
                WINDOW.with(|w| *w.borrow_mut() = retained);
            }

            // Classic (pre-UIScene) window setup: fine for Day's single-window shell.
            #[allow(deprecated)]
            #[unsafe(method(application:didFinishLaunchingWithOptions:))]
            fn did_finish_launching(&self, _app: &UIApplication, _opts: *mut AnyObject) -> bool {
                let mtm = MainThreadMarker::new().unwrap();
                let bounds = UIScreen::mainScreen(mtm).bounds();
                let window = unsafe { UIWindow::initWithFrame(UIWindow::alloc(mtm), bounds) };
                // A DayRootVC (not a plain UIViewController) so `defers_system_gestures`
                // reaches the window root's screen-edge override (docs/cover.md).
                let vc: Retained<UIViewController> = DayRootVC::new(mtm).into_super();
                let holder = unsafe { UIView::initWithFrame(UIView::alloc(mtm), bounds) };
                let root_view = unsafe { UIView::initWithFrame(UIView::alloc(mtm), bounds) };
                // RTL locales (docs/localization): force the semantic content attribute on
                // the window AND the day content roots — descendants left at `.unspecified`
                // resolve their effective direction through the hierarchy, so native controls
                // (slider fill), the nav bar (back chevron side), and system transitions
                // mirror; Day's own frames mirror in the layout engine.
                if day_core::layout_direction() == day_spec::LayoutDirection::Rtl {
                    let rtl = objc2_ui_kit::UISemanticContentAttribute::ForceRightToLeft;
                    window.setSemanticContentAttribute(rtl);
                    holder.setSemanticContentAttribute(rtl);
                    root_view.setSemanticContentAttribute(rtl);
                }
                // DAY_THEME=light|dark forces the interface style window-wide (themed CI
                // screenshot runs; `day launch --env` reaches the sim app's environment);
                // unset ⇒ follow the system.
                if let Ok(theme) = std::env::var("DAY_THEME") {
                    let style = match theme.as_str() {
                        "dark" => Some(objc2_ui_kit::UIUserInterfaceStyle::Dark),
                        "light" => Some(objc2_ui_kit::UIUserInterfaceStyle::Light),
                        _ => None,
                    };
                    if let Some(style) = style {
                        unsafe { window.setOverrideUserInterfaceStyle(style) };
                    }
                }
                unsafe {
                    holder.setBackgroundColor(Some(&UIColor::systemBackgroundColor()));
                    holder.addSubview(&root_view);
                    vc.setView(Some(&holder));
                    window.setRootViewController(Some(&vc));
                    window.makeKeyAndVisible();
                }
                WINDOW.with(|w| *w.borrow_mut() = Some(window.clone()));

                // Safe area as root padding (§7.7 MVP): valid once the window is key.
                let insets = unsafe { window.safeAreaInsets() };
                let inner = CGRect::new(
                    CGPoint::new(insets.left, insets.top),
                    CGSize::new(
                        bounds.size.width - insets.left - insets.right,
                        bounds.size.height - insets.top - insets.bottom,
                    ),
                );
                unsafe { root_view.setFrame(inner) };
                ROOT_VIEW.with(|r| *r.borrow_mut() = Some(root_view.clone()));
                ROOT_BASE_FRAME.with(|f| f.set(inner));
                // Keyboard avoidance (docs/focus.md): shrink the root to the keyboard's top and
                // let the WindowResized rail relayout Day — the same shape as Android's
                // IME-inset margins. WillChangeFrame covers show, hide, and height changes.
                unsafe {
                    objc2_foundation::NSNotificationCenter::defaultCenter()
                        .addObserver_selector_name_object(
                            self,
                            sel!(keyboardWillChange:),
                            Some(objc2_ui_kit::UIKeyboardWillChangeFrameNotification),
                            None,
                        )
                };

                let (backend, _options, ready) = PENDING
                    .with(|p| p.borrow_mut().take())
                    .expect("day-uikit: run() not called");
                let size = Size::new(inner.size.width, inner.size.height);
                ready(backend, view_of(root_view), size);
                true
            }

            // Custom-scheme deep link (docs/navigation.md): route = URL host + path,
            // delivered to the active nav host as Custom("deeplink").
            #[unsafe(method(application:openURL:options:))]
            fn open_url(
                &self,
                _app: &UIApplication,
                url: &objc2_foundation::NSURL,
                _options: *mut AnyObject,
            ) -> bool {
                let host = unsafe { url.host() }
                    .map(|s| s.to_string())
                    .unwrap_or_default();
                let path = unsafe { url.path() }
                    .map(|s| s.to_string())
                    .unwrap_or_default();
                let node = NAV_STATE.with(|m| m.borrow().values().next().map(|s| s.host_node));
                if let Some(node) = node {
                    emit(node, Event::custom("deeplink", format!("{host}{path}")));
                    true
                } else {
                    false
                }
            }

            // Lifecycle (docs/lifecycle.md): the full iOS app lifecycle, mapped 1:1 to day phases.
            #[unsafe(method(applicationDidBecomeActive:))]
            fn did_become_active(&self, _app: &UIApplication) {
                emit(
                    WINDOW_NODE,
                    Event::Lifecycle(day_spec::Lifecycle::DidBecomeActive),
                );
            }
            #[unsafe(method(applicationWillResignActive:))]
            fn will_resign_active(&self, _app: &UIApplication) {
                emit(
                    WINDOW_NODE,
                    Event::Lifecycle(day_spec::Lifecycle::WillResignActive),
                );
            }
            #[unsafe(method(applicationWillEnterForeground:))]
            fn will_enter_foreground(&self, _app: &UIApplication) {
                emit(
                    WINDOW_NODE,
                    Event::Lifecycle(day_spec::Lifecycle::WillEnterForeground),
                );
            }
            #[unsafe(method(applicationDidEnterBackground:))]
            fn did_enter_background(&self, _app: &UIApplication) {
                emit(
                    WINDOW_NODE,
                    Event::Lifecycle(day_spec::Lifecycle::DidEnterBackground),
                );
            }
            #[unsafe(method(applicationDidReceiveMemoryWarning:))]
            fn did_receive_memory_warning(&self, _app: &UIApplication) {
                emit(
                    WINDOW_NODE,
                    Event::Lifecycle(day_spec::Lifecycle::DidReceiveMemoryWarning),
                );
            }
            #[unsafe(method(applicationWillTerminate:))]
            fn will_terminate(&self, _app: &UIApplication) {
                emit(
                    WINDOW_NODE,
                    Event::Lifecycle(day_spec::Lifecycle::WillTerminate),
                );
            }
        }

        // Inherent (non-protocol) selectors: NSNotificationCenter targets land here — objc2
        // verifies protocol impl blocks against the protocol, and keyboardWillChange: is ours.
        impl AppDelegate {
            /// Keyboard show/hide/height change: clamp the root's bottom to the keyboard top
            /// (screen coords), tell Day the root resized, then reveal the focused field.
            #[unsafe(method(keyboardWillChange:))]
            fn keyboard_will_change(&self, notification: &objc2_foundation::NSNotification) {
                let Some(root) = ROOT_VIEW.with(|r| r.borrow().clone()) else {
                    return;
                };
                let Some(info) = (unsafe { notification.userInfo() }) else {
                    return;
                };
                let Some(val) = info
                    .objectForKey(unsafe { objc2_ui_kit::UIKeyboardFrameEndUserInfoKey })
                    .and_then(|o| o.downcast::<objc2_foundation::NSValue>().ok())
                else {
                    return;
                };
                use objc2_ui_kit::NSValueUIGeometryExtensions;
                let kb = unsafe { val.CGRectValue() };
                let base = ROOT_BASE_FRAME.with(|f| f.get());
                // The holder fills the window, so the root's frame is in window == screen
                // coordinates; a hidden keyboard reports an off-screen frame (top >= bottom).
                let base_bottom = base.origin.y + base.size.height;
                let new_h = if kb.origin.y < base_bottom {
                    (kb.origin.y - base.origin.y).max(0.0)
                } else {
                    base.size.height
                };
                let f = CGRect::new(base.origin, CGSize::new(base.size.width, new_h));
                if unsafe { root.frame() }.size.height != new_h {
                    unsafe { root.setFrame(f) };
                    emit(
                        WINDOW_NODE,
                        Event::WindowResized(Size::new(f.size.width, f.size.height)),
                    );
                }
                if new_h < base.size.height {
                    reveal_focused_field();
                }
            }
        }
    );

    /// Mobile backends deliver the FULL lifecycle (docs/lifecycle.md), including the background,
    /// foreground, and memory-warning phases desktops lack. `const` for `day::require_lifecycle!`.
    pub const fn lifecycle_supported(_phase: day_spec::Lifecycle) -> bool {
        true
    }

    /// Register bundled font files (§18.4) with CoreText so `Font::Custom` families resolve via
    /// `UIFont(name:)`. The files ride the DayPieces SwiftPM bundle (`fonts/` copied by `day
    /// build`, which also lists them in the app's `UIAppFonts` — this call covers dev builds and
    /// doubles as the loud failure path). Duplicate registration (UIAppFonts already loaded the
    /// file) fails harmlessly, so failures here are only logged when the family is then missing.
    fn register_bundled_fonts() {
        // CFURLRef is toll-free bridged with NSURL.
        #[link(name = "CoreText", kind = "framework")]
        unsafe extern "C" {
            fn CTFontManagerRegisterFontsForURL(
                font_url: *const std::ffi::c_void,
                scope: u32, // kCTFontManagerScopeProcess = 1
                error: *mut *const std::ffi::c_void,
            ) -> bool;
        }
        let mut dirs: Vec<std::path::PathBuf> = Vec::new();
        // The DayPieces bundle's fonts/ directory (SwiftPM `.copy` resource inside the app).
        let main = unsafe { objc2_foundation::NSBundle::mainBundle() };
        if let Some(res) = unsafe { main.resourcePath() } {
            dirs.push(
                std::path::PathBuf::from(res.to_string())
                    .join("DayPieces_DayPieces.bundle")
                    .join("fonts"),
            );
        }
        if let Some(dev) = day_spec::fonts::font_dir() {
            dirs.push(dev);
        }
        for dir in dirs {
            for path in day_spec::fonts::font_files_in(&dir) {
                let url = unsafe {
                    objc2_foundation::NSURL::fileURLWithPath(&NSString::from_str(
                        &path.to_string_lossy(),
                    ))
                };
                unsafe {
                    let _ = CTFontManagerRegisterFontsForURL(
                        Retained::as_ptr(&url) as *const std::ffi::c_void,
                        1,
                        std::ptr::null_mut(),
                    );
                }
            }
        }
    }

    impl Platform for Uikit {
        const TARGET: &'static str = "ios-uikit";
        const TOOLKIT: &'static str = "uikit";

        fn run(self, options: WindowOptions, ready: Box<dyn FnOnce(Self, Handle, Size)>) {
            // Bundled custom fonts (§18.4) must be registered before the first label realizes.
            register_bundled_fonts();
            PENDING.with(|p| *p.borrow_mut() = Some((self, options, ready)));
            // Force-register the delegate class: UIApplicationMain looks it up by name before
            // any Rust code touches it (pane's exact fix).
            let _ = <AppDelegate as objc2::ClassType>::class();
            let arg0 = c"Day".as_ptr() as *mut c_char;
            let mut argv = [arg0];
            let argv_ptr = NonNull::new(argv.as_mut_ptr()).unwrap();
            let delegate = NSString::from_str("DayAppDelegate");
            #[allow(deprecated)]
            unsafe {
                UIApplicationMain(1 as c_int, argv_ptr, None, Some(&delegate));
            }
        }

        fn post(f: Box<dyn FnOnce() + Send>) {
            dispatch2::DispatchQueue::main().exec_async(f);
        }

        /// Frame clock (§8.4): store the pending callback and un-pause the shared CADisplayLink,
        /// creating it (paused) on first use and attaching it to the main run loop in common modes
        /// so it keeps firing during scroll/tracking. `DayFrameTarget::step` delivers it.
        fn request_frame(cb: Box<dyn FnOnce(f64) + 'static>) {
            let mtm = mtm();
            FRAME.with(|f| {
                let mut f = f.borrow_mut();
                f.1 = Some(cb);
                if f.0.is_none() {
                    let target = DayFrameTarget::new(mtm);
                    let link = unsafe {
                        CADisplayLink::displayLinkWithTarget_selector(&target, sel!(step:))
                    };
                    unsafe {
                        let run_loop = objc2_foundation::NSRunLoop::mainRunLoop();
                        link.addToRunLoop_forMode(
                            &run_loop,
                            objc2_foundation::NSRunLoopCommonModes,
                        );
                    }
                    f.0 = Some(link);
                }
                if let Some(link) = f.0.as_ref() {
                    unsafe { link.setPaused(false) };
                }
            });
        }
    }
}
