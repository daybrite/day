// ---------------------------------------------------------------------------
// AppKit: NSComboBox — the platform's real combo box (a text field with a dropdown button and
// item list). One delegate serves both halves: typing arrives per keystroke through
// NSControlTextEditingDelegate::controlTextDidChange:, and picking an item posts
// NSComboBoxDelegate::comboBoxSelectionDidChange:. The selection notification fires BEFORE the
// control writes the pick into its own stringValue, so that handler reads the SELECTED ITEM's
// string and emits it. Programmatic setStringValue fires neither (no echo guard needed); an
// Items patch's removeAllItems can fire a selection change with index -1, which is dropped.
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
    NSComboBox, NSComboBoxDelegate, NSControlTextEditingDelegate, NSTextField,
    NSTextFieldDelegate, NSView,
};
use objc2_foundation::{NSNotification, NSObject, NSString};

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
    unsafe impl NSTextFieldDelegate for ComboTarget {}

    unsafe impl NSControlTextEditingDelegate for ComboTarget {
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

    unsafe impl NSComboBoxDelegate for ComboTarget {
        #[unsafe(method(comboBoxSelectionDidChange:))]
        fn combo_box_selection_did_change(&self, notification: &NSNotification) {
            let node = self.ivars().node;
            if let Some(obj) = notification.object()
                && let Ok(combo) = obj.downcast::<NSComboBox>()
            {
                let idx = combo.indexOfSelectedItem();
                if idx >= 0
                    && let Ok(s) = combo.itemObjectValueAtIndex(idx).downcast::<NSString>()
                {
                    day_appkit::emit(node, Event::TextChanged(s.to_string()));
                }
            }
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
    // Keep each combo's delegate alive for the view's lifetime (the control holds it weakly).
    static TARGETS: RefCell<HashMap<usize, Retained<ComboTarget>>> = RefCell::new(HashMap::new());
}

fn apply_items(combo: &NSComboBox, items: &[String]) {
    combo.removeAllItems();
    for item in items {
        unsafe { combo.addItemWithObjectValue(&NSString::from_str(item)) };
    }
}

fn make(backend: &mut AppKit, p: &ComboProps, id: NodeId) -> Retained<NSView> {
    let mtm = backend.mtm();
    let combo = NSComboBox::new(mtm);
    apply_items(&combo, &p.items);
    // Inline autocompletion from the item list while typing.
    combo.setCompletes(true);
    if !p.placeholder.is_empty() {
        let tf: &NSTextField = combo.as_ref();
        tf.setPlaceholderString(Some(&NSString::from_str(&p.placeholder)));
    }
    combo.setStringValue(&NSString::from_str(&p.text));
    let target = ComboTarget::new(mtm, id);
    // One delegate registration covers both protocols: NSComboBox forwards the inherited
    // NSTextField text-editing delegate AND watches it for the combo notifications.
    unsafe { combo.setDelegate(Some(ProtocolObject::from_ref(&*target))) };
    let ns: Retained<NSView> = Retained::from(<NSComboBox as AsRef<NSView>>::as_ref(&combo));
    TARGETS.with(|m| {
        m.borrow_mut()
            .insert((ns.as_ref() as *const NSView) as usize, target)
    });
    ns
}

fn update(_backend: &mut AppKit, h: &Retained<NSView>, patch: &ComboPatch) {
    let Some(combo) = h.downcast_ref::<NSComboBox>() else {
        return;
    };
    match patch {
        // The text is the value and lives apart from the list — an items swap keeps it.
        ComboPatch::Items(items) => apply_items(combo, items),
        ComboPatch::SetText(t) => {
            if combo.stringValue().to_string() != *t {
                combo.setStringValue(&NSString::from_str(t));
            }
        }
    }
}

fn measure(_backend: &mut AppKit, h: &Retained<NSView>, p: Proposal) -> Size {
    // Grow to the proposed width (a text entry fills its row); natural single-line height.
    let fit = h.fittingSize();
    let w = p.width.unwrap_or(fit.width).max(120.0);
    Size::new(w, fit.height.ceil().max(22.0))
}

day_pieces::renderer!(day_appkit::RENDERERS, AppKit,
    kind: KIND, props: ComboProps, patch: ComboPatch,
    make: make, update: update, measure: measure);
