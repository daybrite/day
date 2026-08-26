// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

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

use crate::Uikit;
use day_spec::{NodeId, Proposal, Size};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol, ProtocolObject, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
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
    // Return emits `Event::Submitted` instead of a newline (pastes with newlines pass through).
    submit_on_enter: bool,
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
            // Contained (§8.5): the emit dispatches into the app's handlers.
            day_spec::ffi_guard::contain((), || {
                let s = text_view.text().to_string();
                self.ivars().placeholder.setHidden(!s.is_empty());
                crate::emit(self.ivars().node, Event::TextChanged(s));
            });
        }

        // Submit-on-enter: claim exactly the keyboard's Return keystroke (`text == "\n"`) as a
        // submit; multi-character insertions (pastes) keep their newlines. iOS keyboards have no
        // Shift+Return distinction, so Return always submits when the flag is on.
        #[unsafe(method(textView:shouldChangeTextInRange:replacementText:))]
        fn should_change_text(
            &self,
            _tv: &UITextView,
            _range: objc2_foundation::NSRange,
            text: &objc2_foundation::NSString,
        ) -> objc2::runtime::Bool {
            // Contained (§8.5); a panic keeps the keystroke (YES — the native behavior).
            day_spec::ffi_guard::contain(objc2::runtime::Bool::YES, || {
                if self.ivars().submit_on_enter && text.to_string() == "\n" {
                    crate::emit(self.ivars().node, Event::Submitted);
                    return objc2::runtime::Bool::NO;
                }
                objc2::runtime::Bool::YES
            })
        }
    }
);

impl TATarget {
    fn new(
        mtm: MainThreadMarker,
        node: NodeId,
        placeholder: Retained<UILabel>,
        submit_on_enter: bool,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TAIvars {
            node,
            placeholder,
            submit_on_enter,
        });
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

day_core::tls_group! {
    /// Per-view editor state, keyed by view ptr — swept by the backend's `release` through
    /// `day_spec::sidetable` (a plain map here leaked every released editor's retains).
    static STATE: day_spec::sidetable::SideTable<TAState> =
        day_spec::sidetable::SideTable::new();

}

fn key(v: &Retained<UIView>) -> usize {
    Retained::as_ptr(v) as usize
}

/// Set a `UITextInputTraits` `NSInteger` property (`spellCheckingType` / `autocorrectionType` /
/// `smartQuotesType` / `smartDashesType`) on a
/// `UITextView`. On `UITextView` these are resolved dynamically rather than as concrete methods, so
/// objc2's debug-only send verification (`class_getInstanceMethod`) reports them missing and a
/// checked `msg_send!` panics with "method not found" — even though the send succeeds at runtime.
/// Dispatching through the raw runtime entry point skips that static check and resolves the setter
/// exactly as UIKit itself does.
pub fn set_text_input_trait(tv: &UITextView, sel: Sel, value: isize) {
    let recv = (tv as *const UITextView).cast::<AnyObject>().cast_mut();
    // SAFETY: `recv` is a live main-thread `UITextView`; `sel` names a `UITextInputTraits` setter
    // that takes a single `NSInteger` and returns `void`, matching this `(id, SEL, NSInteger) -> ()`
    // retyping of `objc_msgSend` (which is `extern "C-unwind"`).
    let send: unsafe extern "C-unwind" fn(*mut AnyObject, Sel, isize) =
        unsafe { core::mem::transmute(objc2::ffi::objc_msgSend as unsafe extern "C-unwind" fn()) };
    unsafe { send(recv, sel, value) };
}

/// Apply editable / selectable / spell-check to the UITextView. Spell-check drives both the spell
/// checker and autocorrect (0 = Default/on, 1 = No/off — `UITextSpellCheckingType`/`…AutocorrectionType`).
fn apply_attrs(tv: &UITextView, editable: bool, selectable: bool, spell: bool) {
    tv.setEditable(editable);
    tv.setSelectable(selectable);
    let v: isize = if spell { 0 } else { 1 };
    set_text_input_trait(tv, sel!(setSpellCheckingType:), v);
    set_text_input_trait(tv, sel!(setAutocorrectionType:), v);
}

fn make(_backend: &mut Uikit, p: &TextProps, id: NodeId) -> Retained<UIView> {
    let mtm = crate::mtm();
    let font = UIFont::systemFontOfSize(FONT_SIZE);

    let tv = UITextView::new(mtm);
    tv.setFont(Some(&font));
    apply_attrs(&tv, p.editable, p.selectable, p.spellcheck);
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

    let target = TATarget::new(mtm, id, ph.clone(), p.submit_on_enter);
    unsafe { tv.setDelegate(Some(ProtocolObject::from_ref(&*target))) };

    let ns: Retained<UIView> = Retained::from(<UITextView as AsRef<UIView>>::as_ref(&tv));
    STATE.with(|t| {
        t.insert(
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
    STATE.with(|t| {
        t.with(key(h), |st| match patch {
            TextPatch::SetText(txt) => {
                if st.tv.text().to_string() != *txt {
                    st.tv.setText(Some(&NSString::from_str(txt)));
                    st.placeholder.setHidden(!txt.is_empty());
                }
            }
            TextPatch::SetEditable(v) => st.tv.setEditable(*v),
            TextPatch::SetSelectable(v) => st.tv.setSelectable(*v),
            TextPatch::SetSpellCheck(v) => {
                let n: isize = if *v { 0 } else { 1 };
                set_text_input_trait(&st.tv, sel!(setSpellCheckingType:), n);
                set_text_input_trait(&st.tv, sel!(setAutocorrectionType:), n);
            }
        })
    });
}

fn measure(_backend: &mut Uikit, h: &Retained<UIView>, p: Proposal) -> Size {
    let avail_w = p.width.unwrap_or(300.0).max(120.0);
    STATE.with(|t| {
        t.with(key(h), |st| {
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
        .unwrap_or_else(|| Size::new(avail_w, 44.0))
    })
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
    let Some(p) = day_spec::props_of::<TextProps>(day_spec::kinds::TEXT_AREA, "uikit", props)
    else {
        return crate::placeholder_view(day_spec::kinds::TEXT_AREA);
    };
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
