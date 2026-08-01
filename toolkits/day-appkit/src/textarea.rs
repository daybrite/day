// ---------------------------------------------------------------------------
// AppKit: an editable NSTextView inside an NSScrollView (the standard scrollable text editor). A
// per-node delegate implements NSTextDelegate::textDidChange: and dispatches Event::TextChanged;
// programmatic setString: does NOT fire that delegate, so no echo guard is needed on this backend
// (update only writes when the value actually differs). NSTextView has no native placeholder, so an
// empty-state prompt is approximated with a faint NSTextField label added as a subview of the text
// view, toggled hidden whenever the text is non-empty. `measure` grows the editor's height with its
// content between `min_lines` and `max_lines` (then the scroll view scrolls).
// ---------------------------------------------------------------------------

use day_spec::Event;
use day_spec::props::{TextAreaPatch as TextPatch, TextAreaProps as TextProps};
use std::cell::RefCell;
use std::collections::HashMap;

use crate::AppKit;
use day_spec::{NodeId, Proposal, Size};
use objc2::rc::Retained;
use objc2::runtime::{NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSColor, NSFont, NSScrollView, NSTextDelegate, NSTextField,
    NSTextView, NSTextViewDelegate, NSView,
};
use objc2_foundation::{NSNotification, NSObject, NSPoint, NSRect, NSSize, NSString};

// The editor's font size and container inset — fixed so `measure`'s line-height math is deterministic.
const FONT_SIZE: f64 = 13.0;
const INSET: f64 = 6.0;

struct TAIvars {
    node: NodeId,
    // The placeholder overlay, held so textDidChange: can toggle it as the user types.
    placeholder: Retained<NSTextField>,
    // Plain Enter emits `Event::Submitted` instead of a newline (Shift+Enter still breaks).
    submit_on_enter: bool,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DayTextAreaTarget"]
    #[ivars = TAIvars]
    struct TATarget;

    unsafe impl NSObjectProtocol for TATarget {}
    unsafe impl NSTextViewDelegate for TATarget {}

    unsafe impl NSTextDelegate for TATarget {
        #[unsafe(method(textDidChange:))]
        fn text_did_change(&self, notification: &NSNotification) {
            let node = self.ivars().node;
            if let Some(obj) = notification.object()
                && let Ok(tv) = obj.downcast::<NSTextView>()
            {
                let s = tv.string().to_string();
                self.ivars().placeholder.setHidden(!s.is_empty());
                crate::emit(node, Event::TextChanged(s));
            }
        }
    }

    impl TATarget {
        // Submit-on-enter: claim `insertNewline:` (plain Return) as a submit; Shift+Return
        // falls through to the standard line break. Runs after `textDidChange:` for the
        // preceding keystrokes, so the app's bound text signal is already current.
        #[unsafe(method(textView:doCommandBySelector:))]
        fn do_command(&self, _tv: &NSTextView, sel: objc2::runtime::Sel) -> objc2::runtime::Bool {
            if !self.ivars().submit_on_enter || sel != objc2::sel!(insertNewline:) {
                return objc2::runtime::Bool::NO;
            }
            let shift = objc2_app_kit::NSApplication::sharedApplication(self.mtm())
                .currentEvent()
                .map(|e| {
                    e.modifierFlags()
                        .contains(objc2_app_kit::NSEventModifierFlags::Shift)
                })
                .unwrap_or(false);
            if shift {
                return objc2::runtime::Bool::NO;
            }
            crate::emit(self.ivars().node, Event::Submitted);
            objc2::runtime::Bool::YES
        }
    }
);

impl TATarget {
    fn new(
        mtm: MainThreadMarker,
        node: NodeId,
        placeholder: Retained<NSTextField>,
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
    tv: Retained<NSTextView>,
    placeholder: Retained<NSTextField>,
    // Keep the delegate alive for the view's lifetime (the text view holds it weakly).
    #[allow(dead_code)]
    target: Retained<TATarget>,
    line_h: f64,
    min_lines: u32,
    max_lines: u32,
}

thread_local! {
    static STATE: RefCell<HashMap<usize, TAState>> = RefCell::new(HashMap::new());
}

fn key(v: &Retained<NSView>) -> usize {
    Retained::as_ptr(v) as usize
}

/// Apply the editable / selectable / spell-check attributes to the NSTextView. Spell-check drives
/// all three of continuous spelling, autocorrect, and grammar so the squiggles fully disappear.
fn apply_attrs(tv: &NSTextView, editable: bool, selectable: bool, spell: bool) {
    tv.setEditable(editable);
    tv.setSelectable(selectable);
    unsafe {
        let _: () = msg_send![tv, setContinuousSpellCheckingEnabled: spell];
        let _: () = msg_send![tv, setAutomaticSpellingCorrectionEnabled: spell];
        let _: () = msg_send![tv, setGrammarCheckingEnabled: spell];
    }
}

fn make(backend: &mut AppKit, p: &TextProps, id: NodeId) -> Retained<NSView> {
    let mtm = backend.mtm();
    let font = NSFont::systemFontOfSize(FONT_SIZE);

    let scroll = NSScrollView::new(mtm);
    scroll.setDrawsBackground(false);
    scroll.setHasVerticalScroller(true);
    scroll.setHasHorizontalScroller(false);

    let tv = NSTextView::new(mtm);
    tv.setFont(Some(&font));
    tv.setRichText(false);
    apply_attrs(&tv, p.editable, p.selectable, p.spellcheck);
    tv.setUsesFontPanel(false);
    tv.setTextContainerInset(NSSize::new(INSET, INSET));
    tv.setVerticallyResizable(true);
    tv.setHorizontallyResizable(false);
    tv.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
    tv.setMinSize(NSSize::new(0.0, 0.0));
    tv.setMaxSize(NSSize::new(1.0e7, 1.0e7));
    if let Some(tc) = unsafe { tv.textContainer() } {
        tc.setWidthTracksTextView(true);
    }
    if !p.text.is_empty() {
        tv.setString(&NSString::from_str(&p.text));
    }
    let line_h = unsafe { tv.layoutManager() }
        .map(|lm| lm.defaultLineHeightForFont(&font))
        .unwrap_or(FONT_SIZE * 1.3);

    // Placeholder overlay (NSTextView has no native placeholder): a faint label pinned top-left, in the
    // text view's flipped coordinates, hidden while the editor has text.
    let ph = NSTextField::labelWithString(&NSString::from_str(&p.placeholder), mtm);
    ph.setTextColor(Some(&NSColor::placeholderTextColor()));
    ph.setFont(Some(&font));
    ph.setFrame(NSRect::new(
        NSPoint::new(INSET + 5.0, INSET),
        NSSize::new(320.0, line_h.ceil()),
    ));
    ph.setHidden(!p.text.is_empty());
    tv.addSubview(<NSTextField as AsRef<NSView>>::as_ref(&ph));

    let target = TATarget::new(mtm, id, ph.clone(), p.submit_on_enter);
    tv.setDelegate(Some(ProtocolObject::from_ref(&*target)));
    scroll.setDocumentView(Some(&tv));

    let ns: Retained<NSView> = Retained::from(<NSScrollView as AsRef<NSView>>::as_ref(&scroll));
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

fn update(_backend: &mut AppKit, h: &Retained<NSView>, patch: &TextPatch) {
    STATE.with(|m| {
        let m = m.borrow();
        let Some(st) = m.get(&key(h)) else {
            return;
        };
        match patch {
            TextPatch::SetText(t) => {
                if st.tv.string().to_string() != *t {
                    st.tv.setString(&NSString::from_str(t));
                    st.placeholder.setHidden(!t.is_empty());
                }
            }
            TextPatch::SetEditable(v) => st.tv.setEditable(*v),
            TextPatch::SetSelectable(v) => st.tv.setSelectable(*v),
            TextPatch::SetSpellCheck(v) => unsafe {
                let _: () = msg_send![&st.tv, setContinuousSpellCheckingEnabled: *v];
                let _: () = msg_send![&st.tv, setAutomaticSpellingCorrectionEnabled: *v];
                let _: () = msg_send![&st.tv, setGrammarCheckingEnabled: *v];
            },
        }
    });
}

fn measure(_backend: &mut AppKit, h: &Retained<NSView>, p: Proposal) -> Size {
    let avail_w = p.width.unwrap_or(300.0).max(120.0);
    STATE.with(|m| {
        let m = m.borrow();
        let Some(st) = m.get(&key(h)) else {
            return Size::new(avail_w, 44.0);
        };
        let pad = 2.0 * INSET;
        let min_h = (st.min_lines as f64) * st.line_h + pad;
        let max_h = if st.max_lines > 0 {
            (st.max_lines as f64) * st.line_h + pad
        } else {
            f64::MAX
        };
        let (Some(tc), Some(lm)) = (unsafe { st.tv.textContainer() }, unsafe {
            st.tv.layoutManager()
        }) else {
            return Size::new(avail_w, min_h.ceil());
        };
        // Measure wrapped content height at the proposed inner width: fix the container width (temporarily
        // detaching width-tracking) so the query is independent of the not-yet-set frame, then restore it.
        let lfp = tc.lineFragmentPadding();
        let inner_w = (avail_w - 2.0 * INSET - 2.0 * lfp).max(1.0);
        tc.setWidthTracksTextView(false);
        tc.setContainerSize(NSSize::new(inner_w, 1.0e7));
        let _ = lm.glyphRangeForTextContainer(&tc);
        let used = lm.usedRectForTextContainer(&tc);
        tc.setWidthTracksTextView(true);
        let content_h = used.size.height + pad;
        let hgt = content_h.clamp(min_h, max_h);
        Size::new(avail_w, hgt.ceil())
    })
}

// Built-in dispatch adapters: the backend's realize/update matches call these (the downcasts
// the satellite-era `renderer!` macro used to generate).
pub(crate) fn realize_any(
    b: &mut AppKit,
    props: &dyn std::any::Any,
    id: day_spec::NodeId,
) -> crate::Handle {
    let p = props
        .downcast_ref::<TextProps>()
        .expect("day: textarea props type");
    make(b, p, id)
}

pub(crate) fn update_any(b: &mut AppKit, h: &crate::Handle, patch: &dyn std::any::Any) {
    if let Some(p) = patch.downcast_ref::<TextPatch>() {
        update(b, h, p);
    }
}

pub(crate) fn measure_any(
    b: &mut AppKit,
    h: &crate::Handle,
    p: day_spec::Proposal,
) -> day_spec::Size {
    measure(b, h, p)
}
