//! day-piece-combobox — an EXTERNAL Day Piece (DESIGN.md §15 tier 1, Appendix B.1): one Rust
//! API, per-toolkit native renderers registered link-time into each backend's slice, with
//! no edits to Day or its toolkit crates. The Qt renderer even carries its own C++ shim.

use day_core::{AnyPiece, piece_fn, with_tree};
use day_pieces::SignalRw;
use day_reactive::{Signal, bind_seeded};
use day_spec::Event;

pub const KIND: &str = "day.piece.combobox";

/// Full props (realize) — external pieces follow the same sparse-patch convention as built-ins.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ComboProps {
    pub items: Vec<String>,
    pub selected: Option<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ComboPatch {
    Items(Vec<String>),
    Selected(Option<usize>),
}

/// The cross-toolkit front-end: a native dropdown bound two-way to `selected`.
pub fn combo_box(items: Signal<Vec<String>>, selected: Signal<Option<usize>>) -> AnyPiece {
    piece_fn(move |cx| {
        let initial = ComboProps {
            items: items.get_untracked(),
            selected: selected.get_untracked(),
        };
        let node = cx.leaf(KIND, &initial, day_core::Flex::default());
        let seed_items = initial.items.clone();
        bind_seeded(
            seed_items,
            move || items.get(),
            move |v: &Vec<String>| {
                let patch = ComboPatch::Items(v.clone());
                with_tree(|t| t.patch(node, Box::new(patch), true));
            },
        );
        let seed_sel = initial.selected;
        bind_seeded(
            seed_sel,
            move || selected.get(),
            move |v: &Option<usize>| {
                let patch = ComboPatch::Selected(*v);
                with_tree(|t| t.patch(node, Box::new(patch), false));
            },
        );
        cx.on(node, move |ev| {
            if let Event::SelectionChanged(i) = ev {
                let v = if *i < 0 { None } else { Some(*i as usize) };
                selected.set_rw(v);
            }
        });
        node
    })
}

// ---------------------------------------------------------------------------
// AppKit renderer: NSPopUpButton (own target class — full native work in an external crate)
// ---------------------------------------------------------------------------

#[cfg(all(feature = "appkit", target_os = "macos"))]
mod appkit_impl {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    use day_appkit::AppKit;
    use day_spec::{NodeId, Proposal, Size};
    use objc2::rc::Retained;
    use objc2::runtime::NSObjectProtocol;
    use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
    use objc2_app_kit::{NSPopUpButton, NSView};
    use objc2_foundation::{NSObject, NSString};

    struct ComboIvars {
        node: NodeId,
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "DayComboTarget"]
        #[ivars = ComboIvars]
        struct ComboTarget;

        unsafe impl NSObjectProtocol for ComboTarget {}

        impl ComboTarget {
            #[unsafe(method(action:))]
            fn action(&self, sender: &NSPopUpButton) {
                let idx = sender.indexOfSelectedItem();
                day_appkit::emit(self.ivars().node, Event::SelectionChanged(idx as i64));
            }
        }
    );

    impl ComboTarget {
        fn new(mtm: MainThreadMarker, node: NodeId) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(ComboIvars { node });
            unsafe { msg_send![super(this), init] }
        }
    }

    thread_local! {
        static TARGETS: RefCell<HashMap<usize, Retained<ComboTarget>>> =
            RefCell::new(HashMap::new());
    }

    fn apply_items(popup: &NSPopUpButton, items: &[String], selected: Option<usize>) {
        popup.removeAllItems();
        for item in items {
            popup.addItemWithTitle(&NSString::from_str(item));
        }
        if let Some(i) = selected {
            popup.selectItemAtIndex(i as isize);
        }
    }

    fn make(backend: &mut AppKit, p: &ComboProps, id: NodeId) -> Retained<NSView> {
        let mtm = backend.mtm();
        let target = ComboTarget::new(mtm, id);
        let zero = objc2_foundation::NSRect::new(
            objc2_foundation::NSPoint::new(0.0, 0.0),
            objc2_foundation::NSSize::new(0.0, 0.0),
        );
        let popup = NSPopUpButton::initWithFrame_pullsDown(NSPopUpButton::alloc(mtm), zero, false);
        apply_items(&popup, &p.items, p.selected);
        unsafe {
            popup.setTarget(Some(&*target));
            popup.setAction(Some(sel!(action:)));
        }
        let view: Retained<NSView> =
            Retained::from(<NSPopUpButton as AsRef<NSView>>::as_ref(&popup));
        TARGETS.with(|m| {
            m.borrow_mut()
                .insert((view.as_ref() as *const NSView) as usize, target)
        });
        view
    }

    fn update(_backend: &mut AppKit, h: &Retained<NSView>, patch: &ComboPatch) {
        let Some(popup) = h.downcast_ref::<NSPopUpButton>() else {
            return;
        };
        {
            let p = patch;
            match p {
                ComboPatch::Items(items) => apply_items(popup, items, None),
                ComboPatch::Selected(sel) => match sel {
                    Some(i) => {
                        if popup.indexOfSelectedItem() != *i as isize {
                            popup.selectItemAtIndex(*i as isize);
                        }
                    }
                    None => popup.selectItemAtIndex(-1),
                },
            }
        }
    }

    fn measure(_backend: &mut AppKit, h: &Retained<NSView>, _p: Proposal) -> Size {
        let s = h.fittingSize();
        Size::new(s.width.ceil().max(80.0), s.height.ceil())
    }

    day_pieces::renderer!(day_appkit::RENDERERS, AppKit, kind: KIND, props: ComboProps, patch: ComboPatch, make: make, update: update, measure: measure);
}

// ---------------------------------------------------------------------------
// GTK renderer: GtkDropDown
// ---------------------------------------------------------------------------

#[cfg(feature = "gtk")]
mod gtk_impl {
    use super::*;
    use day_gtk::Gtk;
    use day_spec::{NodeId, Proposal, Size};
    use gtk4::prelude::*;

    fn strings(items: &[String]) -> gtk4::StringList {
        let refs: Vec<&str> = items.iter().map(|s| s.as_str()).collect();
        gtk4::StringList::new(&refs)
    }

    fn make(_backend: &mut Gtk, p: &ComboProps, id: NodeId) -> gtk4::Widget {
        let dd = gtk4::DropDown::new(Some(strings(&p.items)), gtk4::Expression::NONE);
        if let Some(i) = p.selected {
            dd.set_selected(i as u32);
        }
        dd.connect_selected_notify(move |d| {
            let sel = d.selected();
            let idx = if sel == gtk4::INVALID_LIST_POSITION {
                -1
            } else {
                sel as i64
            };
            day_gtk::emit(id, Event::SelectionChanged(idx));
        });
        dd.upcast()
    }

    fn update(_backend: &mut Gtk, h: &gtk4::Widget, patch: &ComboPatch) {
        let Some(dd) = h.downcast_ref::<gtk4::DropDown>() else {
            return;
        };
        {
            let p = patch;
            match p {
                ComboPatch::Items(items) => dd.set_model(Some(&strings(items))),
                ComboPatch::Selected(sel) => {
                    let want = sel.map(|i| i as u32).unwrap_or(gtk4::INVALID_LIST_POSITION);
                    if dd.selected() != want {
                        dd.set_selected(want);
                    }
                }
            }
        }
    }

    fn measure(_backend: &mut Gtk, h: &gtk4::Widget, _p: Proposal) -> Size {
        let (_, nat_w, _, _) = h.measure(gtk4::Orientation::Horizontal, -1);
        let (_, nat_h, _, _) = h.measure(gtk4::Orientation::Vertical, -1);
        Size::new((nat_w as f64).max(80.0), nat_h as f64)
    }

    day_pieces::renderer!(day_gtk::RENDERERS, Gtk, kind: KIND, props: ComboProps, patch: ComboPatch, make: make, update: update, measure: measure);
}

// ---------------------------------------------------------------------------
// Qt renderer: QComboBox via this crate's OWN shim (src/qt_shim.cpp)
// ---------------------------------------------------------------------------

#[cfg(feature = "qt")]
mod qt_impl {
    use super::*;
    use std::ffi::CString;
    use std::os::raw::{c_char, c_int, c_void};

    use day_qt::{Qt, QtHandle};
    use day_spec::{NodeId, Proposal, Size};

    unsafe extern "C" {
        fn day_combo_new(
            items_joined: *const c_char,
            selected: c_int,
            id: u64,
            cb: extern "C" fn(u64, c_int),
        ) -> *mut c_void;
        fn day_combo_set_items(w: *mut c_void, items_joined: *const c_char);
        fn day_combo_set_selected(w: *mut c_void, idx: c_int);
        // From day-qt-sys (already linked into the binary):
        fn day_qt_size_hint(w: *mut c_void, out_w: *mut f64, out_h: *mut f64);
    }

    extern "C" fn on_select(id: u64, idx: c_int) {
        day_qt::emit(NodeId(id), Event::SelectionChanged(idx as i64));
    }

    fn joined(items: &[String]) -> CString {
        CString::new(items.join("\n")).unwrap_or_default()
    }

    fn make(_backend: &mut Qt, p: &ComboProps, id: NodeId) -> QtHandle {
        let sel = p.selected.map(|i| i as c_int).unwrap_or(-1);
        QtHandle(unsafe { day_combo_new(joined(&p.items).as_ptr(), sel, id.0, on_select) })
    }

    fn update(_backend: &mut Qt, h: &QtHandle, patch: &ComboPatch) {
        {
            let p = patch;
            unsafe {
                match p {
                    ComboPatch::Items(items) => day_combo_set_items(h.0, joined(items).as_ptr()),
                    ComboPatch::Selected(sel) => {
                        day_combo_set_selected(h.0, sel.map(|i| i as c_int).unwrap_or(-1))
                    }
                }
            }
        }
    }

    fn measure(_backend: &mut Qt, h: &QtHandle, _p: Proposal) -> Size {
        let mut w = 0.0;
        let mut hh = 0.0;
        unsafe { day_qt_size_hint(h.0, &mut w, &mut hh) };
        Size::new(w.max(80.0), hh.max(22.0))
    }

    day_pieces::renderer!(day_qt::RENDERERS, Qt, kind: KIND, props: ComboProps, patch: ComboPatch, make: make, update: update, measure: measure);
}

// ---------------------------------------------------------------------------
// WinUI renderer: a Windows.UI.Xaml ComboBox via the day-winui-sys shim (parent-routed
// Win32 notifications don't apply — XAML fires SelectionChanged straight to our callback).
// ---------------------------------------------------------------------------

#[cfg(all(feature = "winui", windows))]
mod winui_impl {
    use super::*;
    use std::ffi::CString;
    use std::os::raw::c_int;

    use day_spec::{NodeId, Proposal, Size};
    use day_winui::{WinHandle, WinUi};

    fn cstr(s: &str) -> CString {
        CString::new(s).unwrap_or_default()
    }
    fn joined(items: &[String]) -> CString {
        cstr(&items.join("\n"))
    }

    extern "C" fn on_select(id: u64, idx: c_int) {
        day_winui::emit(NodeId(id), Event::SelectionChanged(idx as i64));
    }

    fn make(_backend: &mut WinUi, p: &ComboProps, id: NodeId) -> WinHandle {
        let sel = p.selected.map(|i| i as c_int).unwrap_or(-1);
        WinHandle(unsafe {
            day_winui_sys::day_winui_combo_new(joined(&p.items).as_ptr(), sel, id.0, on_select)
        })
    }

    fn update(_backend: &mut WinUi, h: &WinHandle, patch: &ComboPatch) {
        {
            let p = patch;
            unsafe {
                match p {
                    ComboPatch::Items(items) => {
                        day_winui_sys::day_winui_combo_set_items(h.0, joined(items).as_ptr())
                    }
                    ComboPatch::Selected(sel) => day_winui_sys::day_winui_combo_set_selected(
                        h.0,
                        sel.map(|i| i as c_int).unwrap_or(-1),
                    ),
                }
            }
        }
    }

    fn measure(_backend: &mut WinUi, h: &WinHandle, _p: Proposal) -> Size {
        let mut w = 0.0;
        let mut hh = 0.0;
        unsafe { day_winui_sys::day_winui_measure(h.0, -1.0, -1.0, &mut w, &mut hh) };
        Size::new(w.max(120.0), hh.max(32.0))
    }

    day_pieces::renderer!(day_winui::RENDERERS, WinUi, kind: KIND, props: ComboProps, patch: ComboPatch, make: make, update: update, measure: measure);
}

// ---------------------------------------------------------------------------
// UIKit renderer: UISegmentedControl (a native choice control; a dropdown-menu
// variant via UIButton+UIMenu is a later refinement — documented divergence)
// ---------------------------------------------------------------------------

#[cfg(all(feature = "uikit", target_os = "ios"))]
mod uikit_impl {
    use super::*;
    use day_spec::{NodeId, Proposal, Size};
    use day_uikit::Uikit;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, NSObjectProtocol};
    use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
    use objc2_core_foundation::CGSize;
    use objc2_foundation::{NSObject, NSString};
    use objc2_ui_kit::{UIControlEvents, UISegmentedControl, UIView};

    struct SegIvars {
        node: NodeId,
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "DayComboSegTarget"]
        #[ivars = SegIvars]
        struct SegTarget;

        unsafe impl NSObjectProtocol for SegTarget {}

        impl SegTarget {
            #[unsafe(method(fire:))]
            fn fire(&self, sender: &UISegmentedControl) {
                let idx = sender.selectedSegmentIndex();
                day_uikit::emit(self.ivars().node, Event::SelectionChanged(idx as i64));
            }
        }
    );

    impl SegTarget {
        fn new(mtm: MainThreadMarker, node: NodeId) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(SegIvars { node });
            unsafe { msg_send![super(this), init] }
        }
    }

    thread_local! {
        static TARGETS: std::cell::RefCell<std::collections::HashMap<usize, Retained<SegTarget>>> =
            std::cell::RefCell::new(std::collections::HashMap::new());
    }

    fn apply(seg: &UISegmentedControl, items: &[String], selected: Option<usize>) {
        seg.removeAllSegments();
        for (i, item) in items.iter().enumerate() {
            let title = NSString::from_str(item);
            seg.insertSegmentWithTitle_atIndex_animated(Some(&title), i, false);
        }
        if let Some(i) = selected {
            seg.setSelectedSegmentIndex(i as isize);
        }
    }

    fn make(_backend: &mut Uikit, p: &ComboProps, id: NodeId) -> Retained<UIView> {
        let mtm = MainThreadMarker::new().unwrap();
        let target = SegTarget::new(mtm, id);
        let seg = UISegmentedControl::new(mtm);
        apply(&seg, &p.items, p.selected);
        unsafe {
            let tobj: &AnyObject = target.as_ref();
            seg.addTarget_action_forControlEvents(
                Some(tobj),
                sel!(fire:),
                UIControlEvents::ValueChanged,
            );
        }
        let view: Retained<UIView> =
            Retained::from(<UISegmentedControl as AsRef<UIView>>::as_ref(&seg));
        TARGETS.with(|m| {
            m.borrow_mut()
                .insert((view.as_ref() as *const UIView) as usize, target)
        });
        view
    }

    fn update(_backend: &mut Uikit, h: &Retained<UIView>, patch: &ComboPatch) {
        let Some(seg) = (**h).downcast_ref::<UISegmentedControl>() else {
            return;
        };
        {
            let p = patch;
            match p {
                ComboPatch::Items(items) => apply(seg, items, None),
                ComboPatch::Selected(sel) => {
                    seg.setSelectedSegmentIndex(sel.map(|i| i as isize).unwrap_or(-1));
                }
            }
        }
    }

    fn measure(_backend: &mut Uikit, h: &Retained<UIView>, _p: Proposal) -> Size {
        let s = h.sizeThatFits(CGSize::new(1.0e6, 1.0e6));
        Size::new(s.width.ceil().max(80.0), s.height.ceil().max(28.0))
    }

    day_pieces::renderer!(day_uikit::RENDERERS, Uikit, kind: KIND, props: ComboProps, patch: ComboPatch, make: make, update: update, measure: measure);
}

// ---------------------------------------------------------------------------
// Android renderer: Spinner (via the DayBridge factory — the fully-external
// polyglot path arrives with dayffi, §15.3)
// ---------------------------------------------------------------------------

#[cfg(all(feature = "widget", target_os = "android"))]
mod android_impl {
    use super::*;
    use day_android::jni::objects::JValue;
    use day_android::{AHandle, Android, make_view, with_env};
    use day_spec::NodeId;

    fn make(_backend: &mut Android, p: &ComboProps, id: NodeId) -> AHandle {
        let joined = p.items.join("\n");
        let sel = p.selected.map(|i| i as i32).unwrap_or(-1);
        with_env(|env| {
            let s = env.new_string(&joined).expect("items");
            AHandle(make_view(
                env,
                "makeSpinner",
                "(JLjava/lang/String;I)Landroid/view/View;",
                &[
                    JValue::Long(id.0 as i64),
                    JValue::Object(&s),
                    JValue::Int(sel),
                ],
            ))
        })
    }

    fn update(_backend: &mut Android, h: &AHandle, patch: &ComboPatch) {
        // Android only reflects selection changes (item lists are set once at build).
        if let ComboPatch::Selected(sel) = patch {
            with_env(|env| {
                let _ = env.call_static_method(
                    day_android::BRIDGE,
                    "setSpinnerSelected",
                    "(Landroid/view/View;I)V",
                    &[
                        JValue::Object(h.0.as_obj()),
                        JValue::Int(sel.map(|i| i as i32).unwrap_or(-1)),
                    ],
                );
            });
        }
    }

    day_pieces::renderer!(day_android::RENDERERS, Android, kind: KIND, props: ComboProps, patch: ComboPatch, make: make, update: update);
}
