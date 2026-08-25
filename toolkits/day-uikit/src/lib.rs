// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! day-uikit — the ios-uikit backend (DESIGN.md §9). objc2, pure Rust, no shim.
//!
//! `Handle = Retained<UIView>`; UIKit is top-left/y-down so Day frames apply directly. The app
//! boots via `UIApplicationMain` + a `define_class!` app delegate (pane's proven pattern: the
//! delegate class is force-registered before `UIApplicationMain`, and exposes `window`/
//! `setWindow:` for the no-scene-manifest compat path). iOS-only (`cfg(target_os = "ios")`);
//! host builds see an empty crate.

#![allow(unused_unsafe)]

// `setBadgeCount:` lives in UserNotifications; the class lookup needs the framework linked.
#[cfg(target_os = "ios")]
#[link(name = "UserNotifications", kind = "framework")]
unsafe extern "C" {}

#[cfg(target_os = "ios")]
pub use imp::*;

#[cfg(target_os = "ios")]
mod picker;
#[cfg(target_os = "ios")]
mod textarea;
/// Set a `UITextInputTraits` integer property on a `UITextView` (0 = on/default, 1 = off) —
/// dispatched through the raw runtime, since objc2's checked send does not see these dynamically
/// resolved setters. Public for standalone editor pieces (docs/extending.md).
#[cfg(target_os = "ios")]
pub use textarea::set_text_input_trait;

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
    use objc2::Message as _;
    use objc2_ui_kit::NSIndexPathUIKitAdditions as _;
    use objc2_ui_kit::NSObjectUIAccessibility;
    #[allow(deprecated)]
    use objc2_ui_kit::UIApplicationMain;
    use objc2_ui_kit::UINavigationControllerDelegate;
    use objc2_ui_kit::UISearchResultsUpdating;
    use objc2_ui_kit::UISplitViewControllerDelegate;
    use objc2_ui_kit::UITextViewDelegate;
    use objc2_ui_kit::{
        UIAction, UIContextMenuConfiguration, UIContextMenuInteraction,
        UIContextMenuInteractionDelegate, UIInteraction, UIMenu, UIMenuElement,
        UIMenuElementAttributes, UIMenuOptions,
    };
    use objc2_ui_kit::{
        UIActivityIndicatorView, UIApplication, UIApplicationDelegate, UIButton, UIButtonType,
        UIColor, UIControl, UIControlEvents, UIControlState, UIEdgeInsets, UILabel,
        UIModalPresentationStyle, UIProgressView, UIRectEdge, UIScrollView, UISlider, UISwitch,
        UITextBorderStyle, UITextField, UITextView, UIView, UIViewAnimationOptions,
        UIViewController, UIWindow,
    };
    use objc2_ui_kit::{
        UIBarButtonItem, UIBarButtonItemStyle, UIBarPositioningDelegate, UINavigationBar,
        UINavigationBarDelegate, UINavigationController, UINavigationItem,
    };
    use objc2_ui_kit::{
        UIGestureRecognizer, UIGestureRecognizerState, UIPanGestureRecognizer,
        UIPinchGestureRecognizer, UITapGestureRecognizer,
    };
    use objc2_ui_kit::{
        UIScrollViewDelegate, UITableViewDataSource, UITableViewDelegate, UITableViewDragDelegate,
    };
    use objc2_ui_kit::{UITabBarController, UITabBarControllerDelegate};
    // `.import`/`.exportToService` modes (deprecated in favor of `initFor…ContentTypes:`, which
    // would pull in the UniformTypeIdentifiers crate) remain the simplest UTType-free path.
    #[allow(deprecated)]
    use objc2_ui_kit::UIDocumentPickerMode;
    use objc2_ui_kit::{UIDocumentPickerDelegate, UIDocumentPickerViewController};

    use day_spec::props::*;
    use day_spec::{
        A11yProps, AnimSpec, Builtin, Cap, Curve, DrawOp, Edges, Event, EventSink, Font,
        ListSource, NodeId, PieceKind, Platform, Proposal, RawHandle, Rect, Registry, Renderer,
        Size, Support, Toolkit, Transform, WINDOW_NODE, WindowOptions, kinds,
    };

    pub type Handle = Retained<UIView>;

    /// The day-core event sink (node-id keyed).
    type Sink = Rc<dyn Fn(NodeId, Event)>;

    /// DAY_DIAG_NAV tracing, resolved once — the layout and nav-delegate hot paths run on
    /// every pass and must not re-query the environment each time.
    static DIAG_NAV: std::sync::LazyLock<bool> =
        std::sync::LazyLock::new(|| std::env::var("DAY_DIAG_NAV").is_ok());

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
        /// Connected scenes' windowing state (docs/windows.md). The PRIMARY scene also
        /// mirrors into WINDOW/ROOT_VIEW/ROOT_BASE_FRAME above (every single-window code
        /// path keeps reading those); secondary day windows are registry-only.
        static SCENES: RefCell<Vec<SceneEntry>> = const { RefCell::new(Vec::new()) };
        /// Secondary opens in flight: the day root node ids handed to
        /// `requestSceneSessionActivation`, awaiting their scene's willConnect.
        static PENDING_WINDOWS: RefCell<Vec<(NodeId, String)>> = const { RefCell::new(Vec::new()) };
        /// App-level lifecycle debounce across scenes: whether any scene was
        /// foreground-active / any scene was foregrounded at the last recompute.
        static ANY_SCENE_ACTIVE: Cell<bool> = const { Cell::new(false) };
        static ANY_SCENE_FOREGROUND: Cell<bool> = const { Cell::new(false) };
    }

    /// One connected scene's windowing state.
    struct SceneEntry {
        window: Retained<UIWindow>,
        root_view: Retained<UIView>,
        base_frame: Cell<CGRect>,
        /// `None` = the primary scene; `Some` = a secondary day window's root node.
        node: Option<NodeId>,
    }

    /// The KEY window's scene entry applied to `f` — keyboard avoidance and modal
    /// presentation act on whichever Day window is key; primary statics are the fallback.
    fn with_key_scene<R>(f: impl FnOnce(&SceneEntry) -> R) -> Option<R> {
        SCENES.with(|s| {
            let scenes = s.borrow();
            let key = scenes
                .iter()
                .find(|e| e.window.isKeyWindow())
                .or_else(|| scenes.iter().find(|e| e.node.is_none()));
            key.map(f)
        })
    }

    /// The root node id the keyboard/resize rail should report against for the key window:
    /// a secondary's own root, or `WINDOW_NODE` for the primary.
    fn key_scene_target(entry: &SceneEntry) -> NodeId {
        entry.node.unwrap_or(WINDOW_NODE)
    }

    /// Build one Day window into `scene`: UIWindow + DayRootVC + DayHolderView + the
    /// safe-area-inset day root — the construction `didFinishLaunching` used to own,
    /// factored so every scene (primary and secondary) gets identical chrome.
    fn build_scene_window(
        mtm: MainThreadMarker,
        scene: &objc2_ui_kit::UIWindowScene,
    ) -> (Retained<UIWindow>, Retained<UIView>, CGRect) {
        let bounds = scene.screen().bounds();
        let window = unsafe { UIWindow::initWithWindowScene(UIWindow::alloc(mtm), scene) };
        let vc: Retained<UIViewController> = DayRootVC::new(mtm).into_super();
        let holder = DayHolderView::new(mtm);
        unsafe { holder.setFrame(bounds) };
        let root_view = unsafe { UIView::initWithFrame(UIView::alloc(mtm), bounds) };
        // RTL locales (docs/localization): force the semantic content attribute on the
        // window AND the day content roots — see the module docs.
        if day_core::layout_direction() == day_spec::LayoutDirection::Rtl {
            let rtl = objc2_ui_kit::UISemanticContentAttribute::ForceRightToLeft;
            window.setSemanticContentAttribute(rtl);
            holder.setSemanticContentAttribute(rtl);
            root_view.setSemanticContentAttribute(rtl);
        }
        // DAY_THEME=light|dark forces the interface style window-wide (themed CI runs).
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
        // Safe area as root padding (§7.7): valid once the window is key.
        let insets = unsafe { window.safeAreaInsets() };
        let inner = CGRect::new(
            CGPoint::new(insets.left, insets.top),
            CGSize::new(
                bounds.size.width - insets.left - insets.right,
                bounds.size.height - insets.top - insets.bottom,
            ),
        );
        unsafe { root_view.setFrame(inner) };
        (window, root_view, inner)
    }

    /// Recompute the app-level lifecycle from ALL scenes (docs/windows.md): scene phases
    /// replace the app-delegate callbacks under the scene lifecycle, and focus moving
    /// between two Day windows must not read as an app-level resign/become (the same
    /// debounce day-gtk applies). Emits only on a real transition.
    fn note_scene_lifecycle_changed(mtm: MainThreadMarker) {
        use objc2_ui_kit::UISceneActivationState as S;
        let app = UIApplication::sharedApplication(mtm);
        let mut any_active = false;
        let mut any_foreground = false;
        for scene in unsafe { app.connectedScenes() } {
            match unsafe { scene.activationState() } {
                S::ForegroundActive => {
                    any_active = true;
                    any_foreground = true;
                }
                S::ForegroundInactive => any_foreground = true,
                _ => {}
            }
        }
        if ANY_SCENE_FOREGROUND.with(|c| c.replace(any_foreground)) != any_foreground {
            let phase = if any_foreground {
                day_spec::Lifecycle::WillEnterForeground
            } else {
                day_spec::Lifecycle::DidEnterBackground
            };
            emit(WINDOW_NODE, Event::Lifecycle(phase));
        }
        if ANY_SCENE_ACTIVE.with(|c| c.replace(any_active)) != any_active {
            let phase = if any_active {
                day_spec::Lifecycle::DidBecomeActive
            } else {
                day_spec::Lifecycle::WillResignActive
            };
            emit(WINDOW_NODE, Event::Lifecycle(phase));
        }
    }

    /// The activity type a secondary-window scene request carries; its userInfo holds the
    /// day root node id under `day.node` (docs/windows.md).
    const DAY_WINDOW_ACTIVITY: &str = "dev.daybrite.day.window";

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

    /// The day name for an arrow key's HID usage, or `None` for every other key — the four
    /// [`day_spec::KeyEvent`] names the key route carries (docs/menus.md).
    fn arrow_key_name(code: objc2_ui_kit::UIKeyboardHIDUsage) -> Option<&'static str> {
        use objc2_ui_kit::UIKeyboardHIDUsage as U;
        match code {
            U::KeyboardLeftArrow => Some("ArrowLeft"),
            U::KeyboardRightArrow => Some("ArrowRight"),
            U::KeyboardUpArrow => Some("ArrowUp"),
            U::KeyboardDownArrow => Some("ArrowDown"),
            U::KeyboardDeleteForward => Some("Delete"),
            U::KeyboardDeleteOrBackspace => Some("Backspace"),
            _ => None,
        }
    }

    /// UIKit's modifier flags as day's mask.
    fn key_modifiers(f: objc2_ui_kit::UIKeyModifierFlags) -> u8 {
        let mut m = 0u8;
        if f.contains(objc2_ui_kit::UIKeyModifierFlags::Shift) {
            m |= day_spec::KeyEvent::SHIFT;
        }
        if f.contains(objc2_ui_kit::UIKeyModifierFlags::Command) {
            m |= day_spec::KeyEvent::PRIMARY;
        }
        if f.contains(objc2_ui_kit::UIKeyModifierFlags::Alternate) {
            m |= day_spec::KeyEvent::ALT;
        }
        m
    }
    /// Apply row-level deltas as animated table updates. Indexes are sequential (each
    /// delta describes the set as the previous ones left it), so each gets its own batch —
    /// UITableView's combined-batch index rules would re-interpret them.
    fn apply_row_deltas(table: &objc2_ui_kit::UITableView, deltas: &[day_spec::props::RowDelta]) {
        let path =
            |row: usize| objc2_foundation::NSIndexPath::indexPathForRow_inSection(row as isize, 0);
        for d in deltas {
            unsafe {
                table.beginUpdates();
                match d {
                    day_spec::props::RowDelta::Insert(i) => table
                        .insertRowsAtIndexPaths_withRowAnimation(
                            &objc2_foundation::NSArray::from_retained_slice(&[path(*i)]),
                            objc2_ui_kit::UITableViewRowAnimation::Automatic,
                        ),
                    day_spec::props::RowDelta::Remove(i) => table
                        .deleteRowsAtIndexPaths_withRowAnimation(
                            &objc2_foundation::NSArray::from_retained_slice(&[path(*i)]),
                            objc2_ui_kit::UITableViewRowAnimation::Automatic,
                        ),
                    day_spec::props::RowDelta::Move(from, to) => {
                        table.moveRowAtIndexPath_toIndexPath(&path(*from), &path(*to))
                    }
                }
                table.endUpdates();
            }
        }
    }

    fn view_of<T: AsRef<UIView>>(x: Retained<T>) -> Handle {
        Retained::from(x.as_ref())
    }

    /// The OUTERMOST view controller behind a host handle, or `None` for an ordinary view.
    ///
    /// Outermost matters: an adaptive nav host's handle is the split controller's view, and it is
    /// the split controller — not the secondary column's navigation controller inside it — that
    /// owns that view and must carry the containment. Used by `insert` to re-parent a host that
    /// lands inside a page (docs/navigation.md).
    /// The controller `view` belongs to, resolved the way UIKit resolves it.
    ///
    /// The registered-page lookup above matches only a page's OWN content view, and a nested host
    /// rarely lands there: it arrives inside whatever the page put between them — a `when` arm, a
    /// column, a `.grow()` wrapper — all of them plain views. The responder chain is the general
    /// answer, because `nextResponder` on a view yields its controller where it has one and its
    /// superview where it does not, so walking it stops at the nearest enclosing controller.
    ///
    /// Getting this wrong is not a layout glitch. UIKit raises the moment a controller's root view is
    /// added to a view owned by an unrelated controller:
    ///
    ///     A view can only be associated with at most one view controller at a time!
    fn enclosing_view_controller(view: &UIView) -> Option<Retained<UIViewController>> {
        let mut next = unsafe { view.nextResponder() };
        while let Some(responder) = next {
            if let Some(vc) = responder.downcast_ref::<UIViewController>() {
                return Some(vc.retain());
            }
            next = unsafe { responder.nextResponder() };
        }
        None
    }

    fn host_controller(h: &Handle) -> Option<Retained<UIViewController>> {
        let key = ptr_of(h);
        // `as_ref` stops at the declared superclass, so these go up the chain by deref coercion.
        let nav = NAV_STATE.with(|m| {
            m.borrow().get(&key).map(|s| match s.split.as_ref() {
                Some(parts) => {
                    let vc: &UIViewController = &parts.split_vc;
                    Retained::from(vc)
                }
                None => {
                    let vc: &UIViewController = &s.nav;
                    Retained::from(vc)
                }
            })
        });
        nav
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
                // Every trampoline body that dispatches into the sink runs under
                // ffi_guard::contain (§8.5): a panic unwinding out of this ObjC frame
                // would abort the process.
                day_spec::ffi_guard::contain((), || {
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
                });
            }

            /// A slider's interaction ENDED: the finger lifted (inside or outside the track), so
            /// the value under it is the one the user chose. `UIControlEvents::ValueChanged`
            /// fires continuously while dragging — bindings need that — so the settled value is a
            /// separate control event (day-spec `Event::ValueCommitted`).
            #[unsafe(method(commit:))]
            fn commit(&self, sender: &UIControl) {
                day_spec::ffi_guard::contain((), || {
                    let obj: &AnyObject = sender.as_ref();
                    if let Some(sl) = obj.downcast_ref::<UISlider>() {
                        emit(
                            self.ivars().node,
                            Event::ValueCommitted(unsafe { sl.value() } as f64),
                        );
                    }
                });
            }

            /// EditingDidBegin — the keyboard is up and this field owns it (docs/focus.md).
            #[unsafe(method(editBegan:))]
            fn edit_began(&self, sender: &UIControl) {
                day_spec::ffi_guard::contain((), || {
                    FOCUSED_FIELD
                        .with(|f| *f.borrow_mut() = Some(Retained::from(sender as &UIView)));
                    // The keyboard may already be up (focus moved between fields): reveal now
                    // too, not only from the keyboard-frame notification.
                    reveal_focused_field();
                    emit(self.ivars().node, Event::FocusChanged(true));
                });
            }

            /// EditingDidEnd — the field resigned (keyboard dismissed or focus moved on).
            #[unsafe(method(editEnded:))]
            fn edit_ended(&self, _sender: &UIControl) {
                day_spec::ffi_guard::contain((), || {
                    FOCUSED_FIELD.with(|f| *f.borrow_mut() = None);
                    emit(self.ivars().node, Event::FocusChanged(false));
                });
            }

            /// EditingDidEndOnExit — the Return key. Registering this handler is also what
            /// makes Return dismiss the keyboard (the UIKit convention); an `on_submit` that
            /// moves focus re-raises it on the next field.
            #[unsafe(method(editExit:))]
            fn edit_exit(&self, _sender: &UIControl) {
                day_spec::ffi_guard::contain((), || {
                    emit(self.ivars().node, Event::Submitted);
                });
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
    // DayBarButtonTarget — the target-action sink for a nav bar's trailing action
    // (docs/navigation.md, NavProps::bar_action). A UIBarButtonItem holds its target
    // WEAKLY, so one target is retained for the whole nav host (in NavState) and reused by
    // every page's item; on tap it emits `Event::MenuAction(action)`, which the tree
    // dispatches to the app's registered closure.
    // -----------------------------------------------------------------------

    struct BarButtonIvars {
        host: NodeId,
        action: u64,
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "DayUIKitBarButtonTarget"]
        #[ivars = BarButtonIvars]
        struct DayBarButtonTarget;

        unsafe impl NSObjectProtocol for DayBarButtonTarget {}

        impl DayBarButtonTarget {
            #[unsafe(method(tap:))]
            fn tap(&self, _sender: &AnyObject) {
                day_spec::ffi_guard::contain((), || {
                    emit(self.ivars().host, Event::MenuAction(self.ivars().action));
                });
            }
        }
    );

    impl DayBarButtonTarget {
        fn new(mtm: MainThreadMarker, host: NodeId, action: u64) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(BarButtonIvars { host, action });
            unsafe { msg_send![super(this), init] }
        }
    }

    /// A nav host's resolved trailing bar action (NavProps::bar_actions): the downscaled template
    /// image, its accessible label, the pages it rides, and the retained target every page's bar
    /// button shares.
    struct NavBarButton {
        image: Option<Retained<objc2_ui_kit::UIImage>>,
        label: String,
        scope: day_spec::props::NavBarScope,
        target: Retained<DayBarButtonTarget>,
    }

    impl NavBarButton {
        /// A fresh `UIBarButtonItem` for one page's `navigationItem` (items are not shared across
        /// controllers), wired to the shared target.
        fn make_item(&self, mtm: MainThreadMarker) -> Retained<UIBarButtonItem> {
            let item = unsafe {
                UIBarButtonItem::initWithImage_style_target_action(
                    UIBarButtonItem::alloc(mtm),
                    self.image.as_deref(),
                    UIBarButtonItemStyle::Plain,
                    Some(&self.target),
                    Some(sel!(tap:)),
                )
            };
            unsafe { item.setAccessibilityLabel(Some(&NSString::from_str(&self.label)), mtm) };
            item
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
                // The callback is day-core's frame tick — contained like every other
                // trampoline (§8.5), so a panicking animation can't abort the app.
                day_spec::ffi_guard::contain((), || {
                    let ts = unsafe { link.timestamp() };
                    let cb = FRAME.with(|f| f.borrow_mut().1.take());
                    if let Some(cb) = cb {
                        cb(ts);
                    }
                    let idle = FRAME.with(|f| f.borrow().1.is_none());
                    if idle {
                        unsafe { link.setPaused(true) };
                    }
                });
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
    // DayTextLink — a text view's link delegate (docs/text-runs.md)
    // -----------------------------------------------------------------------

    thread_local! {
        /// Each link-carrying text view's delegate, kept alive for the view's lifetime (a
        /// `UITextView` holds its delegate weakly). Swept on release.
        static TEXT_LINKS: day_spec::sidetable::SideTable<Retained<DayTextLink>> =
            day_spec::sidetable::SideTable::new();
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "DayUIKitTextLink"]
        #[ivars = NodeId]
        struct DayTextLink;

        unsafe impl NSObjectProtocol for DayTextLink {}

        // UITextViewDelegate refines UIScrollViewDelegate; both are declared, and the scroll
        // half stays empty — a non-scrolling text view never calls it.
        unsafe impl UIScrollViewDelegate for DayTextLink {}

        unsafe impl UITextViewDelegate for DayTextLink {
            /// Answer NO so UIKit does not open the URL itself: the target goes to day-core,
            /// and the label's `.on_link()` decides (its default opens it, which is the same
            /// destination by the route Day controls).
            #[unsafe(method(textView:shouldInteractWithURL:inRange:interaction:))]
            fn should_interact(
                &self,
                _tv: &UITextView,
                url: &objc2_foundation::NSURL,
                _range: objc2_foundation::NSRange,
                // `UITextItemInteraction`, taken as the NSInteger it wraps: objc2 deprecates
                // the newtype in favor of iOS 17 text-item methods that do not exist on the
                // versions Day targets. Unused either way.
                _interaction: isize,
            ) -> bool {
                day_spec::ffi_guard::contain((), || {
                    if let Some(s) = unsafe { url.absoluteString() } {
                        emit(*self.ivars(), Event::LinkActivated(s.to_string()));
                    }
                });
                false
            }
        }
    );

    impl DayTextLink {
        /// Make `tv` report its link taps against `node`, and keep the delegate alive with it.
        fn attach(tv: &UITextView, node: NodeId, mtm: MainThreadMarker) {
            let this = Self::alloc(mtm).set_ivars(node);
            let delegate: Retained<Self> = unsafe { msg_send![super(this), init] };
            unsafe { tv.setDelegate(Some(ProtocolObject::from_ref(&*delegate))) };
            TEXT_LINKS.with(|t| t.insert(ptr_of_view(tv), delegate));
        }
    }

    /// The side-table key for a view: its address, the same key `release` sweeps.
    fn ptr_of_view(v: &UITextView) -> usize {
        let v: &UIView = v.as_ref();
        v as *const UIView as usize
    }

    /// A label backing that can activate links: a read-only, non-scrolling `UITextView` laid out
    /// to measure like the `UILabel` it stands in for (zero inset, no line-fragment padding).
    fn link_text_view(p: &LabelProps, id: NodeId, mtm: MainThreadMarker) -> Retained<UITextView> {
        let tv = UITextView::new(mtm);
        let font = resolve_font(p.font);
        unsafe {
            tv.setFont(Some(&font));
            let _: () = msg_send![&*tv, setAdjustsFontForContentSizeCategory: true];
            if let Some(c) = p.color {
                tv.setTextColor(Some(&uicolor(c)));
            }
            tv.setAttributedText(Some(&attributed_label(&p.text, &font, p.color, &p.runs)));
            tv.setEditable(false);
            tv.setSelectable(true); // required for link interaction, not for selection alone
            tv.setScrollEnabled(false);
            tv.setBackgroundColor(None);
            tv.setTextContainerInset(UIEdgeInsets {
                top: 0.0,
                left: 0.0,
                bottom: 0.0,
                right: 0.0,
            });
            let container: *mut AnyObject = msg_send![&*tv, textContainer];
            let _: () = msg_send![container, setLineFragmentPadding: 0.0f64];
        }
        DayTextLink::attach(&tv, id, mtm);
        tv
    }

    // -----------------------------------------------------------------------
    // DayGesture — tap/pan recognizer target, node-id keyed (docs/shapes.md)
    // -----------------------------------------------------------------------

    struct GestureIvars {
        node: NodeId,
        kind: day_spec::GestureKind,
    }

    thread_local! {
        /// Keeps each view's gesture targets alive + records which are attached (idempotent).
        static GESTURES: RefCell<HashMap<usize, Vec<Retained<DayGesture>>>> =
            RefCell::new(HashMap::new());
        /// Per-view context-menu interaction + its delegate (kept alive; replaced on
        /// reconfigure, swept on release via `day_spec::sidetable`). The teardown detaches
        /// the interaction from its view first, so a recycled address can never serve a dead
        /// view's menu, then drops both retains.
        static CTX_MENUS: day_spec::sidetable::SideTable<(
            Retained<UIContextMenuInteraction>,
            Retained<DayContextMenu>,
        )> = day_spec::sidetable::SideTable::with_teardown(
            |(interaction, _delegate): (
                Retained<UIContextMenuInteraction>,
                Retained<DayContextMenu>,
            )| {
                // `view` is the interaction's weak back-pointer — present exactly while it
                // is still attached, which is when the detach matters.
                if let Some(v) = interaction.view() {
                    v.removeInteraction(ProtocolObject::from_ref(&*interaction));
                }
            },
        );
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
                day_spec::ffi_guard::contain((), || {
                    let node = self.ivars().node;
                    let view = unsafe { g.view() };
                    let loc = unsafe { g.locationInView(view.as_deref()) };
                    let at = day_spec::Point::new(loc.x, loc.y);
                    let phase = match unsafe { g.state() } {
                        UIGestureRecognizerState::Began => day_spec::DragPhase::Began,
                        UIGestureRecognizerState::Ended
                        | UIGestureRecognizerState::Cancelled
                        | UIGestureRecognizerState::Failed => day_spec::DragPhase::Ended,
                        _ => day_spec::DragPhase::Changed,
                    };
                    let obj: &AnyObject = g.as_ref();
                    match self.ivars().kind {
                        day_spec::GestureKind::Drag => {
                            let translation = if let Some(pan) =
                                obj.downcast_ref::<UIPanGestureRecognizer>()
                            {
                                let t = unsafe { pan.translationInView(view.as_deref()) };
                                day_spec::Point::new(t.x, t.y)
                            } else {
                                day_spec::Point::ZERO
                            };
                            emit(
                                node,
                                Event::Drag {
                                    phase,
                                    location: at,
                                    translation,
                                },
                            );
                        }
                        day_spec::GestureKind::Pinch => {
                            // UIPinchGestureRecognizer's scale is cumulative since Began —
                            // exactly Event::Pinch's contract.
                            let scale = obj
                                .downcast_ref::<UIPinchGestureRecognizer>()
                                .map(|p| unsafe { p.scale() })
                                .unwrap_or(1.0);
                            emit(
                                node,
                                Event::Pinch {
                                    phase,
                                    scale,
                                    location: at,
                                },
                            );
                        }
                        day_spec::GestureKind::Pan => {
                            // Event::Pan's delta is INCREMENTAL: read the recognizer's
                            // cumulative translation, then zero it so the next fire reports
                            // only the movement since this one.
                            let delta = if let Some(pan) =
                                obj.downcast_ref::<UIPanGestureRecognizer>()
                            {
                                let t = unsafe { pan.translationInView(view.as_deref()) };
                                unsafe {
                                    pan.setTranslation_inView(
                                        CGPoint::new(0.0, 0.0),
                                        view.as_deref(),
                                    )
                                };
                                day_spec::Point::new(t.x, t.y)
                            } else {
                                day_spec::Point::ZERO
                            };
                            emit(
                                node,
                                Event::Pan {
                                    phase,
                                    delta,
                                    location: at,
                                },
                            );
                        }
                        _ => emit(node, Event::Tap(at)),
                    }
                });
            }
        }
    );

    impl DayGesture {
        fn new(mtm: MainThreadMarker, node: NodeId, kind: day_spec::GestureKind) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(GestureIvars { node, kind });
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
                        // A block's object return is +0 by convention: hand back an
                        // autoreleased pointer. `into_raw` (+1) leaked one retain of the
                        // whole menu graph per summon.
                        Retained::autorelease_return(menu.clone())
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
            NewWindow => "New Window",
        }
    }

    /// The UIResponder standard-edit selector a role routes to (None → a no-op labeled action, since
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
    /// The item's glyph: an SF Symbol for a standard symbol (the shared Apple table), or a
    /// bundled image for an app's own vocabulary — the staged vector first, exactly as every
    /// other image channel resolves (docs/vectors.md).
    fn menu_image(icon: Option<&day_spec::Icon>) -> Option<Retained<objc2_ui_kit::UIImage>> {
        match icon? {
            day_spec::Icon::Symbol(s) => {
                let name = day_spec::sf_symbol_name(*s);
                (!name.is_empty())
                    .then(|| objc2_ui_kit::UIImage::systemImageNamed(&NSString::from_str(name)))
                    .flatten()
            }
            day_spec::Icon::Image(name) => {
                let path = day_spec::resource::resolve_vector_svg(name)
                    .map(std::path::PathBuf::from)
                    .or_else(|| day_spec::resource::resolve_image_file(name))?;
                objc2_ui_kit::UIImage::imageWithContentsOfFile(&NSString::from_str(
                    &path.to_string_lossy(),
                ))
            }
        }
    }

    fn ui_action(
        mtm: MainThreadMarker,
        title: &str,
        enabled: bool,
        icon: Option<&day_spec::Icon>,
        handler: impl Fn() + 'static,
    ) -> Retained<UIMenuElement> {
        let block = block2::RcBlock::new(move |_a: NonNull<UIAction>| handler());
        let image = menu_image(icon);
        let action = unsafe {
            UIAction::actionWithTitle_image_identifier_handler(
                &NSString::from_str(title),
                image.as_deref(),
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
                day_spec::MenuItem::Submenu { label, items, .. } => {
                    out.push(Retained::into_super(build_ui_menu(mtm, label, items)));
                }
                day_spec::MenuItem::Action {
                    id,
                    label,
                    shortcut: _,
                    enabled,
                    role,
                    icon,
                } => {
                    if let Some(role) = role {
                        let title = if label.is_empty() {
                            ui_role_label(*role).to_string()
                        } else {
                            label.clone()
                        };
                        let sel = ui_role_selector(*role);
                        let id = *id;
                        out.push(ui_action(mtm, &title, *enabled, icon.as_ref(), move || {
                            if let Some(sel) = sel {
                                let app = UIApplication::sharedApplication(mtm);
                                unsafe {
                                    app.sendAction_to_from_forEvent(sel, None, None, None);
                                }
                            } else if id != 0 {
                                // No UIKit selector for this role (Undo/Redo): the item
                                // carries the day dispatcher id instead — the same route a
                                // labeled action takes, landing on the installed undo bridge.
                                emit(WINDOW_NODE, Event::MenuAction(id));
                            }
                        }));
                    } else {
                        let id = *id;
                        out.push(ui_action(mtm, label, *enabled, icon.as_ref(), move || {
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

    /// The adaptive half of a nav host (docs/size-classes.md) — present only when the host was
    /// lowered as `Split`. A host lowered `Stack` is a stack at every size (a nested `stack()`
    /// under a split host), and realizes as a plain navigation controller instead: a
    /// `UISplitViewController` assumes it owns the window, and nesting one inside a pane breaks
    /// its layout (the embedded-split trap).
    struct SplitParts {
        /// The adaptive host. `NavState::nav` is its SECONDARY column; the sidebar page's
        /// controller is its primary. Retained because a view holds no strong reference to its
        /// controller, and this one owns the columns and the collapse behavior.
        split_vc: Retained<objc2_ui_kit::UISplitViewController>,
        /// The PRIMARY column's navigation controller — the sidebar page's stack.
        ///
        /// It has to be a navigation controller, not a bare view controller: UIKit merges the
        /// secondary column INTO the primary's stack when it collapses, and with nothing to merge
        /// into it drops the navigation bar entirely — the collapsed list rendered with no title
        /// and no bar button. Which of the two is live therefore depends on the presentation,
        /// which is what `active_nav` answers (docs/size-classes.md).
        primary_nav: Retained<DayNavController>,
        _split_delegate: Retained<DaySplitDelegate>,
    }

    struct NavState {
        nav: Retained<DayNavController>,
        host_node: NodeId,
        /// `Some` for the adaptive (Split-lowered) host, `None` for a plain stack host.
        split: Option<SplitParts>,
        /// UIKit's current answer, mirrored so `insert` knows which container a late-arriving
        /// page belongs in and the pop detector knows when a count change was a merge. Always
        /// `false` for a plain stack host.
        collapsed: std::cell::Cell<bool>,
        /// Our mirror of the intended VC stack (index 0 = root page). This is the SOURCE for
        /// `nav_sync_stack`: day-initiated changes apply it wholesale, so it is pruned eagerly
        /// on `NavPatch::Popped` rather than waiting for the `remove()` duty.
        vcs: Vec<Retained<UIViewController>>,
        /// Native user-back pops (swipe / back button) awaiting Day's answering `NavPatch::Popped`.
        /// The native stack already popped, so that answering patch must be ABSORBED (decrement)
        /// rather than re-pruning the mirror for a pop that already happened
        /// (docs/navigation.md). Mirrors Android's DayNavHost.nativePops.
        native_pops: std::cell::Cell<usize>,
        /// The native VC count at the LAST `didShow` — the pop detector's baseline. Comparing
        /// against the `vcs` mirror instead is wrong: a previous transition's pop-`didShow` can
        /// arrive arbitrarily late, after the next push has already appended to the mirror but
        /// before its `remove()` cleaned the popped entry — the fresh push's own `didShow` then
        /// reads `native < vcs.len()` and a phantom NavBack tears down the just-pushed page.
        /// Only an actual DECREASE in the observed native count is a pop.
        last_native: std::cell::Cell<usize>,
        /// The trailing bar actions (NavProps::bar_actions), applied to each page's `navigationItem`
        /// as it joins the stack — `None` when the host declares none (e.g. desktop).
        bar_actions: Vec<NavBarButton>,
        _delegate: Retained<DayNavDelegate>,
        /// Inline search (docs/search.md): the controller lives on the ROOT page's navigation
        /// item, so pulling the top-level list down reveals it. `None` when the surface is not
        /// searchable or its placement resolved elsewhere. Retained here because the navigation
        /// item does not own the updater.
        search: Option<(
            Retained<objc2_ui_kit::UISearchController>,
            Retained<DaySearchUpdater>,
        )>,
    }

    impl NavState {
        /// The navigation controller that currently OWNS the stack (docs/size-classes.md).
        ///
        /// Collapsed, UIKit has merged the secondary's pages into the primary's, so a push has to
        /// land there; expanded, the two are separate and details belong to the secondary.
        /// Everything that pushes, pops, or inspects the stack goes through this rather than
        /// naming a column, so the same code is right in both presentations.
        fn active_nav(&self) -> Retained<DayNavController> {
            match &self.split {
                Some(parts) if self.collapsed.get() => parts.primary_nav.clone(),
                _ => self.nav.clone(),
            }
        }
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
        /// Each nav page's pane, recorded at realize because `insert` sees only handles
        /// (docs/size-classes.md). The SIDEBAR page is the split host's primary column; every
        /// other page is a detail, pushed on the secondary's stack. Swept on release via
        /// `day_spec::sidetable`.
        static PAGE_PANE: day_spec::sidetable::SideTable<day_spec::props::Pane> =
            day_spec::sidetable::SideTable::new();
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
            /// Re-lay whenever the page joins (or rejoins) a window.
            ///
            /// `safeAreaInsets` is only meaningful for a view that is actually IN a window —
            /// UIKit does not recompute it for the off-screen members of a navigation stack. After
            /// a split collapse the sidebar page sits at the bottom of the merged stack, so it
            /// kept the insets it had in the other orientation (a landscape notch inset of 100pt
            /// applied in portrait). Reporting on entry is what makes the geometry right at the
            /// moment it starts to matter (docs/size-classes.md).
            #[unsafe(method(didMoveToWindow))]
            fn did_move_to_window(&self) {
                let _: () = unsafe { msg_send![super(self), didMoveToWindow] };
                if unsafe { self.window() }.is_some() {
                    self.setNeedsLayout();
                }
            }

            /// The designated change hook for the insets themselves: a page can gain or lose
            /// bar height with its bounds unchanged (standard vs large-title bar as it moves
            /// between columns), and `layoutSubviews` alone never re-fires for that.
            #[unsafe(method(safeAreaInsetsDidChange))]
            fn safe_area_insets_did_change(&self) {
                let _: () = unsafe { msg_send![super(self), safeAreaInsetsDidChange] };
                self.setNeedsLayout();
            }

            #[unsafe(method(layoutSubviews))]
            fn layout_subviews(&self) {
                let _: () = unsafe { msg_send![super(self), layoutSubviews] };
                // The FrameChanged report dispatches day-core's relayout — contained (§8.5).
                day_spec::ffi_guard::contain((), || {
                    // Out of any window the insets below are stale, and a report built from
                    // them would size the content for wherever this page last WAS.
                    if unsafe { self.window() }.is_none() {
                        return;
                    }
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
                    let subs = unsafe { self.subviews() };
                    if let Some(content) = subs.firstObject() {
                        unsafe { content.setFrame(frame) };
                        if *DIAG_NAV {
                            let a = content.frame();
                            log::debug!(
                                "DAYDIAG   applied node={} nsubs={} content=({},{} {}x{})",
                                self.ivars().node.0, subs.count(),
                                a.origin.x, a.origin.y, a.size.width, a.size.height,
                            );
                        }
                    }
                    if *DIAG_NAV {
                        let sup = unsafe { self.superview() }.map(|v| v.bounds()).unwrap_or(bounds);
                        let winf = unsafe { self.convertRect_toView(bounds, None) };
                        log::debug!(
                            "DAYDIAG page node={} bounds={}x{} win=({},{} {}x{}) safe(t{} b{} l{} r{}) -> report {}x{} super={}x{} hidden={}",
                            self.ivars().node.0,
                            bounds.size.width, bounds.size.height,
                            winf.origin.x, winf.origin.y, winf.size.width, winf.size.height,
                            insets.top, insets.bottom, insets.left, insets.right,
                            frame.size.width, frame.size.height,
                            sup.size.width, sup.size.height,
                            self.isHidden(),
                        );
                    }
                    emit(
                        self.ivars().node,
                        Event::FrameChanged(Size::new(frame.size.width, frame.size.height)),
                    );
                });
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

    struct NavControllerIvars {
        host: std::cell::Cell<usize>,
        guarded: std::cell::Cell<bool>,
    }

    // A UINavigationController subclass that intercepts the BACK BUTTON via its own bar-delegate
    // `shouldPop` (docs/navigation.md). A nav controller IS its bar's delegate, so overriding
    // the method here is the sanctioned way to veto a back-button pop. While `guarded`, we veto
    // (return false) and emit `NavBack { already_popped: false }` so Rust's guard decides — the
    // sync/async mismatch resolves because the native pop simply never happens; Rust performs it
    // on `Proceed`. The swipe is a separate path (interactivePopGestureRecognizer), disabled in
    // the GuardTop patch.
    define_class!(
        #[unsafe(super(UINavigationController))]
        #[thread_kind = MainThreadOnly]
        #[name = "DayNavController"]
        #[ivars = NavControllerIvars]
        struct DayNavController;

        unsafe impl NSObjectProtocol for DayNavController {}
        unsafe impl UIBarPositioningDelegate for DayNavController {}

        unsafe impl UINavigationBarDelegate for DayNavController {
            #[unsafe(method(navigationBar:shouldPopItem:))]
            fn should_pop(&self, bar: &UINavigationBar, _item: &UINavigationItem) -> bool {
                // Contained (§8.5): a panic can only arise on the guarded branch, whose
                // intended answer is the veto — so the default is `false`.
                day_spec::ffi_guard::contain(false, || {
                    if self.ivars().guarded.get() {
                        emit(
                            NodeId(self.ivars().host.get() as u64),
                            Event::NavBack {
                                already_popped: false,
                            },
                        );
                        // UIKit dims the back button after a vetoed pop; restore the bar's
                        // opacity on the next runloop turn (the documented shouldPop cosmetic
                        // fix).
                        let bar: Retained<UINavigationBar> = Retained::from(bar);
                        modal_after_idle(move || {
                            for v in unsafe { bar.subviews() }.iter() {
                                unsafe { v.setAlpha(1.0) };
                            }
                        });
                        false
                    } else {
                        true
                    }
                })
            }
        }
    );

    impl DayNavController {
        fn new(mtm: MainThreadMarker, host: usize) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(NavControllerIvars {
                host: std::cell::Cell::new(host),
                guarded: std::cell::Cell::new(false),
            });
            unsafe { msg_send![super(this), init] }
        }
    }

    struct NavDelegateIvars {
        host: std::cell::Cell<usize>,
    }

    /// Inline search on a `.searchable()` surface (docs/search.md).
    ///
    /// The iOS convention: a `UISearchController` on the ROOT page's `navigationItem`, hidden
    /// until the list is pulled down (`hidesSearchBarWhenScrolling`, the default). It is not a
    /// toolbar item — the phones have no toolbar — so the placement resolver hands it here
    /// instead, and the field belongs to the navigation surface it filters.
    struct SearchUpdaterIvars {
        /// The nav host's day node, so edits emit against the surface that declared the search.
        node: std::cell::Cell<u64>,
        /// Suppresses the echo while day writes the field's text back into it.
        suppress: std::cell::Cell<bool>,
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "DaySearchUpdater"]
        #[ivars = SearchUpdaterIvars]
        struct DaySearchUpdater;

        unsafe impl NSObjectProtocol for DaySearchUpdater {}

        unsafe impl UISearchResultsUpdating for DaySearchUpdater {
            #[unsafe(method(updateSearchResultsForSearchController:))]
            fn update(&self, sc: &objc2_ui_kit::UISearchController) {
                day_spec::ffi_guard::contain((), || {
                    if self.ivars().suppress.get() {
                        return;
                    }
                    let text = unsafe { sc.searchBar().text() }
                        .map(|t| t.to_string())
                        .unwrap_or_default();
                    emit(NodeId(self.ivars().node.get()), Event::SearchChanged(text));
                });
            }
        }
    );

    impl DaySearchUpdater {
        fn new(mtm: MainThreadMarker, node: NodeId) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(SearchUpdaterIvars {
                node: std::cell::Cell::new(node.0),
                suppress: std::cell::Cell::new(false),
            });
            unsafe { msg_send![super(this), init] }
        }
    }

    // The split host's own delegate (docs/size-classes.md). UIKit owns the decision here — a
    // `UISplitViewController` collapses and expands on its own as the horizontal size class
    // changes, which on a Plus/Pro Max iPhone happens on every rotation — so Day OBSERVES and
    // reports rather than driving. Pushing a presentation back at it would be a second source of
    // truth racing UIKit's own collapse animation.
    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "DaySplitDelegate"]
        #[ivars = NavDelegateIvars]
        struct DaySplitDelegate;

        unsafe impl NSObjectProtocol for DaySplitDelegate {}

        unsafe impl UISplitViewControllerDelegate for DaySplitDelegate {
            #[unsafe(method(splitViewControllerDidCollapse:))]
            fn did_collapse(&self, _svc: &objc2_ui_kit::UISplitViewController) {
                day_spec::ffi_guard::contain((), || {
                    split_presentation_changed(self.ivars().host.get(), false);
                });
            }

            #[unsafe(method(splitViewControllerDidExpand:))]
            fn did_expand(&self, _svc: &objc2_ui_kit::UISplitViewController) {
                day_spec::ffi_guard::contain((), || {
                    split_presentation_changed(self.ivars().host.get(), true);
                });
            }
        }
    );

    impl DaySplitDelegate {
        fn new(mtm: MainThreadMarker, host: usize) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(NavDelegateIvars {
                host: std::cell::Cell::new(host),
            });
            unsafe { msg_send![super(this), init] }
        }
    }

    /// UIKit collapsed or expanded the split host: reconcile Day's mirror, then report.
    ///
    /// The mirror matters because collapsing MERGES the columns — UIKit inserts the primary
    /// column's view controller at the bottom of the secondary's navigation stack, and expanding
    /// takes it back out. Day's `vcs` mirror tracks only the pages it pushed, so both the mirror
    /// and the pop detector's `last_native` baseline have to be rebased in step; otherwise the
    /// next `didShow` reads the count change as a user back and tears down a live page.
    fn split_presentation_changed(host: usize, expanded: bool) {
        let node = NAV_STATE.with(|m| {
            let mut m = m.borrow_mut();
            let state = m.get_mut(&host)?;
            let parts = state.split.as_ref()?;
            state.collapsed.set(!expanded);
            // Reconcile the mirror with the merge UIKit just performed. `vcs` is Day's picture of
            // the LIVE stack, and the pop detector compares its length against the native count —
            // so the sidebar page has to join it exactly when UIKit merges it in and leave when
            // it is lifted back out, or every subsequent back is misread.
            let sidebar_vc = unsafe { parts.primary_nav.viewControllers() }.firstObject();
            if let Some(vc) = sidebar_vc {
                let already = state.vcs.iter().any(|v| std::ptr::eq(&**v, &*vc));
                if expanded && already {
                    state.vcs.retain(|v| !std::ptr::eq(&**v, &*vc));
                } else if !expanded && !already {
                    state.vcs.insert(0, vc);
                }
            }
            // Rebase against what UIKit actually left in the stack, rather than predicting it —
            // the merge runs inside this callback, so the count here is already the new one.
            let native = unsafe { state.active_nav().viewControllers() }.count();
            state.last_native.set(native);
            Some(state.host_node)
        });
        let Some(node) = node else { return };
        emit(
            node,
            Event::NavPresentationChanged(if expanded {
                day_spec::props::NavPresentation::Split
            } else {
                day_spec::props::NavPresentation::Stack
            }),
        );
        // Re-lay every page against its NEW column, one runloop turn later.
        //
        // A merge REPARENTS the page views, and `layoutSubviews` only fires — and only reports
        // `FrameChanged` — when a view's own bounds change. Reparenting alone may not change
        // them in the same pass, so each page can keep the frame it had in the other
        // presentation: the list drawn at sidebar width on top of a detail still sized for the
        // split. Forcing the layout inline does not help either, because this callback runs
        // DURING the transition and the bounds are still mid-animation. Deferring is the same
        // shape as the Qt fix, where hiding a splitter pane does not resize its sibling until Qt
        // has run its own layout pass (docs/size-classes.md).
        dispatch2::DispatchQueue::main().exec_async(move || {
            NAV_STATE.with(|m| {
                let m = m.borrow();
                let Some(state) = m.get(&host) else { return };
                // RESIZE each page to the column that now owns it, then lay it out.
                //
                // Laying a view out does not change its own frame, which is why forcing
                // `layoutIfNeeded` alone never fixed this: UIKit does not resize the OFF-SCREEN
                // members of a navigation stack, and after a collapse the sidebar page sits at
                // the bottom of the merged stack. It kept the column bounds it had while
                // expanded — measured at 420x409 in landscape while the detail had correctly
                // re-laid to 430x839 — so its content stayed sized for the other presentation
                // (docs/size-classes.md).
                //
                // Every page is full-bleed within its own navigation controller, so that
                // controller's view bounds ARE the page's frame.
                // Ask each COLUMN to lay itself out, rather than resizing pages by hand.
                //
                // The navigation controller owns its pages' frames AND their safe-area insets, so
                // laying it out propagates both. Setting a page's frame directly does not: the
                // insets stay whatever they were where the page last lived, which is how a
                // landscape notch inset of 100pt survived into portrait.
                let relayout = |nav: &DayNavController| {
                    if let Some(v) = unsafe { nav.viewIfLoaded() } {
                        v.setNeedsLayout();
                        v.layoutIfNeeded();
                    }
                };
                let Some(parts) = state.split.as_ref() else {
                    return;
                };
                relayout(&parts.primary_nav);
                relayout(&state.nav);
                if let Some(v) = unsafe { parts.split_vc.viewIfLoaded() } {
                    v.setNeedsLayout();
                    v.layoutIfNeeded();
                }
            });
        });
    }

    /// Apply Day's model of the stack to the ACTIVE navigation controller in ONE
    /// `setViewControllers:animated:` — UIKit's atomic stack primitive (docs/navigation.md).
    ///
    /// Incremental push/pop calls raced a fast driver: a selection change is a pop AND a
    /// push, and issuing the second while the first still animates leaves `viewControllers`
    /// reporting a transient state — one mid-flight read hit a 1-count and the detail-column
    /// wipe emptied the MERGED stack, sidebar included; the late didShow train then read as
    /// a user back and tore the route down. Deriving the whole array from the mirror at
    /// execution time is idempotent: however calls interleave, the LAST sync applies the
    /// final model and every intermediate state converges. A sync's settled count equals the
    /// mirror's by construction, so `didShow` can never mistake it for a user pop.
    fn nav_sync_stack(host: usize) {
        let target = NAV_STATE.with(|m| {
            let m = m.borrow();
            m.get(&host).map(|s| (s.active_nav(), s.vcs.clone()))
        });
        let Some((nav, vcs)) = target else { return };
        let current = unsafe { nav.viewControllers() };
        let unchanged = current.count() == vcs.len()
            && (0..vcs.len()).all(|i| std::ptr::eq(&*current.objectAtIndex(i), &*vcs[i]));
        if unchanged {
            return;
        }
        if *DIAG_NAV {
            log::debug!(
                "DAYDIAG exec SYNC native={} -> target={}",
                current.count(),
                vcs.len()
            );
        }
        let arr = objc2_foundation::NSArray::from_retained_slice(&vcs);
        // Re-stamp the transition clock at EXECUTION, not only at dispatch: this closure can
        // sit in the modal queue past `ui_idle`'s 250ms settle margin, and the transition
        // coordinator is only born once the set below runs — without this second stamp a
        // screenshot lands in that blind window and captures a mid-slide frame.
        note_ui_transition();
        // Never ANIMATE to an empty stack (deselecting in the expanded split empties the
        // detail column): with no destination controller the transition sets up but never
        // completes — the stack keeps its old contents and the orphaned transition
        // coordinator holds `ui_idle` false forever, failing every later screenshot.
        let animated = !vcs.is_empty();
        unsafe { nav.setViewControllers_animated(&arr, animated) };
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
                // than native. Day-initiated changes go through `nav_sync_stack`, whose
                // settled count EQUALS the mirror's — so what passes both tests is the
                // user's back button / swipe, which pops the native stack under a mirror
                // that still holds the page.
                // The whole detector runs contained (§8.5): both closures below re-enter
                // day-core (the sink dispatch and the mirror bookkeeping).
                day_spec::ffi_guard::contain((), || {
                    let host = self.ivars().host.get();
                    let suspicious = NAV_STATE.with(|m| {
                        let mut m = m.borrow_mut();
                        let Some(state) = m.get_mut(&host) else {
                            return false;
                        };
                        let native = unsafe { nav.viewControllers() }.count();
                        // A split host MERGES its columns as it collapses and separates them as
                        // it expands, which moves a controller in or out of this stack — a count
                        // change that is not a pop at all (docs/size-classes.md). The delegate
                        // callback that rebases the mirror can arrive after this `didShow`, so
                        // detect the in-flight transition here and rebase rather than reading it
                        // as a user back: absorbed as a phantom pop, it would swallow the NEXT
                        // real one and leave a stale page under the detail.
                        if let Some(parts) = state.split.as_ref()
                            && state.collapsed.get() != unsafe { parts.split_vc.isCollapsed() }
                        {
                            if *DIAG_NAV {
                                log::debug!(
                                    "DAYDIAG didShow REBASE native={native} (collapse flip in flight)"
                                );
                            }
                            state.last_native.set(native);
                            return false;
                        }
                        let prev = state.last_native.replace(native);
                        let popped = native < prev && native < state.vcs.len();
                        if *DIAG_NAV {
                            log::debug!(
                                "DAYDIAG didShow native={native} prev={prev} mirror={} suspicious={popped}",
                                state.vcs.len(),
                            );
                        }
                        popped
                    });
                    if !suspicious {
                        return;
                    }
                    // A pop-shaped didShow with no day-initiated pop in flight is PROBABLY the
                    // user's back button/swipe — but interleaved sibling transitions (day pops
                    // one detail and pushes the next while the pop is still animating) deliver a
                    // LATE duplicate pop-didShow after the push, and treating that as a user
                    // back tears down the just-pushed page. Only a pop that PERSISTS one runloop
                    // turn is a user pop: re-check on the next main-queue turn, when the
                    // interleaved transition has settled and `viewControllers` reports the real
                    // stack.
                    dispatch2::DispatchQueue::main().exec_async(move || {
                        // Its own FFI entry (a posted block), so its own containment.
                        day_spec::ffi_guard::contain((), || {
                            let (emit_back, node) = NAV_STATE.with(|m| {
                                let mut m = m.borrow_mut();
                                let Some(state) = m.get_mut(&host) else {
                                    return (false, NodeId(0));
                                };
                                let native =
                                    unsafe { state.active_nav().viewControllers() }.count();
                                state.last_native.set(native);
                                if *DIAG_NAV {
                                    log::debug!(
                                        "DAYDIAG didShow SETTLE native={native} mirror={} -> user_back={}",
                                        state.vcs.len(),
                                        native < state.vcs.len(),
                                    );
                                }
                                if native < state.vcs.len() {
                                    // Still popped after settling: a real user back. Sync the
                                    // mirror (Day's remove() will find it gone) and record that
                                    // Day's answering NavPatch::Popped must be ABSORBED.
                                    state.vcs.truncate(native);
                                    state.native_pops.set(state.native_pops.get() + 1);
                                    (true, state.host_node)
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
                        });
                    });
                });
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

    /// The native FRONT of the app's one undo stack (docs/model.md): answers the questions
    /// UIKit's undo affordances ask (three-finger gestures, shake, hardware ⌘Z, the iPad menu
    /// bar) from mirrored state, and forwards invocations as `Event::Undo`. The stack lives
    /// in day-model; two histories can never fork.
    pub(super) struct UndoIvars {
        pub(super) state: RefCell<day_spec::UndoState>,
    }

    define_class!(
        #[unsafe(super(objc2_foundation::NSUndoManager))]
        #[thread_kind = MainThreadOnly]
        #[name = "DayUndoManager"]
        #[ivars = UndoIvars]
        pub(super) struct DayUndoManager;

        impl DayUndoManager {
            #[unsafe(method(canUndo))]
            fn can_undo(&self) -> bool {
                self.ivars().state.borrow().can_undo
            }

            #[unsafe(method(canRedo))]
            fn can_redo(&self) -> bool {
                self.ivars().state.borrow().can_redo
            }

            #[unsafe(method(undo))]
            fn do_undo(&self) {
                day_spec::ffi_guard::contain((), || {
                    emit(day_spec::WINDOW_NODE, Event::Undo { redo: false })
                })
            }

            #[unsafe(method(redo))]
            fn do_redo(&self) {
                day_spec::ffi_guard::contain((), || {
                    emit(day_spec::WINDOW_NODE, Event::Undo { redo: true })
                })
            }

            #[unsafe(method_id(undoMenuItemTitle))]
            fn undo_menu_item_title(&self) -> Retained<NSString> {
                let label = self.ivars().state.borrow().undo_label.clone();
                unsafe { self.undoMenuTitleForUndoActionName(&NSString::from_str(&label)) }
            }

            #[unsafe(method_id(redoMenuItemTitle))]
            fn redo_menu_item_title(&self) -> Retained<NSString> {
                let label = self.ivars().state.borrow().redo_label.clone();
                unsafe { self.redoMenuTitleForUndoActionName(&NSString::from_str(&label)) }
            }
        }
    );

    thread_local! {
        pub(super) static UNDO_FRONT: RefCell<Option<Retained<DayUndoManager>>> =
            const { RefCell::new(None) };
    }

    pub(super) fn undo_front(mtm: MainThreadMarker) -> Retained<DayUndoManager> {
        UNDO_FRONT.with(|u| {
            u.borrow_mut()
                .get_or_insert_with(|| {
                    let this = DayUndoManager::alloc(mtm).set_ivars(UndoIvars {
                        state: RefCell::new(day_spec::UndoState::default()),
                    });
                    unsafe { msg_send![super(this), init] }
                })
                .clone()
        })
    }

    thread_local! {
        /// The app's edit-bridge state (`set_edit_state`) — what canPerformAction consults.
        static EDIT_STATE: std::cell::Cell<day_spec::EditState> =
            const { std::cell::Cell::new(day_spec::EditState { can_cut: false, can_copy: false, can_paste: false, can_select_all: false }) };
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

            /// The responder chain's answer while no text field holds focus: the app's one
            /// stack, through its front — a focused field's own manager keeps precedence,
            /// which is exactly the typing rule (docs/model.md).
            #[unsafe(method_id(undoManager))]
            fn undo_manager(&self) -> Option<Retained<objc2_foundation::NSUndoManager>> {
                day_spec::ffi_guard::contain(None, || {
                    UNDO_FRONT
                        .with(|u| u.borrow().as_ref().map(|m| Retained::into_super(m.clone())))
                })
            }
        }
    );

    impl DayRootVC {
        fn new(mtm: MainThreadMarker) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(());
            unsafe { msg_send![super(this), init] }
        }
    }

    define_class!(
        #[unsafe(super(UIView))]
        #[thread_kind = MainThreadOnly]
        #[name = "DayHolderView"]
        #[ivars = ()]
        struct DayHolderView;

        /// The window root's content holder. UIKit resizes it on rotation (and iPad
        /// multitasking) and runs this layout pass — Day's size-change rail: re-pin the day
        /// root to the CURRENT safe area and emit `WindowResized`, the same shape as
        /// Android's configuration-change delivery (§9). Launch computes the initial frame;
        /// this fires only when the BASE frame really changed, so the keyboard rail's
        /// shrunken root (which alters the frame but not the base) is never stomped.
        impl DayHolderView {
            #[unsafe(method(layoutSubviews))]
            fn layout_subviews(&self) {
                let _: () = unsafe { msg_send![super(self), layoutSubviews] };
                // The WindowResized report dispatches day-core's relayout — contained (§8.5).
                day_spec::ffi_guard::contain((), || {
                    // Before launch has published the root, its own frame computation owns
                    // this (it runs once the window is key) — nothing to re-pin yet.
                    let Some(root) = ROOT_VIEW.with(|r| r.borrow().clone()) else {
                        return;
                    };
                    let bounds = self.bounds();
                    let insets = self.safeAreaInsets();
                    let inner = CGRect::new(
                        CGPoint::new(insets.left, insets.top),
                        CGSize::new(
                            (bounds.size.width - insets.left - insets.right).max(0.0),
                            (bounds.size.height - insets.top - insets.bottom).max(0.0),
                        ),
                    );
                    let base = ROOT_BASE_FRAME.with(|f| f.get());
                    if inner.origin.x == base.origin.x
                        && inner.origin.y == base.origin.y
                        && inner.size.width == base.size.width
                        && inner.size.height == base.size.height
                    {
                        return;
                    }
                    ROOT_BASE_FRAME.with(|f| f.set(inner));
                    unsafe { root.setFrame(inner) };
                    if *DIAG_NAV {
                        log::debug!(
                            "DAYDIAG holder bounds={}x{} safe(t{} b{} l{} r{}) -> inner=({},{} {}x{})",
                            bounds.size.width,
                            bounds.size.height,
                            insets.top,
                            insets.bottom,
                            insets.left,
                            insets.right,
                            inner.origin.x,
                            inner.origin.y,
                            inner.size.width,
                            inner.size.height,
                        );
                    }
                    emit(
                        WINDOW_NODE,
                        Event::WindowResized(Size::new(inner.size.width, inner.size.height)),
                    );
                });
            }
        }
    );

    impl DayHolderView {
        fn new(mtm: MainThreadMarker) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(());
            unsafe { msg_send![super(this), init] }
        }
    }

    /// Queue the cover's presentation behind any in-flight modal transition (§dialogs FIFO).
    fn cover_present(vc: Retained<DayCoverVC>) {
        modal_enqueue(ModalOp::Cover(vc, 0));
    }

    /// Queue the cover's dismissal; the completion reports `CoverHidden` so the piece can
    /// dispose the content only after it left the screen.
    fn cover_dismiss(vc: Retained<DayCoverVC>, node: NodeId) {
        modal_enqueue(ModalOp::Run(Box::new(move || {
            let Some(presenting) = vc.presentingViewController() else {
                emit(node, Event::CoverHidden);
                return;
            };
            modal_begin_transition();
            // The completion is the normal `CoverHidden` source — but UIKit can drop a
            // transition completion outright (same failure the modal watchdog exists for),
            // and the piece would then never dispose the hidden content. The fallback
            // watchdog emits once the VC has actually left the hierarchy; the piece's
            // closing gate makes a duplicate report harmless.
            let fired = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let completion = {
                let fired = fired.clone();
                block2::RcBlock::new(move || {
                    fired.store(true, std::sync::atomic::Ordering::Relaxed);
                    emit(node, Event::CoverHidden);
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
                    log::warn!("cover dismissal completion lost — reporting CoverHidden");
                    emit(node, Event::CoverHidden);
                }
            });
        })));
    }

    // -------------------------------------------------------------------
    // Tabs (docs/navigation.md): UITabBarController child-contained in the root VC.
    // Each tab page is a UIViewController wrapping a DayNavPageView (safe-area
    // pinned content + FrameChanged), identical to a nav page.
    // -------------------------------------------------------------------

    // -------------------------------------------------------------------
    // Adaptive tabs (docs/navigation.md): a NAV host lowered `Tabs` becomes a
    // `UITabBarController` in `.tabSidebar` mode — ONE controller that draws a tab bar when the
    // window is compact and a sidebar when it is not, with UIKit's own animation and the
    // iPadOS user-facing toggle. It is what SwiftUI's `.tabViewStyle(.sidebarAdaptable)`
    // compiles down to.
    //
    // Two consequences shape everything below. UIKit draws the SIDEBAR itself, from the same
    // tabs — so Day's `Pane::Sidebar` page has nothing to render and is left out of the
    // controller entirely. And every tab keeps its own view controller at every width, so the
    // host reports `Tabs` once and stays there: it never flips to push/pop as it widens, which
    // is why day-core keeps its pages resident and drives them with `NavPatch::Select`.
    // -------------------------------------------------------------------

    struct NavTabsState {
        tabbar: Retained<UITabBarController>,
        /// Detail pages in insertion order — index i IS the `Select(i)` index.
        vcs: Vec<Retained<UIViewController>>,
        /// Row labels and glyphs from the host's NAV_MENU, which is where a selector's rows live.
        titles: Vec<String>,
        icons: Vec<Option<Retained<objc2_ui_kit::UIImage>>>,
        /// The NAV_MENU's node — a tab tap emits against it, exactly as a sidebar row click does,
        /// so the two are one event to everything above this backend.
        menu_node: std::cell::Cell<i64>,
        _delegate: Retained<DayNavTabsDelegate>,
    }

    thread_local! {
        static NAV_TABS: RefCell<HashMap<usize, NavTabsState>> = RefCell::new(HashMap::new());
        /// A realized NAV_MENU's rows, by its own view ptr. Recorded at realize because that is
        /// where the props are, and consumed at INSERT, which is the first moment the menu is in
        /// a view hierarchy and its enclosing host can be found.
        /// A tabs host's page content views → the host, so a nav menu inside a page that is not
        /// in the controller's hierarchy can still find it.
        static TABS_PAGE_HOST: RefCell<HashMap<usize, usize>> = RefCell::new(HashMap::new());
        static NAV_MENU_ROWS: RefCell<
            HashMap<usize, (i64, Vec<String>, Vec<Option<Retained<objc2_ui_kit::UIImage>>>)>,
        > = RefCell::new(HashMap::new());
    }

    /// Walk up from `v` for a `.tabSidebar` host — either the host's own view, or a PAGE known
    /// to belong to one.
    ///
    /// The page lookup is what makes the sidebar page reachable. Its view is deliberately never
    /// added to the controller (UIKit draws the sidebar itself), so it has no superview chain
    /// running to the host — but the rows Day needs for the tab labels live inside it. Recording
    /// the page → host edge at insert is what lets the menu find its way home anyway.
    fn enclosing_tabs_host(v: &UIView) -> Option<usize> {
        let mut cur = Some(v.retain());
        while let Some(view) = cur {
            let p = ptr_of(&view_of(view.clone()));
            if NAV_TABS.with(|m| m.borrow().contains_key(&p)) {
                return Some(p);
            }
            if let Some(host) = TABS_PAGE_HOST.with(|m| m.borrow().get(&p).copied()) {
                return Some(host);
            }
            cur = unsafe { view.superview() };
        }
        None
    }

    struct NavTabsDelegateIvars {
        host: std::cell::Cell<usize>,
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "DayNavTabsDelegate"]
        #[ivars = NavTabsDelegateIvars]
        struct DayNavTabsDelegate;

        unsafe impl NSObjectProtocol for DayNavTabsDelegate {}

        unsafe impl UITabBarControllerDelegate for DayNavTabsDelegate {
            #[unsafe(method(tabBarController:didSelectViewController:))]
            fn did_select(&self, tabbar: &UITabBarController, _vc: &UIViewController) {
                // UIKit calls this only for user taps, not programmatic selection — no echo
                // guard needed; the panic containment is §8.5.
                day_spec::ffi_guard::contain((), || {
                    let idx = unsafe { tabbar.selectedIndex() } as i64;
                    let node = NAV_TABS.with(|m| {
                        m.borrow()
                            .get(&self.ivars().host.get())
                            .map(|t| t.menu_node.get())
                    });
                    if let Some(n) = node.filter(|n| *n != 0) {
                        emit(NodeId(n as u64), Event::SelectionChanged(idx));
                    }
                });
            }
        }
    );

    impl DayNavTabsDelegate {
        fn new(mtm: MainThreadMarker, host: usize) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(NavTabsDelegateIvars {
                host: std::cell::Cell::new(host),
            });
            unsafe { msg_send![super(this), init] }
        }
    }

    /// Re-apply each tab's label and glyph from the host's rows, and install the controllers.
    /// Called whenever either side changes — a page joining, or the rows arriving/being re-derived.
    fn nav_tabs_sync(host: usize) {
        NAV_TABS.with(|m| {
            let m = m.borrow();
            let Some(t) = m.get(&host) else { return };
            for (i, vc) in t.vcs.iter().enumerate() {
                let title = t.titles.get(i).cloned().unwrap_or_default();
                let image = t.icons.get(i).and_then(|o| o.clone());
                unsafe {
                    let item = objc2_ui_kit::UITabBarItem::initWithTitle_image_tag(
                        objc2_ui_kit::UITabBarItem::alloc(MainThreadMarker::new_unchecked()),
                        Some(&NSString::from_str(&title)),
                        image.as_deref(),
                        i as isize,
                    );
                    vc.setTabBarItem(Some(&item));
                    vc.setTitle(Some(&NSString::from_str(&title)));
                }
            }
            let arr = objc2_foundation::NSArray::from_retained_slice(&t.vcs);
            unsafe { t.tabbar.setViewControllers_animated(Some(&arr), false) };
        });
    }

    // -------------------------------------------------------------------
    // -----------------------------------------------------------------------
    // DayNavCell — a nav row whose icon reads as a natural iOS glyph: a small
    // (20pt) template image tinted with the neutral secondaryLabel color (NOT
    // the accent), vertically centered on the row like the label, so the glyph's
    // optical center matches the text's line center (the UIListContentConfiguration
    // idiom). The stock UITableViewCell accent-tints its imageView, so we lay the
    // row out ourselves (docs/navigation.md).
    // -----------------------------------------------------------------------
    struct NavCellIvars {
        icon: Retained<objc2_ui_kit::UIImageView>,
        title: Retained<UILabel>,
        /// Trailing status glyph (docs/navigation.md) — a starred page's star. Laid out inside
        /// the accessory's margin, so it never collides with the disclosure chevron.
        badge_icon: Retained<objc2_ui_kit::UIImageView>,
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
                const LEADING: f64 = 16.0;
                const ICON: f64 = 20.0;
                const GAP: f64 = 12.0;
                let has_icon = unsafe { iv.icon.image() }.is_some();
                let text_x = if has_icon { LEADING + ICON + GAP } else { LEADING };
                // The status glyph takes width off the label's right edge rather than overlaying
                // it, so a long title truncates instead of running under the star.
                let has_badge = unsafe { iv.badge_icon.image() }.is_some();
                let badge_w = if has_badge { ICON + GAP } else { 0.0 };
                unsafe {
                    iv.title.setFrame(CGRect::new(
                        CGPoint::new(text_x, label_y),
                        CGSize::new((cw - text_x - 6.0 - badge_w).max(0.0), line_h),
                    ));
                    iv.badge_icon.setHidden(!has_badge);
                    if has_badge {
                        iv.badge_icon.setFrame(CGRect::new(
                            CGPoint::new((cw - ICON).max(0.0), ((ch - ICON) / 2.0).max(0.0)),
                            CGSize::new(ICON, ICON),
                        ));
                    }
                    iv.icon.setHidden(!has_icon);
                    if has_icon {
                        // Center the icon on the row, matching the centered label. Bottoming
                        // the box on the text BASELINE rode visibly high: Material template
                        // PNGs pad the glyph inside the canvas, so the box must align by
                        // optical center, not by edge.
                        iv.icon.setFrame(CGRect::new(
                            CGPoint::new(LEADING, ((ch - ICON) / 2.0).max(0.0)),
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
                badge_icon: unsafe { objc2_ui_kit::UIImageView::new(mtm) },
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
                iv.badge_icon
                    .setContentMode(objc2_ui_kit::UIViewContentMode::ScaleAspectFit);
                cell.contentView().addSubview(&iv.title);
                cell.contentView().addSubview(&iv.icon);
                cell.contentView().addSubview(&iv.badge_icon);
                cell.setAccessoryType(
                    objc2_ui_kit::UITableViewCellAccessoryType::DisclosureIndicator,
                );
            }
            cell
        }

        fn configure(
            &self,
            title: &NSString,
            image: Option<&objc2_ui_kit::UIImage>,
            tint: Option<day_spec::Color>,
            badge_image: Option<&objc2_ui_kit::UIImage>,
            badge_tint: Option<day_spec::Color>,
        ) {
            let iv = self.ivars();
            unsafe {
                iv.title.setText(Some(title));
                iv.icon.setImage(image);
                // Per-row tint (docs/vectors.md): recolor the template glyph; None keeps the
                // neutral secondaryLabel look.
                match tint {
                    Some(t) => iv.icon.setTintColor(Some(&uicolor(t))),
                    None => iv.icon.setTintColor(Some(&UIColor::secondaryLabelColor())),
                }
                iv.badge_icon.setImage(badge_image);
                match badge_tint {
                    Some(t) => iv.badge_icon.setTintColor(Some(&uicolor(t))),
                    None => iv
                        .badge_icon
                        .setTintColor(Some(&UIColor::secondaryLabelColor())),
                }
            }
            self.setNeedsLayout();
        }
    }

    // DayNavTableData — nav_menu() as inset-grouped rows with chevrons
    // -------------------------------------------------------------------

    struct NavTableIvars {
        node: NodeId,
        items: RefCell<Vec<Retained<NSString>>>,
        /// Per-row icon tint (docs/vectors.md); `None` keeps the neutral template look.
        tints: RefCell<Vec<Option<day_spec::Color>>>,
        /// Pre-resolved template icons per row (docs/navigation.md), `None` where a row has none.
        /// Template mode tints them with the cell's tint color (the iOS list idiom).
        icons: RefCell<Vec<Option<Retained<objc2_ui_kit::UIImage>>>>,
        /// Trailing status glyphs per row, resolved the same way as `icons`.
        badge_icons: RefCell<Vec<Option<Retained<objc2_ui_kit::UIImage>>>>,
        /// Tint for `badge_icons`; `None` keeps the neutral template look.
        badge_tints: RefCell<Vec<Option<day_spec::Color>>>,
        /// Per-row context menu (docs/menus.md), empty = none — served through the table
        /// delegate's row-context hook, the standard iOS long-press row menu.
        menus: RefCell<Vec<Vec<day_spec::MenuItem>>>,
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
                let tint = self.ivars().tints.borrow().get(row).copied().flatten();
                let bimg = self
                    .ivars()
                    .badge_icons
                    .borrow()
                    .get(row)
                    .and_then(|o| o.clone());
                let btint = self
                    .ivars()
                    .badge_tints
                    .borrow()
                    .get(row)
                    .copied()
                    .flatten();
                cell.configure(&title, img.as_deref(), tint, bimg.as_deref(), btint);
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
                day_spec::ffi_guard::contain((), || {
                    let row = unsafe { index_path.row() };
                    unsafe { tv.deselectRowAtIndexPath_animated(index_path, true) };
                    emit(self.ivars().node, Event::SelectionChanged(row as i64));
                });
            }

            /// The row's context menu (docs/menus.md): the same UIMenu the piece decorator
            /// builds, served through the table's own long-press affordance.
            #[unsafe(method_id(tableView:contextMenuConfigurationForRowAtIndexPath:point:))]
            fn context_menu_for_row(
                &self,
                _tv: &objc2_ui_kit::UITableView,
                index_path: &objc2_foundation::NSIndexPath,
                _point: CGPoint,
            ) -> Option<Retained<UIContextMenuConfiguration>> {
                let row = unsafe { index_path.row() } as usize;
                let items = self
                    .ivars()
                    .menus
                    .borrow()
                    .get(row)
                    .cloned()
                    .unwrap_or_default();
                if items.is_empty() {
                    // `define_class!` rewrites the return, so no early `return` — one expression.
                    None
                } else {
                    let menu = build_ui_menu(self.mtm(), "", &items);
                    let provider = block2::RcBlock::new(
                        move |_suggested: NonNull<objc2_foundation::NSArray<UIMenuElement>>| -> *mut UIMenu {
                            // +0 block return: autorelease, don't leak a retain per summon
                            // (see DayContextMenu::configuration_for_menu).
                            Retained::autorelease_return(menu.clone())
                        },
                    );
                    Some(unsafe {
                        UIContextMenuConfiguration::configurationWithIdentifier_previewProvider_actionProvider(
                            None,
                            std::ptr::null_mut(),
                            block2::RcBlock::as_ptr(&provider),
                            self.mtm(),
                        )
                    })
                }
            }
        }
    );

    /// Load a bundled image by NAME for a nav/tab icon (docs/navigation.md): by-name from the
    /// DayPieces asset catalog first — the reliable iOS path, same as the `image()` piece — then a
    /// loose staged file (dev / assets). Callers apply `.alwaysTemplate` so it tints with the
    /// control's color.
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

    /// Resolve nav glyph names to template images once per rebuild (docs/navigation.md).
    fn resolve_nav_images(
        names: &[Option<String>],
    ) -> Vec<Option<Retained<objc2_ui_kit::UIImage>>> {
        names
            .iter()
            .map(|ic| {
                let img = load_bundled_uiimage(ic.as_deref()?)?;
                Some(unsafe {
                    img.imageWithRenderingMode(objc2_ui_kit::UIImageRenderingMode::AlwaysTemplate)
                })
            })
            .collect()
    }

    impl DayNavTableData {
        // The parameters ARE `NavMenuProps`, minus `selected`: index-aligned per-row
        // decoration arrays. Taking the props struct instead would tie this to one caller —
        // `NavMenuPatch::Items` carries the same arrays without a props value to hand over.
        #[allow(clippy::too_many_arguments)]
        fn new(
            mtm: MainThreadMarker,
            node: NodeId,
            items: &[String],
            icons: &[Option<String>],
            tints: &[Option<day_spec::Color>],
            menus: &[Vec<day_spec::MenuItem>],
            badge_icons: &[Option<String>],
            badge_tints: &[Option<day_spec::Color>],
        ) -> Retained<Self> {
            let resolved = resolve_nav_images(icons);
            let this = Self::alloc(mtm).set_ivars(NavTableIvars {
                node,
                items: RefCell::new(items.iter().map(|s| NSString::from_str(s)).collect()),
                icons: RefCell::new(resolved),
                badge_icons: RefCell::new(resolve_nav_images(badge_icons)),
                badge_tints: RefCell::new(badge_tints.to_vec()),
                tints: RefCell::new(tints.to_vec()),
                menus: RefCell::new(menus.to_vec()),
            });
            unsafe { msg_send![super(this), init] }
        }

        /// Data-driven rows changed (`NavMenuPatch::Items`): swap labels/icons in place.
        fn set_items(
            &self,
            items: &[String],
            icons: &[Option<String>],
            tints: &[Option<day_spec::Color>],
            menus: &[Vec<day_spec::MenuItem>],
            badge_icons: &[Option<String>],
            badge_tints: &[Option<day_spec::Color>],
        ) {
            *self.ivars().items.borrow_mut() =
                items.iter().map(|s| NSString::from_str(s)).collect();
            *self.ivars().tints.borrow_mut() = tints.to_vec();
            *self.ivars().menus.borrow_mut() = menus.to_vec();
            *self.ivars().icons.borrow_mut() = resolve_nav_images(icons);
            *self.ivars().badge_icons.borrow_mut() = resolve_nav_images(badge_icons);
            *self.ivars().badge_tints.borrow_mut() = badge_tints.to_vec();
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
        /// The app's localized word for the swipe action (docs/list.md); empty ⇒ trash glyph.
        delete_label: RefCell<String>,
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
                // Snapshot-only read (no tree) — safe during reloadData inside a with_tree
                // borrow. `len` is an app closure, so the body is contained (§8.5).
                day_spec::ffi_guard::contain(0, || {
                    self.ivars()
                        .source
                        .borrow()
                        .as_ref()
                        .map(|s| (s.len)() as isize)
                        .unwrap_or(0)
                })
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
                // Day builds/rebinds its row content inside the cell's contentView. The bind
                // runs day-core's row builder — contained (§8.5); a panicking row leaves the
                // recycled cell blank rather than aborting.
                let content = cell.contentView();
                let row = unsafe { index_path.row() } as usize;
                if let Some(source) = self.ivars().source.borrow().as_ref() {
                    let raw = Retained::as_ptr(&content) as RawHandle;
                    day_spec::ffi_guard::contain((), || (source.bind_row)(row, raw));
                }
                cell
            }

            // --- drag-to-reorder (docs/list.md): the data-source half. With a drag delegate and
            // `dragInteractionEnabled`, UITableView runs its whole native reorder UX — long-press
            // lift, the gap under the finger, haptics — and commits through `moveRow`.

            #[unsafe(method(tableView:canMoveRowAtIndexPath:))]
            fn can_move_row(
                &self,
                _tv: &objc2_ui_kit::UITableView,
                index_path: &objc2_foundation::NSIndexPath,
            ) -> objc2::runtime::Bool {
                // A row the guard won't move ANYWHERE (a pinned row) refuses the lift itself:
                // probing (row -> row) is the cheapest "may this row drag at all" question.
                // The verdict runs the app's guard closure — contained (§8.5), refusing on panic.
                let row = unsafe { index_path.row() } as usize;
                objc2::runtime::Bool::new(day_spec::ffi_guard::contain(false, || {
                    self.reorder_verdict(row, row) >= 0
                }))
            }

            #[unsafe(method(tableView:moveRowAtIndexPath:toIndexPath:))]
            fn move_row(
                &self,
                _tv: &objc2_ui_kit::UITableView,
                from_path: &objc2_foundation::NSIndexPath,
                to_path: &objc2_foundation::NSIndexPath,
            ) {
                // UIKit hands FINAL indices (post-removal semantics — the seam's own contract).
                // The table has already animated the move; commit rotates Day's snapshot and
                // defers the app callback — contained (§8.5).
                day_spec::ffi_guard::contain((), || {
                    let (from, to) = unsafe { (from_path.row() as usize, to_path.row() as usize) };
                    if from == to {
                        return;
                    }
                    let mv = self
                        .ivars()
                        .source
                        .borrow()
                        .as_ref()
                        .and_then(|s| s.reorder.as_ref().map(|r| r.move_row.clone()));
                    if let Some(mv) = mv {
                        mv(from, to);
                    }
                });
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
                day_spec::ffi_guard::contain((), || {
                    let row = unsafe { index_path.row() };
                    unsafe { tv.deselectRowAtIndexPath_animated(index_path, true) };
                    if self.ivars().selectable.get() {
                        emit(self.ivars().node, Event::SelectionChanged(row as i64));
                    }
                });
            }

            // --- swipe-to-delete (docs/list.md). `UISwipeActionsConfiguration` is the modern
            // spelling: it gives the full native UX — the row tracking the finger, the red
            // action revealing behind it, the full-swipe shortcut — where the older
            // `commitEditingStyle` pair only offered a fixed Delete button. Returning `None`
            // means this row has no swipe action, which is exactly how a guarded row declines.
            #[unsafe(method_id(tableView:trailingSwipeActionsConfigurationForRowAtIndexPath:))]
            fn trailing_swipe_actions(
                &self,
                tv: &objc2_ui_kit::UITableView,
                index_path: &objc2_foundation::NSIndexPath,
            ) -> Option<Retained<objc2_ui_kit::UISwipeActionsConfiguration>> {
                // The whole body runs inside a closure: `define_class!`'s `method_id` return
                // shim leaves no room for an early `return None`, but a closure gives `?` back.
                // `contain` doubles as the invoker (§8.5) — the guard seam runs an app closure,
                // and a panic degrades to "no swipe action".
                let body = || -> Option<Retained<objc2_ui_kit::UISwipeActionsConfiguration>> {
                    let row = unsafe { index_path.row() } as usize;
                    let del = self
                        .ivars()
                        .source
                        .borrow()
                        .as_ref()
                        .and_then(|s| s.delete.clone())?;
                    // A guarded row offers NO action rather than one that fails on use.
                    if !(del.can_delete)(row) {
                        return None;
                    }
                    let mtm = MainThreadMarker::new()?;
                    let label = self.ivars().delete_label.borrow().clone();
                    let title = (!label.is_empty()).then(|| NSString::from_str(&label));
                    let tv: Retained<objc2_ui_kit::UITableView> = Retained::from(tv);
                    let path: Retained<objc2_foundation::NSIndexPath> = Retained::from(index_path);
                    let handler = block2::RcBlock::new(
                        move |_a: NonNull<objc2_ui_kit::UIContextualAction>,
                              _v: NonNull<UIView>,
                              done: NonNull<block2::DynBlock<dyn Fn(objc2::runtime::Bool)>>| {
                            // Commit through the seam FIRST — it shortens Day's snapshot
                            // synchronously — then let the table animate the row away. Deleting
                            // the row natively (rather than reloading) keeps the swipe's own
                            // animation continuous into the removal.
                            (del.delete_row)(row);
                            let paths =
                                objc2_foundation::NSArray::from_retained_slice(std::slice::from_ref(&path));
                            unsafe {
                                tv.deleteRowsAtIndexPaths_withRowAnimation(
                                    &paths,
                                    objc2_ui_kit::UITableViewRowAnimation::Automatic,
                                );
                                // Report the action finished; the row is gone, so the swipe must
                                // not spring back.
                                done.as_ref().call((objc2::runtime::Bool::YES,));
                            }
                        },
                    );
                    let action = unsafe {
                        objc2_ui_kit::UIContextualAction::contextualActionWithStyle_title_handler(
                            objc2_ui_kit::UIContextualActionStyle::Destructive,
                            title.as_deref(),
                            block2::RcBlock::as_ptr(&handler),
                            mtm,
                        )
                    };
                    // No app label ⇒ the wordless idiom: a trash glyph, legible in every locale.
                    if title.is_none()
                        && let Some(img) =
                            objc2_ui_kit::UIImage::systemImageNamed(&NSString::from_str("trash"))
                    {
                        unsafe { action.setImage(Some(&img)) };
                    }
                    Some(
                        objc2_ui_kit::UISwipeActionsConfiguration::configurationWithActions(
                            &objc2_foundation::NSArray::from_retained_slice(&[action]),
                            mtm,
                        ),
                    )
                };
                day_spec::ffi_guard::contain(None, body)
            }

            // The guard's live veto/override: UIKit proposes a landing slot while the finger
            // moves; returning the source path refuses it (the gap stays home), another path
            // retargets it — the affordance mirrors the app's answer before the drop.
            #[unsafe(method_id(tableView:targetIndexPathForMoveFromRowAtIndexPath:toProposedIndexPath:))]
            fn target_for_move(
                &self,
                _tv: &objc2_ui_kit::UITableView,
                from_path: &objc2_foundation::NSIndexPath,
                proposed: &objc2_foundation::NSIndexPath,
            ) -> Retained<objc2_foundation::NSIndexPath> {
                // (Closure body: define_class converts only the tail expression.)
                let target = || {
                    let (from, to) = unsafe { (from_path.row() as usize, proposed.row() as usize) };
                    let accepted = self.reorder_verdict(from, to);
                    if accepted < 0 {
                        return from_path.retain();
                    }
                    if accepted as usize == to {
                        return proposed.retain();
                    }
                    objc2_foundation::NSIndexPath::indexPathForRow_inSection(
                        accepted as isize,
                        proposed.section(),
                    )
                };
                // Contained (§8.5): the verdict runs the app's guard closure; a panic refuses
                // the move (the source path keeps the gap home).
                day_spec::ffi_guard::contain(from_path.retain(), target)
            }
        }

        // The drag delegate that lets rows lift WITHOUT editing mode (docs/list.md): one drag
        // item with an empty provider — nothing leaves the table; UIKit treats it as a local
        // reorder and drives the data-source move above.
        unsafe impl UITableViewDragDelegate for DayListData {
            #[unsafe(method_id(tableView:itemsForBeginningDragSession:atIndexPath:))]
            fn items_for_drag(
                &self,
                _tv: &objc2_ui_kit::UITableView,
                _session: &ProtocolObject<dyn objc2_ui_kit::UIDragSession>,
                index_path: &objc2_foundation::NSIndexPath,
            ) -> Retained<objc2_foundation::NSArray<objc2_ui_kit::UIDragItem>> {
                // (Closure body: define_class converts only the tail expression.)
                let items = || {
                    let row = unsafe { index_path.row() } as usize;
                    if self.reorder_verdict(row, row) < 0 {
                        return objc2_foundation::NSArray::new();
                    }
                    let provider = objc2_foundation::NSItemProvider::new();
                    let item = unsafe {
                        objc2_ui_kit::UIDragItem::initWithItemProvider(
                            objc2_ui_kit::UIDragItem::alloc(self.mtm()),
                            &provider,
                        )
                    };
                    objc2_foundation::NSArray::from_retained_slice(&[item])
                };
                // Contained (§8.5): the verdict runs the app's guard closure; a panic refuses
                // the lift (no drag items).
                day_spec::ffi_guard::contain(objc2_foundation::NSArray::new(), items)
            }
        }
    );

    impl DayListData {
        fn new(
            mtm: MainThreadMarker,
            node: NodeId,
            selectable: bool,
            row_height: f64,
            delete_label: String,
        ) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(ListIvars {
                node,
                source: RefCell::new(None),
                row_height: std::cell::Cell::new(row_height),
                selectable: std::cell::Cell::new(selectable),
                delete_label: RefCell::new(delete_label),
            });
            unsafe { msg_send![super(this), init] }
        }

        /// The guard's verdict for `from -> to` through the sync seam (accepted index, or -1 —
        /// also -1 when the list has no reorder seam at all).
        fn reorder_verdict(&self, from: usize, to: usize) -> i64 {
            self.ivars()
                .source
                .borrow()
                .as_ref()
                .and_then(|s| s.reorder.as_ref().map(|r| (r.can_move)(from, to)))
                .unwrap_or(-1)
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
        /// Canvas view ptr → its display list. Swept on release via `day_spec::sidetable` —
        /// a stale entry made a NEW DayCanvasView at a dead canvas's recycled address replay
        /// the old display list until its first `replay`.
        static OPS: day_spec::sidetable::SideTable<Vec<day_spec::DrawOp>> =
            day_spec::sidetable::SideTable::new();
        /// Canvas view ptr → its node, so the view's own key handling knows who to report to
        /// (docs/menus.md). Every canvas is registered at realize: focus, not a gesture, is
        /// what decides who hears a key.
        static KEY_NODES: day_spec::sidetable::SideTable<NodeId> =
            day_spec::sidetable::SideTable::new();
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
                let ops = OPS.with(|t| t.get(ptr)).unwrap_or_default();
                for op in &ops {
                    draw_op(op);
                }
            }

            // Focus, and with it a hardware keyboard's arrows (docs/menus.md, docs/focus.md).
            // A plain UIView is never first responder, so nothing an app DRAWS could hear a
            // key. Focus on iOS is also the software keyboard — but a canvas has no text input
            // to raise one, so becoming first responder here costs nothing on a touch-only
            // device and buys the arrows on iPad with a keyboard attached.
            #[unsafe(method(canBecomeFirstResponder))]
            fn can_become_first_responder(&self) -> bool {
                true
            }

            #[unsafe(method(becomeFirstResponder))]
            fn become_first_responder(&self) -> bool {
                let became: bool = unsafe { msg_send![super(self), becomeFirstResponder] };
                if became {
                    let ptr = (self as *const DayCanvasView).cast::<UIView>() as usize;
                    if let Some(node) = KEY_NODES.with(|t| t.get(ptr)) {
                        day_spec::ffi_guard::contain((), || emit(node, Event::FocusChanged(true)));
                    }
                }
                became
            }

            #[unsafe(method(resignFirstResponder))]
            fn resign_first_responder(&self) -> bool {
                let resigned: bool = unsafe { msg_send![super(self), resignFirstResponder] };
                if resigned {
                    let ptr = (self as *const DayCanvasView).cast::<UIView>() as usize;
                    if let Some(node) = KEY_NODES.with(|t| t.get(ptr)) {
                        day_spec::ffi_guard::contain((), || emit(node, Event::FocusChanged(false)));
                    }
                }
                resigned
            }

            // A touch focuses the canvas, the way a press does on the desktops. The gesture
            // recognizers still see it: this runs before `super`, which forwards to them.
            #[unsafe(method(touchesBegan:withEvent:))]
            fn touches_began(
                &self,
                touches: &objc2_foundation::NSSet<objc2_ui_kit::UITouch>,
                event: Option<&objc2_ui_kit::UIEvent>,
            ) {
                if !self.isFirstResponder() {
                    let _ = self.becomeFirstResponder();
                }
                let _: () = unsafe { msg_send![super(self), touchesBegan: touches, withEvent: event] };
            }

            /// Hardware-keyboard presses while this canvas is first responder. Anything that is
            /// not a claimed arrow goes to `super`, which walks the responder chain exactly as
            /// it would have — so a key nobody wanted still reaches whatever else wants it.
            #[unsafe(method(pressesBegan:withEvent:))]
            fn presses_began(
                &self,
                presses: &objc2_foundation::NSSet<objc2_ui_kit::UIPress>,
                event: Option<&objc2_ui_kit::UIPressesEvent>,
            ) {
                let ptr = (self as *const DayCanvasView).cast::<UIView>() as usize;
                let handled = day_spec::ffi_guard::contain(false, || {
                    let Some(node) = KEY_NODES.with(|t| t.get(ptr)) else {
                        return false;
                    };
                    if !day_spec::keys::handled(node) {
                        return false;
                    }
                    let mut any = false;
                    for press in presses.iter() {
                        let Some(key) = (unsafe { press.key(self.mtm()) }) else {
                            continue;
                        };
                        let Some(name) = arrow_key_name(unsafe { key.keyCode() }) else {
                            continue;
                        };
                        emit(
                            node,
                            Event::Key(day_spec::KeyEvent {
                                key: name.to_string(),
                                modifiers: key_modifiers(unsafe { key.modifierFlags() }),
                            }),
                        );
                        any = true;
                    }
                    any
                });
                if !handled {
                    let _: () =
                        unsafe { msg_send![super(self), pressesBegan: presses, withEvent: event] };
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

    /// Put a [`day_spec::props::ButtonStyleSpec`] on a `UIButton`, keeping it a UIButton.
    ///
    /// Bordered / Prominent map to `UIButtonConfiguration` tiers (iOS 15+) — the plain system
    /// button reads as a LINK, not a button. A tint is the FILLED configuration with
    /// `baseBackgroundColor`, so UIKit still draws the press dimming, the disabled state and the
    /// focus/pointer effects itself.
    ///
    /// A configured button takes its title from the configuration, so the title is set on
    /// whichever path this takes.
    fn apply_button_style(
        btn: &UIButton,
        title: &str,
        style: day_spec::props::ButtonStyleSpec,
        mtm: MainThreadMarker,
    ) {
        use day_spec::props::ButtonStyleSpec as S;
        use objc2_ui_kit::UIButtonConfiguration;
        unsafe {
            let config = match style {
                S::Automatic => {
                    btn.setConfiguration(None);
                    btn.setTitle_forState(Some(&NSString::from_str(title)), UIControlState::Normal);
                    return;
                }
                S::Prominent => UIButtonConfiguration::borderedProminentButtonConfiguration(mtm),
                S::Bordered => UIButtonConfiguration::borderedButtonConfiguration(mtm),
                S::Tinted(c) => {
                    let config = UIButtonConfiguration::filledButtonConfiguration(mtm);
                    config.setBaseBackgroundColor(Some(&uicolor(c)));
                    config.setBaseForegroundColor(Some(&uicolor(S::on_tint(c))));
                    config
                }
            };
            config.setTitle(Some(&NSString::from_str(title)));
            btn.setConfiguration(Some(&config));
        }
    }

    fn uicolor(c: day_spec::Color) -> Retained<UIColor> {
        unsafe { UIColor::colorWithRed_green_blue_alpha(c.r, c.g, c.b, c.a) }
    }

    /// Apply a `background`/`corner_radius` surface to a container view: UIView carries a native
    /// `backgroundColor`; the corner radius / rounded clip go on its typed CALayer
    /// (objc2-quartz-core). Idempotent — called at realize and on a background patch.
    fn apply_surface(v: &UIView, bg: Option<day_spec::Color>, corner_radius: f64, clips: bool) {
        unsafe {
            match bg {
                Some(c) => v.setBackgroundColor(Some(&uicolor(c))),
                None => v.setBackgroundColor(None),
            }
            let layer = v.layer();
            layer.setCornerRadius(corner_radius);
            layer.setMasksToBounds(clips || corner_radius > 0.0);
        }
    }

    fn cg(r: day_spec::Rect) -> CGRect {
        CGRect::new(
            CGPoint::new(r.origin.x, r.origin.y),
            CGSize::new(r.size.width, r.size.height),
        )
    }

    /// Put a [`day_spec::StrokeStyle`] onto a path: width always, and dash/cap/join/miter only
    /// when they differ from the defaults, so a plain stroke costs no extra messages.
    fn apply_stroke_style(p: &objc2_ui_kit::UIBezierPath, style: &day_spec::StrokeStyle) {
        use day_spec::{LineCap, LineJoin};
        unsafe {
            p.setLineWidth(style.width);
            if style.is_plain() {
                return;
            }
            p.setLineCapStyle(match style.cap {
                LineCap::Butt => objc2_core_graphics::CGLineCap::Butt,
                LineCap::Round => objc2_core_graphics::CGLineCap::Round,
                LineCap::Square => objc2_core_graphics::CGLineCap::Square,
            });
            p.setLineJoinStyle(match style.join {
                LineJoin::Miter => objc2_core_graphics::CGLineJoin::Miter,
                LineJoin::Round => objc2_core_graphics::CGLineJoin::Round,
                LineJoin::Bevel => objc2_core_graphics::CGLineJoin::Bevel,
            });
            p.setMiterLimit(style.miter_limit);
            if !style.dash.is_empty() {
                let pattern: Vec<CGFloat> = style.dash.iter().map(|d| *d as CGFloat).collect();
                p.setLineDash_count_phase(
                    pattern.as_ptr(),
                    pattern.len() as isize,
                    style.dash_phase,
                );
            }
        }
    }

    /// Draw a gradient through whatever clip is currently installed. Shared by the gradient
    /// FILL arms and the gradient STROKE arm, which differ only in what they clip to first.
    fn draw_gradient_in(ctx: &CGContext, paint: &day_spec::Paint, bounds: day_spec::Rect) {
        let opts = objc2_core_graphics::CGGradientDrawingOptions::DrawsBeforeStartLocation
            | objc2_core_graphics::CGGradientDrawingOptions::DrawsAfterEndLocation;
        unsafe {
            match paint {
                day_spec::Paint::Linear(g) => {
                    let Some(grad) = cggradient(&g.stops) else {
                        return;
                    };
                    let (s, e) = (g.start.resolve(bounds), g.end.resolve(bounds));
                    CGContext::draw_linear_gradient(
                        Some(ctx),
                        Some(&grad),
                        CGPoint::new(s.x, s.y),
                        CGPoint::new(e.x, e.y),
                        opts,
                    );
                }
                day_spec::Paint::Radial(g) => {
                    let Some(grad) = cggradient(&g.stops) else {
                        return;
                    };
                    CGContext::save_g_state(Some(ctx));
                    CGContext::translate_ctm(Some(ctx), bounds.origin.x, bounds.origin.y);
                    CGContext::scale_ctm(Some(ctx), bounds.size.width, bounds.size.height);
                    let c = CGPoint::new(g.center.x, g.center.y);
                    CGContext::draw_radial_gradient(
                        Some(ctx),
                        Some(&grad),
                        c,
                        0.0,
                        c,
                        g.radius,
                        opts,
                    );
                    CGContext::restore_g_state(Some(ctx));
                }
                day_spec::Paint::Solid(_) => {}
            }
        }
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
                DrawOp::Stroke(shape, paint, style) => {
                    let Some(p) = bezier(shape) else { return };
                    apply_stroke_style(&p, style);
                    match paint {
                        day_spec::Paint::Solid(color) => {
                            uicolor(*color).setStroke();
                            p.stroke();
                        }
                        // A gradient stroke has no CoreGraphics primitive: convert the stroke
                        // to the region it covers (`CGPathCreateCopyByStrokingPath` via
                        // `bezierPathByStrokingPath` is unavailable here), clip to it, and draw
                        // the gradient through. `replacePathWithStrokedPath` on the context is
                        // the documented way to get exactly that region.
                        day_spec::Paint::Linear(_) | day_spec::Paint::Radial(_) => {
                            let Some(ctx) = objc2_ui_kit::UIGraphicsGetCurrentContext() else {
                                return;
                            };
                            CGContext::save_g_state(Some(&ctx));
                            CGContext::add_path(Some(&ctx), Some(&p.CGPath()));
                            CGContext::replace_path_with_stroked_path(Some(&ctx));
                            CGContext::clip(Some(&ctx));
                            draw_gradient_in(&ctx, paint, shape.bounds());
                            CGContext::restore_g_state(Some(&ctx));
                        }
                    }
                }
                DrawOp::Clip(shape) => {
                    // `addClip` INTERSECTS with the context's current clip and reads the
                    // path's own even-odd flag, which is exactly Day's contract.
                    if let Some(p) = bezier(shape) {
                        p.addClip();
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
                Shape::Path(path) => {
                    use day_spec::PathSeg;
                    if path.segs.is_empty() {
                        return None;
                    }
                    let p = UIBezierPath::bezierPath();
                    for seg in &path.segs {
                        match seg {
                            PathSeg::Move(a) => p.moveToPoint(CGPoint::new(a.x, a.y)),
                            PathSeg::Line(a) => p.addLineToPoint(CGPoint::new(a.x, a.y)),
                            PathSeg::Quad(c, a) => p.addQuadCurveToPoint_controlPoint(
                                CGPoint::new(a.x, a.y),
                                CGPoint::new(c.x, c.y),
                            ),
                            PathSeg::Cubic(c1, c2, a) => p
                                .addCurveToPoint_controlPoint1_controlPoint2(
                                    CGPoint::new(a.x, a.y),
                                    CGPoint::new(c1.x, c1.y),
                                    CGPoint::new(c2.x, c2.y),
                                ),
                            PathSeg::Close => p.closePath(),
                        }
                    }
                    // The fill rule travels ON the path, which is also how `addClip` reads it.
                    p.setUsesEvenOddFillRule(path.rule == day_spec::FillRule::EvenOdd);
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

    /// Rasterize this app's own window to PNG (docs/window-image.md).
    ///
    /// `UIGraphicsImageRenderer` + `drawViewHierarchyInRect:afterScreenUpdates:` — the standard iOS
    /// way, and SYNCHRONOUS, which is what lets `day::window_image()` stay a plain call on every
    /// backend. `afterScreenUpdates: true` so a capture taken right after a state change shows the
    /// change rather than the frame before it.
    ///
    /// `chrome` picks the whole window over Day's content view; both are views in the same tree.
    fn snapshot_uikit(chrome: bool) -> Result<Vec<u8>, String> {
        let view: Retained<UIView> = with_key_scene(|e| {
            if chrome {
                Retained::from(&*e.window as &UIView)
            } else {
                e.root_view.clone()
            }
        })
        .ok_or("no window to capture")?;
        snapshot_view(&view)
    }

    /// Rasterize one view (a window's content container — the primary root, or a secondary
    /// "window"'s host view, which on iOS is a fullscreen cover's content).
    fn snapshot_view(view: &UIView) -> Result<Vec<u8>, String> {
        // A view outside any window cannot be drawn with `afterScreenUpdates: true`: UIKit
        // moves it into a temporary window to force the commit, and when the view is
        // controller-backed its hierarchy check raises an NSException — which is foreign to
        // Rust and aborts the process. This happens to the PRIMARY root while a fullscreen
        // cover is presented (UIKit detaches the underlay), so refuse rather than raise.
        if unsafe { view.window() }.is_none() {
            return Err("view is not in a window".into());
        }
        let bounds = view.bounds();
        if bounds.size.width <= 0.0 || bounds.size.height <= 0.0 {
            return Err("zero-size window".into());
        }
        // SAFETY: main thread (a Toolkit duty); the renderer draws synchronously inside the block.
        let image: Retained<objc2_ui_kit::UIImage> = unsafe {
            use objc2::AllocAnyThread as _;
            let renderer = objc2_ui_kit::UIGraphicsImageRenderer::initWithBounds(
                objc2_ui_kit::UIGraphicsImageRenderer::alloc(),
                bounds,
            );
            let v: Retained<UIView> = Retained::from(view);
            let block = block2::RcBlock::new(
                move |_ctx: core::ptr::NonNull<objc2_ui_kit::UIGraphicsImageRendererContext>| {
                    v.drawViewHierarchyInRect_afterScreenUpdates(v.bounds(), true);
                },
            );
            renderer.imageWithActions(&*block as *const _ as *mut _)
        };
        let data = image.png_representation().ok_or("png encode failed")?;
        Ok(data.to_vec())
    }

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

    // The expect is an invariant, not a runtime failure mode: every Toolkit duty runs on the
    // main thread by contract (§8.1).
    pub(crate) fn mtm() -> MainThreadMarker {
        MainThreadMarker::new().expect("day-uikit: not on the main thread")
    }

    impl Uikit {
        /// The main-thread marker every UIKit call needs, for a standalone piece's renderer
        /// (docs/extending.md) — the same accessor day-appkit offers. Sound for the same reason
        /// the free function above is: holding `&mut Uikit` means a Toolkit duty is running.
        pub fn mtm(&self) -> MainThreadMarker {
            mtm()
        }
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

    /// Resolve a [`day_spec::FontSpec`] to its concrete `UIFont` — semantic style, weight,
    /// italic, tabular figures, all Dynamic Type scaled. Shared by the `UILabel` path and the
    /// read-only `UITextView` a `.selectable()` label swaps to (`set_selectable`).
    fn resolve_font(spec: day_spec::FontSpec) -> Retained<objc2_ui_kit::UIFont> {
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
                        log::warn!(
                            "unknown font family {name:?} — falling back to the system \
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
        // Tabular figures: UIKit exposes them as a whole font (like AppKit), so re-pick the
        // system face at the resolved size/weight. System styles only — a bundled family keeps
        // its own figures rather than being silently swapped for the system typeface. The result
        // still goes through UIFontMetrics below, so Dynamic Type keeps working.
        let font = if spec.tabular && !matches!(spec.style, Font::Custom(..)) {
            unsafe {
                let w = spec.weight.map(ui_weight).unwrap_or(UIFontWeightRegular);
                let raw = UIFont::monospacedDigitSystemFontOfSize_weight(font.pointSize(), w);
                UIFontMetrics::metricsForTextStyle(UIFontTextStyleBody).scaledFontForFont(&raw)
            }
        } else {
            font
        };
        // Monospace, by the same rule and for the same reason (docs/text-runs.md): a whole face
        // on UIKit too, kept inside UIFontMetrics so inline code still scales with Dynamic Type.
        let font = if spec.monospace && !matches!(spec.style, Font::Custom(..)) {
            unsafe {
                let w = spec.weight.map(ui_weight).unwrap_or(UIFontWeightRegular);
                let raw = UIFont::monospacedSystemFontOfSize_weight(font.pointSize(), w);
                UIFontMetrics::metricsForTextStyle(UIFontTextStyleBody).scaledFontForFont(&raw)
            }
        } else {
            font
        };
        // Relative size (`FontSpec::scale`), applied LAST over whatever face the traits settled
        // on. `fontWithSize:` keeps the typeface, and because the size it scales is the one
        // Dynamic Type already produced, a scaled run keeps tracking the reader's setting.
        if spec.scale != 1.0 {
            unsafe { font.fontWithSize(spec.resolved_points(font.pointSize())) }
        } else {
            font
        }
    }

    /// Build a `UILabel`'s attributed text from its runs (docs/text-runs.md).
    ///
    /// Byte ranges convert to UTF-16 per run: `NSAttributedString` indexes UTF-16, and any
    /// emoji or CJK in the string makes the two disagree.
    fn attributed_label(
        text: &str,
        base_font: &objc2_ui_kit::UIFont,
        color: Option<day_spec::Color>,
        runs: &[day_spec::TextRun],
    ) -> Retained<objc2_foundation::NSAttributedString> {
        use objc2::AllocAnyThread as _;
        use objc2_foundation::{NSMutableAttributedString, NSRange};
        let ns = NSString::from_str(text);
        let s = unsafe {
            NSMutableAttributedString::initWithString(NSMutableAttributedString::alloc(), &ns)
        };
        let whole = NSRange::new(0, ns.length());
        unsafe {
            s.addAttribute_value_range(objc2_ui_kit::NSFontAttributeName, base_font, whole);
            // ALWAYS a foreground: a UITextView draws an attributed range with no color
            // attribute in black, which is invisible in dark mode. `labelColor` is the adaptive
            // default a plain label would have used.
            let fg = color.map(uicolor).unwrap_or_else(UIColor::labelColor);
            s.addAttribute_value_range(objc2_ui_kit::NSForegroundColorAttributeName, &fg, whole);
        }
        for r in runs {
            let Some(range) = utf16_range(text, &r.range) else {
                continue;
            };
            unsafe {
                s.addAttribute_value_range(
                    objc2_ui_kit::NSFontAttributeName,
                    &resolve_font(r.font),
                    range,
                );
                if let Some(c) = r.color {
                    s.addAttribute_value_range(
                        objc2_ui_kit::NSForegroundColorAttributeName,
                        &uicolor(c),
                        range,
                    );
                }
                if let Some(c) = r.background {
                    s.addAttribute_value_range(
                        objc2_ui_kit::NSBackgroundColorAttributeName,
                        &uicolor(c),
                        range,
                    );
                }
                if r.underline.is_on() {
                    let style = objc2_foundation::NSNumber::new_i64(ns_underline(r.underline));
                    s.addAttribute_value_range(
                        objc2_ui_kit::NSUnderlineStyleAttributeName,
                        &style,
                        range,
                    );
                }
                if r.strikethrough {
                    let one = objc2_foundation::NSNumber::new_i64(1);
                    s.addAttribute_value_range(
                        objc2_ui_kit::NSStrikethroughStyleAttributeName,
                        &one,
                        range,
                    );
                }
                if let Some(url) = r.link.as_deref() {
                    // Drawn as a link. ACTIVATION needs a UITextView (a UILabel has no hit
                    // testing at all), which is Phase 4 — `Cap::TextLinks` stays Unsupported.
                    let value = NSString::from_str(url);
                    s.addAttribute_value_range(objc2_ui_kit::NSLinkAttributeName, &value, range);
                }
            }
        }
        s.into_super()
    }

    /// [`Underline`](day_spec::Underline) as an `NSUnderlineStyle` bitmask — the line style in
    /// the low byte, the pattern in the second, the same encoding AppKit uses.
    fn ns_underline(u: day_spec::Underline) -> i64 {
        use day_spec::Underline as U;
        match u {
            U::None => 0,
            U::Single => 0x01,
            U::Double => 0x09,
            U::Dotted => 0x01 | 0x0100,
            U::Wavy => 0x01 | 0x0400,
        }
    }

    /// A byte range in `text` as an `NSRange` in UTF-16 units.
    fn utf16_range(text: &str, r: &std::ops::Range<usize>) -> Option<objc2_foundation::NSRange> {
        let start = text.get(..r.start)?.encode_utf16().count();
        let len = text.get(r.clone())?.encode_utf16().count();
        Some(objc2_foundation::NSRange::new(start, len))
    }

    fn apply_font(label: &UILabel, spec: day_spec::FontSpec) {
        let font = resolve_font(spec);
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
        day_spec::placeholder::report(kind, "uikit");
    }

    /// The visible placeholder for a kind this backend cannot realize — no registered
    /// renderer, or a props payload of the wrong type (`day_spec::props_of` reports the
    /// mismatch before the arm degrades here).
    pub(crate) fn placeholder_view(kind: PieceKind) -> Handle {
        let label = unsafe { UILabel::new(mtm()) };
        unsafe { label.setText(Some(&NSString::from_str(&format!("⟨{kind}⟩")))) };
        view_of(label)
    }

    impl Toolkit for Uikit {
        type Handle = Handle;

        /// iOS badges are NUMBERS ONLY, and they are part of the notification grant: without the
        /// user allowing notifications the count is simply not drawn (docs/badge.md).
        ///
        /// `UNUserNotificationCenter.setBadgeCount:` (iOS 16+) rather than the deprecated
        /// `UIApplication.applicationIconBadgeNumber`. Hand-rolled through `msg_send!` on two
        /// selectors instead of taking `objc2-user-notifications` as a toolkit dependency — the
        /// same budget `day-part-permissions` keeps for this exact class.
        fn set_app_badge(&mut self, badge: &day_spec::AppBadge) {
            use day_spec::AppBadge;
            let count: isize = match badge {
                AppBadge::None => 0,
                AppBadge::Count(n) => *n as isize,
                // No text and no dot on iOS. Substituting a number here would invent a value the
                // caller never asked for, so these clear instead (Cap says they are unsupported).
                AppBadge::Text(_) | AppBadge::Dot => return,
            };
            let Some(cls) = objc2::runtime::AnyClass::get(c"UNUserNotificationCenter") else {
                return;
            };
            unsafe {
                let center: *mut objc2::runtime::AnyObject =
                    msg_send![cls, currentNotificationCenter];
                if center.is_null() {
                    return;
                }
                // A nil completion handler is allowed; a failure surfaces in the system log, and
                // there is nothing the caller could do with it synchronously.
                let _: () = msg_send![center, setBadgeCount: count, withCompletionHandler: std::ptr::null::<objc2::runtime::AnyObject>()];
            }
        }

        fn capability(&self, cap: Cap) -> Support {
            match cap {
                // UIGraphicsImageRenderer draws this app's own window into a bitmap
                // (docs/window-image.md).
                // A label carrying a link run is built as a read-only UITextView, whose delegate
                // reports the tap — a UILabel could draw the link but never hit-test it
                // (docs/text-runs.md).
                Cap::TextRuns
                | Cap::TextLinks
                // NSUndoManager fronted through the root VC's responder chain: three-finger
                // gestures, shake-to-undo, hardware ⌘Z and the iPad menu bar all land
                // (docs/model.md).
                | Cap::UndoBridge
                | Cap::Snapshot
                // UITextView natively honors editable / selectable / spell-check.
                | Cap::Dialogs
                | Cap::FileDialogs
                | Cap::EditBridge
                | Cap::Animation
                | Cap::Cover
                // Every page rides a UINavigationController, whose UINavigationBar names the
                // destination — content needn't repeat the title (docs/navigation.md).
                | Cap::NavHeader
                | Cap::TextEditable
                // A number on the home-screen icon, gated on the notification grant
                // (docs/badge.md). Text and Dot have no iOS equivalent.
                | Cap::AppBadgeCount
                | Cap::TextSelectable
                | Cap::TextSpellCheck
                // UITableView's own drag pipeline: long-press lift + gap, no editing mode.
                | Cap::ListReorder
                // Trailing swipe actions: the row tracks the finger, the destructive action
                // reveals behind it, and a full swipe commits (docs/list.md).
                | Cap::ListDelete
                // A `UISplitViewController` hosts every `selector(Sidebar)`, so two columns are
                // available wherever the window is wide enough — an iPad, and a Plus/Pro Max
                // iPhone in landscape (docs/size-classes.md).
                // `.tabSidebar` (docs/navigation.md): ONE `UITabBarController` that draws a tab
                // bar when compact and a sidebar when not — what SwiftUI's `.sidebarAdaptable`
                // compiles down to, and the container adaptive navigation exists for.
                | Cap::NavTabs
                | Cap::NavTabsAdaptive
                | Cap::NavSplit
                | Cap::Appearance => Support::Native,
                // Derived from the control's font: UIKit publishes baselines only as constraint
                // anchors, with no number to read (docs/baseline.md).
                Cap::BaselineAlignment => Support::Emulated,
                // EMULATED, and the distinction is the whole design: UIKit owns the collapse and
                // expand, on its own schedule and with its own animation, so Day observes it
                // through `Event::NavPresentationChanged` rather than pushing a presentation
                // into it (docs/size-classes.md).
                Cap::NavRepresent => Support::Emulated,
                // Real UIScenes on iPad (docs/windows.md); iPhone shows one scene, so the
                // cover fallback is the honest answer there.
                Cap::MultiWindow => {
                    let app = UIApplication::sharedApplication(mtm());
                    if unsafe { app.supportsMultipleScenes() } {
                        Support::Native
                    } else {
                        Support::Unsupported
                    }
                }
                _ => Support::Unsupported,
            }
        }

        fn realize(&mut self, kind: PieceKind, props: &dyn Any, id: NodeId) -> Handle {
            let mtm = mtm();
            match Builtin::from_key(kind) {
                Some(Builtin::Container) => {
                    let v = unsafe { UIView::new(mtm) };
                    // A mismatched payload still yields a usable (undecorated) container;
                    // `props_of` reports it.
                    if let Some(p) = day_spec::props_of::<ContainerProps>(kind, "uikit", props) {
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
                Some(Builtin::Nav) => {
                    // Mismatched props degrade to the placeholder rather than panicking in a
                    // native up-call (§8.5); `props_of` reports once per kind. Same pattern on
                    // every arm below.
                    let Some(p) = day_spec::props_of::<NavProps>(kind, "uikit", props) else {
                        return placeholder_view(kind);
                    };
                    // Resolve the optional trailing bar action once (docs/navigation.md): downscale
                    // the shared 96px asset to a bar-sized template glyph (tints with the bar), and
                    // retain one target the per-page items reuse. Applied in `insert` as pages join.
                    let bar_actions: Vec<NavBarButton> = p
                        .bar_actions
                        .iter()
                        .map(|a| {
                            let image =
                                a.icon.as_deref().and_then(load_bundled_uiimage).map(|img| {
                                    let sized = unsafe {
                                        img.imageByPreparingThumbnailOfSize(CGSize::new(24.0, 24.0))
                                    }
                                    .unwrap_or(img);
                                    unsafe {
                                        sized.imageWithRenderingMode(
                                            objc2_ui_kit::UIImageRenderingMode::AlwaysTemplate,
                                        )
                                    }
                                });
                            NavBarButton {
                                image,
                                label: a.label.clone(),
                                scope: a.scope,
                                target: DayBarButtonTarget::new(mtm, id, a.action),
                            }
                        })
                        .collect();
                    let nav = DayNavController::new(mtm, 0); // host ptr set just below
                    // Child-VC containment under the window's root VC (v1: app root).
                    let root_vc = WINDOW
                        .with(|w| w.borrow().clone())
                        .and_then(|w| w.rootViewController());
                    // `presentation: Stack` in props means a stack at EVERY size — a nested
                    // `stack()` under a split host (docs/size-classes.md) — realized as a PLAIN
                    // navigation controller. A `UISplitViewController` assumes it owns the
                    // window; nested inside a detail pane its column layout collapses into
                    // garbage (the embedded-split trap), which is exactly what a pane-sized
                    // gray void looked like.
                    // An ADAPTIVE TABS host (docs/navigation.md): `.tabSidebar` is the container
                    // that wears both chromes itself, so there is no split to build and no
                    // presentation for Day to drive — the controller decides, and reports once.
                    if p.presentation == day_spec::props::NavPresentation::Tabs {
                        let tabbar = unsafe { UITabBarController::new(mtm) };
                        unsafe {
                            tabbar.setMode(objc2_ui_kit::UITabBarControllerMode::TabSidebar);
                        }
                        if let Some(root_vc) = &root_vc {
                            unsafe {
                                root_vc.addChildViewController(&tabbar);
                                tabbar.didMoveToParentViewController(Some(root_vc));
                            }
                        }
                        let host = view_of(unsafe { tabbar.view() }.expect("tabbar view"));
                        let hp = ptr_of(&host);
                        let delegate = DayNavTabsDelegate::new(mtm, hp);
                        unsafe { tabbar.setDelegate(Some(ProtocolObject::from_ref(&*delegate))) };
                        NAV_TABS.with(|m| {
                            m.borrow_mut().insert(
                                hp,
                                NavTabsState {
                                    tabbar,
                                    vcs: Vec::new(),
                                    titles: Vec::new(),
                                    icons: Vec::new(),
                                    menu_node: std::cell::Cell::new(0),
                                    _delegate: delegate,
                                },
                            )
                        });
                        // Tell Day this host is a tabs host and stays one. Its pages are resident
                        // at every width, so day-core must not flip to push/pop as the window
                        // widens — UIKit swaps the chrome underneath without Day's help.
                        emit(
                            id,
                            Event::NavPresentationChanged(day_spec::props::NavPresentation::Tabs),
                        );
                        return host;
                    }
                    let (host, split) = if p.presentation == day_spec::props::NavPresentation::Stack
                    {
                        if let Some(root_vc) = root_vc {
                            unsafe {
                                root_vc.addChildViewController(&nav);
                                nav.didMoveToParentViewController(Some(&root_vc));
                            }
                        }
                        let host = view_of(unsafe { nav.view() }.expect("nav view"));
                        (host, None)
                    } else {
                        // The adaptive host (docs/size-classes.md): a two-column
                        // UISplitViewController whose SECONDARY column is Day's navigation stack
                        // and whose PRIMARY is the sidebar page. UIKit collapses it to a single
                        // stack at compact width and expands it at regular — which is a rotation
                        // away on a Plus/Pro Max iPhone and the standing state on an iPad.
                        //
                        // Collapsing MERGES: UIKit inserts the primary's controller at the bottom
                        // of the secondary's navigation stack. That lands on exactly the shape
                        // Day's model already has in a stack presentation — the sidebar page as
                        // the stack's root — so the phone path is unchanged and only the mirror
                        // needs rebasing.
                        let split_vc = unsafe {
                            objc2_ui_kit::UISplitViewController::initWithStyle(
                                objc2_ui_kit::UISplitViewController::alloc(mtm),
                                objc2_ui_kit::UISplitViewControllerStyle::DoubleColumn,
                            )
                        };
                        let primary_nav = DayNavController::new(mtm, 0); // host ptr set below
                        unsafe {
                            split_vc.setViewController_forColumn(
                                Some(&primary_nav),
                                objc2_ui_kit::UISplitViewControllerColumn::Primary,
                            );
                            split_vc.setViewController_forColumn(
                                Some(&nav),
                                objc2_ui_kit::UISplitViewControllerColumn::Secondary,
                            );
                            // Both columns side by side when there is room; UIKit still collapses
                            // to one at compact width.
                            split_vc.setPreferredDisplayMode(
                                objc2_ui_kit::UISplitViewControllerDisplayMode::OneBesideSecondary,
                            );
                            // TILE, explicitly. Left automatic, UIKit picks an OVERLAY on a
                            // portrait iPad: the sidebar floats above a dimmed detail, and the
                            // detail keeps the full window width — so Day lays its content out
                            // for a width the user cannot see the left edge of. Tiling gives the
                            // detail column its own narrower bounds, which is what the page then
                            // reports through `FrameChanged` (docs/size-classes.md).
                            split_vc.setPreferredSplitBehavior(
                                objc2_ui_kit::UISplitViewControllerSplitBehavior::Tile,
                            );
                        }
                        if let Some(root_vc) = root_vc {
                            unsafe {
                                root_vc.addChildViewController(&split_vc);
                                split_vc.didMoveToParentViewController(Some(&root_vc));
                            }
                        }
                        let host = view_of(unsafe { split_vc.view() }.expect("split view"));
                        primary_nav.ivars().host.set(ptr_of(&host));
                        let split_delegate = DaySplitDelegate::new(mtm, ptr_of(&host));
                        unsafe {
                            split_vc.setDelegate(Some(ProtocolObject::from_ref(&*split_delegate)))
                        };
                        (
                            host,
                            Some(SplitParts {
                                split_vc,
                                primary_nav,
                                _split_delegate: split_delegate,
                            }),
                        )
                    };
                    nav.ivars().host.set(ptr_of(&host));
                    let delegate = DayNavDelegate::new(mtm, ptr_of(&host));
                    // One delegate for both columns: they are the same Day host, and only one of
                    // them owns the stack at a time.
                    unsafe {
                        nav.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
                        if let Some(parts) = split.as_ref() {
                            parts
                                .primary_nav
                                .setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
                        }
                    }
                    // Inline search (docs/search.md): build the controller now; it is attached to
                    // the ROOT page's navigation item as that page joins the stack, which is what
                    // puts it behind the pull-down on the top-level list.
                    let search = p
                        .search
                        .as_ref()
                        .filter(|sp| sp.placement == day_spec::props::SearchPlacement::Inline)
                        .map(|sp| {
                            let updater = DaySearchUpdater::new(mtm, id);
                            let sc = unsafe {
                                objc2_ui_kit::UISearchController::initWithSearchResultsController(
                                    objc2_ui_kit::UISearchController::alloc(mtm),
                                    None,
                                )
                            };
                            unsafe {
                                sc.setSearchResultsUpdater(Some(ProtocolObject::from_ref(
                                    &*updater,
                                )));
                                // The results controller is the app's own list, not a separate
                                // one, so dimming it while typing would gray out the very rows
                                // being filtered.
                                sc.setObscuresBackgroundDuringPresentation(false);
                                let bar = sc.searchBar();
                                bar.setPlaceholder(Some(&NSString::from_str(&sp.prompt)));
                                if !sp.text.is_empty() {
                                    bar.setText(Some(&NSString::from_str(&sp.text)));
                                }
                            }
                            (sc, updater)
                        });
                    NAV_STATE.with(|m| {
                        m.borrow_mut().insert(
                            ptr_of(&host),
                            NavState {
                                nav,
                                host_node: id,
                                collapsed: std::cell::Cell::new(
                                    split
                                        .as_ref()
                                        .is_some_and(|s| unsafe { s.split_vc.isCollapsed() }),
                                ),
                                split,
                                vcs: Vec::new(),
                                native_pops: std::cell::Cell::new(0),
                                last_native: std::cell::Cell::new(0),
                                bar_actions,
                                _delegate: delegate,
                                search,
                            },
                        )
                    });
                    host
                }
                Some(Builtin::NavPage) => {
                    let Some(p) = day_spec::props_of::<NavPageProps>(kind, "uikit", props) else {
                        return placeholder_view(kind);
                    };
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
                    PAGE_PANE.with(|t| t.insert(ptr_of(&handle), p.pane));
                    handle
                }
                // Fullscreen cover (docs/cover.md): a DayCoverVC over a DayNavPageView (safe-
                // area pinning + FrameChanged reports, like a nav page), created detached;
                // CoverPatch::Present shows it modally over the whole window.
                Some(Builtin::Cover) => {
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
                Some(Builtin::NavMenu) => {
                    let Some(p) = day_spec::props_of::<NavMenuProps>(kind, "uikit", props) else {
                        return placeholder_view(kind);
                    };
                    let data = DayNavTableData::new(
                        mtm,
                        id,
                        &p.items,
                        &p.icons,
                        &p.tints,
                        &p.menus,
                        &p.badge_icons,
                        &p.badge_tints,
                    );
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
                    // Remember the rows for a `.tabSidebar` host: UIKit draws BOTH its tab bar
                    // and its sidebar from the tabs, so a selector's row labels have to reach the
                    // tabs rather than only this table (docs/navigation.md).
                    NAV_MENU_ROWS.with(|m| {
                        m.borrow_mut().insert(
                            ptr_of(&view),
                            (
                                id.0 as i64,
                                p.items.clone(),
                                p.icons
                                    .iter()
                                    .map(|n| n.as_deref().and_then(load_bundled_uiimage))
                                    .collect(),
                            ),
                        )
                    });
                    view
                }

                Some(Builtin::List) => {
                    let Some(p) = day_spec::props_of::<ListProps>(kind, "uikit", props) else {
                        return placeholder_view(kind);
                    };
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
                    let data =
                        DayListData::new(mtm, id, p.selectable, row_height, p.delete_label.clone());
                    unsafe {
                        table.setRowHeight(row_height);
                        table.setDataSource(Some(ProtocolObject::from_ref(&*data)));
                        table.setDelegate(Some(ProtocolObject::from_ref(&*data)));
                        if !p.selectable {
                            table.setAllowsSelection(false);
                        }
                        if p.reorderable {
                            // Native drag-to-reorder (docs/list.md): the drag delegate lifts a
                            // row on long-press (no editing mode); the data-source move methods
                            // + target-for-move delegate drive commit and the live guard.
                            table.setDragDelegate(Some(ProtocolObject::from_ref(&*data)));
                            table.setDragInteractionEnabled(true);
                        }
                    }
                    let view = view_of(table.clone());
                    LIST_STATE.with(|m| m.borrow_mut().insert(ptr_of(&view), (table, data)));
                    view
                }
                Some(Builtin::Scroll) => {
                    let sv = unsafe { UIScrollView::new(mtm) };
                    view_of(sv)
                }
                Some(Builtin::Label) => {
                    let Some(p) = day_spec::props_of::<LabelProps>(kind, "uikit", props) else {
                        return placeholder_view(kind);
                    };
                    // A UILabel does no hit testing at all, so a label that arrives WITH a link
                    // run is built as a read-only text view instead — the same backing
                    // `.selectable()` swaps to, and the only one UIKit can activate a link in
                    // (docs/text-runs.md). A link that first appears in a later patch cannot
                    // upgrade the backing: `patch` has no way to hand back a new handle.
                    if p.runs.iter().any(|r| r.link.is_some()) {
                        let tv = link_text_view(p, id, mtm);
                        return view_of(tv);
                    }
                    let label = unsafe { UILabel::new(mtm) };
                    unsafe {
                        label.setText(Some(&NSString::from_str(&p.text)));
                        label.setNumberOfLines(0);
                    }
                    apply_font(&label, p.font);
                    if let Some(c) = p.color {
                        unsafe { label.setTextColor(Some(&uicolor(c))) };
                    }
                    if !p.runs.is_empty() {
                        let s = attributed_label(&p.text, &resolve_font(p.font), p.color, &p.runs);
                        unsafe { label.setAttributedText(Some(&s)) };
                    }
                    view_of(label)
                }
                Some(Builtin::Button) => {
                    let Some(p) = day_spec::props_of::<ButtonProps>(kind, "uikit", props) else {
                        return placeholder_view(kind);
                    };
                    let target = DayTarget::new(mtm, id);
                    let btn = unsafe { UIButton::buttonWithType(UIButtonType::System, mtm) };
                    apply_button_style(&btn, &p.title, p.style, mtm);
                    unsafe {
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
                Some(Builtin::Toggle) => {
                    let Some(p) = day_spec::props_of::<ToggleProps>(kind, "uikit", props) else {
                        return placeholder_view(kind);
                    };
                    let target = DayTarget::new(mtm, id);
                    let sw = unsafe { UISwitch::new(mtm) };
                    unsafe {
                        sw.setOn(p.on);
                        sw.setEnabled(p.enabled);
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
                Some(Builtin::Slider) => {
                    let Some(p) = day_spec::props_of::<SliderProps>(kind, "uikit", props) else {
                        return placeholder_view(kind);
                    };
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
                        // Both lift events: a drag that ends off the track still ends.
                        sl.addTarget_action_forControlEvents(
                            Some(tobj),
                            sel!(commit:),
                            UIControlEvents::TouchUpInside | UIControlEvents::TouchUpOutside,
                        );
                    }
                    let view = view_of(sl);
                    TARGETS.with(|m| m.borrow_mut().insert(ptr_of(&view), target));
                    view
                }
                Some(Builtin::Picker) => crate::picker::realize_any(self, props, id),
                Some(Builtin::TextArea) => crate::textarea::realize_any(self, props, id),
                Some(Builtin::TextField) => {
                    let Some(p) = day_spec::props_of::<TextFieldProps>(kind, "uikit", props) else {
                        return placeholder_view(kind);
                    };
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
                Some(Builtin::Divider) => {
                    let v = unsafe { UIView::new(mtm) };
                    unsafe { v.setBackgroundColor(Some(&UIColor::separatorColor())) };
                    view_of(v)
                }
                Some(Builtin::Progress) => {
                    let Some(p) = day_spec::props_of::<ProgressProps>(kind, "uikit", props) else {
                        return placeholder_view(kind);
                    };
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
                Some(Builtin::Canvas) => {
                    let canvas = DayCanvasView::new(mtm);
                    KEY_NODES.with(|t| t.insert(Retained::as_ptr(&canvas) as usize, id));
                    view_of(canvas)
                }
                Some(Builtin::Image) => {
                    let Some(p) = day_spec::props_of::<ImageProps>(kind, "uikit", props) else {
                        return placeholder_view(kind);
                    };
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
                    // Vector-glyph tint (docs/vectors.md): template rendering + the view's tint —
                    // UIKit recolors the alpha mask natively.
                    if let Some(t) = p.tint {
                        if let Some(img) = unsafe { iv.image() } {
                            let templ = unsafe {
                                img.imageWithRenderingMode(
                                    objc2_ui_kit::UIImageRenderingMode::AlwaysTemplate,
                                )
                            };
                            unsafe { iv.setImage(Some(&templ)) };
                        }
                        unsafe { iv.setTintColor(Some(&uicolor(t))) };
                    }
                    view_of(iv)
                }
                // A recycled list cell is ADOPTED from the native list, never realized
                // through this path; anything else is an extension piece.
                Some(Builtin::ListCell)
                | Some(Builtin::Inspector)
                | Some(Builtin::InspectorPane)
                | None => {
                    if let Some(make) = self.registry.get(kind).map(|r| r.make) {
                        return make(self, props, id);
                    }
                    warn_missing_renderer(kind);
                    placeholder_view(kind)
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
                kinds::IMAGE => {
                    if let (Some(day_spec::props::ImagePatch::Tint(c)), Some(iv)) = (
                        patch.downcast_ref::<day_spec::props::ImagePatch>(),
                        h.downcast_ref::<objc2_ui_kit::UIImageView>(),
                    ) {
                        // Template rendering + the view's tint, as at realize (docs/vectors.md).
                        if let Some(img) = unsafe { iv.image() } {
                            let mode = match c {
                                Some(_) => objc2_ui_kit::UIImageRenderingMode::AlwaysTemplate,
                                None => objc2_ui_kit::UIImageRenderingMode::AlwaysOriginal,
                            };
                            let next = unsafe { img.imageWithRenderingMode(mode) };
                            unsafe { iv.setImage(Some(&next)) };
                        }
                        unsafe { iv.setTintColor(c.map(uicolor).as_deref()) };
                    }
                }
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
                // Data-driven sidebar rows (docs/navigation.md): rebuild the UITableView rows.
                kinds::NAV_MENU => {
                    if let Some(NavMenuPatch::Items {
                        items,
                        icons,
                        tints,
                        menus,
                        badge_icons,
                        badge_tints,
                        ..
                    }) = patch.downcast_ref::<NavMenuPatch>()
                    {
                        NAV_MENUS.with(|m| {
                            if let Some((data, n)) = m.borrow_mut().get_mut(&ptr_of(h)) {
                                data.set_items(
                                    items,
                                    icons,
                                    tints,
                                    menus,
                                    badge_icons,
                                    badge_tints,
                                );
                                *n = items.len();
                                if let Some(tv) = h.downcast_ref::<objc2_ui_kit::UITableView>() {
                                    unsafe { tv.reloadData() };
                                }
                            }
                        });
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
                    // Inline search: the app writing its query patches the live field, so the
                    // sync never rebuilds it or takes the insertion point (docs/search.md). The
                    // suppress flag stops UISearchResultsUpdating echoing our own write back.
                    if let Some(sp) = patch.downcast_ref::<day_spec::props::SearchPatch>() {
                        NAV_STATE.with(|m| {
                            let m = m.borrow();
                            let Some((sc, updater)) =
                                m.get(&ptr_of(h)).and_then(|st| st.search.as_ref())
                            else {
                                return;
                            };
                            if let day_spec::props::SearchPatch::Text(t) = sp {
                                updater.ivars().suppress.set(true);
                                unsafe { sc.searchBar().setText(Some(&NSString::from_str(t))) };
                                updater.ivars().suppress.set(false);
                            }
                            // Scope and suggestion patches have no UIKit surface yet
                            // (docs/search.md).
                        });
                    }
                    if let Some(NavPatch::Select(i)) = patch.downcast_ref::<NavPatch>() {
                        // A `.tabSidebar` host has no `NavState` — it is not a navigation stack — so
                        // this is handled before that lookup.
                        let i = *i;
                        let hp = ptr_of(h);
                        let found = NAV_TABS
                            .with(|m| m.borrow().get(&hp).map(|t| (t.tabbar.clone(), t.vcs.len())));
                        if let Some((tabbar, n)) = found {
                            if i < n {
                                unsafe { tabbar.setSelectedIndex(i) };
                            }
                            return;
                        }
                    }
                    if let Some(p) = patch.downcast_ref::<NavPatch>() {
                        // Copy out of NAV_STATE BEFORE touching UIKit: push/pop can invoke
                        // the delegate synchronously, which re-borrows NAV_STATE.
                        enum Act {
                            Sync,
                            Title(Retained<UIViewController>, String),
                            None,
                        }
                        let act = NAV_STATE.with(|m| {
                            let mut m = m.borrow_mut();
                            let Some(state) = m.get_mut(&ptr_of(h)) else {
                                return Act::None;
                            };
                            match p {
                                NavPatch::Pushed { .. } => Act::Sync,
                                NavPatch::Popped => {
                                    // Answering a native user-back? The stack already popped, so
                                    // absorb it — syncing again would be a no-op anyway, but the
                                    // counter keeps the mirror bookkeeping honest.
                                    if state.native_pops.get() > 0 {
                                        state.native_pops.set(state.native_pops.get() - 1);
                                        Act::None
                                    } else {
                                        // Day-initiated: prune the mirror NOW — the sync target
                                        // derives from it, and the remove() duty only arrives
                                        // after this patch. Never below the merged stack's
                                        // sidebar root (docs/size-classes.md).
                                        let floor = usize::from(state.collapsed.get());
                                        if state.vcs.len() > floor {
                                            state.vcs.pop();
                                        }
                                        Act::Sync
                                    }
                                }
                                // Retitle the TOP page's controller — the navigation bar
                                // mirrors the top item's title live.
                                NavPatch::Title(t) => state
                                    .vcs
                                    .last()
                                    .map(|vc| Act::Title(vc.clone(), t.clone()))
                                    .unwrap_or(Act::None),
                                // Arm the back guard: shouldPop vetoes the back button, and the
                                // swipe gesture is disabled (docs/navigation.md).
                                NavPatch::GuardTop(on) => {
                                    state.active_nav().ivars().guarded.set(*on);
                                    if let Some(g) = unsafe {
                                        state.active_nav().interactivePopGestureRecognizer()
                                    } {
                                        g.setEnabled(!*on);
                                    }
                                    Act::None
                                }
                                // Unreachable: this backend answers `Cap::NavRepresent =
                                // Unsupported`, so the pieces layer never sends it. The plan for
                                // iOS is to adopt `UISplitViewController` and OBSERVE its own
                                // collapse/expand rather than be told (docs/size-classes.md).
                                NavPatch::Presentation(_) => Act::None,
                                // Resident-page switch: `.tabSidebar` keeps a controller per tab
                                // at every width, so switching is a selection rather than a push.
                                NavPatch::Select(_) => Act::None,
                            }
                        });
                        // Defer past any in-flight modal transition: a stack change issued the
                        // instant a (scripted) dialog dismissal starts races the dismissal
                        // transition and wedges the navigation controller.
                        match act {
                            Act::Sync => {
                                note_ui_transition();
                                let host = ptr_of(h);
                                modal_after_idle(move || nav_sync_stack(host));
                            }
                            Act::Title(vc, t) => unsafe {
                                vc.setTitle(Some(&NSString::from_str(&t)));
                            },
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
                            LabelPatch::Runs(text, runs) => {
                                let base = unsafe { label.font() };
                                if let Some(f) = base {
                                    let s = attributed_label(text, &f, None, runs);
                                    unsafe { label.setAttributedText(Some(&s)) };
                                }
                            }
                            // `None` restores the adaptive default (labelColor tracks dark mode).
                            LabelPatch::Color(c) => unsafe {
                                match c {
                                    Some(c) => label.setTextColor(Some(&uicolor(*c))),
                                    None => label.setTextColor(Some(&UIColor::labelColor())),
                                }
                            },
                        }
                    } else if let (Some(p), Some(tv)) = (
                        patch.downcast_ref::<LabelPatch>(),
                        (**h).downcast_ref::<UITextView>(),
                    ) {
                        // A `.selectable()` label rides a read-only UITextView (the
                        // `set_selectable` swap); the same patches route there.
                        match p {
                            LabelPatch::Text(t) => unsafe {
                                tv.setText(Some(&NSString::from_str(t)))
                            },
                            LabelPatch::Font(f) => {
                                let font = resolve_font(*f);
                                unsafe {
                                    tv.setFont(Some(&font));
                                    let _: () =
                                        msg_send![tv, setAdjustsFontForContentSizeCategory: true];
                                }
                            }
                            LabelPatch::Runs(text, runs) => {
                                // A selectable label is a UITextView, which renders attributed
                                // text the same way — and, unlike UILabel, could hit-test its
                                // links (Phase 4).
                                if let Some(f) = unsafe { tv.font() } {
                                    let s = attributed_label(text, &f, None, runs);
                                    unsafe { tv.setAttributedText(Some(&s)) };
                                }
                            }
                            LabelPatch::Color(c) => unsafe {
                                match c {
                                    Some(c) => tv.setTextColor(Some(&uicolor(*c))),
                                    None => tv.setTextColor(Some(&UIColor::labelColor())),
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
                            ButtonPatch::Style(s) => {
                                // Re-apply with the CURRENT title: a configured button carries
                                // its title in the configuration, which this replaces.
                                let title = unsafe {
                                    btn.configuration()
                                        .and_then(|c| c.title())
                                        .or_else(|| btn.titleForState(UIControlState::Normal))
                                        .map(|s| s.to_string())
                                }
                                .unwrap_or_default();
                                apply_button_style(btn, &title, *s, mtm());
                            }
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
                    Some(ListPatch::Splice(deltas)) => {
                        let (key, deltas) = (ptr_of(h), deltas.clone());
                        // Deferred like reload realization: row updates realize cells
                        // synchronously, which must happen outside this tree borrow.
                        <Uikit as Platform>::post(Box::new(move || {
                            LIST_STATE.with(|m| {
                                if let Some((table, _)) = m.borrow().get(&key) {
                                    apply_row_deltas(table, &deltas);
                                }
                            });
                        }));
                    }

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
                    Some(ListPatch::ScrollToRow(row)) => {
                        LIST_STATE.with(|m| {
                            if let Some((table, data)) = m.borrow().get(&ptr_of(h)) {
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
                                            (*row).min(n - 1) as isize,
                                            0,
                                        );
                                    unsafe {
                                        table.scrollToRowAtIndexPath_atScrollPosition_animated(
                                            &ip,
                                            objc2_ui_kit::UITableViewScrollPosition::Top,
                                            true,
                                        )
                                    };
                                }
                            }
                        });
                    }
                    // Not implemented: RowSizeInvalidated (the row keeps its height until the
                    // next Reload) and Selected (no programmatic selection sync on UIKit yet).
                    Some(ListPatch::RowSizeInvalidated(_))
                    | Some(ListPatch::Selected(_))
                    | None => {}
                },
                _ => {
                    if let Some(update) = self.registry.get(kind).map(|r| r.update) {
                        update(self, h, patch);
                    }
                }
            }
        }

        /// Offer a satellite piece its teardown hook before `release` frees the handle (§15.2).
        fn release_piece(&mut self, kind: day_spec::PieceKind, h: &Self::Handle) {
            // Copy the fn pointer out first: the registry lookup borrows `self` immutably and
            // the hook needs it mutably.
            let f = self.registry.get(kind).and_then(|r| r.release);
            if let Some(f) = f {
                f(self, h);
            }
        }
        fn release(&mut self, h: Handle) {
            // Backstop for a released window root whose scene never disconnected
            // (docs/windows.md — disconnect normally prunes first).
            SCENES.with(|s| s.borrow_mut().retain(|e| !std::ptr::eq(&*e.root_view, &*h)));
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
            GESTURES.with(|m| {
                m.borrow_mut().remove(&ptr_of(&h));
            });
            // ONE sweep clears every `day_spec::sidetable::SideTable` on this thread —
            // CTX_MENUS (whose teardown detaches the interaction from the view first),
            // PAGE_PANE, canvas OPS, and the picker/textarea state tables — present and
            // future, so a new table can never be forgotten here again. The RefCell maps
            // above predate the mechanism and are still cleared by hand.
            day_spec::sidetable::sweep(ptr_of(&h));
            unsafe { h.removeFromSuperview() };
        }

        fn insert(&mut self, parent: &Handle, child: &Handle, index: usize) {
            // A NAV_MENU joining the tree: if it lands anywhere inside a `.tabSidebar` host, its
            // rows ARE that host's tabs. This is the first moment the menu has a superview chain
            // to find its host through.
            if let Some((node, titles, icons)) =
                NAV_MENU_ROWS.with(|m| m.borrow_mut().remove(&ptr_of(child)))
            {
                if let Some(hp) = enclosing_tabs_host(parent) {
                    NAV_TABS.with(|m| {
                        if let Some(t) = m.borrow_mut().get_mut(&hp) {
                            t.menu_node.set(node);
                            t.titles = titles;
                            t.icons = icons;
                        }
                    });
                    nav_tabs_sync(hp);
                }
            }
            // Adaptive tabs host (`.tabSidebar`). Its DETAIL pages become tabs; its `Pane::Sidebar`
            // page does not, because UIKit draws the sidebar itself from the same tabs — adding
            // Day's rows as well would show the list twice, once in each chrome.
            let is_tabs_host = NAV_TABS.with(|m| m.borrow().contains_key(&ptr_of(parent)));
            if is_tabs_host {
                TABS_PAGE_HOST.with(|m| m.borrow_mut().insert(ptr_of(child), ptr_of(parent)));
                let pane = PAGE_PANE.with(|t| t.get(ptr_of(child)));
                if pane == Some(day_spec::props::Pane::Sidebar) {
                    // UIKit draws the sidebar from the tabs, so Day's rows page would show the
                    // list twice — once in each chrome. It stays out of the controller.
                    return;
                }
                if let Some(vc) = PAGE_VCS.with(|m| m.borrow().get(&ptr_of(child)).cloned()) {
                    NAV_TABS.with(|m| {
                        if let Some(t) = m.borrow_mut().get_mut(&ptr_of(parent)) {
                            let at = index.min(t.vcs.len());
                            t.vcs.insert(at, vc);
                        }
                    });
                    nav_tabs_sync(ptr_of(parent));
                }
                return;
            }
            // The SIDEBAR page is the split host's primary column, not a member of the stack
            // (docs/size-classes.md). It goes in whatever the current presentation calls for:
            // its own column while expanded, and the stack's root while collapsed — which is the
            // shape the phone path has always had, so nothing below changes for it.
            let is_sidebar =
                PAGE_PANE.with(|t| t.get(ptr_of(child))) == Some(day_spec::props::Pane::Sidebar);
            // Nav host: pages join the VC stack; the first one becomes the root VC now, later
            // pages are presented by the Pushed patch.
            // Copy out of NAV_STATE before setViewControllers (same re-entrancy rule).
            let set_root = NAV_STATE.with(|m| {
                let mut m = m.borrow_mut();
                let state = m.get_mut(&ptr_of(parent))?;
                let vc = PAGE_VCS.with(|p| p.borrow().get(&ptr_of(child)).cloned())?;
                state.vcs.push(vc.clone());
                // The host's trailing bar actions ride this page's navigation bar
                // (docs/navigation.md): fresh UIBarButtonItems wired to the shared targets, set on
                // this page's navigationItem as it joins the stack. `RootPage` actions are on the
                // list only — the same rule inline search follows just below, and `is_sidebar` is
                // how both recognize the list.
                // The host's ROOT page, for `NavBarScope::RootPage`. Two shapes qualify and both
                // have to: a split selector's list lives in the SIDEBAR pane, while a plain
                // `stack()` has no sidebar at all and its root is simply the first page to
                // arrive — `vcs` was just pushed, so length 1 is that page. Testing only for the
                // sidebar left every stack's list-scoped action filtered out everywhere, which
                // renders as a nav bar that is present but empty.
                let is_root = is_sidebar || state.vcs.len() == 1;
                let mine: Vec<_> = state
                    .bar_actions
                    .iter()
                    .filter(|ba| is_root || ba.scope == day_spec::props::NavBarScope::EveryPage)
                    .collect();
                if !mine.is_empty() {
                    let mtm =
                        MainThreadMarker::new().expect("uikit insert runs on the main thread");
                    // REVERSED: `setRightBarButtonItems` fills from the trailing edge inward, so
                    // element 0 lands rightmost. Reversing puts the app's first-declared action
                    // leftmost, which is the order it wrote them in and the order every other
                    // backend draws them.
                    let items = objc2_foundation::NSArray::from_retained_slice(
                        &mine
                            .iter()
                            .rev()
                            .map(|ba| ba.make_item(mtm))
                            .collect::<Vec<_>>(),
                    );
                    unsafe { vc.navigationItem().setRightBarButtonItems(Some(&items)) };
                }
                // Inline search rides the ROOT page only (docs/search.md): it filters the
                // TOP-LEVEL list, so it belongs to that list's navigation item and not to every
                // pushed detail page. `hidesSearchBarWhenScrolling` defaults to true, which is
                // what puts it behind the pull-down.
                if is_sidebar && let Some((sc, _)) = state.search.as_ref() {
                    let item = unsafe { vc.navigationItem() };
                    unsafe {
                        item.setSearchController(Some(sc));
                        // Auto-hide, explicitly rather than by default — and LARGE TITLES on this
                        // page, because that is the configuration the collapse actually belongs
                        // to. Mail, Settings and Files all do this: the search bar sits under a
                        // large title and the two collapse together as the list scrolls, then
                        // come back on a pull down. With a small centered title UIKit keeps the
                        // field pinned and nothing hides (docs/search.md).
                        item.setHidesSearchBarWhenScrolling(true);
                        // The collapse is driven by a SCROLL VIEW the navigation controller can
                        // track, and tracking needs the content to extend UNDER the bar rather
                        // than start below it. `DayNavPageView` pins to the safe area, so without
                        // these two the list never overlaps the bar, UIKit has nothing to couple
                        // to, and the field stays put no matter how far you scroll — which is
                        // exactly what the flag alone did not fix.
                        vc.setEdgesForExtendedLayout(objc2_ui_kit::UIRectEdge::All);
                        vc.setExtendedLayoutIncludesOpaqueBars(true);
                    }
                    {
                        item.setLargeTitleDisplayMode(
                            objc2_ui_kit::UINavigationItemLargeTitleDisplayMode::Always,
                        );
                        state
                            .active_nav()
                            .navigationBar()
                            .setPrefersLargeTitles(true);
                    }
                } else if !is_sidebar {
                    // Pushed detail pages keep the compact title: the large one belongs to the
                    // top-level list that owns the search field.
                    unsafe {
                        vc.navigationItem().setLargeTitleDisplayMode(
                            objc2_ui_kit::UINavigationItemLargeTitleDisplayMode::Never,
                        )
                    };
                }
                // The sidebar page ALWAYS becomes the primary column, in both presentations
                // (docs/size-classes.md). Never conditionally the stack's root: `isCollapsed`
                // is not yet meaningful when a host is realized — it has no window — so branching
                // on it here put the page in the stack AND left UIKit's own merge with nothing to
                // move, stranding a phantom entry under every detail. Letting UIKit own the move
                // means one code path and no guess: it merges the column into the stack when it
                // collapses, which IS the phone shape, and lifts it back out when it expands.
                if is_sidebar && let Some(parts) = state.split.as_ref() {
                    state.vcs.retain(|v| !std::ptr::eq(&**v, &*vc));
                    let arr = objc2_foundation::NSArray::from_retained_slice(&[vc]);
                    unsafe { parts.primary_nav.setViewControllers(&arr) };
                    // Handled — Some(None), NOT None: the outer match reads None as "parent is
                    // not a nav host" and reparents the child via addSubview, which STEALS the
                    // content view out of its DayNavPageView (addSubview moves a view). The page
                    // then has nothing to pin to the safe area, and the sidebar's content draws
                    // from the split view's origin at whatever size it was last laid out for —
                    // the cut-off landscape list and the stale overlay after a collapse.
                    return Some(None);
                }
                // A PLAIN host's first page becomes its root right here: a nested stack's root
                // is part of the host build and no `NavPatch::Pushed` follows it (only pushed
                // destinations patch). The adaptive host never takes this path — UIKit's
                // collapse puts the sidebar column at the stack root, and detail pages arrive
                // through `NavPatch::Pushed`.
                if state.split.is_none() && state.vcs.len() == 1 {
                    return Some(Some((state.nav.clone(), vc)));
                }
                Some(None::<(Retained<DayNavController>, Retained<UIViewController>)>)
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
                    // A controller-backed child (a nested nav host, or a tab bar) landing inside
                    // a PAGE's content view has to move its UIViewController containment to that
                    // page's controller. Every host parents itself to the WINDOW's root VC when
                    // it is realized, because at that moment it does not know where it will be
                    // inserted; leaving it there is what UIKit rejects the instant the view
                    // enters a window:
                    //
                    //     UIViewControllerHierarchyInconsistency: child view controller
                    //     <DayNavController> should have parent view controller <UIViewController>
                    //     but actual parent is <DayRootVC>
                    //
                    // Two stacks under a tab bar is what surfaced it (one per tab, Day Tradr's
                    // phone shell). A single one hid: only the SELECTED tab's view reaches a
                    // window at launch, and the check runs on the way in.
                    let host_vc = host_controller(child);
                    let page_vc = PAGE_VCS
                        .with(|m| m.borrow().get(&ptr_of(parent)).cloned())
                        .or_else(|| enclosing_view_controller(parent));
                    match (host_vc, page_vc) {
                        (Some(host_vc), Some(page_vc)) => unsafe {
                            // Order is UIKit's: leave the old parent, join the new one, THEN the
                            // view move, then confirm. `didMove` last is what the containment
                            // contract asks for.
                            host_vc.willMoveToParentViewController(None);
                            host_vc.removeFromParentViewController();
                            page_vc.addChildViewController(&host_vc);
                            parent.addSubview(child);
                            host_vc.didMoveToParentViewController(Some(&page_vc));
                        },
                        _ => unsafe { parent.addSubview(child) },
                    }
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

        fn set_selectable(&mut self, h: &Handle, selectable: bool) -> Option<Handle> {
            // A backing that already is a text view (an earlier swap): flip the flag in place.
            if let Some(tv) = (**h).downcast_ref::<UITextView>() {
                unsafe { tv.setSelectable(selectable) };
                return None;
            }
            // UIKit reserves selection for UITextInput views, so a UILabel has no flag to flip
            // (SwiftUI's selectable Text is its own renderer with the system selection UI
            // attached — not a UILabel either). The standard emulation ships here instead: the
            // label is rebuilt as a read-only, non-scrolling UITextView, geometry-matched to
            // the label (zero inset and padding, so `sizeThatFits` measures the same), and
            // day-core re-points the node's handle at the replacement (docs/text.md).
            let label = (**h).downcast_ref::<UILabel>()?;
            if !selectable {
                return None; // a plain UILabel is already unselectable
            }
            let tv = UITextView::new(mtm());
            unsafe {
                tv.setFont(label.font().as_deref());
                tv.setTextColor(label.textColor().as_deref());
                // Styled runs live in the label's ATTRIBUTED text; copying `text()` alone would
                // hand the text view the plain string and silently drop every run. Font and
                // color go on first so they still stand as the view's defaults.
                match label.attributedText() {
                    Some(a) => tv.setAttributedText(Some(&a)),
                    None => tv.setText(label.text().as_deref()),
                }
                let adj: bool = msg_send![label, adjustsFontForContentSizeCategory];
                let _: () = msg_send![&*tv, setAdjustsFontForContentSizeCategory: adj];
                tv.setEditable(false);
                tv.setSelectable(true);
                tv.setScrollEnabled(false);
                tv.setBackgroundColor(None);
                tv.setTextContainerInset(UIEdgeInsets {
                    top: 0.0,
                    left: 0.0,
                    bottom: 0.0,
                    right: 0.0,
                });
                // The container pads 5pt per side by default; raw sends spare day-uikit the
                // NSTextContainer binding for this one call.
                let container: *mut AnyObject = msg_send![&*tv, textContainer];
                let _: () = msg_send![container, setLineFragmentPadding: 0.0f64];
                // `.id()` may have run before `.selectable()` — carry the identifier over.
                let ident: Option<Retained<NSString>> = msg_send![&**h, accessibilityIdentifier];
                if let Some(i) = ident {
                    let _: () = msg_send![&*tv, setAccessibilityIdentifier: &*i];
                }
                // A swap on a LIVE node (a `.tweak` after mount): take the label's place in the
                // view tree; the re-pointed handle routes later layout and patches here.
                if let Some(sup) = label.superview() {
                    tv.setFrame(label.frame());
                    sup.insertSubview_aboveSubview(
                        <UITextView as AsRef<UIView>>::as_ref(&tv),
                        <UILabel as AsRef<UIView>>::as_ref(label),
                    );
                    label.removeFromSuperview();
                }
            }
            Some(view_of(tv))
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

        /// UIKit exposes baselines only as layout ANCHORS (`firstBaselineAnchor`), which are
        /// constraint endpoints with no readable number, so this derives the offset from the
        /// view's own font instead (docs/baseline.md — `Cap::BaselineAlignment` is `Emulated`
        /// here for exactly that reason).
        ///
        /// The model is the one UIKit itself uses for a single-line control: center the font's
        /// line box in the view's height, and the baseline sits an ascender below the line's
        /// top. For a label day has sized to its text that reduces to the ascender, and for a
        /// bordered field it accounts for the inset the border adds.
        fn first_baseline(&mut self, h: &Handle, kind: PieceKind, size: Size) -> Option<f64> {
            if !day_spec::kind_has_baseline(kind) {
                return None;
            }
            let font = unsafe {
                if let Some(l) = (**h).downcast_ref::<UILabel>() {
                    l.font()
                } else if let Some(f) = (**h).downcast_ref::<UITextField>() {
                    f.font()
                } else if let Some(v) = (**h).downcast_ref::<UITextView>() {
                    v.font()
                } else if let Some(b) = (**h).downcast_ref::<UIButton>() {
                    b.titleLabel().and_then(|l| l.font())
                } else {
                    None
                }
            }?;
            let (ascender, line_height) = unsafe { (font.ascender(), font.lineHeight()) };
            Some(((size.height - line_height) / 2.0).max(0.0) + ascender)
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
                if !focused {
                    if h.isFirstResponder() {
                        h.resignFirstResponder();
                    }
                    return;
                }
                if h.becomeFirstResponder() {
                    return;
                }
            }
            // A refusal can be TRANSIENT — the outgoing responder is still tearing its keyboard
            // down, and UIKit will not hand focus over mid-transition — so retry once on the
            // next turn (GTK's un-mapped-widget retry, rule 4 in docs/focus.md).
            //
            // It can also be permanent, and correctly so: a view that is not in a WINDOW cannot
            // hold the keyboard, and a full-screen modal takes the presenting view out of the
            // window for as long as it covers it. A canvas behind a compact inspector sheet
            // (docs/inspector.md) refuses focus until the sheet closes — the retry lapses and
            // the binding's signal snaps back, which is rule 2.
            let Some(mtm) = MainThreadMarker::new() else {
                return;
            };
            let view = dispatch2::MainThreadBound::new(h.clone(), mtm);
            dispatch2::DispatchQueue::main().exec_async(move || {
                day_spec::ffi_guard::contain((), || {
                    let Some(mtm) = MainThreadMarker::new() else {
                        return;
                    };
                    let view = view.get(mtm);
                    if !view.isFirstResponder() {
                        let _ = unsafe { view.becomeFirstResponder() };
                    }
                });
            });
        }

        fn set_event_sink(&mut self, sink: EventSink) {
            SINK.with(|s| *s.borrow_mut() = Some(Rc::from(sink)));
        }

        fn enable_gesture(&mut self, h: &Handle, node: NodeId, kind: day_spec::GestureKind) {
            let key = ptr_of(h);
            let already = GESTURES.with(|m| {
                m.borrow()
                    .get(&key)
                    .is_some_and(|v| v.iter().any(|t| t.ivars().kind == kind))
            });
            if already {
                return;
            }
            let mtm = mtm();
            let target = DayGesture::new(mtm, node, kind);
            unsafe {
                let recognizer: Retained<UIGestureRecognizer> = match kind {
                    day_spec::GestureKind::Drag => {
                        let pan = UIPanGestureRecognizer::initWithTarget_action(
                            UIPanGestureRecognizer::alloc(mtm),
                            Some(&target),
                            Some(sel!(fire:)),
                        );
                        Retained::into_super(pan)
                    }
                    day_spec::GestureKind::Pinch => {
                        let pinch = UIPinchGestureRecognizer::initWithTarget_action(
                            UIPinchGestureRecognizer::alloc(mtm),
                            Some(&target),
                            Some(sel!(fire:)),
                        );
                        Retained::into_super(pinch)
                    }
                    day_spec::GestureKind::Pan => {
                        // Two fingers, so single-finger drags still reach a Drag gesture on
                        // the same view.
                        let pan = UIPanGestureRecognizer::initWithTarget_action(
                            UIPanGestureRecognizer::alloc(mtm),
                            Some(&target),
                            Some(sel!(fire:)),
                        );
                        pan.setMinimumNumberOfTouches(2);
                        pan.setMaximumNumberOfTouches(2);
                        Retained::into_super(pan)
                    }
                    _ => {
                        let tap = UITapGestureRecognizer::initWithTarget_action(
                            UITapGestureRecognizer::alloc(mtm),
                            Some(&target),
                            Some(sel!(fire:)),
                        );
                        Retained::into_super(tap)
                    }
                };
                h.setUserInteractionEnabled(true);
                h.addGestureRecognizer(&recognizer);
            }
            GESTURES.with(|m| m.borrow_mut().entry(key).or_default().push(target));
        }

        fn set_context_menu(&mut self, h: &Handle, _node: NodeId, items: &[day_spec::MenuItem]) {
            let key = ptr_of(h);
            // Remove any prior interaction (replace-on-reconfigure): the table's teardown
            // detaches it from the view before dropping the retains.
            CTX_MENUS.with(|t| t.remove(key));
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
            CTX_MENUS.with(|t| t.insert(key, (interaction, delegate)));
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

        fn set_edit_state(&mut self, state: &day_spec::EditState) {
            EDIT_STATE.with(|s| s.set(*state));
        }

        fn set_undo_state(&mut self, state: &day_spec::UndoState) {
            let front = undo_front(self.mtm());
            *front.ivars().state.borrow_mut() = state.clone();
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
            OPS.with(|t| t.insert(ptr_of(h), ops.to_vec()));
            unsafe { h.setNeedsDisplay() };
        }

        fn snapshot_window(&mut self) -> Result<Vec<u8>, String> {
            snapshot_uikit(false)
        }

        /// The window rather than Day's content view. On iOS the "chrome" is the navigation bar,
        /// which lives IN the window's own hierarchy — so this is a wider capture of the same
        /// tree, not a different mechanism. The status bar is not in it: that belongs to the
        /// system, not to this process, and no in-app API can draw it (docs/window-image.md).
        fn snapshot_window_chrome(&mut self) -> Result<Vec<u8>, String> {
            snapshot_uikit(true)
        }

        /// A secondary "window" on iOS is a fullscreen cover (`open_window` below answers
        /// Unsupported for the Preferences kind), so `host` is the cover's content view —
        /// capture IT, not the key scene's root, which the cover's presentation has
        /// detached from the window (drawing a detached root raises, see `snapshot_view`).
        fn snapshot_window_of(&mut self, host: &Handle) -> Result<Vec<u8>, String> {
            snapshot_view(host)
        }

        fn open_window(
            &mut self,
            id: NodeId,
            options: &day_spec::WindowOptions,
            kind: day_spec::WindowKind,
        ) -> day_spec::WindowOpenReply<Handle> {
            let m = mtm();
            let app = UIApplication::sharedApplication(m);
            // iPhone (single visible scene) — and the Preferences kind everywhere on
            // mobile, where settings are modal, not a detached window (docs/windows.md).
            if !unsafe { app.supportsMultipleScenes() } || kind == day_spec::WindowKind::Preferences
            {
                return day_spec::WindowOpenReply::Unsupported;
            }
            // Ask UIKit for a new scene; its willConnect completes the open through
            // `finish_window_open` (the Pending path).
            use objc2::AnyThread as _;
            let activity = unsafe {
                objc2_foundation::NSUserActivity::initWithActivityType(
                    objc2_foundation::NSUserActivity::alloc(),
                    &NSString::from_str(DAY_WINDOW_ACTIVITY),
                )
            };
            let key = NSString::from_str("day.node");
            let num = objc2_foundation::NSNumber::new_u64(id.0);
            let obj: Retained<AnyObject> = num.into_super().into_super().into();
            let dict: Retained<objc2_foundation::NSDictionary<NSString, AnyObject>> =
                objc2_foundation::NSDictionary::from_retained_objects(&[&*key], &[obj]);
            // The API takes the untyped dictionary; the typed one IS that object.
            let untyped =
                unsafe { &*(Retained::as_ptr(&dict) as *const objc2_foundation::NSDictionary) };
            unsafe { activity.addUserInfoEntriesFromDictionary(untyped) };
            // The non-deprecated activateSceneSessionForRequest: needs iOS 17; this form
            // covers the whole deployment range.
            #[allow(deprecated)]
            unsafe {
                app.requestSceneSessionActivation_userActivity_options_errorHandler(
                    None,
                    Some(&activity),
                    None,
                    None,
                );
            }
            PENDING_WINDOWS.with(|p| p.borrow_mut().push((id, options.title.clone())));
            day_spec::WindowOpenReply::Pending
        }

        fn close_window(&mut self, host: &Handle) {
            let m = mtm();
            let session = SCENES.with(|s| {
                s.borrow()
                    .iter()
                    .find(|e| std::ptr::eq(&*e.root_view, &**host))
                    .and_then(|e| e.window.windowScene())
                    .map(|ws| unsafe { ws.session() })
            });
            if let Some(session) = session {
                request_scene_destruction(m, &session);
            }
        }

        fn focus_window(&mut self, host: &Handle) {
            let m = mtm();
            let app = UIApplication::sharedApplication(m);
            let session = SCENES.with(|s| {
                s.borrow()
                    .iter()
                    .find(|e| std::ptr::eq(&*e.root_view, &**host))
                    .and_then(|e| e.window.windowScene())
                    .map(|ws| unsafe { ws.session() })
            });
            if let Some(session) = session {
                // See open_window: the deprecated form covers pre-iOS-17 deployment.
                #[allow(deprecated)]
                unsafe {
                    app.requestSceneSessionActivation_userActivity_options_errorHandler(
                        Some(&session),
                        None,
                        None,
                        None,
                    );
                }
            }
        }

        fn set_window_title(&mut self, host: &Handle, title: &str) {
            // Shown in the iPad app switcher / multitasking UI.
            SCENES.with(|s| {
                if let Some(ws) = s
                    .borrow()
                    .iter()
                    .find(|e| std::ptr::eq(&*e.root_view, &**host))
                    .and_then(|e| e.window.windowScene())
                {
                    unsafe { ws.setTitle(Some(&NSString::from_str(title))) };
                }
            });
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
                    // On iPad an action sheet presents as a POPOVER, and a popover without an
                    // anchor is an NSGenericException at transition time — the app dies. The
                    // dialog surface has no anchor concept (a sheet is logically modal,
                    // docs/dialogs.md), so anchor it to the window's center, arrowless: the
                    // pad convention for source-less sheets. On iPhone the popover controller
                    // is unused and this is inert.
                    if *sheet
                        && let Some(pop) = unsafe { ac.popoverPresentationController() }
                        && let Some(w) = WINDOW.with(|win| win.borrow().clone())
                    {
                        let b = w.bounds();
                        unsafe {
                            pop.setSourceView(Some(w.as_ref()));
                            pop.setSourceRect(CGRect::new(
                                CGPoint::new(b.size.width / 2.0, b.size.height / 2.0),
                                CGSize::new(0.0, 0.0),
                            ));
                            pop.setPermittedArrowDirections(
                                objc2_ui_kit::UIPopoverArrowDirection::empty(),
                            );
                        }
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
            // `openURL:options:completionHandler:`, NOT the one-argument `openURL:`. The old form
            // is deprecated, and on a current iOS it returns NO and opens nothing — a link that
            // silently does nothing, with no exception and no log line to explain it. The options
            // dictionary is empty (the defaults are what a plain link wants) and the completion
            // block is nil, which this fire-and-forget call is allowed to pass.
            unsafe {
                UIApplication::sharedApplication(mtm()).openURL_options_completionHandler(
                    &nsurl,
                    &objc2_foundation::NSDictionary::new(),
                    None,
                );
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

        fn set_appearance(&mut self, dark: Option<bool>) {
            WINDOW.with(|w| {
                if let Some(window) = w.borrow().as_ref() {
                    let style = match dark {
                        Some(true) => objc2_ui_kit::UIUserInterfaceStyle::Dark,
                        Some(false) => objc2_ui_kit::UIUserInterfaceStyle::Light,
                        None => objc2_ui_kit::UIUserInterfaceStyle::Unspecified,
                    };
                    unsafe { window.setOverrideUserInterfaceStyle(style) };
                }
            });
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
                || topmost_vc().is_some_and(|top| top.transitionCoordinator().is_some())
                // A nav push/pop animates on its UINavigationController, which topmost_vc()
                // (presented modals only) never reaches — so without this a scripted screenshot
                // taken right after `navigate` catches the outgoing page (or a mid-slide frame),
                // the way the iOS gallery captures did. Any registered nav host with a live
                // transition coordinator counts as still-settling.
                || NAV_STATE.with(|m| {
                    m.borrow()
                        .values()
                        .any(|s| s.active_nav().transitionCoordinator().is_some())
                });
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
        /// Present a cover (docs/cover.md) + how many 50ms defer-retries it has already made.
        ///
        /// Its OWN op rather than a `Run` closure, so it gets the same treatment a dialog does:
        /// `Run` executes unconditionally, and a cover presented across an animating transition is
        /// refused by UIKit with no completion — the same refusal `Present` below waits out. As a
        /// closure it also had nowhere to put a retry and no way to report the drop, so the panel
        /// just never appeared, with no watchdog (that is armed only after the closure's early
        /// returns) and nothing in the log.
        Cover(Retained<DayCoverVC>, u32),
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
                log::warn!("modal transition completion lost — unjamming the queue");
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
    /// Mark the transition clock the instant a nav push/pop is REQUESTED. `pushViewController`
    /// sets up its transition coordinator on a later run-loop turn, so `ui_idle`'s coordinator
    /// check has a brief blind window right after the request; stamping here keeps `ui_idle` false
    /// across it (the 250ms settle margin), so a screenshot issued immediately after `navigate`
    /// never captures the outgoing page before the incoming one has begun to slide in.
    fn note_ui_transition() {
        UI_LAST_ACTIVE.with(|t| t.set(Some(std::time::Instant::now())));
    }

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
            ModalOp::Cover(vc, tries) => {
                if vc.presentingViewController().is_some() {
                    modal_pump(); // already up (a re-present while closing was cancelled)
                    return;
                }
                // The same wait `Present` above does, and for the same reason: presenting across
                // an animating transition is refused with no completion, and a cover that loses
                // its presentation this way is invisible — no dialog future to resolve, no
                // watchdog, nothing in the log. 40 × 50ms is the two seconds a nav push and a
                // dismissal together take, well past any single transition.
                let animating = topmost_vc().is_some_and(|t| t.transitionCoordinator().is_some());
                if animating || topmost_vc().is_none() {
                    if tries < 40 {
                        modal_defer_retry(ModalOp::Cover(vc, tries + 1));
                        return;
                    }
                    // Out of retries: say so. Silence here is what made this cost a CI run to
                    // find — the panel simply never appeared and every later step read as a
                    // missing element.
                    log::warn!(
                        "a cover could not be presented after {tries} retries \
                         (transition still animating, or no window to present on) — \
                         the app continues without it"
                    );
                    modal_pump();
                    return;
                }
                let Some(top) = topmost_vc() else {
                    modal_pump();
                    return;
                };
                modal_begin_transition();
                let completion = block2::RcBlock::new(modal_end_transition);
                unsafe {
                    top.presentViewController_animated_completion(&vc, true, Some(&completion));
                }
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
                day_spec::ffi_guard::contain((), || {
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
                });
            }

            #[unsafe(method(documentPickerWasCancelled:))]
            fn was_cancelled(&self, _picker: &UIDocumentPickerViewController) {
                day_spec::ffi_guard::contain((), || {
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
                });
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
        // UIResponder, not NSObject: nil-target actions (`sendAction(cut:, nil)` from the
        // menu, the system's edit commands) reach the app delegate ONLY when it is a
        // responder — the standard iOS template shape, and the same end-of-chain catch the
        // macOS window delegate provides.
        #[unsafe(super(objc2_ui_kit::UIResponder))]
        #[thread_kind = MainThreadOnly]
        #[name = "DayAppDelegate"]
        struct AppDelegate;

        unsafe impl NSObjectProtocol for AppDelegate {}

        /// The end of the responder chain for the standard edit selectors: a focused text
        /// field answered them long before the chain got here, so what arrives is the app's.
        impl AppDelegate {
            #[unsafe(method(cut:))]
            fn edit_cut(&self, _sender: Option<&AnyObject>) {
                day_spec::ffi_guard::contain((), || {
                    emit(WINDOW_NODE, Event::Edit(day_spec::EditOp::Cut));
                });
            }

            #[unsafe(method(copy:))]
            fn edit_copy(&self, _sender: Option<&AnyObject>) {
                day_spec::ffi_guard::contain((), || {
                    emit(WINDOW_NODE, Event::Edit(day_spec::EditOp::Copy));
                });
            }

            #[unsafe(method(paste:))]
            fn edit_paste(&self, _sender: Option<&AnyObject>) {
                day_spec::ffi_guard::contain((), || {
                    emit(WINDOW_NODE, Event::Edit(day_spec::EditOp::Paste));
                });
            }

            #[unsafe(method(selectAll:))]
            fn edit_select_all(&self, _sender: Option<&AnyObject>) {
                day_spec::ffi_guard::contain((), || {
                    emit(WINDOW_NODE, Event::Edit(day_spec::EditOp::SelectAll));
                });
            }

            /// UIKit's enablement question for the standard commands — the bridge state
            /// answers for the edit trio; everything else falls to UIResponder's default.
            #[unsafe(method(canPerformAction:withSender:))]
            fn can_perform(&self, action: objc2::runtime::Sel, sender: *mut AnyObject) -> bool {
                day_spec::ffi_guard::contain(false, || {
                    let state = EDIT_STATE.with(|s| s.get());
                    if action == sel!(selectAll:) {
                        state.can_select_all
                    } else if action == sel!(cut:) {
                        state.can_cut
                    } else if action == sel!(copy:) {
                        state.can_copy
                    } else if action == sel!(paste:) {
                        state.can_paste
                            && unsafe { objc2_ui_kit::UIPasteboard::generalPasteboard().hasStrings() }
                    } else {
                        unsafe {
                            msg_send![super(self), canPerformAction: action, withSender: sender]
                        }
                    }
                })
            }
        }

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

            // Scene-based lifecycle (docs/windows.md): the window is built by
            // DaySceneDelegate when the (primary) scene connects; launching only arms the
            // app-level observers that stay app-scoped under scenes.
            #[unsafe(method(application:didFinishLaunchingWithOptions:))]
            fn did_finish_launching(&self, _app: &UIApplication, _opts: *mut AnyObject) -> bool {
                // Keyboard avoidance (docs/focus.md): one app-level observer; the handler
                // resolves the KEY window's scene, so it follows whichever Day window the
                // field lives in. WillChangeFrame covers show, hide, and height changes.
                unsafe {
                    objc2_foundation::NSNotificationCenter::defaultCenter()
                        .addObserver_selector_name_object(
                            self,
                            sel!(keyboardWillChange:),
                            Some(objc2_ui_kit::UIKeyboardWillChangeFrameNotification),
                            None,
                        )
                };
                true
            }

            // Every connecting scene — the primary at launch, each secondary day window
            // (docs/windows.md), and any system-restored session — runs DaySceneDelegate.
            #[unsafe(method_id(application:configurationForConnectingSceneSession:options:))]
            fn configuration_for_scene(
                &self,
                _app: &UIApplication,
                session: &objc2_ui_kit::UISceneSession,
                _options: &objc2_ui_kit::UISceneConnectionOptions,
            ) -> Retained<objc2_ui_kit::UISceneConfiguration> {
                use objc2::ClassType as _;
                let role = unsafe { session.role() };
                let config = objc2_ui_kit::UISceneConfiguration::configurationWithName_sessionRole(
                    None,
                    &role,
                    self.mtm(),
                );
                unsafe { config.setDelegateClass(Some(DaySceneDelegate::class())) };
                config
            }

            // Custom-scheme deep link (docs/navigation.md): route = URL host + path,
            // delivered to the active nav host as RouteRequested.
            #[unsafe(method(application:openURL:options:))]
            fn open_url(
                &self,
                _app: &UIApplication,
                url: &objc2_foundation::NSURL,
                _options: *mut AnyObject,
            ) -> bool {
                // The shared URL → route mapping (docs/deep-links.md): absoluteString keeps
                // the query (route params ride it) and the original percent-encoding — the
                // route parser decodes, not this layer.
                day_spec::ffi_guard::contain(false, || {
                    let route = unsafe { url.absoluteString() }
                        .map(|s| day_spec::route_of_url(&s.to_string()))
                        .unwrap_or_default();
                    let node = NAV_STATE.with(|m| m.borrow().values().next().map(|s| s.host_node));
                    if let (Some(node), false) = (node, route.is_empty()) {
                        emit(node, Event::RouteRequested(route));
                        true
                    } else {
                        false
                    }
                })
            }

            // Lifecycle (docs/lifecycle.md): under the scene lifecycle the activation and
            // foreground phases are SCENE events — DaySceneDelegate derives the app-level
            // day phases from all scenes (debounced, docs/windows.md). Memory warnings and
            // termination stay app-scoped and keep arriving here.
            #[unsafe(method(applicationDidReceiveMemoryWarning:))]
            fn did_receive_memory_warning(&self, _app: &UIApplication) {
                day_spec::ffi_guard::contain((), || {
                    emit(
                        WINDOW_NODE,
                        Event::Lifecycle(day_spec::Lifecycle::DidReceiveMemoryWarning),
                    );
                });
            }
            #[unsafe(method(applicationWillTerminate:))]
            fn will_terminate(&self, _app: &UIApplication) {
                day_spec::ffi_guard::contain((), || {
                    emit(
                        WINDOW_NODE,
                        Event::Lifecycle(day_spec::Lifecycle::WillTerminate),
                    );
                });
            }
        }

        // Inherent (non-protocol) selectors: NSNotificationCenter targets land here — objc2
        // verifies protocol impl blocks against the protocol, and keyboardWillChange: is ours.
        impl AppDelegate {
            /// Keyboard show/hide/height change: clamp the root's bottom to the keyboard top
            /// (screen coords), tell Day the root resized, then reveal the focused field.
            #[unsafe(method(keyboardWillChange:))]
            fn keyboard_will_change(&self, notification: &objc2_foundation::NSNotification) {
                day_spec::ffi_guard::contain((), || {
                    // The KEY window's scene (docs/windows.md): the keyboard belongs to
                    // whichever Day window holds the focused field.
                    let Some((root, base, target)) = with_key_scene(|e| {
                        (e.root_view.clone(), e.base_frame.get(), key_scene_target(e))
                    }) else {
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
                            target,
                            Event::WindowResized(Size::new(f.size.width, f.size.height)),
                        );
                    }
                    if new_h < base.size.height {
                        reveal_focused_field();
                    }
                });
            }
        }
    );

    // -----------------------------------------------------------------------------------
    // DaySceneDelegate — every scene's window lifecycle (docs/windows.md). The PRIMARY
    // scene (the one consuming the parked `run` payload) mounts the day tree exactly as
    // the pre-scene app delegate did; a SECONDARY scene completes a pending
    // `day::open_window` through `finish_window_open`, or — when the record is gone or the
    // session is a stale restoration — asks the system to destroy itself.
    // -----------------------------------------------------------------------------------

    use objc2_ui_kit::{UISceneDelegate, UIWindowSceneDelegate};

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "DaySceneDelegate"]
        #[ivars = ()]
        struct DaySceneDelegate;

        unsafe impl NSObjectProtocol for DaySceneDelegate {}

        unsafe impl UISceneDelegate for DaySceneDelegate {
            #[unsafe(method(scene:willConnectToSession:options:))]
            fn scene_will_connect(
                &self,
                scene: &objc2_ui_kit::UIScene,
                session: &objc2_ui_kit::UISceneSession,
                options: &objc2_ui_kit::UISceneConnectionOptions,
            ) {
                // Contained (§8.5): `ready` mounts the whole day tree, and
                // `finish_window_open` re-enters day-core.
                day_spec::ffi_guard::contain((), || {
                    let mtm = self.mtm();
                    let Some(win_scene) = scene.downcast_ref::<objc2_ui_kit::UIWindowScene>()
                    else {
                        return;
                    };
                    // Secondary day window? The request's NSUserActivity names the root node.
                    let node = scene_activity_node(options);
                    if PENDING.with(|p| p.borrow().is_some()) && node.is_none() {
                        // The primary scene: build the window and mount the day tree.
                        let (window, root_view, inner) = build_scene_window(mtm, win_scene);
                        WINDOW.with(|w| *w.borrow_mut() = Some(window.clone()));
                        ROOT_VIEW.with(|r| *r.borrow_mut() = Some(root_view.clone()));
                        ROOT_BASE_FRAME.with(|f| f.set(inner));
                        SCENES.with(|s| {
                            s.borrow_mut().push(SceneEntry {
                                window,
                                root_view: root_view.clone(),
                                base_frame: Cell::new(inner),
                                node: None,
                            })
                        });
                        // The take() cannot miss: the `is_some` gate above just read it, and
                        // both run on the main thread.
                        let Some((backend, _options, ready)) =
                            PENDING.with(|p| p.borrow_mut().take())
                        else {
                            return;
                        };
                        let size = Size::new(inner.size.width, inner.size.height);
                        ready(backend, view_of(root_view), size);
                        // Cold launch via deep link or quick action (docs/deep-links.md): both
                        // ride the connection options; `request_route` buffers until the mount
                        // that `ready` just kicked off completes.
                        scene_connection_routes(options);
                        return;
                    }
                    let Some(node) = node else {
                        // A restored session from a previous run: nothing to mount behind it.
                        request_scene_destruction(mtm, session);
                        return;
                    };
                    PENDING_WINDOWS.with(|p| p.borrow_mut().retain(|(n, _)| *n != node));
                    let (window, root_view, inner) = build_scene_window(mtm, win_scene);
                    let size = Size::new(inner.size.width, inner.size.height);
                    let raw = Retained::as_ptr(&root_view) as *mut std::ffi::c_void
                        as day_spec::RawHandle;
                    SCENES.with(|s| {
                        s.borrow_mut().push(SceneEntry {
                            window,
                            root_view: root_view.clone(),
                            base_frame: Cell::new(inner),
                            node: Some(node),
                        })
                    });
                    // Keep the adopted root alive for the entry's lifetime; the tree holds the
                    // other retain through `Toolkit::adopt`.
                    if !day_core::finish_window_open(node, raw, size) {
                        // Closed before the scene connected — drop the scene again.
                        SCENES.with(|s| s.borrow_mut().retain(|e| e.node != Some(node)));
                        request_scene_destruction(mtm, session);
                    }
                });
            }

            #[unsafe(method(sceneDidDisconnect:))]
            fn scene_did_disconnect(&self, scene: &objc2_ui_kit::UIScene) {
                day_spec::ffi_guard::contain((), || {
                    let mtm = self.mtm();
                    if let Some(node) = scene_entry_node_for(scene) {
                        SCENES.with(|s| s.borrow_mut().retain(|e| e.node != Some(node)));
                        // The platform committed the close (app-switcher swipe or our
                        // destruction request): day-core tears the subtree down on receipt.
                        emit(node, Event::WindowClosed);
                    }
                    note_scene_lifecycle_changed(mtm);
                });
            }

            #[unsafe(method(sceneDidBecomeActive:))]
            fn scene_did_become_active(&self, scene: &objc2_ui_kit::UIScene) {
                day_spec::ffi_guard::contain((), || {
                    let mtm = self.mtm();
                    if let Some(node) = scene_entry_node_for(scene) {
                        emit(node, Event::WindowFocused(true));
                    }
                    note_scene_lifecycle_changed(mtm);
                });
            }

            #[unsafe(method(sceneWillResignActive:))]
            fn scene_will_resign_active(&self, scene: &objc2_ui_kit::UIScene) {
                day_spec::ffi_guard::contain((), || {
                    let mtm = self.mtm();
                    if let Some(node) = scene_entry_node_for(scene) {
                        emit(node, Event::WindowFocused(false));
                    }
                    note_scene_lifecycle_changed(mtm);
                });
            }

            #[unsafe(method(sceneWillEnterForeground:))]
            fn scene_will_enter_foreground(&self, _scene: &objc2_ui_kit::UIScene) {
                day_spec::ffi_guard::contain((), || {
                    note_scene_lifecycle_changed(self.mtm());
                });
            }

            #[unsafe(method(sceneDidEnterBackground:))]
            fn scene_did_enter_background(&self, _scene: &objc2_ui_kit::UIScene) {
                day_spec::ffi_guard::contain((), || {
                    note_scene_lifecycle_changed(self.mtm());
                });
            }

            // Warm deep link under the scene lifecycle (docs/deep-links.md): once an app
            // adopts scenes, URL opens arrive HERE, not at the app delegate's
            // `application:openURL:options:` (kept for the pre-scene path).
            #[unsafe(method(scene:openURLContexts:))]
            fn scene_open_url_contexts(
                &self,
                _scene: &objc2_ui_kit::UIScene,
                contexts: &objc2_foundation::NSSet<objc2_ui_kit::UIOpenURLContext>,
            ) {
                day_spec::ffi_guard::contain((), || {
                    for ctx in contexts {
                        if let Some(s) = unsafe { ctx.URL().absoluteString() } {
                            day_core::request_route(&day_spec::route_of_url(&s.to_string()));
                        }
                    }
                });
            }
        }

        unsafe impl UIWindowSceneDelegate for DaySceneDelegate {
            // A home-screen quick action while the app runs (cold arrivals ride the
            // connection options). Its type string IS the saved deep link
            // (docs/deep-links.md "Shortcuts are saved deep links").
            #[unsafe(method(windowScene:performActionForShortcutItem:completionHandler:))]
            fn perform_shortcut(
                &self,
                _scene: &objc2_ui_kit::UIWindowScene,
                item: &objc2_ui_kit::UIApplicationShortcutItem,
                completion: &block2::DynBlock<dyn Fn(objc2::runtime::Bool)>,
            ) {
                day_spec::ffi_guard::contain((), || {
                    day_core::request_route(&day_spec::route_of_url(&item.r#type().to_string()));
                });
                // Report handled even when the route dispatch was contained — the system's
                // completion contract is unconditional.
                completion.call((objc2::runtime::Bool::YES,));
            }
        }
    );

    /// Deep links riding a scene's connection options — the URL that launched the app, or a
    /// quick action's type string. One rail either way: `day_core::request_route`, buffered
    /// until the first mount (docs/deep-links.md).
    fn scene_connection_routes(options: &objc2_ui_kit::UISceneConnectionOptions) {
        // Raw message send: the generated `URLContexts()` binding declares the return
        // non-null, but a plain launch (no URL) hands back nil and the binding panics —
        // caught by the dayscript walkthrough on first run.
        let contexts: Option<Retained<objc2_foundation::NSSet<objc2_ui_kit::UIOpenURLContext>>> =
            unsafe { objc2::msg_send![options, URLContexts] };
        for ctx in contexts.into_iter().flatten() {
            if let Some(s) = unsafe { ctx.URL().absoluteString() } {
                day_core::request_route(&day_spec::route_of_url(&s.to_string()));
            }
        }
        if let Some(item) = options.shortcutItem() {
            day_core::request_route(&day_spec::route_of_url(&item.r#type().to_string()));
        }
    }

    /// The day root node a secondary-scene connection carries (`DAY_WINDOW_ACTIVITY`
    /// userActivity, `day.node` userInfo), if any.
    fn scene_activity_node(options: &objc2_ui_kit::UISceneConnectionOptions) -> Option<NodeId> {
        for activity in unsafe { options.userActivities() } {
            if unsafe { activity.activityType() }.to_string() == DAY_WINDOW_ACTIVITY
                && let Some(info) = unsafe { activity.userInfo() }
                && let Some(num) = info
                    .objectForKey(&*objc2_foundation::NSString::from_str("day.node"))
                    .and_then(|o| o.downcast::<objc2_foundation::NSNumber>().ok())
            {
                return Some(NodeId(num.as_u64()));
            }
        }
        None
    }

    /// The registry node of the scene owning this window, if it is a secondary day window.
    fn scene_entry_node_for(scene: &objc2_ui_kit::UIScene) -> Option<NodeId> {
        let win_scene = scene.downcast_ref::<objc2_ui_kit::UIWindowScene>()?;
        SCENES.with(|s| {
            s.borrow()
                .iter()
                .find(|e| {
                    e.window
                        .windowScene()
                        .is_some_and(|ws| std::ptr::eq(&*ws, win_scene))
                })
                .and_then(|e| e.node)
        })
    }

    /// Ask the system to drop a scene session (no undo UI, no animation preference).
    fn request_scene_destruction(mtm: MainThreadMarker, session: &objc2_ui_kit::UISceneSession) {
        let app = UIApplication::sharedApplication(mtm);
        unsafe {
            app.requestSceneSessionDestruction_options_errorHandler(session, None, None);
        }
    }

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

        fn locale_hints(&self) -> Vec<String> {
            // The user's ordered language preference from Settings ("fr-FR", "en-US", …), which is
            // the ambient locale Day negotiates its catalogs against (§12.2, docs/localization.md).
            objc2_foundation::NSLocale::preferredLanguages()
                .iter()
                .map(|s| s.to_string())
                .collect()
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
