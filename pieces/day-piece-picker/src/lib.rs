//! day-piece-picker — an EXTERNAL Day Piece (DESIGN.md §15 tier 1): one Rust API, three SwiftUI-style
//! stylings (`.menu`, `.segmented`, `.inline`) each realized as a NATIVE control per toolkit, registered
//! link-time into each backend's renderer slice with **zero edits** to day. Bound two-way to a selection.

use day_core::{BuildCx, Flex, Piece, RNode, with_tree};
use day_pieces::SignalRw;
use day_reactive::{Signal, bind_seeded};
use day_spec::Event;

pub const KIND: &str = "day.piece.picker";

/// SwiftUI's `pickerStyle` analogue. Each maps to a distinct native control per toolkit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PickerStyle {
    /// A dropdown/pop-up menu (NSPopUpButton / GtkDropDown / QComboBox / UIButton+UIMenu / Spinner).
    #[default]
    Menu,
    /// A horizontal segmented control (NSSegmentedControl / UISegmentedControl / linked toggles / …).
    Segmented,
    /// A vertical radio-button group laid out inline (NSButton radios / GtkCheckButton group / …).
    Inline,
}

/// Full props (realize). `options`/`style` are set once at build; only `selected` patches.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PickerProps {
    pub options: Vec<String>,
    pub selected: usize,
    pub style: PickerStyle,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PickerPatch {
    Selected(usize),
}

/// A native picker bound two-way to `selected`. Style via `.menu()`/`.segmented()`/`.inline()`.
pub struct Picker {
    options: Vec<String>,
    selected: Signal<usize>,
    style: PickerStyle,
}

/// `picker(["A", "B", "C"], choice).segmented()` — options are fixed, `selected` is the bound index.
pub fn picker<S: Into<String>>(
    options: impl IntoIterator<Item = S>,
    selected: Signal<usize>,
) -> Picker {
    Picker {
        options: options.into_iter().map(Into::into).collect(),
        selected,
        style: PickerStyle::Menu,
    }
}

impl Picker {
    pub fn menu(mut self) -> Self {
        self.style = PickerStyle::Menu;
        self
    }
    pub fn segmented(mut self) -> Self {
        self.style = PickerStyle::Segmented;
        self
    }
    pub fn inline(mut self) -> Self {
        self.style = PickerStyle::Inline;
        self
    }
    pub fn style(mut self, style: PickerStyle) -> Self {
        self.style = style;
        self
    }
}

impl Piece for Picker {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let Picker {
            options,
            selected,
            style,
        } = self;
        let initial = PickerProps {
            options,
            selected: selected.get_untracked(),
            style,
        };
        let node = cx.leaf(KIND, &initial, Flex::default());
        bind_seeded(
            initial.selected,
            move || selected.get(),
            move |v: &usize| {
                with_tree(|t| t.patch(node, Box::new(PickerPatch::Selected(*v)), false));
            },
        );
        cx.on(node, move |ev| {
            if let Event::SelectionChanged(i) = ev
                && *i >= 0
            {
                selected.set_rw(*i as usize);
            }
        });
        node
    }
}

// ---------------------------------------------------------------------------
// AppKit: NSPopUpButton (menu) / NSSegmentedControl (segmented) / NSButton radio group (inline)
// ---------------------------------------------------------------------------

#[cfg(all(feature = "appkit", target_os = "macos"))]
mod appkit_impl {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    use day_appkit::AppKit;
    use day_spec::{NodeId, Proposal, Renderer, Size};
    use linkme::distributed_slice;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, NSObjectProtocol};
    use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
    use objc2_app_kit::{
        NSButton, NSControlStateValueOn, NSPopUpButton, NSSegmentSwitchTracking,
        NSSegmentedControl, NSStackView, NSUserInterfaceLayoutOrientation, NSView,
    };
    use objc2_foundation::{NSObject, NSPoint, NSRect, NSSize, NSString};

    struct PickerIvars {
        node: NodeId,
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "DayPickerTarget"]
        #[ivars = PickerIvars]
        struct PickerTarget;

        unsafe impl NSObjectProtocol for PickerTarget {}

        impl PickerTarget {
            // One action for all three styles — read the selected index off whichever sender fired.
            #[unsafe(method(fire:))]
            fn fire(&self, sender: &AnyObject) {
                let idx = if let Some(p) = sender.downcast_ref::<NSPopUpButton>() {
                    p.indexOfSelectedItem()
                } else if let Some(s) = sender.downcast_ref::<NSSegmentedControl>() {
                    s.selectedSegment()
                } else if let Some(b) = sender.downcast_ref::<NSButton>() {
                    b.tag()
                } else {
                    -1
                };
                if idx >= 0 {
                    day_appkit::emit(self.ivars().node, Event::SelectionChanged(idx as i64));
                }
            }
        }
    );

    impl PickerTarget {
        fn new(mtm: MainThreadMarker, node: NodeId) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(PickerIvars { node });
            unsafe { msg_send![super(this), init] }
        }
    }

    thread_local! {
        static TARGETS: RefCell<HashMap<usize, Retained<PickerTarget>>> =
            RefCell::new(HashMap::new());
    }

    fn zero_rect() -> NSRect {
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(0.0, 0.0))
    }

    fn make_menu(
        mtm: MainThreadMarker,
        p: &PickerProps,
        target: &PickerTarget,
    ) -> Retained<NSView> {
        let popup =
            NSPopUpButton::initWithFrame_pullsDown(NSPopUpButton::alloc(mtm), zero_rect(), false);
        for opt in &p.options {
            popup.addItemWithTitle(&NSString::from_str(opt));
        }
        popup.selectItemAtIndex(p.selected as isize);
        unsafe {
            popup.setTarget(Some(target));
            popup.setAction(Some(sel!(fire:)));
        }
        Retained::from(<NSPopUpButton as AsRef<NSView>>::as_ref(&popup))
    }

    fn make_segmented(
        mtm: MainThreadMarker,
        p: &PickerProps,
        target: &PickerTarget,
    ) -> Retained<NSView> {
        let seg = NSSegmentedControl::new(mtm);
        seg.setSegmentCount(p.options.len() as isize);
        seg.setTrackingMode(NSSegmentSwitchTracking::SelectOne);
        for (i, opt) in p.options.iter().enumerate() {
            seg.setLabel_forSegment(&NSString::from_str(opt), i as isize);
        }
        if p.selected < p.options.len() {
            seg.setSelectedSegment(p.selected as isize);
        }
        unsafe {
            seg.setTarget(Some(target));
            seg.setAction(Some(sel!(fire:)));
        }
        Retained::from(<NSSegmentedControl as AsRef<NSView>>::as_ref(&seg))
    }

    fn make_inline(
        mtm: MainThreadMarker,
        p: &PickerProps,
        target: &PickerTarget,
    ) -> Retained<NSView> {
        let stack = NSStackView::new(mtm);
        stack.setOrientation(NSUserInterfaceLayoutOrientation::Vertical);
        stack.setSpacing(4.0);
        stack.setAlignment(objc2_app_kit::NSLayoutAttribute::Leading);
        for (i, opt) in p.options.iter().enumerate() {
            // Radio buttons sharing a superview + action auto-group (mutually exclusive).
            let radio = unsafe {
                NSButton::radioButtonWithTitle_target_action(
                    &NSString::from_str(opt),
                    Some(target),
                    Some(sel!(fire:)),
                    mtm,
                )
            };
            radio.setTag(i as isize);
            if i == p.selected {
                radio.setState(NSControlStateValueOn);
            }
            stack.addArrangedSubview(<NSButton as AsRef<NSView>>::as_ref(&radio));
        }
        Retained::from(<NSStackView as AsRef<NSView>>::as_ref(&stack))
    }

    fn make(backend: &mut AppKit, props: &dyn std::any::Any, id: NodeId) -> Retained<NSView> {
        let p = props.downcast_ref::<PickerProps>().unwrap();
        let mtm = backend.mtm();
        let target = PickerTarget::new(mtm, id);
        let view = match p.style {
            PickerStyle::Menu => make_menu(mtm, p, &target),
            PickerStyle::Segmented => make_segmented(mtm, p, &target),
            PickerStyle::Inline => make_inline(mtm, p, &target),
        };
        TARGETS.with(|m| {
            m.borrow_mut()
                .insert((view.as_ref() as *const NSView) as usize, target)
        });
        view
    }

    fn update(_backend: &mut AppKit, h: &Retained<NSView>, patch: &dyn std::any::Any) {
        let Some(PickerPatch::Selected(i)) = patch.downcast_ref::<PickerPatch>() else {
            return;
        };
        let i = *i;
        if let Some(popup) = h.downcast_ref::<NSPopUpButton>() {
            if popup.indexOfSelectedItem() != i as isize {
                popup.selectItemAtIndex(i as isize);
            }
        } else if let Some(seg) = h.downcast_ref::<NSSegmentedControl>() {
            if seg.selectedSegment() != i as isize {
                seg.setSelectedSegment(i as isize);
            }
        } else if let Some(stack) = h.downcast_ref::<NSStackView>() {
            // Inline: turn on the i-th radio (its group turns the others off).
            let subs = stack.arrangedSubviews();
            if let Some(v) = subs.iter().nth(i)
                && let Some(b) = v.downcast_ref::<NSButton>()
            {
                b.setState(NSControlStateValueOn);
            }
        }
    }

    fn measure(_backend: &mut AppKit, h: &Retained<NSView>, _p: Proposal) -> Size {
        let s = h.fittingSize();
        Size::new(s.width.ceil().max(60.0), s.height.ceil().max(22.0))
    }

    #[distributed_slice(day_appkit::RENDERERS)]
    static PICKER_APPKIT: fn() -> Renderer<AppKit> = || Renderer {
        kind: KIND,
        make,
        update,
        measure: Some(measure),
    };
}

// ---------------------------------------------------------------------------
// GTK: GtkDropDown (menu) / `.linked` grouped ToggleButtons (segmented) / grouped
// CheckButton radios (inline). Echo-guarded so programmatic selection doesn't loop.
// ---------------------------------------------------------------------------

#[cfg(feature = "gtk")]
mod gtk_impl {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;
    use std::rc::Rc;

    use day_gtk::Gtk;
    use day_spec::{NodeId, Proposal, Renderer, Size};
    use gtk4::prelude::*;
    use linkme::distributed_slice;

    struct PickerState {
        dropdown: Option<gtk4::DropDown>,
        toggles: Vec<gtk4::ToggleButton>, // segmented
        checks: Vec<gtk4::CheckButton>,   // inline (radio)
        suppress: Rc<Cell<bool>>,
    }

    thread_local! {
        static STATE: RefCell<HashMap<usize, PickerState>> = RefCell::new(HashMap::new());
    }

    fn key(w: &gtk4::Widget) -> usize {
        w.as_ptr() as usize
    }

    fn make_menu(p: &PickerProps, id: NodeId, suppress: Rc<Cell<bool>>) -> gtk4::DropDown {
        let refs: Vec<&str> = p.options.iter().map(|s| s.as_str()).collect();
        let dd = gtk4::DropDown::new(Some(gtk4::StringList::new(&refs)), gtk4::Expression::NONE);
        dd.set_selected(p.selected as u32);
        dd.connect_selected_notify(move |d| {
            if suppress.get() {
                return;
            }
            let sel = d.selected();
            if sel != gtk4::INVALID_LIST_POSITION {
                day_gtk::emit(id, Event::SelectionChanged(sel as i64));
            }
        });
        dd
    }

    fn make(_backend: &mut Gtk, props: &dyn std::any::Any, id: NodeId) -> gtk4::Widget {
        let p = props.downcast_ref::<PickerProps>().unwrap();
        let suppress = Rc::new(Cell::new(false));
        let (root, state): (gtk4::Widget, PickerState) = match p.style {
            PickerStyle::Menu => {
                let dd = make_menu(p, id, suppress.clone());
                (
                    dd.clone().upcast(),
                    PickerState {
                        dropdown: Some(dd),
                        toggles: vec![],
                        checks: vec![],
                        suppress,
                    },
                )
            }
            PickerStyle::Segmented => {
                let bx = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
                bx.add_css_class("linked"); // segmented appearance
                bx.set_halign(gtk4::Align::Start);
                let mut toggles = Vec::new();
                for (i, opt) in p.options.iter().enumerate() {
                    let t = gtk4::ToggleButton::with_label(opt);
                    if let Some(first) = toggles.first() {
                        t.set_group(Some(first)); // mutually exclusive
                    }
                    let suppress = suppress.clone();
                    t.connect_toggled(move |t| {
                        if suppress.get() || !t.is_active() {
                            return;
                        }
                        day_gtk::emit(id, Event::SelectionChanged(i as i64));
                    });
                    bx.append(&t);
                    toggles.push(t);
                }
                if let Some(t) = toggles.get(p.selected) {
                    suppress.set(true);
                    t.set_active(true);
                    suppress.set(false);
                }
                (
                    bx.upcast(),
                    PickerState {
                        dropdown: None,
                        toggles,
                        checks: vec![],
                        suppress,
                    },
                )
            }
            PickerStyle::Inline => {
                let bx = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
                bx.set_halign(gtk4::Align::Start);
                let mut checks = Vec::new();
                for (i, opt) in p.options.iter().enumerate() {
                    let c = gtk4::CheckButton::with_label(opt); // grouped ⇒ radio
                    if let Some(first) = checks.first() {
                        c.set_group(Some(first));
                    }
                    let suppress = suppress.clone();
                    c.connect_toggled(move |c| {
                        if suppress.get() || !c.is_active() {
                            return;
                        }
                        day_gtk::emit(id, Event::SelectionChanged(i as i64));
                    });
                    bx.append(&c);
                    checks.push(c);
                }
                if let Some(c) = checks.get(p.selected) {
                    suppress.set(true);
                    c.set_active(true);
                    suppress.set(false);
                }
                (
                    bx.upcast(),
                    PickerState {
                        dropdown: None,
                        toggles: vec![],
                        checks,
                        suppress,
                    },
                )
            }
        };
        STATE.with(|m| m.borrow_mut().insert(key(&root), state));
        root
    }

    fn update(_backend: &mut Gtk, h: &gtk4::Widget, patch: &dyn std::any::Any) {
        let Some(PickerPatch::Selected(i)) = patch.downcast_ref::<PickerPatch>() else {
            return;
        };
        let i = *i;
        STATE.with(|m| {
            let m = m.borrow();
            let Some(st) = m.get(&key(h)) else {
                return;
            };
            st.suppress.set(true);
            if let Some(dd) = &st.dropdown {
                if dd.selected() as usize != i {
                    dd.set_selected(i as u32);
                }
            } else if let Some(t) = st.toggles.get(i) {
                t.set_active(true);
            } else if let Some(c) = st.checks.get(i) {
                c.set_active(true);
            }
            st.suppress.set(false);
        });
    }

    fn measure(_backend: &mut Gtk, h: &gtk4::Widget, _p: Proposal) -> Size {
        let (_, nat_w, _, _) = h.measure(gtk4::Orientation::Horizontal, -1);
        let (_, nat_h, _, _) = h.measure(gtk4::Orientation::Vertical, -1);
        Size::new((nat_w as f64).max(60.0), (nat_h as f64).max(22.0))
    }

    #[distributed_slice(day_gtk::RENDERERS)]
    static PICKER_GTK: fn() -> Renderer<Gtk> = || Renderer {
        kind: KIND,
        make,
        update,
        measure: Some(measure),
    };
}

// ---------------------------------------------------------------------------
// Qt: this crate's OWN shim (src/qt_shim.cpp) — QComboBox / checkable QPushButtons /
// QRadioButtons, one DayPicker widget per style behind a flat C ABI.
// ---------------------------------------------------------------------------

#[cfg(feature = "qt")]
mod qt_impl {
    use super::*;
    use std::ffi::CString;
    use std::os::raw::{c_char, c_int, c_void};

    use day_qt::{Qt, QtHandle};
    use day_spec::{NodeId, Proposal, Renderer, Size};
    use linkme::distributed_slice;

    unsafe extern "C" {
        fn day_picker_new(
            style: c_int,
            items_joined: *const c_char,
            selected: c_int,
            id: u64,
            cb: extern "C" fn(u64, c_int),
        ) -> *mut c_void;
        fn day_picker_set_selected(w: *mut c_void, idx: c_int);
        // From day-qt-sys (already linked into the binary):
        fn day_qt_size_hint(w: *mut c_void, out_w: *mut f64, out_h: *mut f64);
    }

    extern "C" fn on_select(id: u64, idx: c_int) {
        day_qt::emit(NodeId(id), Event::SelectionChanged(idx as i64));
    }

    fn joined(items: &[String]) -> CString {
        CString::new(items.join("\n")).unwrap_or_default()
    }

    fn style_code(s: PickerStyle) -> c_int {
        match s {
            PickerStyle::Menu => 0,
            PickerStyle::Segmented => 1,
            PickerStyle::Inline => 2,
        }
    }

    fn make(_backend: &mut Qt, props: &dyn std::any::Any, id: NodeId) -> QtHandle {
        let p = props.downcast_ref::<PickerProps>().unwrap();
        QtHandle(unsafe {
            day_picker_new(
                style_code(p.style),
                joined(&p.options).as_ptr(),
                p.selected as c_int,
                id.0,
                on_select,
            )
        })
    }

    fn update(_backend: &mut Qt, h: &QtHandle, patch: &dyn std::any::Any) {
        if let Some(PickerPatch::Selected(i)) = patch.downcast_ref::<PickerPatch>() {
            unsafe { day_picker_set_selected(h.0, *i as c_int) };
        }
    }

    fn measure(_backend: &mut Qt, h: &QtHandle, _p: Proposal) -> Size {
        let mut w = 0.0;
        let mut hh = 0.0;
        unsafe { day_qt_size_hint(h.0, &mut w, &mut hh) };
        Size::new(w.max(60.0), hh.max(22.0))
    }

    #[distributed_slice(day_qt::RENDERERS)]
    static PICKER_QT: fn() -> Renderer<Qt> = || Renderer {
        kind: KIND,
        make,
        update,
        measure: Some(measure),
    };
}

// ---------------------------------------------------------------------------
// UIKit: UIButton+UIMenu pull-down (menu) / UISegmentedControl (segmented) /
// checkmark-row UIStackView (inline).
// ---------------------------------------------------------------------------

#[cfg(all(feature = "uikit", target_os = "ios"))]
mod uikit_impl {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    use block2::RcBlock;
    use day_spec::{NodeId, Proposal, Renderer, Size};
    use day_uikit::Uikit;
    use linkme::distributed_slice;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, NSObjectProtocol};
    use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
    use objc2_core_foundation::CGSize;
    use objc2_foundation::{NSArray, NSString};
    use objc2_ui_kit::{
        UIAction, UIButton, UIControlEvents, UIControlState, UIImage, UILayoutConstraintAxis,
        UIMenu, UIMenuElement, UISegmentedControl, UIStackView, UIStackViewAlignment,
        UIStackViewDistribution, UIView,
    };

    struct TargetIvars {
        node: NodeId,
    }

    define_class!(
        #[unsafe(super(objc2_foundation::NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "DayPickerUIKitTarget"]
        #[ivars = TargetIvars]
        struct PickerTarget;

        unsafe impl NSObjectProtocol for PickerTarget {}

        impl PickerTarget {
            #[unsafe(method(fire:))]
            fn fire(&self, sender: &AnyObject) {
                let idx = if let Some(s) = sender.downcast_ref::<UISegmentedControl>() {
                    s.selectedSegmentIndex()
                } else if let Some(b) = sender.downcast_ref::<UIButton>() {
                    b.tag()
                } else {
                    -1
                };
                if idx >= 0 {
                    day_uikit::emit(self.ivars().node, Event::SelectionChanged(idx as i64));
                }
            }
        }
    );

    impl PickerTarget {
        fn new(mtm: MainThreadMarker, node: NodeId) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(TargetIvars { node });
            unsafe { msg_send![super(this), init] }
        }
    }

    /// Per-inline-view state so `update` can move the checkmark; per-menu-view so it can retitle.
    struct ViewState {
        buttons: Vec<Retained<UIButton>>, // inline rows
        menu_button: Option<Retained<UIButton>>,
        options: Vec<String>,
        _target: Retained<PickerTarget>,
    }

    thread_local! {
        static STATE: RefCell<HashMap<usize, ViewState>> = RefCell::new(HashMap::new());
    }

    fn make_segmented(
        mtm: MainThreadMarker,
        p: &PickerProps,
        target: &PickerTarget,
    ) -> Retained<UIView> {
        let seg = UISegmentedControl::new(mtm);
        for (i, opt) in p.options.iter().enumerate() {
            seg.insertSegmentWithTitle_atIndex_animated(Some(&NSString::from_str(opt)), i, false);
        }
        seg.setSelectedSegmentIndex(p.selected as isize);
        unsafe {
            seg.addTarget_action_forControlEvents(
                Some(target as &AnyObject),
                sel!(fire:),
                UIControlEvents::ValueChanged,
            );
        }
        Retained::from(<UISegmentedControl as AsRef<UIView>>::as_ref(&seg))
    }

    fn checkmark(on: bool) -> Option<Retained<UIImage>> {
        let name = if on {
            "checkmark.circle.fill"
        } else {
            "circle"
        };
        UIImage::systemImageNamed(&NSString::from_str(name))
    }

    fn make_inline(
        mtm: MainThreadMarker,
        p: &PickerProps,
        target: &PickerTarget,
    ) -> (Retained<UIView>, Vec<Retained<UIButton>>) {
        let stack = UIStackView::new(mtm);
        stack.setAxis(UILayoutConstraintAxis::Vertical);
        stack.setAlignment(UIStackViewAlignment::Leading);
        stack.setDistribution(UIStackViewDistribution::EqualSpacing);
        stack.setSpacing(6.0);
        let mut buttons = Vec::new();
        for (i, opt) in p.options.iter().enumerate() {
            let btn = UIButton::buttonWithType(objc2_ui_kit::UIButtonType::System, mtm);
            btn.setTag(i as isize);
            btn.setTitle_forState(Some(&NSString::from_str(opt)), UIControlState::Normal);
            if let Some(img) = checkmark(i == p.selected) {
                btn.setImage_forState(Some(&img), UIControlState::Normal);
            }
            unsafe {
                btn.addTarget_action_forControlEvents(
                    Some(target as &AnyObject),
                    sel!(fire:),
                    UIControlEvents::TouchUpInside,
                );
                stack.addArrangedSubview(<UIButton as AsRef<UIView>>::as_ref(&btn));
            }
            buttons.push(btn);
        }
        (
            Retained::from(<UIStackView as AsRef<UIView>>::as_ref(&stack)),
            buttons,
        )
    }

    fn make_menu(mtm: MainThreadMarker, p: &PickerProps, node: NodeId) -> Retained<UIButton> {
        let btn = UIButton::buttonWithType(objc2_ui_kit::UIButtonType::System, mtm);
        let mut actions: Vec<Retained<UIMenuElement>> = Vec::new();
        for (i, opt) in p.options.iter().enumerate() {
            let handler = RcBlock::new(move |_action: core::ptr::NonNull<UIAction>| {
                day_uikit::emit(node, Event::SelectionChanged(i as i64));
            });
            let action = unsafe {
                UIAction::actionWithTitle_image_identifier_handler(
                    &NSString::from_str(opt),
                    None,
                    None,
                    RcBlock::as_ptr(&handler),
                    mtm,
                )
            };
            if i == p.selected {
                action.setState(objc2_ui_kit::UIMenuElementState::On);
            }
            actions.push(Retained::from(<UIAction as AsRef<UIMenuElement>>::as_ref(
                &action,
            )));
        }
        let arr = NSArray::from_retained_slice(&actions);
        let menu = UIMenu::menuWithTitle_children(&NSString::from_str(""), &arr, mtm);
        btn.setMenu(Some(&menu));
        btn.setShowsMenuAsPrimaryAction(true);
        let title = p.options.get(p.selected).cloned().unwrap_or_default();
        btn.setTitle_forState(Some(&NSString::from_str(&title)), UIControlState::Normal);
        btn
    }

    fn make(_backend: &mut Uikit, props: &dyn std::any::Any, id: NodeId) -> Retained<UIView> {
        let p = props.downcast_ref::<PickerProps>().unwrap();
        let mtm = MainThreadMarker::new().unwrap();
        let target = PickerTarget::new(mtm, id);
        let (view, buttons, menu_button) = match p.style {
            PickerStyle::Segmented => (make_segmented(mtm, p, &target), vec![], None),
            PickerStyle::Inline => {
                let (v, b) = make_inline(mtm, p, &target);
                (v, b, None)
            }
            PickerStyle::Menu => {
                let btn = make_menu(mtm, p, id);
                let v = Retained::from(<UIButton as AsRef<UIView>>::as_ref(&btn));
                (v, vec![], Some(btn))
            }
        };
        STATE.with(|m| {
            m.borrow_mut().insert(
                (view.as_ref() as *const UIView) as usize,
                ViewState {
                    buttons,
                    menu_button,
                    options: p.options.clone(),
                    _target: target,
                },
            )
        });
        view
    }

    fn update(_backend: &mut Uikit, h: &Retained<UIView>, patch: &dyn std::any::Any) {
        let Some(PickerPatch::Selected(i)) = patch.downcast_ref::<PickerPatch>() else {
            return;
        };
        let i = *i;
        if let Some(seg) = (**h).downcast_ref::<UISegmentedControl>() {
            if seg.selectedSegmentIndex() != i as isize {
                seg.setSelectedSegmentIndex(i as isize);
            }
            return;
        }
        STATE.with(|m| {
            let m = m.borrow();
            let Some(st) = m.get(&((h.as_ref() as *const UIView) as usize)) else {
                return;
            };
            if let Some(btn) = &st.menu_button {
                let title = st.options.get(i).cloned().unwrap_or_default();
                btn.setTitle_forState(Some(&NSString::from_str(&title)), UIControlState::Normal);
            }
            for (j, b) in st.buttons.iter().enumerate() {
                if let Some(img) = checkmark(j == i) {
                    b.setImage_forState(Some(&img), UIControlState::Normal);
                }
            }
        });
    }

    fn measure(_backend: &mut Uikit, h: &Retained<UIView>, _p: Proposal) -> Size {
        // A vertical UIStackView is autolayout-driven — `sizeThatFits` under-reports it (rows
        // collapse); ask the constraint solver for the compressed fitting size instead.
        let s = if (**h).downcast_ref::<UIStackView>().is_some() {
            h.systemLayoutSizeFittingSize(CGSize::new(0.0, 0.0))
        } else {
            h.sizeThatFits(CGSize::new(1.0e6, 1.0e6))
        };
        Size::new(s.width.ceil().max(60.0), s.height.ceil().max(28.0))
    }

    #[distributed_slice(day_uikit::RENDERERS)]
    static PICKER_UIKIT: fn() -> Renderer<Uikit> = || Renderer {
        kind: KIND,
        make,
        update,
        measure: Some(measure),
    };
}

// ---------------------------------------------------------------------------
// Android: Spinner (menu) / button-row LinearLayout (segmented) / RadioGroup (inline),
// built by the DayBridge Java factory. `setPickerSelected` handles all three view types.
// ---------------------------------------------------------------------------

#[cfg(all(feature = "widget", target_os = "android"))]
mod android_impl {
    use super::*;
    use day_android::jni::objects::JValue;
    use day_android::{AHandle, Android, make_view, with_env};
    use day_spec::{NodeId, Renderer};
    use linkme::distributed_slice;

    fn style_code(s: PickerStyle) -> i32 {
        match s {
            PickerStyle::Menu => 0,
            PickerStyle::Segmented => 1,
            PickerStyle::Inline => 2,
        }
    }

    fn make(_backend: &mut Android, props: &dyn std::any::Any, id: NodeId) -> AHandle {
        let p = props.downcast_ref::<PickerProps>().unwrap();
        let joined = p.options.join("\n");
        with_env(|env| {
            let s = env.new_string(&joined).expect("items");
            AHandle(make_view(
                env,
                "makePicker",
                "(JILjava/lang/String;I)Landroid/view/View;",
                &[
                    JValue::Long(id.0 as i64),
                    JValue::Int(style_code(p.style)),
                    JValue::Object(&s),
                    JValue::Int(p.selected as i32),
                ],
            ))
        })
    }

    fn update(_backend: &mut Android, h: &AHandle, patch: &dyn std::any::Any) {
        if let Some(PickerPatch::Selected(i)) = patch.downcast_ref::<PickerPatch>() {
            with_env(|env| {
                let _ = env.call_static_method(
                    day_android::BRIDGE,
                    "setPickerSelected",
                    "(Landroid/view/View;I)V",
                    &[JValue::Object(h.0.as_obj()), JValue::Int(*i as i32)],
                );
            });
        }
    }

    #[distributed_slice(day_android::RENDERERS)]
    static PICKER_ANDROID: fn() -> Renderer<Android> = || Renderer {
        kind: KIND,
        make,
        update,
        measure: None,
    };
}

// ---------------------------------------------------------------------------
// WinUI: this crate's OWN C++/WinRT shim (src/winui_shim.cpp) — ComboBox / RadioButton StackPanels,
// boxed into day handles via the `day_winui_box`/`day_winui_unbox` seam day-winui-sys exports. This
// mirrors the Qt renderer (own shim for the control; reuse the sys crate's generic measure).
// Windows-only, built in CI, not verified locally.
// ---------------------------------------------------------------------------

#[cfg(all(feature = "winui", windows))]
mod winui_impl {
    use super::*;
    use std::ffi::CString;
    use std::os::raw::{c_char, c_int, c_void};

    use day_spec::{NodeId, Proposal, Renderer, Size};
    use day_winui::{WinHandle, WinUi};
    use linkme::distributed_slice;

    unsafe extern "C" {
        fn day_picker_winui_new(
            style: c_int,
            items_joined: *const c_char,
            selected: c_int,
            id: u64,
            cb: extern "C" fn(u64, c_int),
        ) -> *mut c_void;
        fn day_picker_winui_set_selected(w: *mut c_void, idx: c_int);
        // Generic size hint from day-winui-sys (already linked) — like the Qt renderer reusing
        // day-qt-sys's `day_qt_size_hint`.
        fn day_winui_measure(
            w: *mut c_void,
            avail_w: f64,
            avail_h: f64,
            out_w: *mut f64,
            out_h: *mut f64,
        );
    }

    extern "C" fn on_select(id: u64, idx: c_int) {
        day_winui::emit(NodeId(id), Event::SelectionChanged(idx as i64));
    }

    fn style_code(s: PickerStyle) -> c_int {
        match s {
            PickerStyle::Menu => 0,
            PickerStyle::Segmented => 1,
            PickerStyle::Inline => 2,
        }
    }

    fn make(_backend: &mut WinUi, props: &dyn std::any::Any, id: NodeId) -> WinHandle {
        let p = props.downcast_ref::<PickerProps>().unwrap();
        let joined = CString::new(p.options.join("\n")).unwrap_or_default();
        WinHandle(unsafe {
            day_picker_winui_new(
                style_code(p.style),
                joined.as_ptr(),
                p.selected as c_int,
                id.0,
                on_select,
            )
        })
    }

    fn update(_backend: &mut WinUi, h: &WinHandle, patch: &dyn std::any::Any) {
        if let Some(PickerPatch::Selected(i)) = patch.downcast_ref::<PickerPatch>() {
            unsafe { day_picker_winui_set_selected(h.0, *i as c_int) };
        }
    }

    fn measure(_backend: &mut WinUi, h: &WinHandle, _p: Proposal) -> Size {
        let mut w = 0.0;
        let mut hh = 0.0;
        unsafe { day_winui_measure(h.0, -1.0, -1.0, &mut w, &mut hh) };
        Size::new(w.max(120.0), hh.max(32.0))
    }

    #[distributed_slice(day_winui::RENDERERS)]
    static PICKER_WINUI: fn() -> Renderer<WinUi> = || Renderer {
        kind: KIND,
        make,
        update,
        measure: Some(measure),
    };
}
