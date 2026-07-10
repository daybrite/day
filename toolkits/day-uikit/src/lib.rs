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
pub mod ext;
#[cfg(target_os = "ios")]
pub use ext::*;

#[cfg(target_os = "ios")]
mod imp {
    use std::any::Any;
    use std::cell::RefCell;
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
        UIColor, UIControl, UIControlEvents, UIControlState, UILabel, UIProgressView, UIScreen,
        UIScrollView, UISlider, UISwitch, UITextBorderStyle, UITextField, UIView, UIViewController,
        UIWindow,
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
        A11yProps, AnimSpec, Cap, DrawOp, Event, EventSink, Font, ListSource, NodeId, PieceKind,
        Platform, Proposal, RawHandle, Rect, Registry, Renderer, Size, Support, Toolkit,
        WINDOW_NODE, WindowOptions, kinds,
    };

    pub type Handle = Retained<UIView>;

    /// The day-core event sink (node-id keyed).
    type Sink = Rc<dyn Fn(NodeId, Event)>;

    thread_local! {
        static SINK: RefCell<Option<Sink>> = const { RefCell::new(None) };
        static TARGETS: RefCell<HashMap<usize, Retained<DayTarget>>> = RefCell::new(HashMap::new());
        static WINDOW: RefCell<Option<Retained<UIWindow>>> = const { RefCell::new(None) };
        #[allow(clippy::type_complexity)]
        static PENDING: RefCell<Option<(Uikit, WindowOptions, Box<dyn FnOnce(Uikit, Handle, Size)>)>> =
            RefCell::new(None);
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
        }
    );

    impl DayTarget {
        fn new(mtm: MainThreadMarker, node: NodeId) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(TargetIvars { node });
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
                // Fewer native VCs than our mirror = a pop completed. Day-initiated pops
                // set expect_pop; anything else is the user's back button / swipe.
                let host = self.ivars().host.get();
                let (emit_back, node) = NAV_STATE.with(|m| {
                    let mut m = m.borrow_mut();
                    let Some(state) = m.get_mut(&host) else {
                        return (false, NodeId(0));
                    };
                    let native = unsafe { nav.viewControllers() }.count();
                    if native < state.vcs.len() {
                        if state.expect_pop.replace(false) {
                            (false, NodeId(0))
                        } else {
                            // Sync the mirror now; Day's remove() will find it gone.
                            state.vcs.truncate(native);
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
    // DayNavTableData — nav_menu() as inset-grouped rows with chevrons
    // -------------------------------------------------------------------

    struct NavTableIvars {
        node: NodeId,
        items: RefCell<Vec<Retained<NSString>>>,
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
                let cell = unsafe {
                    objc2_ui_kit::UITableViewCell::initWithStyle_reuseIdentifier(
                        objc2_ui_kit::UITableViewCell::alloc(mtm),
                        objc2_ui_kit::UITableViewCellStyle::Default,
                        None,
                    )
                };
                let row = unsafe { index_path.row() } as usize;
                let items = self.ivars().items.borrow();
                if let Some(title) = items.get(row) {
                    // textLabel is soft-deprecated for UIListContentConfiguration; the
                    // classic API keeps this dependency-light and renders identically.
                    #[allow(deprecated)]
                    if let Some(label) = unsafe { cell.textLabel() } {
                        unsafe { label.setText(Some(title)) };
                    }
                }
                unsafe {
                    cell.setAccessoryType(
                        objc2_ui_kit::UITableViewCellAccessoryType::DisclosureIndicator,
                    )
                };
                cell
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

    impl DayNavTableData {
        fn new(mtm: MainThreadMarker, node: NodeId, items: &[String]) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(NavTableIvars {
                node,
                items: RefCell::new(items.iter().map(|s| NSString::from_str(s)).collect()),
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
                DrawOp::Fill(shape, color) => {
                    uicolor(*color).setFill();
                    if let Some(p) = bezier(shape) {
                        p.fill();
                    }
                }
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
                Cap::Dialogs | Cap::FileDialogs => Support::Native,
                _ => Support::Unsupported,
            }
        }

        fn realize(&mut self, kind: PieceKind, props: &dyn Any, id: NodeId) -> Handle {
            let mtm = mtm();
            match kind {
                kinds::CONTAINER => {
                    let v = unsafe { UIView::new(mtm) };
                    if let Some(p) = props.downcast_ref::<ContainerProps>()
                        && (p.background.is_some() || p.corner_radius > 0.0 || p.clips)
                    {
                        apply_surface(&v, p.background, p.corner_radius, p.clips);
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
                    let handle = view_of(content);
                    TABS_PAGE_VCS.with(|m| m.borrow_mut().insert(ptr_of(&handle), vc));
                    NAV_PAGES.with(|set| set.borrow_mut().insert(ptr_of(&handle)));
                    handle
                }
                kinds::NAV_MENU => {
                    let p = props.downcast_ref::<NavMenuProps>().unwrap();
                    let data = DayNavTableData::new(mtm, id, &p.items);
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
                    view_of(label)
                }
                kinds::BUTTON => {
                    let p = props.downcast_ref::<ButtonProps>().unwrap();
                    let target = DayTarget::new(mtm, id);
                    let btn = unsafe { UIButton::buttonWithType(UIButtonType::System, mtm) };
                    unsafe {
                        btn.setTitle_forState(
                            Some(&NSString::from_str(&p.title)),
                            UIControlState::Normal,
                        );
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
            _anim: Option<&AnimSpec>,
        ) {
            match kind {
                kinds::CONTAINER => {
                    if let Some(ContainerPatch::Background(c)) =
                        patch.downcast_ref::<ContainerPatch>()
                    {
                        unsafe {
                            match c {
                                Some(c) => h.setBackgroundColor(Some(&uicolor(*c))),
                                None => h.setBackgroundColor(None),
                            }
                        }
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
                                    state.expect_pop.set(true);
                                    Act::Pop(state.nav.clone())
                                }
                                NavPatch::Title(_) => Act::None,
                            }
                        });
                        match act {
                            Act::Push(vc, nav) => unsafe {
                                nav.pushViewController_animated(&vc, true)
                            },
                            Act::Pop(nav) => {
                                let _ = unsafe { nav.popViewControllerAnimated(true) };
                            }
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
                            LabelPatch::Color(_) => {}
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
                                btn.setTitle_forState(
                                    Some(&NSString::from_str(t)),
                                    UIControlState::Normal,
                                )
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
                None => unsafe { parent.addSubview(child) },
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

        fn set_frame(&mut self, h: &Handle, frame: Rect, _anim: Option<&AnimSpec>) {
            // Nav page content: the page view pins it to the safe area (native-owned).
            if NAV_PAGES.with(|set| set.borrow().contains(&ptr_of(h))) {
                return;
            }
            let f = CGRect::new(
                CGPoint::new(frame.origin.x, frame.origin.y),
                CGSize::new(frame.size.width, frame.size.height),
            );
            unsafe { h.setFrame(f) };
        }

        fn set_scroll_content(&mut self, h: &Handle, content: Size) {
            if let Some(sv) = (**h).downcast_ref::<UIScrollView>() {
                unsafe { sv.setContentSize(CGSize::new(content.width, content.height)) };
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
            let Some(top) = topmost_vc() else {
                emit(
                    WINDOW_NODE,
                    Event::PresentResult {
                        req,
                        result: PresentResult::Dismissed,
                    },
                );
                return;
            };
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
                            PRESENT_VCS.with(|p| {
                                p.borrow_mut().remove(&req);
                            });
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
                    unsafe { top.presentViewController_animated_completion(&ac, true, None) };
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
                        PRESENT_VCS.with(|p| {
                            p.borrow_mut().remove(&req);
                        });
                    });
                    let cancel_handler = block2::RcBlock::new(move |_: NonNull<UIAlertAction>| {
                        emit(
                            WINDOW_NODE,
                            Event::PresentResult {
                                req,
                                result: PresentResult::Dismissed,
                            },
                        );
                        PRESENT_VCS.with(|p| {
                            p.borrow_mut().remove(&req);
                        });
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
                    unsafe { top.presentViewController_animated_completion(&ac, true, None) };
                }
                // Native file pickers: UIDocumentPickerViewController with a delegate. Open uses
                // `.import` mode (the system hands back an app-local copy, readable via std::fs);
                // save exports the Day-staged temp file to the chosen destination.
                PresentSpec::OpenFile { .. } => {
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
                    present_doc_picker(req, m, &top, picker);
                }
                PresentSpec::SaveFile { src_path, .. } => {
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
                    present_doc_picker(req, m, &top, picker);
                }
            }
        }

        fn dismiss(&mut self, req: u64) {
            if let Some(ac) = PRESENT_VCS.with(|p| p.borrow_mut().remove(&req)) {
                unsafe { ac.dismissViewControllerAnimated_completion(true, None) };
            }
            if let Some((picker, _)) = PRESENT_PICKERS.with(|p| p.borrow_mut().remove(&req)) {
                unsafe { picker.dismissViewControllerAnimated_completion(true, None) };
            }
        }
    }

    /// Wire a document picker's delegate, retain both, and present it on `top`.
    fn present_doc_picker(
        req: u64,
        m: MainThreadMarker,
        top: &UIViewController,
        picker: Retained<UIDocumentPickerViewController>,
    ) {
        unsafe { picker.setAllowsMultipleSelection(false) };
        let delegate = DayDocPicker::new(m, req);
        unsafe { picker.setDelegate(Some(ProtocolObject::from_ref(&*delegate))) };
        PRESENT_PICKERS.with(|p| p.borrow_mut().insert(req, (picker.clone(), delegate)));
        unsafe { top.presentViewController_animated_completion(&picker, true, None) };
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
    }

    /// The frontmost view controller (walk past any already-presented modal).
    fn topmost_vc() -> Option<Retained<UIViewController>> {
        let mut vc = WINDOW.with(|w| w.borrow().clone())?.rootViewController()?;
        while let Some(p) = vc.presentedViewController() {
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
                let vc = unsafe { UIViewController::new(mtm) };
                let holder = unsafe { UIView::initWithFrame(UIView::alloc(mtm), bounds) };
                let root_view = unsafe { UIView::initWithFrame(UIView::alloc(mtm), bounds) };
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
    }
}
