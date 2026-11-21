// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Text on the mock toolkit (docs/text.md, docs/text-runs.md, docs/markdown-view.md): what a
//! label hands the toolkit for its paragraph attributes, decorations, links and selection,
//! how a text-scale cap and a subtree's reading direction reach it, and that the Markdown
//! view updates the rows it has instead of building new ones.

use std::cell::RefCell;
use std::rc::Rc;

use day_core::AnyPiece;
use day_mock::{MockProbe, MockToolkit, MockWidget};
use day_pieces::prelude::*;
use day_pieces::{SelectedText, markdown};
use day_reactive::flush_sync;
use day_spec::props::LabelParagraph;
use day_spec::{Event, LayoutDirection, NodeId, WindowOptions};

fn boot_with(
    setup: impl FnOnce(&MockProbe),
    root: impl FnOnce() -> AnyPiece + 'static,
) -> MockProbe {
    day_core::uninstall_tree();
    let (mock, probe) = MockToolkit::new();
    setup(&probe);
    let options = WindowOptions {
        title: "test".into(),
        size: Size::new(400.0, 600.0),
        ..Default::default()
    };
    day_core::launch_with(mock, options, root);
    flush_sync();
    probe
}

fn boot(root: impl FnOnce() -> AnyPiece + 'static) -> MockProbe {
    boot_with(|_| {}, root)
}

fn labels(probe: &MockProbe) -> Vec<MockWidget> {
    let mut found = probe.find_by_kind("day.label");
    found.sort_by_key(|(h, _)| h.0);
    found.into_iter().map(|(_, w)| w).collect()
}

fn realized(probe: &MockProbe) -> usize {
    probe
        .log()
        .iter()
        .filter(|op| op.starts_with("realize"))
        .count()
}

#[test]
fn paragraph_attributes_reach_the_toolkit_and_follow_their_signals() {
    let lines = Signal::new(3u32);
    let spacing = Signal::new(0.0);
    let probe = boot(move || {
        label("one two three four five six seven eight nine ten eleven twelve")
            .max_lines(lines)
            .truncation(Truncation::Middle)
            .letter_spacing(spacing)
            .line_height(1.5)
            .width(80.0)
            .any()
    });
    let label = &labels(&probe)[0];
    assert_eq!(
        label.paragraph,
        LabelParagraph {
            max_lines: 3,
            truncation: Truncation::Middle,
            letter_spacing: 0.0,
            line_height: 1.5,
        }
    );
    // 17-point body at 1.5 is a 25.5-point line; three lines.
    assert!(
        (label.frame.size.height - 76.5).abs() < 0.01,
        "{:?}",
        label.frame
    );

    lines.set(1);
    spacing.set(0.1);
    flush_sync();
    let label = &labels(&probe)[0];
    assert_eq!(label.paragraph.max_lines, 1);
    assert!((label.paragraph.letter_spacing - 0.1).abs() < 1e-9);
    assert!(
        (label.frame.size.height - 25.5).abs() < 0.01,
        "{:?}",
        label.frame
    );
}

#[test]
fn a_constant_paragraph_costs_no_patch() {
    let probe = boot(|| label("plain").max_lines(2).letter_spacing(0.05).any());
    assert!(
        !probe.log().iter().any(|op| op.contains("paragraph=")),
        "constants are realize props, never patches: {:?}",
        probe.log()
    );
}

#[test]
fn decorations_become_runs() {
    let red = Color::hex(0x00D0_021B);
    let probe = boot(move || {
        column((
            label("").runs_from(
                TextBuilder::new()
                    .text("see ")
                    .underline_styled("unknown", Decoration::wavy().color(red))
                    .text(" ")
                    .strikethrough_styled("old", DecorationStyle::Double),
            ),
            label("whole").underline(Decoration::dotted()),
            label("**bold** rest")
                .markdown()
                .strikethrough(Decoration::SINGLE),
        ))
        .any()
    });
    let found = labels(&probe);
    let runs = &found[0].runs;
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].underline, Some(Decoration::wavy().color(red)));
    assert_eq!(runs[1].strikethrough, Some(Decoration::double()));

    // A whole-label decoration is one run over the whole text, in the label's font.
    let whole = &found[1];
    assert_eq!(whole.runs.len(), 1);
    assert_eq!(whole.runs[0].range, 0..5);
    assert_eq!(whole.runs[0].underline, Some(Decoration::dotted()));

    // Over Markdown runs it fills the gaps and joins the runs that are there.
    let mixed = &found[2];
    assert_eq!(mixed.text, "bold rest");
    assert_eq!(mixed.runs.len(), 2);
    assert!(
        mixed
            .runs
            .iter()
            .all(|r| r.strikethrough == Some(Decoration::SINGLE))
    );
    assert_eq!(mixed.runs[0].font.weight, Some(FontWeight::Bold));
    assert_eq!(mixed.runs[1].range, 4..9);
    assert!(day_spec::runs_are_valid(&mixed.text, &mixed.runs).is_ok());
}

#[test]
fn font_features_ride_the_font_spec() {
    let probe = boot(|| {
        label("Chapter 12")
            .small_caps()
            .oldstyle_figures()
            .tabular()
            .any()
    });
    let font = labels(&probe)[0].font.expect("a label has a font");
    assert!(font.small_caps && font.oldstyle_figures && font.tabular);
    assert_eq!(font.feature_tags(), vec!["tnum", "smcp", "onum"]);
}

#[test]
fn a_link_is_a_link_run_where_the_toolkit_activates_them() {
    let title = Signal::new("Docs".to_owned());
    let handled = Rc::new(RefCell::new(Vec::<String>::new()));
    let seen = handled.clone();
    let probe = boot_with(
        |p| p.set_text_links(true),
        move || {
            column((
                link(title, "https://daybrite.dev/docs").bold(),
                link("Custom", "app:thing").on_link(move |t| seen.borrow_mut().push(t.to_owned())),
            ))
            .any()
        },
    );
    let found = labels(&probe);
    let run = &found[0].runs[0];
    assert_eq!(run.range, 0..4);
    assert_eq!(run.link.as_deref(), Some("https://daybrite.dev/docs"));
    assert_eq!(run.font.weight, Some(FontWeight::Bold));

    // The run follows the text.
    title.set("Documentation".into());
    flush_sync();
    let found = labels(&probe);
    assert_eq!(found[0].text, "Documentation");
    assert_eq!(found[0].runs[0].range, 0..13);

    // Native activation opens the target; a handler replaces the opening.
    probe.emit(
        NodeId(found[0].node),
        Event::LinkActivated("https://daybrite.dev/docs".into()),
    );
    probe.emit(
        NodeId(found[1].node),
        Event::LinkActivated("app:thing".into()),
    );
    flush_sync();
    assert!(
        probe
            .log()
            .iter()
            .any(|op| op == "open_url https://daybrite.dev/docs"),
        "{:?}",
        probe.log()
    );
    assert_eq!(*handled.borrow(), vec!["app:thing".to_owned()]);
}

#[test]
fn a_disabled_group_keeps_its_links_shut() {
    let on = Signal::new(false);
    let probe = boot_with(
        |p| p.set_text_links(true),
        move || {
            column((
                link("Docs", "https://daybrite.dev/docs"),
                markdown("[Guide](https://daybrite.dev/guide)"),
            ))
            .enabled(on)
            .any()
        },
    );
    let opened = |probe: &MockProbe| {
        probe
            .log()
            .iter()
            .filter(|op| op.starts_with("open_url "))
            .count()
    };
    let found = labels(&probe);
    assert_eq!(found.len(), 2);
    let activate = |probe: &MockProbe| {
        probe.emit(
            NodeId(found[0].node),
            Event::LinkActivated("https://daybrite.dev/docs".into()),
        );
        probe.emit(
            NodeId(found[1].node),
            Event::LinkActivated("https://daybrite.dev/guide".into()),
        );
        flush_sync();
    };
    // Disabled: the native activation of either link is dropped.
    activate(&probe);
    assert_eq!(opened(&probe), 0, "{:?}", probe.log());
    // Enabled again: both open.
    on.set(true);
    flush_sync();
    activate(&probe);
    assert_eq!(opened(&probe), 2, "{:?}", probe.log());
}

#[test]
fn a_link_without_link_runs_is_tappable_text() {
    let probe = boot(|| link("Docs", "https://daybrite.dev/docs").any());
    let found = labels(&probe);
    assert_eq!(found[0].runs, [] as [TextRun; 0]);
    probe.emit(NodeId(found[0].node), Event::Tap(Point::new(1.0, 1.0)));
    flush_sync();
    assert!(
        probe
            .log()
            .iter()
            .any(|op| op == "open_url https://daybrite.dev/docs")
    );
}

#[test]
fn a_selection_report_carries_the_displayed_text() {
    let seen = Rc::new(RefCell::new(Vec::<SelectedText>::new()));
    let sink = seen.clone();
    let probe = boot(move || {
        label("**Bold** déjà vu")
            .markdown()
            .on_selection(move |s| sink.borrow_mut().push(s.clone()))
            .any()
    });
    let label = &labels(&probe)[0];
    assert!(
        label.selectable,
        "asking for reports makes the label selectable"
    );
    assert!(label.reports_selection);
    assert_eq!(label.text, "Bold déjà vu");

    // Bytes 5..11 of the displayed text are "déjà" (two two-byte letters).
    probe.emit(
        NodeId(label.node),
        Event::TextSelected { start: 5, end: 11 },
    );
    probe.emit(NodeId(label.node), Event::TextSelected { start: 3, end: 3 });
    flush_sync();
    let seen = seen.borrow();
    assert_eq!(seen[0].text, "déjà");
    assert_eq!(seen[0].range, 5..11);
    assert!(seen[1].is_empty());
}

#[test]
fn a_text_scale_cap_holds_the_label_and_follows_the_setting() {
    let probe = boot(|| {
        column((
            label("capped").max_text_scale(1.3),
            label("free"),
            label(|| format!("{:.1}", day_core::text_scale())),
        ))
        .any()
    });
    let scale_of = |probe: &MockProbe, i: usize| labels(probe)[i].font.map_or(1.0, |f| f.scale);
    assert!((scale_of(&probe, 0) - 1.0).abs() < 1e-9);
    assert_eq!(labels(&probe)[2].text, "1.0");

    // The reader doubles the text size: the capped label is scaled down to 1.3 of its
    // default, the other is left to the toolkit, and the tracked reader runs again.
    probe.set_text_scale(2.0);
    day_core::note_text_size_changed();
    flush_sync();
    assert!(
        (scale_of(&probe, 0) - 0.65).abs() < 1e-9,
        "{}",
        scale_of(&probe, 0)
    );
    assert!((scale_of(&probe, 1) - 1.0).abs() < 1e-9);
    assert_eq!(labels(&probe)[2].text, "2.0");

    // Below the cap nothing is held back.
    probe.set_text_scale(1.2);
    day_core::note_text_size_changed();
    flush_sync();
    assert!((scale_of(&probe, 0) - 1.0).abs() < 1e-9);
}

#[test]
fn a_direction_override_mirrors_placement_and_tells_each_widget() {
    let shown = Signal::new(false);
    let probe = boot(move || {
        column((
            row((label("a"), label("bb")))
                .width(100.0)
                .layout_direction(LayoutDirection::Rtl),
            row((
                label("c"),
                label("dd"),
                when(move || shown.get(), || label("late")),
            ))
            .width(100.0),
        ))
        .align(HAlign::Leading)
        .any()
    });
    let found = labels(&probe);
    // Mirrored: "a" (8 wide) ends at the row's right edge, "bb" sits to its left.
    assert!(
        (found[0].frame.origin.x - 92.0).abs() < 0.01,
        "{:?}",
        found[0].frame
    );
    assert!(
        (found[1].frame.origin.x - 76.0).abs() < 0.01,
        "{:?}",
        found[1].frame
    );
    assert_eq!(found[0].direction, Some(LayoutDirection::Rtl));
    assert_eq!(found[1].direction, Some(LayoutDirection::Rtl));
    // The sibling row keeps the app's direction and its widgets are never told otherwise.
    assert!(found[2].frame.origin.x.abs() < 0.01);
    assert_eq!(found[2].direction, None);

    // A widget that joins later outside the subtree is not told either.
    shown.set(true);
    flush_sync();
    let late = labels(&probe)
        .into_iter()
        .find(|w| w.text == "late")
        .expect("the late label");
    assert_eq!(late.direction, None);
}

#[test]
fn a_widget_added_to_an_overridden_subtree_is_told_on_its_first_layout() {
    let shown = Signal::new(false);
    let probe = boot(move || {
        column((label("first"), when(move || shown.get(), || label("late"))))
            .layout_direction(LayoutDirection::Rtl)
            .any()
    });
    shown.set(true);
    flush_sync();
    let late = labels(&probe)
        .into_iter()
        .find(|w| w.text == "late")
        .expect("the late label");
    assert_eq!(late.direction, Some(LayoutDirection::Rtl));
}

#[test]
fn the_markdown_view_patches_the_block_being_written() {
    let reply = Signal::new("# Title\n\nFirst paragraph.\n\nSecond".to_owned());
    let probe = boot(move || markdown(reply).width(300.0).any());
    let texts =
        |probe: &MockProbe| -> Vec<String> { labels(probe).into_iter().map(|w| w.text).collect() };
    assert_eq!(texts(&probe), ["Title", "First paragraph.", "Second"]);

    // A token appended to the last paragraph: one label patch, nothing realized.
    probe.clear_log();
    reply.update(|r| r.push_str(" *one*"));
    flush_sync();
    assert_eq!(texts(&probe), ["Title", "First paragraph.", "Second one"]);
    assert_eq!(realized(&probe), 0, "{:?}", probe.log());
    let updates: Vec<String> = probe
        .log()
        .into_iter()
        .filter(|op| op.starts_with("update"))
        .collect();
    assert_eq!(updates.len(), 1, "one patch for one token: {updates:?}");

    // A new block realizes its own row and leaves the others alone.
    probe.clear_log();
    reply.update(|r| r.push_str("\n\n- item"));
    flush_sync();
    assert_eq!(
        texts(&probe),
        [
            "Title",
            "First paragraph.",
            "Second one",
            "\u{2022}",
            "item"
        ]
    );
    assert!(
        !probe.log().iter().any(|op| op.starts_with("update")),
        "no existing label changed: {:?}",
        probe.log()
    );

    // A shorter text removes the rows it no longer has.
    reply.set("# Title".into());
    flush_sync();
    assert_eq!(texts(&probe), ["Title"]);
}

#[test]
fn the_markdown_view_lays_blocks_out_by_depth() {
    let probe = boot(|| {
        markdown("Para\n\n- item\n\n  more\n\n> quote\n\n```\ncode\n```")
            .width(300.0)
            .any()
    });
    let found = labels(&probe);
    let by_text = |t: &str| {
        found
            .iter()
            .find(|w| w.text == t)
            .unwrap_or_else(|| panic!("no label {t:?}"))
            .clone()
    };
    let para = by_text("Para");
    let item = by_text("item");
    let more = by_text("more");
    let quote = by_text("quote");
    let code = by_text("code");
    // A list item's text sits one indent in, and its second paragraph lines up under it.
    assert!(
        item.frame.origin.x > para.frame.origin.x + 20.0,
        "{:?}",
        item.frame
    );
    assert!((more.frame.origin.x - item.frame.origin.x).abs() < 0.01);
    // A quote sits beside its bar. (Frames are in each row's own coordinates, so only the
    // horizontal offsets compare across rows.)
    assert!(quote.frame.origin.x > para.frame.origin.x);
    assert!(code.font.is_some_and(|f| f.monospace));
    assert_eq!(probe.find_by_kind("day.divider").len(), 1, "one quote bar");
}
