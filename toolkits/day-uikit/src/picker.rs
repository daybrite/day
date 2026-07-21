// ---------------------------------------------------------------------------
// UIKit: UIButton+UIMenu pull-down (menu) / UISegmentedControl (segmented) /
// checkmark-row UIStackView (inline).
// ---------------------------------------------------------------------------

use day_spec::Event;
use day_spec::props::{PickerPatch, PickerProps, PickerStyle};
use std::cell::RefCell;
use std::collections::HashMap;

use block2::RcBlock;
use day_spec::{NodeId, Proposal, Size};
use crate::Uikit;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_core_foundation::CGSize;
use objc2_foundation::{NSArray, NSString};
use objc2_ui_kit::{
    UIAction, UIButton, UIControlEvents, UIControlState, UIImage, UILayoutConstraintAxis, UIMenu,
    UIMenuElement, UISegmentedControl, UIStackView, UIStackViewAlignment, UIStackViewDistribution,
    UIView,
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
                crate::emit(self.ivars().node, Event::SelectionChanged(idx as i64));
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
            crate::emit(node, Event::SelectionChanged(i as i64));
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

fn make(_backend: &mut Uikit, p: &PickerProps, id: NodeId) -> Retained<UIView> {
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

fn update(_backend: &mut Uikit, h: &Retained<UIView>, patch: &PickerPatch) {
    let PickerPatch::Selected(i) = patch;
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


// Built-in dispatch adapters: the backend's realize/update matches call these (the downcasts
// the satellite-era `renderer!` macro used to generate).
pub(crate) fn realize_any(
    b: &mut crate::Uikit,
    props: &dyn std::any::Any,
    id: day_spec::NodeId,
) -> crate::Handle {
    let p = props
        .downcast_ref::<PickerProps>()
        .expect("day: picker props type");
    make(b, p, id)
}

pub(crate) fn update_any(b: &mut crate::Uikit, h: &crate::Handle, patch: &dyn std::any::Any) {
    if let Some(p) = patch.downcast_ref::<PickerPatch>() {
        update(b, h, p);
    }
}

pub(crate) fn measure_any(
    b: &mut crate::Uikit,
    h: &crate::Handle,
    p: day_spec::Proposal,
) -> day_spec::Size {
    measure(b, h, p)
}
