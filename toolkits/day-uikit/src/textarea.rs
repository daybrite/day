// ---------------------------------------------------------------------------
// UIKit: an editable UITextView (which is itself a scroll view). A per-node delegate implements
// UITextViewDelegate::textViewDidChange: and dispatches Event::TextChanged; programmatic setText does
// NOT fire that delegate, so no echo guard is needed here (update only writes when the value actually
// differs). UITextView has no native placeholder, so an empty-state prompt is approximated with a faint
// UILabel added as a subview and toggled hidden while the editor has text. `measure` grows the editor's
// height with its content (via sizeThatFits) between `min_lines` and `max_lines`, then it scrolls.
// ---------------------------------------------------------------------------

use day_spec::Event;
use day_spec::props::{TextAreaPatch as TextPatch, TextAreaProps as TextProps};
use std::cell::RefCell;
use std::collections::HashMap;

use day_spec::{NodeId, Proposal, Size};
use crate::Uikit;
use objc2::rc::Retained;
use objc2::runtime::{NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_foundation::NSString;
use objc2_ui_kit::{
    UIColor, UIEdgeInsets, UIFont, UILabel, UIScrollViewDelegate, UITextView, UITextViewDelegate,
    UIView,
};

const FONT_SIZE: f64 = 16.0;
const INSET_TOP: f64 = 8.0;
const INSET_SIDE: f64 = 5.0;

struct TAIvars {
    node: NodeId,
    // The placeholder overlay, held so textViewDidChange: can toggle it as the user types.
    placeholder: Retained<UILabel>,
}

define_class!(
    #[unsafe(super(objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DayUIKitTextAreaTarget"]
    #[ivars = TAIvars]
    struct TATarget;

    unsafe impl NSObjectProtocol for TATarget {}
    unsafe impl UIScrollViewDelegate for TATarget {}

    unsafe impl UITextViewDelegate for TATarget {
        #[unsafe(method(textViewDidChange:))]
        fn text_view_did_change(&self, text_view: &UITextView) {
            let s = text_view.text().to_string();
            self.ivars().placeholder.setHidden(!s.is_empty());
            crate::emit(self.ivars().node, Event::TextChanged(s));
        }
    }
);

impl TATarget {
    fn new(mtm: MainThreadMarker, node: NodeId, placeholder: Retained<UILabel>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TAIvars { node, placeholder });
        unsafe { msg_send![super(this), init] }
    }
}

struct TAState {
    tv: Retained<UITextView>,
    placeholder: Retained<UILabel>,
    #[allow(dead_code)]
    target: Retained<TATarget>,
    line_h: f64,
    min_lines: u32,
    max_lines: u32,
}

thread_local! {
    static STATE: RefCell<HashMap<usize, TAState>> = RefCell::new(HashMap::new());
}

fn key(v: &Retained<UIView>) -> usize {
    Retained::as_ptr(v) as usize
}

fn make(_backend: &mut Uikit, p: &TextProps, id: NodeId) -> Retained<UIView> {
    let mtm = MainThreadMarker::new().unwrap();
    let font = UIFont::systemFontOfSize(FONT_SIZE);

    let tv = UITextView::new(mtm);
    tv.setFont(Some(&font));
    tv.setEditable(true);
    tv.setTextContainerInset(UIEdgeInsets {
        top: INSET_TOP,
        left: 0.0,
        bottom: INSET_TOP,
        right: 0.0,
    });
    if !p.text.is_empty() {
        tv.setText(Some(&NSString::from_str(&p.text)));
    }
    let line_h = unsafe { font.lineHeight() };

    // Placeholder overlay (UITextView has no native placeholder): a faint label pinned near the top-left
    // text origin, hidden while the editor has text.
    let ph = UILabel::new(mtm);
    ph.setText(Some(&NSString::from_str(&p.placeholder)));
    unsafe {
        ph.setFont(Some(&font));
        ph.setTextColor(Some(&UIColor::lightGrayColor()));
    }
    ph.setNumberOfLines(0);
    ph.setFrame(CGRect::new(
        CGPoint::new(INSET_SIDE, INSET_TOP),
        CGSize::new(320.0, line_h.ceil()),
    ));
    ph.setHidden(!p.text.is_empty());
    tv.addSubview(<UILabel as AsRef<UIView>>::as_ref(&ph));

    let target = TATarget::new(mtm, id, ph.clone());
    unsafe { tv.setDelegate(Some(ProtocolObject::from_ref(&*target))) };

    let ns: Retained<UIView> = Retained::from(<UITextView as AsRef<UIView>>::as_ref(&tv));
    STATE.with(|m| {
        m.borrow_mut().insert(
            key(&ns),
            TAState {
                tv,
                placeholder: ph,
                target,
                line_h,
                min_lines: p.min_lines,
                max_lines: p.max_lines,
            },
        )
    });
    ns
}

fn update(_backend: &mut Uikit, h: &Retained<UIView>, patch: &TextPatch) {
    let TextPatch::SetText(t) = patch;
    STATE.with(|m| {
        let m = m.borrow();
        let Some(st) = m.get(&key(h)) else {
            return;
        };
        if st.tv.text().to_string() != *t {
            st.tv.setText(Some(&NSString::from_str(t)));
            st.placeholder.setHidden(!t.is_empty());
        }
    });
}

fn measure(_backend: &mut Uikit, h: &Retained<UIView>, p: Proposal) -> Size {
    let avail_w = p.width.unwrap_or(300.0).max(120.0);
    STATE.with(|m| {
        let m = m.borrow();
        let Some(st) = m.get(&key(h)) else {
            return Size::new(avail_w, 44.0);
        };
        let pad = 2.0 * INSET_TOP;
        let min_h = (st.min_lines as f64) * st.line_h + pad;
        let max_h = if st.max_lines > 0 {
            (st.max_lines as f64) * st.line_h + pad
        } else {
            f64::MAX
        };
        let fit = st.tv.sizeThatFits(CGSize::new(avail_w, 1.0e7));
        let hgt = fit.height.clamp(min_h, max_h);
        Size::new(avail_w, hgt.ceil())
    })
}


// Built-in dispatch adapters: the backend's realize/update matches call these (the downcasts
// the satellite-era `renderer!` macro used to generate).
pub(crate) fn realize_any(
    b: &mut crate::Uikit,
    props: &dyn std::any::Any,
    id: day_spec::NodeId,
) -> crate::Handle {
    let p = props
        .downcast_ref::<TextProps>()
        .expect("day: textarea props type");
    make(b, p, id)
}

pub(crate) fn update_any(b: &mut crate::Uikit, h: &crate::Handle, patch: &dyn std::any::Any) {
    if let Some(p) = patch.downcast_ref::<TextPatch>() {
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
