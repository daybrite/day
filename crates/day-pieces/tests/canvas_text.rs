// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Canvas fonts and the platform font list on the mock toolkit (docs/fonts.md): a drawn
//! `DrawOp::Text` carries its font, `font_families()` answers the mock's fixed list once, and
//! `measure_text()` answers the mock's formula.

use day_core::AnyPiece;
use day_mock::{MockProbe, MockToolkit};
use day_pieces::prelude::*;
use day_spec::{Size, WindowOptions};

fn boot(root: impl FnOnce() -> AnyPiece + 'static) -> MockProbe {
    day_core::uninstall_tree();
    let (mock, probe) = MockToolkit::new();
    let options = WindowOptions {
        title: "test".into(),
        size: Size::new(400.0, 600.0),
        ..Default::default()
    };
    day_core::launch_with(mock, options, root);
    probe
}

#[test]
fn a_drawn_text_records_its_font() {
    let probe = boot(|| {
        canvas(|d, _size| {
            d.text(
                "hi",
                Point::new(4.0, 5.0),
                TextStyle {
                    size: 20.0,
                    font: CanvasFont {
                        family: Some("Day Sans".into()),
                        weight: Some(FontWeight::Bold),
                        italic: true,
                    },
                    ..Default::default()
                },
            );
            d.text("plain", Point::new(0.0, 0.0), TextStyle::default());
        })
        .any()
    });
    let wells = probe.find_by_kind("day.canvas");
    assert_eq!(wells.len(), 1);
    let ops = probe.widget(wells[0].0).ops;
    let fonts: Vec<(String, CanvasFont)> = ops
        .iter()
        .filter_map(|op| match op {
            DrawOp::Text { text, font, .. } => Some((text.clone(), font.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(fonts.len(), 2);
    assert_eq!(fonts[0].0, "hi");
    assert_eq!(fonts[0].1.family.as_deref(), Some("Day Sans"));
    assert_eq!(fonts[0].1.weight, Some(FontWeight::Bold));
    assert!(fonts[0].1.italic);
    assert_eq!(fonts[1].1, CanvasFont::default());
}

#[test]
fn the_font_list_is_answered_once_and_the_faces_resolve() {
    let probe = boot(|| label("x").any());
    assert_eq!(
        day_core::capability(day_spec::Cap::FontList),
        day_spec::Support::Emulated
    );
    let list = day_core::font_families();
    let names: Vec<&str> = list.iter().map(|f| f.family.as_str()).collect();
    assert_eq!(names, ["Day Mono", "Day Sans"], "sorted by family");
    let sans = &list[1];
    assert!(sans.has_bold() && sans.has_italic());
    assert_eq!(
        sans.face_for(FontWeight::Bold, true)
            .map(|f| f.name.as_str()),
        Some("Bold Italic")
    );
    let mono = &list[0];
    assert!(!mono.has_bold() && !mono.has_italic());
    // Cached: a second call is the same allocation and the toolkit is not asked again.
    let again = day_core::font_families();
    assert!(std::rc::Rc::ptr_eq(&list, &again));
    assert_eq!(
        probe.log().iter().filter(|l| *l == "font_families").count(),
        1
    );
}

#[test]
fn measurement_uses_the_toolkit_and_the_facade_formula_agrees() {
    let _probe = boot(|| label("x").any());
    let m = day_core::measure_text("abcd", 10.0, &CanvasFont::default());
    assert_eq!((m.width, m.height, m.ascent), (24.0, 12.0, 9.0));
    assert_eq!(m, TextMetrics::approximate("abcd", 10.0));
}
