// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// AppKit: a rich `NSTextView` in an `NSScrollView`, over the `NSTextStorage` TextKit already
// gives it — the model every other arm here is compared against.
//
// Three things this arm does NOT do, each on purpose and each shared by every other arm:
//
// - **No font panel.** `setUsesFontPanel(false)` and no `NSFontManager` wiring, so ⌘B and the
//   Format menu cannot change attributes behind Day's back. Attributes travel Day → native only
//   (see the crate docs); the app's toolbar goes through the bound signal instead.
// - **No rich paste.** `setImportsGraphics(false)`, and the piece reports the pasted TEXT through
//   the ordinary change path. Pasting styled text keeps its characters and takes the surrounding
//   style, which is what "paste and match style" does — and is the only paste whose result Day's
//   model can describe.
// - **No attribute read-back.** `textDidChange:` reports characters; `NSTextStorageDelegate`'s
//   `editedAttributes` mask is the hook a future read-back would use, and is named in
//   docs/texteditor.md rather than half-implemented here.
// ---------------------------------------------------------------------------

use super::*;

use day_appkit::AppKit;
use day_spec::ffi_guard;
use day_spec::sidetable::SideTable;
use day_spec::{NodeId, ParagraphAlign, Proposal, Size, Underline};
use objc2::rc::Retained;
use objc2::runtime::{NSObjectProtocol, ProtocolObject};
use objc2::{AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSColor, NSFont, NSFontManager, NSFontTraitMask, NSMutableParagraphStyle,
    NSScrollView, NSScrollerStyle, NSTextAlignment, NSTextDelegate, NSTextField, NSTextView,
    NSTextViewDelegate, NSView,
};
use objc2_foundation::{
    NSAttributedString, NSDictionary, NSMutableAttributedString, NSNotification, NSNumber, NSObject,
    NSPoint, NSRange, NSSize, NSString,
};

/// The base point size the editor draws at, and the container inset. Fixed so `measure`'s line
/// math is deterministic, exactly as the built-in text area does it.
const FONT_SIZE: f64 = 13.0;
const INSET: f64 = 6.0;

struct EdIvars {
    node: NodeId,
    /// The empty-state prompt, held so the change delegate can show and hide it (NSTextView has
    /// no placeholder of its own).
    placeholder: Retained<NSTextField>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DayTextEditorTarget"]
    #[ivars = EdIvars]
    struct EdTarget;

    unsafe impl NSObjectProtocol for EdTarget {}
    unsafe impl NSTextViewDelegate for EdTarget {}

    unsafe impl NSTextDelegate for EdTarget {
        // Contained (§8.5): a panic must not unwind into AppKit.
        #[unsafe(method(textDidChange:))]
        fn text_did_change(&self, notification: &NSNotification) {
            ffi_guard::contain((), || {
                if let Some(obj) = notification.object()
                    && let Ok(tv) = obj.downcast::<NSTextView>()
                {
                    let text = tv.string().to_string();
                    self.ivars().placeholder.setHidden(!text.is_empty());
                    day_appkit::emit(self.ivars().node, Event::TextChanged(text));
                }
            })
        }
    }

    impl EdTarget {
        #[unsafe(method(textViewDidChangeSelection:))]
        fn selection_changed(&self, notification: &NSNotification) {
            ffi_guard::contain((), || {
                let Some(obj) = notification.object() else { return };
                let Ok(tv) = obj.downcast::<NSTextView>() else { return };
                let text = tv.string().to_string();
                let r = tv.selectedRange();
                // UTF-16 back to BYTES: the two disagree the moment the document has an emoji,
                // and a selection reported in the wrong unit styles the wrong words.
                let start = byte_of_utf16(&text, r.location);
                let end = byte_of_utf16(&text, r.location + r.length);
                day_appkit::emit(
                    self.ivars().node,
                    Event::custom("texteditor:sel", selection_payload(start, end)),
                );
            })
        }
    }
);

impl EdTarget {
    fn new(mtm: MainThreadMarker, node: NodeId, placeholder: Retained<NSTextField>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(EdIvars { node, placeholder });
        unsafe { msg_send![super(this), init] }
    }
}

struct EdState {
    tv: Retained<NSTextView>,
    placeholder: Retained<NSTextField>,
    #[allow(dead_code)]
    target: Retained<EdTarget>,
    base: Font,
    line_h: f64,
    min_lines: u32,
    max_lines: u32,
}

thread_local! {
    static STATE: SideTable<EdState> = SideTable::with_teardown(|st: EdState| {
        // The text view outlives this state until its scroll view deallocs: detach the delegate
        // so the dropped target is never reached through a stale reference.
        st.tv.setDelegate(None);
    });
}

fn key(v: &Retained<NSView>) -> usize {
    Retained::as_ptr(v) as usize
}

/// The `NSFont` a run resolves to. `FontSpec::resolved_points` is what applies the relative
/// scale, so every backend's editor scales the same way its labels do.
fn run_font(spec: day_spec::FontSpec, mtm: MainThreadMarker) -> Retained<NSFont> {
    let pts = spec.resolved_points(FONT_SIZE);
    let weight = if spec.weight.is_some_and(|w| w >= day_spec::FontWeight::Semibold) {
        unsafe { objc2_app_kit::NSFontWeightBold }
    } else {
        unsafe { objc2_app_kit::NSFontWeightRegular }
    };
    let base = if spec.monospace {
        NSFont::monospacedSystemFontOfSize_weight(pts, weight)
    } else {
        NSFont::systemFontOfSize_weight(pts, weight)
    };
    if spec.italic {
        NSFontManager::sharedFontManager(mtm)
            .convertFont_toHaveTrait(&base, NSFontTraitMask::ItalicFontMask)
    } else {
        base
    }
}

/// `Underline` as an `NSUnderlineStyle` bitmask — line style low, pattern second byte.
fn underline_bits(u: Underline) -> i64 {
    match u {
        Underline::None => 0,
        Underline::Single => 0x01,
        Underline::Double => 0x09,
        Underline::Dotted => 0x01 | 0x0100,
        Underline::Wavy => 0x01 | 0x0400,
    }
}

fn nscolor(c: day_spec::Color) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(c.r, c.g, c.b, c.a)
}

/// Build the attributed string for a document — the one place runs become AppKit attributes, so
/// realize and every attribute patch produce identical results.
fn attributed(doc: &StyledText, base: Font, mtm: MainThreadMarker) -> Retained<NSAttributedString> {
    let ns = NSString::from_str(&doc.text);
    let s = NSMutableAttributedString::initWithString(NSMutableAttributedString::alloc(), &ns);
    let whole = NSRange::new(0, ns.length());
    unsafe {
        s.addAttribute_value_range(
            objc2_app_kit::NSFontAttributeName,
            &run_font(day_spec::FontSpec::new(base), mtm),
            whole,
        );
        // ALWAYS a foreground: an attributed range with no color draws black, unreadable in dark
        // mode. `labelColor` is the adaptive default the view would have picked itself.
        s.addAttribute_value_range(
            objc2_app_kit::NSForegroundColorAttributeName,
            &NSColor::labelColor(),
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
                objc2_app_kit::NSFontAttributeName,
                &run_font(r.font, mtm),
                range,
            );
            if let Some(c) = r.color {
                s.addAttribute_value_range(
                    objc2_app_kit::NSForegroundColorAttributeName,
                    &nscolor(c),
                    range,
                );
            }
            if let Some(c) = r.background {
                s.addAttribute_value_range(
                    objc2_app_kit::NSBackgroundColorAttributeName,
                    &nscolor(c),
                    range,
                );
            }
            if r.underline.is_on() {
                s.addAttribute_value_range(
                    objc2_app_kit::NSUnderlineStyleAttributeName,
                    &NSNumber::new_i64(underline_bits(r.underline)),
                    range,
                );
            }
            if r.strikethrough {
                s.addAttribute_value_range(
                    objc2_app_kit::NSStrikethroughStyleAttributeName,
                    &NSNumber::new_i64(1),
                    range,
                );
            }
            if let Some(url) = r.link.as_deref() {
                s.addAttribute_value_range(
                    objc2_app_kit::NSLinkAttributeName,
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
        // A list item's marker sits in the space the indent reserves, which is why the head
        // indent is the full one and the FIRST-line indent stops short of it.
        let indent = p.indent + f64::from(p.list_level) * 24.0;
        ps.setHeadIndent(indent + if p.list == day_spec::ListStyle::None { 0.0 } else { 18.0 });
        ps.setFirstLineHeadIndent(indent);
        ps.setParagraphSpacingBefore(p.space_before);
        ps.setParagraphSpacing(p.space_after);
        unsafe {
            s.addAttribute_value_range(
                objc2_app_kit::NSParagraphStyleAttributeName,
                &ps,
                NSRange::new(start, len),
            );
        }
    }
    s.into_super()
}

/// The typing attributes a `RunStyle` becomes — what AppKit applies to the next typed character.
fn typing_attributes(
    style: &RunStyle,
    mtm: MainThreadMarker,
) -> Retained<NSDictionary<NSString, objc2::runtime::AnyObject>> {
    let mut keys: Vec<&NSString> = Vec::new();
    let mut objs: Vec<Retained<objc2::runtime::AnyObject>> = Vec::new();
    let font = run_font(style.font, mtm);
    unsafe {
        keys.push(objc2_app_kit::NSFontAttributeName);
        objs.push(Retained::cast_unchecked(font));
        keys.push(objc2_app_kit::NSForegroundColorAttributeName);
        objs.push(Retained::cast_unchecked(
            style.color.map(nscolor).unwrap_or_else(NSColor::labelColor),
        ));
        if let Some(c) = style.background {
            keys.push(objc2_app_kit::NSBackgroundColorAttributeName);
            objs.push(Retained::cast_unchecked(nscolor(c)));
        }
        if style.underline.is_on() {
            keys.push(objc2_app_kit::NSUnderlineStyleAttributeName);
            objs.push(Retained::cast_unchecked(NSNumber::new_i64(underline_bits(
                style.underline,
            ))));
        }
        if style.strikethrough {
            keys.push(objc2_app_kit::NSStrikethroughStyleAttributeName);
            objs.push(Retained::cast_unchecked(NSNumber::new_i64(1)));
        }
        let refs: Vec<&objc2::runtime::AnyObject> = objs.iter().map(|o| &**o).collect();
        NSDictionary::from_slices(&keys, &refs)
    }
}

/// Replace the view's attributed string while KEEPING the caret where the user left it.
///
/// The whole reason a live syntax highlighter is usable: it pushes fresh runs on every keystroke,
/// and a naive `setAttributedString:` would send the caret to the start of the document each time.
fn set_attributed_preserving_selection(tv: &NSTextView, s: &NSAttributedString) {
    let sel = tv.selectedRange();
    let Some(storage) = (unsafe { tv.textStorage() }) else {
        return;
    };
    // One begin/end pair, so TextKit relays out once rather than per attribute.
    storage.beginEditing();
    storage.setAttributedString(s);
    storage.endEditing();
    let len = tv.string().length();
    let start = sel.location.min(len);
    tv.setSelectedRange(NSRange::new(start, sel.length.min(len - start)));
}

fn make(backend: &mut AppKit, p: &EditorProps, id: NodeId) -> Retained<NSView> {
    let mtm = backend.mtm();
    let scroll = NSScrollView::new(mtm);
    scroll.setDrawsBackground(false);
    scroll.setHasVerticalScroller(true);
    scroll.setScrollerStyle(NSScrollerStyle::Overlay);
    scroll.setHasHorizontalScroller(false);

    let tv = NSTextView::new(mtm);
    // RICH text — the one line that separates this from the built-in `text_area`.
    tv.setRichText(true);
    tv.setEditable(p.editable);
    tv.setSelectable(true);
    // No font panel and no graphics import: attributes are Day's (see the header).
    tv.setUsesFontPanel(false);
    tv.setImportsGraphics(false);
    unsafe {
        let _: () = msg_send![&tv, setContinuousSpellCheckingEnabled: p.spellcheck];
        let _: () = msg_send![&tv, setAutomaticSpellingCorrectionEnabled: p.spellcheck];
        let _: () = msg_send![&tv, setGrammarCheckingEnabled: p.spellcheck];
        // Smart quotes and dashes REPLACE what was typed, which would be an edit Day never asked
        // for in a document whose text an app may be parsing (the syntax-highlighting case).
        let _: () = msg_send![&tv, setAutomaticQuoteSubstitutionEnabled: false];
        let _: () = msg_send![&tv, setAutomaticDashSubstitutionEnabled: false];
    }
    tv.setTextContainerInset(NSSize::new(INSET, INSET));
    tv.setVerticallyResizable(true);
    tv.setHorizontallyResizable(false);
    tv.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
    tv.setMinSize(NSSize::new(0.0, 0.0));
    tv.setMaxSize(NSSize::new(1.0e7, 1.0e7));
    if let Some(tc) = unsafe { tv.textContainer() } {
        tc.setWidthTracksTextView(true);
    }
    let base_font = run_font(day_spec::FontSpec::new(p.base), mtm);
    tv.setFont(Some(&base_font));
    if !p.doc.is_empty() {
        set_attributed_preserving_selection(&tv, &attributed(&p.doc, p.base, mtm));
    }
    // SAFETY: the dictionary's values are the attribute types AppKit documents for these keys
    // (an NSFont, NSColors, NSNumbers) — which is the whole of what makes this setter unsafe.
    unsafe { tv.setTypingAttributes(&typing_attributes(&RunStyle::plain(p.base), mtm)) };
    let line_h = unsafe { tv.layoutManager() }
        .map(|lm| lm.defaultLineHeightForFont(&base_font))
        .unwrap_or(FONT_SIZE * 1.3);

    // Empty-state prompt: a dim label at the text origin, a subview of the text view itself so it
    // sits in the same coordinate space as the first line. Non-interactive, hidden once there is
    // text — NSTextView, unlike NSTextField, has no placeholder of its own.
    let ph = NSTextField::labelWithString(&NSString::from_str(&p.placeholder), mtm);
    ph.setFont(Some(&base_font));
    ph.setTextColor(Some(&NSColor::tertiaryLabelColor()));
    ph.setSelectable(false);
    ph.sizeToFit();
    let lfp = unsafe { tv.textContainer() }
        .map(|tc| tc.lineFragmentPadding())
        .unwrap_or(0.0);
    ph.setFrameOrigin(NSPoint::new(INSET + lfp, INSET));
    ph.setHidden(p.placeholder.is_empty() || !p.doc.is_empty());
    tv.addSubview(<NSTextField as AsRef<NSView>>::as_ref(&ph));

    let target = EdTarget::new(mtm, id, ph.clone());
    tv.setDelegate(Some(ProtocolObject::from_ref(&*target)));
    scroll.setDocumentView(Some(&tv));

    let ns: Retained<NSView> = Retained::from(<NSScrollView as AsRef<NSView>>::as_ref(&scroll));
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

fn update(backend: &mut AppKit, h: &Retained<NSView>, patch: &EditorPatch) {
    let mtm = backend.mtm();
    STATE.with(|t| {
        t.with(key(h), |st| match patch {
            EditorPatch::SetDocument(doc) => {
                set_attributed_preserving_selection(&st.tv, &attributed(doc, st.base, mtm));
                st.placeholder.setHidden(!doc.is_empty());
            }
            EditorPatch::SetAttributes(attrs) => {
                // Same characters: rebuild the attributed string over the text the VIEW holds,
                // so a document one edit stale can never replace what the user just typed.
                let doc = StyledText {
                    text: st.tv.string().to_string(),
                    runs: attrs.runs.clone(),
                    paragraphs: attrs.paragraphs.clone(),
                };
                set_attributed_preserving_selection(&st.tv, &attributed(&doc, st.base, mtm));
            }
            EditorPatch::SetSelection(r) => {
                let text = st.tv.string().to_string();
                let Some((start, len)) = utf16_range(&text, r) else {
                    return;
                };
                st.tv.setSelectedRange(NSRange::new(start, len));
            }
            EditorPatch::SetTypingStyle(style) => {
                // SAFETY: as at realize — `typing_attributes` builds only documented pairings.
                unsafe { st.tv.setTypingAttributes(&typing_attributes(style, mtm)) };
            }
            EditorPatch::SetEditable(v) => st.tv.setEditable(*v),
        });
    });
}

fn measure(_backend: &mut AppKit, h: &Retained<NSView>, p: Proposal) -> Size {
    let avail_w = p.width.unwrap_or(320.0).max(120.0);
    STATE
        .with(|t| {
            t.with(key(h), |st| {
                let pad = 2.0 * INSET;
                let min_h = (st.min_lines as f64) * st.line_h + pad;
                let max_h = if st.max_lines > 0 {
                    (st.max_lines as f64) * st.line_h + pad
                } else {
                    f64::MAX
                };
                let (Some(tc), Some(lm)) =
                    (unsafe { st.tv.textContainer() }, unsafe { st.tv.layoutManager() })
                else {
                    return Size::new(avail_w, min_h.ceil());
                };
                // Measure the wrapped content at the proposed inner width, with width-tracking
                // temporarily off so the query does not depend on a frame that is not set yet.
                let lfp = tc.lineFragmentPadding();
                let inner_w = (avail_w - 2.0 * INSET - 2.0 * lfp).max(1.0);
                tc.setWidthTracksTextView(false);
                tc.setContainerSize(NSSize::new(inner_w, 1.0e7));
                let _ = lm.glyphRangeForTextContainer(&tc);
                let used = lm.usedRectForTextContainer(&tc);
                tc.setWidthTracksTextView(true);
                let h = (used.size.height + pad).clamp(min_h, max_h);
                Size::new(avail_w, h.ceil())
            })
        })
        .unwrap_or(Size::new(avail_w, 88.0))
}

fn release(_backend: &mut AppKit, h: &Retained<NSView>) {
    STATE.with(|t| {
        t.remove(key(h));
    });
}

day_pieces::renderer!(day_appkit::RENDERERS, AppKit,
    kind: KIND, props: EditorProps, patch: EditorPatch,
    make: make, update: update, measure: measure, release: release);
