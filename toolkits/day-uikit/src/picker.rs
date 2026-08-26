// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// UIKit: UIButton+UIMenu pull-down (menu) / UISegmentedControl (segmented) /
// checkmark-row UIStackView (inline).
// ---------------------------------------------------------------------------

use day_spec::Event;
use day_spec::props::{PickerPatch, PickerProps, PickerStyle};

use crate::Uikit;
use block2::RcBlock;
use day_spec::{NodeId, Proposal, Size};
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
            // Contained (§8.5): the emit dispatches into the app's handlers.
            day_spec::ffi_guard::contain((), || {
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
            });
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
    /// The menu style's actions, in option order, so `update` can move the ✓ there too. A
    /// UIMenu's children are immutable, so the selection change reattaches a fresh menu built
    /// over these same actions (their handlers stay wired).
    menu_actions: Vec<Retained<UIAction>>,
    options: Vec<String>,
    /// The picker's node and its live selection — what an option patch needs to rebuild the
    /// menu style's actions (each handler captures its own index, so relabeling is not an
    /// option there) and to keep the selection across the swap.
    node: NodeId,
    selected: usize,
    _target: Retained<PickerTarget>,
}

day_core::tls_group! {
    /// Per-view picker state, keyed by view ptr — swept by the backend's `release` through
    /// `day_spec::sidetable` (a plain map here leaked every released picker's retains).
    static STATE: day_spec::sidetable::SideTable<ViewState> =
        day_spec::sidetable::SideTable::new();

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

fn make_menu(
    mtm: MainThreadMarker,
    p: &PickerProps,
    node: NodeId,
) -> (Retained<UIButton>, Vec<Retained<UIAction>>) {
    let btn = UIButton::buttonWithType(objc2_ui_kit::UIButtonType::System, mtm);
    let actions = menu_actions(mtm, &p.options, p.selected, node);
    attach_menu(mtm, &btn, &actions);
    btn.setShowsMenuAsPrimaryAction(true);
    let title = p.options.get(p.selected).cloned().unwrap_or_default();
    btn.setTitle_forState(Some(&NSString::from_str(&title)), UIControlState::Normal);
    (btn, actions)
}

/// One `UIAction` per option, the `i`-th marked when it is the selection. Each handler
/// captures its index, so a changed option LIST needs fresh actions rather than new titles.
fn menu_actions(
    mtm: MainThreadMarker,
    options: &[String],
    selected: usize,
    node: NodeId,
) -> Vec<Retained<UIAction>> {
    let mut actions: Vec<Retained<UIAction>> = Vec::new();
    for (i, opt) in options.iter().enumerate() {
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
        if i == selected {
            action.setState(objc2_ui_kit::UIMenuElementState::On);
        }
        actions.push(action);
    }
    actions
}

/// (Re)attach a UIMenu built over `actions` — creation and every selection change go through
/// here, because an attached menu is a snapshot: editing an action's state alone leaves an
/// already-materialized menu showing the old ✓.
fn attach_menu(mtm: MainThreadMarker, btn: &UIButton, actions: &[Retained<UIAction>]) {
    let elems: Vec<Retained<UIMenuElement>> = actions
        .iter()
        .map(|a| Retained::from(<UIAction as AsRef<UIMenuElement>>::as_ref(a)))
        .collect();
    let arr = NSArray::from_retained_slice(&elems);
    let menu = UIMenu::menuWithTitle_children(&NSString::from_str(""), &arr, mtm);
    btn.setMenu(Some(&menu));
}

/// New option labels, in place — the selected index survives where it still exists.
///
/// Segmented relabels its segments and adds or drops the tail; the inline rows relabel (each
/// carries its index as its tag, so its wiring survives) and hide past the new end; the menu
/// style rebuilds its actions, whose handlers capture the index.
fn set_options(h: &Retained<UIView>, opts: &[String]) {
    let mtm = crate::mtm();
    if let Some(seg) = (**h).downcast_ref::<UISegmentedControl>() {
        let keep = seg.selectedSegmentIndex().max(0) as usize;
        for (i, o) in opts.iter().enumerate() {
            if i < seg.numberOfSegments() {
                seg.setTitle_forSegmentAtIndex(Some(&NSString::from_str(o)), i);
            } else {
                seg.insertSegmentWithTitle_atIndex_animated(Some(&NSString::from_str(o)), i, false);
            }
        }
        while seg.numberOfSegments() > opts.len() {
            seg.removeSegmentAtIndex_animated(seg.numberOfSegments() - 1, false);
        }
        if !opts.is_empty() {
            seg.setSelectedSegmentIndex(keep.min(opts.len() - 1) as isize);
        }
        return;
    }
    STATE.with(|t| {
        t.with((h.as_ref() as *const UIView) as usize, |st| {
            st.options = opts.to_vec();
            st.selected = st.selected.min(opts.len().saturating_sub(1));
            if let Some(btn) = &st.menu_button {
                st.menu_actions = menu_actions(mtm, opts, st.selected, st.node);
                attach_menu(mtm, btn, &st.menu_actions);
                let title = opts.get(st.selected).cloned().unwrap_or_default();
                btn.setTitle_forState(Some(&NSString::from_str(&title)), UIControlState::Normal);
            }
            for (i, b) in st.buttons.iter().enumerate() {
                match opts.get(i) {
                    Some(o) => {
                        b.setTitle_forState(Some(&NSString::from_str(o)), UIControlState::Normal);
                        b.setHidden(false);
                    }
                    // Rows past the new end hide rather than unwire: the stack owns them, and
                    // a re-grown list needs their target/action back.
                    None => b.setHidden(true),
                }
            }
        })
    });
}

fn make(_backend: &mut Uikit, p: &PickerProps, id: NodeId) -> Retained<UIView> {
    let mtm = crate::mtm();
    let target = PickerTarget::new(mtm, id);
    let (view, buttons, menu_button) = match p.style {
        PickerStyle::Segmented => (make_segmented(mtm, p, &target), vec![], None),
        PickerStyle::Inline => {
            let (v, b) = make_inline(mtm, p, &target);
            (v, b, None)
        }
        PickerStyle::Menu => {
            let (btn, actions) = make_menu(mtm, p, id);
            let v = Retained::from(<UIButton as AsRef<UIView>>::as_ref(&btn));
            (v, vec![], Some((btn, actions)))
        }
    };
    let (menu_button, menu_actions) = match menu_button {
        Some((b, a)) => (Some(b), a),
        None => (None, Vec::new()),
    };
    STATE.with(|t| {
        t.insert(
            (view.as_ref() as *const UIView) as usize,
            ViewState {
                buttons,
                menu_button,
                menu_actions,
                options: p.options.clone(),
                node: id,
                selected: p.selected,
                _target: target,
            },
        )
    });
    view
}

fn update(_backend: &mut Uikit, h: &Retained<UIView>, patch: &PickerPatch) {
    let i = match patch {
        PickerPatch::Selected(i) => *i,
        PickerPatch::Options(opts) => return set_options(h, opts),
    };
    if let Some(seg) = (**h).downcast_ref::<UISegmentedControl>() {
        if seg.selectedSegmentIndex() != i as isize {
            seg.setSelectedSegmentIndex(i as isize);
        }
        return;
    }
    STATE.with(|t| {
        t.with((h.as_ref() as *const UIView) as usize, |st| {
            st.selected = i;
            if let Some(btn) = &st.menu_button {
                let title = st.options.get(i).cloned().unwrap_or_default();
                btn.setTitle_forState(Some(&NSString::from_str(&title)), UIControlState::Normal);
                // Move the ✓ with the selection — without this the menu kept showing the
                // build-time mark no matter what was picked since.
                for (j, a) in st.menu_actions.iter().enumerate() {
                    a.setState(if j == i {
                        objc2_ui_kit::UIMenuElementState::On
                    } else {
                        objc2_ui_kit::UIMenuElementState::Off
                    });
                }
                if !st.menu_actions.is_empty() {
                    attach_menu(crate::mtm(), btn, &st.menu_actions);
                }
            }
            for (j, b) in st.buttons.iter().enumerate() {
                if let Some(img) = checkmark(j == i) {
                    b.setImage_forState(Some(&img), UIControlState::Normal);
                }
            }
        })
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
    // Mismatched props degrade to the placeholder rather than panicking in a native
    // up-call (§8.5); `props_of` reports once per kind.
    let Some(p) = day_spec::props_of::<PickerProps>(day_spec::kinds::PICKER, "uikit", props) else {
        return crate::placeholder_view(day_spec::kinds::PICKER);
    };
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
