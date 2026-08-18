// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Mock e2e for the styled-text editor (the day-pieces mock_e2e pattern).
//!
//! What these pin is the CONTRACT every native arm is written against, on the one backend where it
//! can be driven without a window: a character edit reported as plain text is diffed and reflowed
//! rather than re-sent, an attribute change patches WITHOUT replacing the text (so the caret and
//! the undo stack survive a live syntax highlighter), a selection report reaches the bound signal,
//! and a patch Day itself sent does not echo back into the document.

use std::cell::Cell;
use std::rc::Rc;

use day_core::AnyPiece;
use day_mock::{MockProbe, MockToolkit};
use day_piece_texteditor::{EditorPatch, KIND, selection_payload, text_editor};
use day_pieces::prelude::*;
use day_reactive::{Signal, flush_sync};
use day_spec::{Event, Font, NodeId, RunStyle, Size, StyledText, WindowOptions};

fn boot(root: impl FnOnce() -> AnyPiece + 'static) -> MockProbe {
    day_core::uninstall_tree();
    let (mock, probe) = MockToolkit::new();
    // The editor's patch is this crate's own type, so the mock needs to be told how to name it.
    probe.describe_patch::<EditorPatch>(|p| match p {
        EditorPatch::SetDocument(d) => format!("SetDocument {:?}", d.text),
        EditorPatch::SetAttributes(d) => {
            format!(
                "SetAttributes runs={} paragraphs={}",
                d.runs.len(),
                d.paragraphs.len()
            )
        }
        EditorPatch::SetSelection(r) => format!("SetSelection {r:?}"),
        EditorPatch::SetTypingStyle(s) => format!("SetTypingStyle bold={}", s.bold()),
        EditorPatch::SetEditable(e) => format!("SetEditable {e}"),
    });
    let options = WindowOptions {
        title: "test".into(),
        size: Size::new(500.0, 600.0),
        ..Default::default()
    };
    day_core::launch_with(mock, options, root);
    probe
}

/// "hello world", with "hello" bold.
fn seed() -> StyledText {
    let mut d = StyledText::plain("hello world");
    d.apply(0..5, Font::Body, |s| s.set_bold(true));
    d
}

struct Fixture {
    probe: MockProbe,
    doc: Signal<StyledText>,
    sel: Signal<std::ops::Range<usize>>,
    node: NodeId,
}

/// The signals a booted editor hands back, for the `Cell` the root closure passes them through.
type DocAndSelection = (Signal<StyledText>, Signal<std::ops::Range<usize>>);

fn boot_editor() -> Fixture {
    let cell: Rc<Cell<Option<DocAndSelection>>> = Rc::new(Cell::new(None));
    let cell2 = cell.clone();
    let probe = boot(move || {
        let doc = Signal::new(seed());
        let sel = Signal::new(0..0);
        cell2.set(Some((doc, sel)));
        text_editor(doc).selection(sel).any()
    });
    let (doc, sel) = cell.get().unwrap();
    let found = probe.find_by_kind(KIND);
    assert_eq!(found.len(), 1, "exactly one {KIND} realized");
    let node = NodeId(found[0].1.node);
    Fixture {
        probe,
        doc,
        sel,
        node,
    }
}

#[test]
fn a_keystroke_is_diffed_and_the_runs_reflow() {
    let f = boot_editor();
    // The native view reports only its new TEXT — no delta, no attributes. Everything else the
    // piece works out for itself, which is what makes this one path serve all eight arms.
    f.probe
        .emit(f.node, Event::TextChanged("hello there world".into()));
    flush_sync();
    let d = f.doc.get_untracked();
    assert_eq!(d.text, "hello there world");
    assert_eq!(
        d.runs[0].range,
        0..5,
        "the bold run kept its own characters"
    );
    assert!(d.validate().is_ok());
}

#[test]
fn typing_at_the_end_of_a_run_extends_it() {
    let f = boot_editor();
    f.probe
        .emit(f.node, Event::TextChanged("hellos world".into()));
    flush_sync();
    assert_eq!(
        f.doc.get_untracked().runs[0].range,
        0..6,
        "a character typed at a run's end joins it, as every editor does"
    );
}

#[test]
fn deleting_a_styled_word_drops_its_run() {
    let f = boot_editor();
    f.probe.emit(f.node, Event::TextChanged(" world".into()));
    flush_sync();
    let d = f.doc.get_untracked();
    assert_eq!(d.text, " world");
    assert!(d.runs.is_empty(), "{:?}", d.runs);
}

#[test]
fn an_app_write_that_only_changes_attributes_does_not_replace_the_text() {
    // The syntax-highlighting path: same characters, fresh runs, every keystroke. Sending a
    // document there would move the caret back to wherever the app last put it.
    let f = boot_editor();
    let mark = f.probe.log_len();
    f.doc
        .update(|d| d.apply(6..11, Font::Body, |s| s.set_italic(true)));
    flush_sync();
    let log = f.probe.log_since(mark);
    assert!(
        log.iter().any(|l| l.contains("SetAttributes")),
        "expected an attributes-only patch, got {log:?}"
    );
    assert!(
        !log.iter().any(|l| l.contains("SetDocument")),
        "the text did not change, so the document must not be replaced: {log:?}"
    );
}

#[test]
fn an_app_write_that_changes_the_text_replaces_the_document() {
    let f = boot_editor();
    let mark = f.probe.log_len();
    f.doc.update(|d| d.splice(5..5, ","));
    flush_sync();
    let log = f.probe.log_since(mark);
    assert!(
        log.iter().any(|l| l.contains("SetDocument")),
        "expected a document patch, got {log:?}"
    );
}

#[test]
fn the_echo_of_a_day_write_is_not_written_back() {
    // A backend that re-reports the text Day just set (AppKit's setString: does not, GTK's
    // set_text does) must not push it through the diff again — the document would be rebuilt
    // from a delta of nothing, losing every run.
    let f = boot_editor();
    f.doc.update(|d| d.splice(5..5, ","));
    flush_sync();
    let before = f.doc.get_untracked();
    f.probe
        .emit(f.node, Event::TextChanged("hello, world".into()));
    flush_sync();
    assert_eq!(f.doc.get_untracked(), before, "the echo changed nothing");
}

#[test]
fn a_selection_report_reaches_the_bound_signal() {
    let f = boot_editor();
    f.probe.emit(
        f.node,
        Event::custom("texteditor:sel", selection_payload(2, 7)),
    );
    flush_sync();
    assert_eq!(f.sel.get_untracked(), 2..7);

    // Across a JNI / C-ABI / JS boundary the tag cannot cross, so the payload alone identifies it.
    f.probe.emit(
        f.node,
        Event::Custom {
            tag: "",
            num: 0.0,
            text: selection_payload(9, 3),
        },
    );
    flush_sync();
    assert_eq!(
        f.sel.get_untracked(),
        3..9,
        "a backwards drag comes back ordered"
    );
}

#[test]
fn a_selection_the_view_reported_is_not_written_back() {
    // The drag bug: a `selectionchange` fires on every mouse-move, and each report used to come
    // straight back as a `SetSelection`. Re-anchoring a selection mid-drag collapses it — in
    // Safari the caret jumps around and a mouse selection is impossible to make at all.
    let f = boot_editor();
    let mark = f.probe.log_len();
    for (a, b) in [(4usize, 4usize), (4, 7), (4, 9), (4, 11)] {
        f.probe.emit(
            f.node,
            Event::custom("texteditor:sel", selection_payload(a, b)),
        );
        flush_sync();
    }
    let log = f.probe.log_since(mark);
    assert!(
        !log.iter().any(|l| l.contains("SetSelection")),
        "a reported selection must not be patched back into the view it came from: {log:?}"
    );
    assert_eq!(
        f.sel.get_untracked(),
        4..11,
        "and it still reaches the signal"
    );
}

#[test]
fn an_app_write_still_moves_the_caret() {
    // The other half: the guard must not swallow a selection the APP set, which is what a
    // find-and-select or a "select all" does.
    let f = boot_editor();
    f.probe.emit(
        f.node,
        Event::custom("texteditor:sel", selection_payload(2, 5)),
    );
    flush_sync();
    let mark = f.probe.log_len();
    f.sel.set(0..11);
    flush_sync();
    let log = f.probe.log_since(mark);
    assert!(
        log.iter().any(|l| l.contains("SetSelection 0..11")),
        "expected the app's write to patch through, got {log:?}"
    );
}

#[test]
fn restyling_leaves_the_selection_on_the_same_characters() {
    // A restyle must not move the selection. The piece's half of that is simply not to patch one:
    // an arm that rebuilds its view to apply attributes (the web's does) restores the selection
    // itself, and a `SetSelection` on top of that would fight it.
    let f = boot_editor();
    f.probe.emit(
        f.node,
        Event::custom("texteditor:sel", selection_payload(6, 11)),
    );
    flush_sync();
    let mark = f.probe.log_len();
    f.doc
        .update(|d| d.apply(6..11, Font::Body, |s| s.set_bold(true)));
    flush_sync();
    assert_eq!(
        f.sel.get_untracked(),
        6..11,
        "the selection still covers the same characters"
    );
    let log = f.probe.log_since(mark);
    assert!(
        log.iter().any(|l| l.contains("SetAttributes")),
        "the restyle did patch: {log:?}"
    );
    assert!(
        !log.iter().any(|l| l.contains("SetSelection")),
        "and it moved no selection: {log:?}"
    );
}

#[test]
fn an_unchanged_typing_style_does_not_patch() {
    // Also per mouse-move: reading the caret's style back writes the typing signal, and an
    // unchanged value used to re-patch the native typing attributes every time.
    let (probe, _doc, _typing, _sel, node) = boot_with_typing();
    probe.emit(
        node,
        Event::custom("texteditor:sel", selection_payload(7, 7)),
    );
    flush_sync();
    let mark = probe.log_len();
    for at in [7usize, 8, 9] {
        probe.emit(
            node,
            Event::custom("texteditor:sel", selection_payload(at, at)),
        );
        flush_sync();
    }
    let log = probe.log_since(mark);
    assert!(
        !log.iter().any(|l| l.contains("SetTypingStyle")),
        "the style did not change across those carets: {log:?}"
    );
}

#[test]
fn the_toolbar_round_trip_is_pure_rust() {
    // What every arm's toolbar does, with no backend involved: read the selection's style, flip
    // it, write it back. This is the whole reason `style_of`/`apply` live in day-spec.
    let f = boot_editor();
    f.sel.set(0..11);
    flush_sync();
    let mixed = f.doc.get_untracked().style_of(0..11, Font::Body);
    assert!(
        !mixed.bold(),
        "half the selection is bold, so it reads mixed"
    );

    f.doc
        .update(|d| d.apply(0..11, Font::Body, |s| s.set_bold(true)));
    flush_sync();
    assert!(
        f.doc.get_untracked().style_of(0..11, Font::Body).bold(),
        "pressing the button once bolds all of it"
    );
}

#[test]
fn a_typing_style_patches_without_touching_the_document() {
    let cell: Rc<Cell<Option<Signal<RunStyle>>>> = Rc::new(Cell::new(None));
    let cell2 = cell.clone();
    let probe = boot(move || {
        let doc = Signal::new(seed());
        let typing = Signal::new(RunStyle::plain(Font::Body));
        cell2.set(Some(typing));
        text_editor(doc).typing_style(typing).any()
    });
    let typing = cell.get().unwrap();
    let mark = probe.log_len();
    typing.update(|s| s.set_bold(true));
    flush_sync();
    let log = probe.log_since(mark);
    assert!(
        log.iter().any(|l| l.contains("SetTypingStyle")),
        "got {log:?}"
    );
}

/// Boot an editor with all three bindings, for the typing-style contract.
fn boot_with_typing() -> (
    MockProbe,
    Signal<StyledText>,
    Signal<RunStyle>,
    Signal<std::ops::Range<usize>>,
    NodeId,
) {
    type Bound = (
        Signal<StyledText>,
        Signal<RunStyle>,
        Signal<std::ops::Range<usize>>,
    );
    let cell: Rc<Cell<Option<Bound>>> = Rc::new(Cell::new(None));
    let cell2 = cell.clone();
    let probe = boot(move || {
        let doc = Signal::new(seed());
        let typing = Signal::new(RunStyle::plain(Font::Body));
        let sel = Signal::new(0..0);
        cell2.set(Some((doc, typing, sel)));
        text_editor(doc).typing_style(typing).selection(sel).any()
    });
    let (doc, typing, sel) = cell.get().unwrap();
    let node = NodeId(probe.find_by_kind(KIND)[0].1.node);
    (probe, doc, typing, sel, node)
}

#[test]
fn a_pending_typing_style_styles_the_characters_it_was_set_for() {
    // Without this, the typing style is cosmetic for exactly one frame: the native view styles the
    // keystroke from its own typing attributes, and Day's next attribute patch repaints it from a
    // model that never heard about the style.
    let (probe, doc, typing, _sel, node) = boot_with_typing();
    typing.update(|s| s.set_italic(true));
    flush_sync();
    probe.emit(node, Event::TextChanged("hello world!".into()));
    flush_sync();
    let d = doc.get_untracked();
    assert!(
        d.style_of(11..12, Font::Body).italic(),
        "the typed character took the pending style: {:?}",
        d.runs
    );
    assert!(
        !d.style_of(6..11, Font::Body).italic(),
        "and nothing around it did"
    );
}

#[test]
fn moving_the_caret_reads_the_style_back_into_the_typing_signal() {
    // The other half of the two-way binding: a toolbar bound to the typing style shows the state
    // of the text the caret sits in, so pressing B once turns bold OFF inside a bold word.
    let (probe, _doc, typing, _sel, node) = boot_with_typing();
    probe.emit(
        node,
        Event::custom("texteditor:sel", selection_payload(2, 2)),
    );
    flush_sync();
    assert!(typing.get_untracked().bold(), "the caret is inside 'hello'");

    probe.emit(
        node,
        Event::custom("texteditor:sel", selection_payload(8, 8)),
    );
    flush_sync();
    assert!(!typing.get_untracked().bold(), "and now inside 'world'");
}

#[test]
fn with_no_typing_style_bound_typed_text_inherits() {
    let f = boot_editor();
    f.probe
        .emit(f.node, Event::TextChanged("helloX world".into()));
    flush_sync();
    assert!(
        f.doc.get_untracked().style_of(5..6, Font::Body).bold(),
        "typing at the end of a bold word stays bold"
    );
}

#[test]
fn a_document_with_paragraph_attributes_realizes_and_stays_valid() {
    let probe = boot(|| {
        let mut d = StyledText::markdown("# Title\n- one\n- two", Font::Body);
        d.apply_paragraph(0..1, |p| p.align = day_spec::ParagraphAlign::Center);
        assert!(d.validate().is_ok());
        text_editor(Signal::new(d)).any()
    });
    assert_eq!(probe.find_by_kind(KIND).len(), 1);
}
