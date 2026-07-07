// ---------------------------------------------------------------------------
// UIKit: UISearchTextField (iOS 13+) — a UITextField subclass with the search field's rounded
// background + magnifier + clear button. A per-node target fires on UIControlEvents::EditingChanged
// and dispatches Event::TextChanged; programmatic setText does NOT fire EditingChanged, so no echo
// guard is needed here (update only writes when the value actually differs).
// ---------------------------------------------------------------------------

use super::*;
use std::cell::RefCell;
use std::collections::HashMap;

use day_spec::{NodeId, Proposal, Size};
use day_uikit::Uikit;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_core_foundation::CGSize;
use objc2_foundation::NSString;
use objc2_ui_kit::{UIControlEvents, UISearchTextField, UITextField, UIView};

struct SearchIvars {
    node: NodeId,
}

define_class!(
    #[unsafe(super(objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DayUIKitSearchTarget"]
    #[ivars = SearchIvars]
    struct SearchTarget;

    unsafe impl NSObjectProtocol for SearchTarget {}

    impl SearchTarget {
        #[unsafe(method(fire:))]
        fn fire(&self, sender: &AnyObject) {
            if let Some(tf) = sender.downcast_ref::<UITextField>() {
                let s = tf.text().map(|s| s.to_string()).unwrap_or_default();
                day_uikit::emit(self.ivars().node, Event::TextChanged(s));
            }
        }
    }
);

impl SearchTarget {
    fn new(mtm: MainThreadMarker, node: NodeId) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(SearchIvars { node });
        unsafe { msg_send![super(this), init] }
    }
}

thread_local! {
    // Keep each field's target alive for the view's lifetime (the control retains it weakly).
    static TARGETS: RefCell<HashMap<usize, Retained<SearchTarget>>> = RefCell::new(HashMap::new());
}

fn make(_backend: &mut Uikit, p: &SearchProps, id: NodeId) -> Retained<UIView> {
    let mtm = MainThreadMarker::new().unwrap();
    let field = UISearchTextField::new(mtm);
    if !p.placeholder.is_empty() {
        field.setPlaceholder(Some(&NSString::from_str(&p.placeholder)));
    }
    if !p.text.is_empty() {
        field.setText(Some(&NSString::from_str(&p.text)));
    }
    let target = SearchTarget::new(mtm, id);
    unsafe {
        field.addTarget_action_forControlEvents(
            Some(&target),
            sel!(fire:),
            UIControlEvents::EditingChanged,
        );
    }
    let ns: Retained<UIView> = Retained::from(<UISearchTextField as AsRef<UIView>>::as_ref(&field));
    TARGETS.with(|m| {
        m.borrow_mut()
            .insert((ns.as_ref() as *const UIView) as usize, target)
    });
    ns
}

fn update(_backend: &mut Uikit, h: &Retained<UIView>, patch: &SearchPatch) {
    let SearchPatch::SetText(t) = patch;
    if let Some(field) = (**h).downcast_ref::<UISearchTextField>() {
        let cur = field.text().map(|s| s.to_string()).unwrap_or_default();
        if cur != *t {
            field.setText(Some(&NSString::from_str(t)));
        }
    }
}

fn measure(_backend: &mut Uikit, h: &Retained<UIView>, p: Proposal) -> Size {
    // Grow to the proposed width; natural single-line height from the control's intrinsic size.
    let fit = h.sizeThatFits(CGSize::new(1.0e6, 1.0e6));
    let w = p.width.unwrap_or(fit.width).max(120.0);
    Size::new(w, fit.height.ceil().max(28.0))
}

day_pieces::renderer!(day_uikit::RENDERERS, Uikit,
    kind: KIND, props: SearchProps, patch: SearchPatch,
    make: make, update: update, measure: measure);
