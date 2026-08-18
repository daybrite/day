// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// UIKit: a `UITextView` over the same TextKit `NSTextStorage` the AppKit arm drives, so the two
// build their attributed strings from identical rules and differ only in the class names.
//
// The one thing iOS has that macOS doesn't is `allowsEditingTextAttributes`, which puts B/I/U into
// the selection's edit menu and lets the user change attributes Day would never learn about. It is
// OFF here for the same reason the macOS font panel is: attributes travel Day → native, and the
// app's own toolbar writes them through the bound signal.
//
// `UITextView` IS a scroll view, so unlike AppKit there is nothing to wrap it in — but that also
// means `sizeThatFits:` reports the CONTENT height, which is what `measure` wants anyway.
// ---------------------------------------------------------------------------

use super::*;

use day_spec::sidetable::SideTable;
use day_spec::{NodeId, ParagraphAlign, Proposal, Size, Underline, ffi_guard};
use day_uikit::Uikit;
use objc2::rc::Retained;
use objc2::runtime::{NSObjectProtocol, ProtocolObject};
use objc2::{AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_foundation::{
    NSAttributedString, NSDictionary, NSMutableAttributedString, NSNumber, NSObject, NSRange,
    NSString,
};
use objc2_ui_kit::{
    NSMutableParagraphStyle, NSTextAlignment, UIColor, UIEdgeInsets, UIFont,
    UIFontDescriptorSymbolicTraits, UILabel, UIScrollViewDelegate, UITextView, UITextViewDelegate,
    UIView,
};

/// The base point size the editor draws at, and its container inset. iOS text is a size larger
/// than macOS text, matching the built-in `text_area` on each.
const FONT_SIZE: f64 = 16.0;
const INSET: f64 = 8.0;

struct EdIvars {
    node: NodeId,
    /// The empty-state prompt, held so the change delegate can show and hide it (UITextView has
    /// no placeholder of its own).
    placeholder: Retained<UILabel>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DayTextEditorUIKitTarget"]
    #[ivars = EdIvars]
    struct EdTarget;

    unsafe impl NSObjectProtocol for EdTarget {}
    // UITextViewDelegate refines UIScrollViewDelegate, so both are declared.
    unsafe impl UIScrollViewDelegate for EdTarget {}

    unsafe impl UITextViewDelegate for EdTarget {
        // Contained (§8.5): a panic must not unwind into UIKit.
        #[unsafe(method(textViewDidChange:))]
        fn text_view_did_change(&self, tv: &UITextView) {
            ffi_guard::contain((), || {
                let text = tv.text().to_string();
                self.ivars().placeholder.setHidden(!text.is_empty());
                day_uikit::emit(self.ivars().node, Event::TextChanged(text));
            })
        }

        #[unsafe(method(textViewDidChangeSelection:))]
        fn selection_changed(&self, tv: &UITextView) {
            ffi_guard::contain((), || {
                let text = tv.text().to_string();
                let r = sel_range(tv);
                // UTF-16 back to BYTES — see the AppKit arm; the units diverge at the first emoji.
                let start = byte_of_utf16(&text, r.location);
                let end = byte_of_utf16(&text, r.location + r.length);
                day_uikit::emit(
                    self.ivars().node,
                    Event::custom("texteditor:sel", selection_payload(start, end)),
                );
            })
        }
    }
);

impl EdTarget {
    fn new(mtm: MainThreadMarker, node: NodeId, placeholder: Retained<UILabel>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(EdIvars { node, placeholder });
        unsafe { msg_send![super(this), init] }
    }
}

struct EdState {
    tv: Retained<UITextView>,
    placeholder: Retained<UILabel>,
    #[allow(dead_code)]
    target: Retained<EdTarget>,
    base: Font,
    line_h: f64,
    min_lines: u32,
    max_lines: u32,
}

thread_local! {
    static STATE: SideTable<EdState> = SideTable::with_teardown(|st: EdState| {
        // The delegate is held weakly by the text view: detach it so a view outliving this state
        // can never message the dropped target.
        unsafe { st.tv.setDelegate(None) };
    });
}

fn key(v: &Retained<UIView>) -> usize {
    Retained::as_ptr(v) as usize
}

// `selectedRange` / `setSelectedRange:` carry a deprecation in the newest SDK, superseded by the
// multi-range `selectedRanges` that only the newest systems answer. Day's iOS floor is 15, where
// sending `selectedRanges` is an unrecognized selector — so the single-range pair is the correct
// call here, and the two accessors below are the only place the allow is needed.
#[allow(deprecated)]
fn sel_range(tv: &UITextView) -> NSRange {
    tv.selectedRange()
}

#[allow(deprecated)]
fn set_sel_range(tv: &UITextView, r: NSRange) {
    tv.setSelectedRange(r);
}

/// The `UIFont` a run resolves to — `FontSpec::resolved_points` applies the relative scale, so the
/// editor sizes text exactly as this backend's labels do.
fn run_font(spec: day_spec::FontSpec) -> Retained<UIFont> {
    let pts = spec.resolved_points(FONT_SIZE);
    let weight = if spec.weight.is_some_and(|w| w >= day_spec::FontWeight::Semibold) {
        unsafe { objc2_ui_kit::UIFontWeightBold }
    } else {
        unsafe { objc2_ui_kit::UIFontWeightRegular }
    };
    let base = if spec.monospace {
        UIFont::monospacedSystemFontOfSize_weight(pts, weight)
    } else {
        UIFont::systemFontOfSize_weight(pts, weight)
    };
    if !spec.italic {
        return base;
    }
    // Italic is a descriptor trait on iOS. Ask for it ON TOP of the traits the font already has,
    // so bold+italic stays bold — and fall back to the upright face if the family has no italic.
    let desc = unsafe { base.fontDescriptor() };
    let want = unsafe { desc.symbolicTraits() } | UIFontDescriptorSymbolicTraits::TraitItalic;
    match desc.fontDescriptorWithSymbolicTraits(want) {
        Some(d) => UIFont::fontWithDescriptor_size(&d, pts),
        None => base,
    }
}

/// `Underline` as an `NSUnderlineStyle` bitmask — identical to AppKit's, the constants are shared.
fn underline_bits(u: Underline) -> i64 {
    match u {
        Underline::None => 0,
        Underline::Single => 0x01,
        Underline::Double => 0x09,
        Underline::Dotted => 0x01 | 0x0100,
        Underline::Wavy => 0x01 | 0x0400,
    }
}

fn uicolor(c: day_spec::Color) -> Retained<UIColor> {
    UIColor::colorWithRed_green_blue_alpha(c.r, c.g, c.b, c.a)
}

/// Build the attributed string for a document — the single place runs become UIKit attributes.
fn attributed(doc: &StyledText, base: Font) -> Retained<NSAttributedString> {
    let ns = NSString::from_str(&doc.text);
    let s = NSMutableAttributedString::initWithString(NSMutableAttributedString::alloc(), &ns);
    let whole = NSRange::new(0, ns.length());
    unsafe {
        s.addAttribute_value_range(
            objc2_ui_kit::NSFontAttributeName,
            &run_font(day_spec::FontSpec::new(base)),
            whole,
        );
        // ALWAYS a foreground: an attributed run with none draws black, which is unreadable in
        // dark mode. `labelColor` is the adaptive default the view would have used.
        s.addAttribute_value_range(
            objc2_ui_kit::NSForegroundColorAttributeName,
            &UIColor::labelColor(),
            whole,
        );
    }
    for r in &doc.runs {
        let Some((start, len)) = utf16_range(&doc.text, &r.range) else {
            continue;
        };
        let range = NSRange::new(start, len);
        unsafe {
            s.addAttribute_value_range(
                objc2_ui_kit::NSFontAttributeName,
                &run_font(r.font),
                range,
            );
            if let Some(c) = r.color {
                s.addAttribute_value_range(
                    objc2_ui_kit::NSForegroundColorAttributeName,
                    &uicolor(c),
                    range,
                );
            }
            if let Some(c) = r.background {
                s.addAttribute_value_range(
                    objc2_ui_kit::NSBackgroundColorAttributeName,
                    &uicolor(c),
                    range,
                );
            }
            if r.underline.is_on() {
                s.addAttribute_value_range(
                    objc2_ui_kit::NSUnderlineStyleAttributeName,
                    &NSNumber::new_i64(underline_bits(r.underline)),
                    range,
                );
            }
            if r.strikethrough {
                s.addAttribute_value_range(
                    objc2_ui_kit::NSStrikethroughStyleAttributeName,
                    &NSNumber::new_i64(1),
                    range,
                );
            }
            if let Some(url) = r.link.as_deref() {
                s.addAttribute_value_range(
                    objc2_ui_kit::NSLinkAttributeName,
                    &NSString::from_str(url),
                    range,
                );
            }
        }
    }
    for p in &doc.paragraphs {
        let Some((start, len)) = utf16_range(&doc.text, &p.range) else {
            continue;
        };
        let ps = NSMutableParagraphStyle::new();
        ps.setAlignment(match p.align {
            ParagraphAlign::Natural => NSTextAlignment::Natural,
            ParagraphAlign::Center => NSTextAlignment::Center,
            ParagraphAlign::Trailing => NSTextAlignment::Right,
            ParagraphAlign::Justified => NSTextAlignment::Justified,
        });
        // The list marker sits in the gap between the first-line indent and the head indent.
        let indent = p.indent + f64::from(p.list_level) * 24.0;
        let marker = if p.list == day_spec::ListStyle::None {
            0.0
        } else {
            18.0
        };
        ps.setHeadIndent(indent + marker);
        ps.setFirstLineHeadIndent(indent);
        ps.setParagraphSpacingBefore(p.space_before);
        ps.setParagraphSpacing(p.space_after);
        unsafe {
            s.addAttribute_value_range(
                objc2_ui_kit::NSParagraphStyleAttributeName,
                &ps,
                NSRange::new(start, len),
            );
        }
    }
    s.into_super()
}

/// The typing attributes a `RunStyle` becomes — what UIKit applies to the next typed character.
fn typing_attributes(style: &RunStyle) -> Retained<NSDictionary<NSString, objc2::runtime::AnyObject>>
{
    let mut keys: Vec<&NSString> = Vec::new();
    let mut objs: Vec<Retained<objc2::runtime::AnyObject>> = Vec::new();
    unsafe {
        keys.push(objc2_ui_kit::NSFontAttributeName);
        objs.push(Retained::cast_unchecked(run_font(style.font)));
        keys.push(objc2_ui_kit::NSForegroundColorAttributeName);
        objs.push(Retained::cast_unchecked(
            style.color.map(uicolor).unwrap_or_else(UIColor::labelColor),
        ));
        if let Some(c) = style.background {
            keys.push(objc2_ui_kit::NSBackgroundColorAttributeName);
            objs.push(Retained::cast_unchecked(uicolor(c)));
        }
        if style.underline.is_on() {
            keys.push(objc2_ui_kit::NSUnderlineStyleAttributeName);
            objs.push(Retained::cast_unchecked(NSNumber::new_i64(underline_bits(
                style.underline,
            ))));
        }
        if style.strikethrough {
            keys.push(objc2_ui_kit::NSStrikethroughStyleAttributeName);
            objs.push(Retained::cast_unchecked(NSNumber::new_i64(1)));
        }
        let refs: Vec<&objc2::runtime::AnyObject> = objs.iter().map(|o| &**o).collect();
        NSDictionary::from_slices(&keys, &refs)
    }
}

/// Replace the attributed text while KEEPING the caret. The live-highlighting case: fresh runs on
/// every keystroke, and a plain `setAttributedText:` would drop the caret to the document start
/// each time — and on iOS also dismiss the keyboard's inline candidate bar.
fn set_attributed_preserving_selection(tv: &UITextView, s: &NSAttributedString) {
    let sel = sel_range(tv);
    let storage = tv.textStorage();
    // One begin/end pair: TextKit relays out once rather than once per attribute.
    storage.beginEditing();
    storage.setAttributedString(s);
    storage.endEditing();
    let len = tv.text().length();
    let start = sel.location.min(len);
    set_sel_range(tv, NSRange::new(start, sel.length.min(len - start)));
}

fn make(backend: &mut Uikit, p: &EditorProps, id: NodeId) -> Retained<UIView> {
    let mtm = backend.mtm();
    let tv = UITextView::new(mtm);
    tv.setEditable(p.editable);
    tv.setSelectable(true);
    // The iOS formatting menu is OFF: attributes are Day's (see the header).
    tv.setAllowsEditingTextAttributes(false);
    tv.setTextContainerInset(UIEdgeInsets {
        top: INSET,
        left: 0.0,
        bottom: INSET,
        right: 0.0,
    });
    // Spell-check and autocorrect, and never the smart substitutions: replacing a typed quote
    // would be an edit Day never asked for in a document an app may be re-parsing.
    let no = if p.spellcheck { 0isize } else { 1isize };
    day_uikit::set_text_input_trait(&tv, objc2::sel!(setSpellCheckingType:), no);
    day_uikit::set_text_input_trait(&tv, objc2::sel!(setAutocorrectionType:), no);
    day_uikit::set_text_input_trait(&tv, objc2::sel!(setSmartQuotesType:), 1);
    day_uikit::set_text_input_trait(&tv, objc2::sel!(setSmartDashesType:), 1);

    let base_font = run_font(day_spec::FontSpec::new(p.base));
    tv.setFont(Some(&base_font));
    if !p.doc.is_empty() {
        set_attributed_preserving_selection(&tv, &attributed(&p.doc, p.base));
    }
    // SAFETY: the dictionary holds exactly the value types UIKit documents for these attribute
    // keys (a UIFont, UIColors, NSNumbers) — which is the whole of what makes this setter unsafe.
    unsafe { tv.setTypingAttributes(&typing_attributes(&RunStyle::plain(p.base))) };
    let line_h = unsafe { base_font.lineHeight() };

    // Empty-state prompt: a dim label pinned at the text origin, a subview of the text view so it
    // shares the first line's coordinate space. Hidden as soon as there is text.
    let ph = UILabel::new(mtm);
    ph.setText(Some(&NSString::from_str(&p.placeholder)));
    unsafe {
        ph.setFont(Some(&base_font));
        ph.setTextColor(Some(&UIColor::placeholderTextColor()));
    }
    ph.setNumberOfLines(0);
    let lfp = tv.textContainer().lineFragmentPadding();
    ph.setFrame(CGRect::new(
        CGPoint::new(lfp, INSET),
        CGSize::new(320.0, line_h.ceil()),
    ));
    ph.setHidden(p.placeholder.is_empty() || !p.doc.is_empty());
    tv.addSubview(<UILabel as AsRef<UIView>>::as_ref(&ph));

    let target = EdTarget::new(mtm, id, ph.clone());
    unsafe { tv.setDelegate(Some(ProtocolObject::from_ref(&*target))) };

    let ns: Retained<UIView> = Retained::from(<UITextView as AsRef<UIView>>::as_ref(&tv));
    STATE.with(|t| {
        t.insert(
            key(&ns),
            EdState {
                tv,
                placeholder: ph,
                target,
                base: p.base,
                line_h,
                min_lines: p.min_lines,
                max_lines: p.max_lines,
            },
        )
    });
    ns
}

fn update(_backend: &mut Uikit, h: &Retained<UIView>, patch: &EditorPatch) {
    STATE.with(|t| {
        t.with(key(h), |st| match patch {
            EditorPatch::SetDocument(doc) => {
                set_attributed_preserving_selection(&st.tv, &attributed(doc, st.base));
                st.placeholder.setHidden(!doc.is_empty());
            }
            EditorPatch::SetAttributes(attrs) => {
                // Same characters: rebuild over the text the VIEW holds, so a document one edit
                // stale can never replace what the user just typed.
                let doc = StyledText {
                    text: st.tv.text().to_string(),
                    runs: attrs.runs.clone(),
                    paragraphs: attrs.paragraphs.clone(),
                };
                set_attributed_preserving_selection(&st.tv, &attributed(&doc, st.base));
            }
            EditorPatch::SetSelection(r) => {
                let text = st.tv.text().to_string();
                let Some((start, len)) = utf16_range(&text, r) else {
                    return;
                };
                set_sel_range(&st.tv, NSRange::new(start, len));
            }
            // SAFETY: as at realize — `typing_attributes` builds only documented pairings.
            EditorPatch::SetTypingStyle(style) => unsafe {
                st.tv.setTypingAttributes(&typing_attributes(style))
            },
            EditorPatch::SetEditable(v) => st.tv.setEditable(*v),
        });
    });
}

fn measure(_backend: &mut Uikit, h: &Retained<UIView>, p: Proposal) -> Size {
    let avail_w = p.width.unwrap_or(320.0).max(120.0);
    STATE.with(|t| {
        t.with(key(h), |st| {
            let pad = 2.0 * INSET;
            let min_h = (st.min_lines as f64) * st.line_h + pad;
            let max_h = if st.max_lines > 0 {
                (st.max_lines as f64) * st.line_h + pad
            } else {
                f64::MAX
            };
            let fit = st.tv.sizeThatFits(CGSize::new(avail_w, 1.0e7));
            Size::new(avail_w, fit.height.clamp(min_h, max_h).ceil())
        })
        .unwrap_or_else(|| Size::new(avail_w, 88.0))
    })
}

fn release(_backend: &mut Uikit, h: &Retained<UIView>) {
    STATE.with(|t| {
        t.remove(key(h));
    });
}

day_pieces::renderer!(day_uikit::RENDERERS, Uikit,
    kind: KIND, props: EditorProps, patch: EditorPatch,
    make: make, update: update, measure: measure, release: release);
