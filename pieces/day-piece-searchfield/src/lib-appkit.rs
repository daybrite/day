// ---------------------------------------------------------------------------
// AppKit: NSSearchField (a rounded search NSTextField with a magnifier + clear button for free). A
// per-node delegate implements NSControlTextEditingDelegate::controlTextDidChange: and dispatches
// Event::TextChanged; programmatic setStringValue does NOT fire that delegate, so no echo guard is
// needed on this backend (update only writes when the value actually differs).
// ---------------------------------------------------------------------------

use super::*;
use std::cell::RefCell;
use std::collections::HashMap;

use day_appkit::AppKit;
use day_spec::{NodeId, Proposal, Size};
use objc2::rc::Retained;
use objc2::runtime::{NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSControlTextEditingDelegate, NSSearchField, NSTextField, NSTextFieldDelegate, NSView,
};
use objc2_foundation::{NSNotification, NSObject, NSString};

struct SearchIvars {
    node: NodeId,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DaySearchFieldTarget"]
    #[ivars = SearchIvars]
    struct SearchTarget;

    unsafe impl NSObjectProtocol for SearchTarget {}
    unsafe impl NSTextFieldDelegate for SearchTarget {}

    unsafe impl NSControlTextEditingDelegate for SearchTarget {
        #[unsafe(method(controlTextDidChange:))]
        fn control_text_did_change(&self, notification: &NSNotification) {
            let node = self.ivars().node;
            if let Some(obj) = notification.object()
                && let Ok(tf) = obj.downcast::<NSTextField>()
            {
                day_appkit::emit(node, Event::TextChanged(tf.stringValue().to_string()));
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
    // Keep each field's delegate alive for the view's lifetime (the control holds it weakly).
    static TARGETS: RefCell<HashMap<usize, Retained<SearchTarget>>> = RefCell::new(HashMap::new());
}

fn make(backend: &mut AppKit, p: &SearchProps, id: NodeId) -> Retained<NSView> {
    let mtm = backend.mtm();
    let field = NSSearchField::new(mtm);
    if !p.placeholder.is_empty() {
        field.setPlaceholderString(Some(&NSString::from_str(&p.placeholder)));
    }
    field.setStringValue(&NSString::from_str(&p.text));
    let target = SearchTarget::new(mtm, id);
    // NSSearchField is an NSTextField, so its NSControl text-editing delegate delivers per-keystroke
    // controlTextDidChange: to our NSTextFieldDelegate.
    let tf: &NSTextField = field.as_ref();
    unsafe { tf.setDelegate(Some(ProtocolObject::from_ref(&*target))) };
    let ns: Retained<NSView> = Retained::from(<NSSearchField as AsRef<NSView>>::as_ref(&field));
    TARGETS.with(|m| {
        m.borrow_mut()
            .insert((ns.as_ref() as *const NSView) as usize, target)
    });
    ns
}

fn update(_backend: &mut AppKit, h: &Retained<NSView>, patch: &SearchPatch) {
    let SearchPatch::SetText(t) = patch;
    if let Some(field) = h.downcast_ref::<NSSearchField>()
        && field.stringValue().to_string() != *t
    {
        field.setStringValue(&NSString::from_str(t));
    }
}

fn measure(_backend: &mut AppKit, h: &Retained<NSView>, p: Proposal) -> Size {
    // Grow to the proposed width (a search field fills its row); natural single-line height.
    let fit = h.fittingSize();
    let w = p.width.unwrap_or(fit.width).max(120.0);
    Size::new(w, fit.height.ceil().max(22.0))
}

day_pieces::renderer!(day_appkit::RENDERERS, AppKit,
    kind: KIND, props: SearchProps, patch: SearchPatch,
    make: make, update: update, measure: measure);
