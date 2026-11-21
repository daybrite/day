// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

#[cfg(not(target_os = "macos"))]
fn main() {}

/// Label paragraph attributes, decorations, font features, reading direction and selection
/// reports on real AppKit widgets (docs/text.md, docs/text-runs.md). The selection is moved
/// through the field editor's own API; no events are posted.
#[cfg(target_os = "macos")]
#[expect(
    clippy::too_many_lines,
    reason = "one sequential native scenario on the main thread"
)]
fn main() {
    use std::{cell::RefCell, rc::Rc};

    use day_spec::props::{
        LabelParagraph, LabelPatch, LabelProps, SliderProps, TextAlign, Truncation,
    };
    use day_spec::{
        Cap, Color, Decoration, Event, Font, FontSpec, LayoutDirection, NodeId, Proposal, Support,
        TextRun, Toolkit, kinds,
    };
    use day_toolkit_appkit::AppKit;
    use objc2::{MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{
        NSApplication, NSBackingStoreType, NSFontFeatureSettingsAttribute, NSLineBreakMode,
        NSSlider, NSTextAlignment, NSTextField, NSTextView, NSUnderlineColorAttributeName,
        NSUserInterfaceLayoutDirection, NSWindow, NSWindowStyleMask, NSWritingDirection,
    };
    use objc2_foundation::{NSPoint, NSRange, NSRect, NSSize};

    let mtm = MainThreadMarker::new().expect("native test runs on main thread");
    let _app = NSApplication::sharedApplication(mtm);
    let mut toolkit = AppKit::new();
    assert_eq!(toolkit.capability(Cap::LabelSelection), Support::Native);
    assert!((toolkit.text_scale(Font::Body) - 1.0).abs() < f64::EPSILON);
    let events = Rc::new(RefCell::new(Vec::new()));
    let captured = events.clone();
    toolkit.set_event_sink(Box::new(move |node, event| {
        if let Event::TextSelected { start, end } = event {
            captured.borrow_mut().push((node.0, start, end));
        }
    }));

    let label = |text: &str| LabelProps {
        text: text.into(),
        wraps: true,
        ..Default::default()
    };
    let width = Proposal {
        width: Some(100.0),
        height: None,
    };

    // Line limit: realized unlimited, then patched to two lines and one, it measures like a
    // two-line and a one-line label.
    let long = "Day lays this sentence out over many lines when its label is narrow, so a line \
                limit has plenty of text to cut.";
    let limited = toolkit.realize(kinds::LABEL, &label(long), NodeId(1));
    let two = toolkit.realize(kinds::LABEL, &label("Ag\nAg"), NodeId(2));
    let one = toolkit.realize(kinds::LABEL, &label("Ag"), NodeId(3));
    let two_h = toolkit.measure(&two, kinds::LABEL, width).height;
    let one_h = toolkit.measure(&one, kinds::LABEL, width).height;
    assert!(toolkit.measure(&limited, kinds::LABEL, width).height > two_h);
    for (lines, expect) in [(2, two_h), (1, one_h)] {
        let patch = LabelPatch::Paragraph(LabelParagraph {
            max_lines: lines,
            ..Default::default()
        });
        toolkit.update(&limited, kinds::LABEL, &patch, None);
        let h = toolkit.measure(&limited, kinds::LABEL, width).height;
        assert!((h - expect).abs() <= 1.0, "{lines} lines: {h} vs {expect}");
    }
    let field = limited.downcast_ref::<NSTextField>().unwrap();
    assert_eq!(
        field.stringValue().to_string(),
        long,
        "the text stays whole"
    );

    // Truncation on one line follows the mode; several lines wrap and end the last line.
    for (mode, expect) in [
        (Truncation::Head, NSLineBreakMode::ByTruncatingHead),
        (Truncation::Middle, NSLineBreakMode::ByTruncatingMiddle),
        (Truncation::Tail, NSLineBreakMode::ByTruncatingTail),
    ] {
        let patch = LabelPatch::Paragraph(LabelParagraph {
            max_lines: 1,
            truncation: mode,
            ..Default::default()
        });
        toolkit.update(&limited, kinds::LABEL, &patch, None);
        assert_eq!(field.lineBreakMode(), expect);
    }
    let patch = LabelPatch::Paragraph(LabelParagraph {
        max_lines: 3,
        truncation: Truncation::Middle,
        ..Default::default()
    });
    toolkit.update(&limited, kinds::LABEL, &patch, None);
    assert_eq!(field.lineBreakMode(), NSLineBreakMode::ByWordWrapping);
    assert!(field.cell().unwrap().truncatesLastVisibleLine());
    assert_eq!(field.maximumNumberOfLines(), 3);

    // Letter spacing in ems: four characters at 20 points and half an em are 30 to 40 points
    // wider; a font patch keeps the spacing (it scales with the new size).
    let hhhh = |spacing: f64| LabelProps {
        font: FontSpec::new(Font::System(20.0)),
        letter_spacing: spacing,
        ..label("HHHH")
    };
    let plain = toolkit.realize(kinds::LABEL, &hhhh(0.0), NodeId(4));
    let spaced = toolkit.realize(kinds::LABEL, &hhhh(0.5), NodeId(5));
    let free = Proposal::UNCONSTRAINED;
    let gap = toolkit.measure(&spaced, kinds::LABEL, free).width
        - toolkit.measure(&plain, kinds::LABEL, free).width;
    assert!((29.0..=41.0).contains(&gap), "letter spacing added {gap}");
    let bigger = LabelPatch::Font(FontSpec::new(Font::System(40.0)));
    toolkit.update(&spaced, kinds::LABEL, &bigger, None);
    toolkit.update(&plain, kinds::LABEL, &bigger, None);
    let gap = toolkit.measure(&spaced, kinds::LABEL, free).width
        - toolkit.measure(&plain, kinds::LABEL, free).width;
    assert!(
        (59.0..=81.0).contains(&gap),
        "letter spacing at 40 pt added {gap}"
    );
    // Back to zero: the label measures as the plain one again.
    let unspaced = LabelPatch::Paragraph(LabelParagraph::default());
    toolkit.update(&spaced, kinds::LABEL, &unspaced, None);
    let gap = toolkit.measure(&spaced, kinds::LABEL, free).width
        - toolkit.measure(&plain, kinds::LABEL, free).width;
    assert!(gap.abs() <= 1.0, "zero spacing left {gap}");

    // Line height as a multiple of the font size, also for one line, and patched.
    let tall = |text: &str, multiple: f64| LabelProps {
        font: FontSpec::new(Font::System(20.0)),
        line_height: multiple,
        ..label(text)
    };
    let lh_one = toolkit.realize(kinds::LABEL, &tall("Ag", 2.0), NodeId(6));
    let lh_two = toolkit.realize(kinds::LABEL, &tall("Ag\nAg", 2.0), NodeId(7));
    let h1 = toolkit.measure(&lh_one, kinds::LABEL, free).height;
    let h2 = toolkit.measure(&lh_two, kinds::LABEL, free).height;
    assert!((h1 - 40.0).abs() <= 1.5, "one line at 2.0: {h1}");
    assert!((h2 - 80.0).abs() <= 1.5, "two lines at 2.0: {h2}");
    let tighter = LabelPatch::Paragraph(LabelParagraph {
        line_height: 1.5,
        ..Default::default()
    });
    toolkit.update(&lh_one, kinds::LABEL, &tighter, None);
    let h1 = toolkit.measure(&lh_one, kinds::LABEL, free).height;
    assert!((h1 - 30.0).abs() <= 1.5, "one line at 1.5: {h1}");
    // A text patch keeps the stamped paragraph.
    toolkit.update(&lh_one, kinds::LABEL, &LabelPatch::Text("Bg".into()), None);
    let h1 = toolkit.measure(&lh_one, kinds::LABEL, free).height;
    assert!((h1 - 30.0).abs() <= 1.5, "after a text patch: {h1}");

    // Justified is AppKit's own alignment.
    let justified = toolkit.realize(
        kinds::LABEL,
        &LabelProps {
            align: TextAlign::Justified,
            ..label(long)
        },
        NodeId(8),
    );
    let jf = justified.downcast_ref::<NSTextField>().unwrap();
    assert_eq!(jf.alignment(), NSTextAlignment::Justified);

    // Decorations carry their color; small capitals reach the font descriptor.
    let red = Color::hex(0x00D0_021B);
    let runs = vec![TextRun {
        range: 0..5,
        underline: Some(Decoration::wavy().color(red)),
        strikethrough: Some(Decoration::double()),
        ..Default::default()
    }];
    let decorated = toolkit.realize(
        kinds::LABEL,
        &LabelProps {
            runs,
            ..label("Check this")
        },
        NodeId(9),
    );
    let df = decorated.downcast_ref::<NSTextField>().unwrap();
    let s = df.attributedStringValue();
    let mut range = NSRange::new(0, 0);
    let color = unsafe {
        s.attribute_atIndex_effectiveRange(NSUnderlineColorAttributeName, 1, &raw mut range)
    };
    assert!(
        color.is_some(),
        "the underline color rides beside its style"
    );
    let caps = toolkit.realize(
        kinds::LABEL,
        &LabelProps {
            font: FontSpec {
                small_caps: true,
                oldstyle_figures: true,
                ..FontSpec::new(Font::Body)
            },
            ..label("Chapter 12")
        },
        NodeId(10),
    );
    let font = caps.downcast_ref::<NSTextField>().unwrap().font().unwrap();
    let settings = font
        .fontDescriptor()
        .objectForKey(unsafe { NSFontFeatureSettingsAttribute });
    assert!(
        settings.is_some(),
        "smcp/onum are descriptor feature settings"
    );

    // Reading direction: a label in a right-to-left subtree writes right to left, Leading
    // follows it and Trailing is the left edge; a slider takes the direction; a container
    // is left alone.
    let trailing = toolkit.realize(
        kinds::LABEL,
        &LabelProps {
            align: TextAlign::Trailing,
            ..label("Trailing")
        },
        NodeId(11),
    );
    toolkit.set_layout_direction(&trailing, kinds::LABEL, LayoutDirection::Rtl);
    let tf = trailing.downcast_ref::<NSTextField>().unwrap();
    assert_eq!(tf.alignment(), NSTextAlignment::Left);
    assert_eq!(tf.baseWritingDirection(), NSWritingDirection::RightToLeft);
    toolkit.set_layout_direction(&trailing, kinds::LABEL, LayoutDirection::Ltr);
    assert_eq!(tf.alignment(), NSTextAlignment::Right);
    assert_eq!(tf.baseWritingDirection(), NSWritingDirection::LeftToRight);
    let slider = toolkit.realize(kinds::SLIDER, &SliderProps::default(), NodeId(12));
    toolkit.set_layout_direction(&slider, kinds::SLIDER, LayoutDirection::Rtl);
    let sl = slider.downcast_ref::<NSSlider>().unwrap();
    assert_eq!(
        sl.userInterfaceLayoutDirection(),
        NSUserInterfaceLayoutDirection::RightToLeft
    );
    let container = toolkit.realize(
        kinds::CONTAINER,
        &day_spec::props::ContainerProps::default(),
        NodeId(13),
    );
    let before = container.userInterfaceLayoutDirection();
    toolkit.set_layout_direction(&container, kinds::CONTAINER, LayoutDirection::Rtl);
    assert_eq!(container.userInterfaceLayoutDirection(), before);

    // Selection reports: byte offsets into the displayed text, once per change, cleared when
    // editing ends.
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(440.0, 120.0)),
            NSWindowStyleMask::Titled,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    unsafe { window.setReleasedWhenClosed(false) };
    let text = "Say déjà vu 🌅 now";
    let reporting = toolkit.realize(
        kinds::LABEL,
        &LabelProps {
            reports_selection: true,
            ..label(text)
        },
        NodeId(14),
    );
    toolkit.set_selectable(&reporting, true);
    reporting.setFrame(NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(440.0, 40.0),
    ));
    window.contentView().unwrap().addSubview(&reporting);
    let rf = reporting.downcast_ref::<NSTextField>().unwrap();
    unsafe { rf.selectText(None) };
    let editor = rf.currentEditor().expect("label has a field editor");
    let editor = editor.downcast::<NSTextView>().unwrap();
    events.borrow_mut().clear();
    // UTF-16 4..8 is "déjà"; 12..14 is the sunrise emoji (a surrogate pair).
    editor.setSelectedRange(NSRange::new(4, 4));
    editor.setSelectedRange(NSRange::new(4, 4));
    editor.setSelectedRange(NSRange::new(12, 2));
    editor.setSelectedRange(NSRange::new(2, 0));
    editor.setSelectedRange(NSRange::new(3, 0));
    let deja = text.find("déjà").unwrap();
    let sun = text.find('🌅').unwrap();
    assert_eq!(
        *events.borrow(),
        [
            (14, deja, deja + "déjà".len()),
            (14, sun, sun + '🌅'.len_utf8()),
            (14, 2, 2),
        ],
        "one report per change, a moved caret reports nothing"
    );
    editor.setSelectedRange(NSRange::new(0, 3));
    events.borrow_mut().clear();
    unsafe { window.endEditingFor(None) };
    assert_eq!(*events.borrow(), [(14, 3, 3)], "ending the edit clears it");

    for h in [
        limited, two, one, plain, spaced, lh_one, lh_two, justified, decorated, caps, trailing,
        slider, container,
    ] {
        toolkit.release(h);
    }
    reporting.removeFromSuperview();
    toolkit.release(reporting);
    window.close();
    println!("native label text: paragraph, decorations, features, direction, selection passed");
}
