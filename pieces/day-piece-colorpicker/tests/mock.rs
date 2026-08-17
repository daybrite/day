// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Mock e2e for the color picker (the day-pieces mock_e2e pattern), across both idioms.
//!
//! NATIVE: the piece realizes its kind, a native pick (`Event::Custom` carrying the component
//! form) and dayscript's `input:` step (`Event::TextChanged` carrying hex) both drive the bound
//! signal, an app write patches through to the well, and `.alpha(false)` refuses to let a picker
//! clear the app's opacity.
//!
//! COMPOSED: the panel realizes no native kind at all — it is ordinary pieces — so what these
//! assert instead is that the well is a real control, that the panel mounts on press and its
//! canvases and buttons come with it, and that Cancel puts back the color the panel opened on.

use std::cell::Cell;
use std::rc::Rc;

use day_core::AnyPiece;
use day_mock::{MockProbe, MockToolkit};
use day_piece_colorpicker::{KIND, PICK_TAG, color_picker};
use day_pieces::prelude::*;
use day_reactive::{Signal, flush_sync};
use day_spec::{Color, Event, NodeId, Size, WindowOptions};

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

fn picker_node(probe: &MockProbe) -> NodeId {
    let found = probe.find_by_kind(KIND);
    assert_eq!(found.len(), 1, "exactly one {KIND} realized");
    NodeId(found[0].1.node)
}

/// Boot a picker over a signal the test can keep driving.
fn with_picker(initial: Color, alpha: bool) -> (MockProbe, Signal<Color>, NodeId) {
    let cell: Rc<Cell<Option<Signal<Color>>>> = Rc::new(Cell::new(None));
    let cell2 = cell.clone();
    let probe = boot(move || {
        let color = Signal::new(initial);
        cell2.set(Some(color));
        color_picker(color).native().alpha(alpha).any()
    });
    let color = cell.get().unwrap();
    let node = picker_node(&probe);
    (probe, color, node)
}

#[test]
fn picks_drive_the_signal() {
    let (probe, color, node) = with_picker(Color::hex(0xE86A3C), true);

    // A native pick: the lossless component form every arm reports.
    probe.emit(node, Event::custom(PICK_TAG, "0.25 0.5 0.75 1"));
    flush_sync();
    assert_eq!(color.get_untracked(), Color::rgba(0.25, 0.5, 0.75, 1.0));

    // Across a JNI / C-ABI boundary the tag cannot cross, so only the payload arrives.
    probe.emit(
        node,
        Event::Custom {
            tag: "",
            num: 0.0,
            text: "0 1 0 0.5".into(),
        },
    );
    flush_sync();
    assert_eq!(color.get_untracked(), Color::rgba(0.0, 1.0, 0.0, 0.5));

    // dayscript's `input:` step → TextChanged, where a human types hex.
    probe.emit(node, Event::TextChanged("#2f6fde".into()));
    flush_sync();
    assert_eq!(color.get_untracked(), Color::hex(0x2F6FDE));

    // Garbage is ignored rather than resetting the color to black.
    probe.emit(node, Event::TextChanged("chartreuse".into()));
    flush_sync();
    assert_eq!(color.get_untracked(), Color::hex(0x2F6FDE));
}

#[test]
fn app_writes_patch_native() {
    let (probe, color, _node) = with_picker(Color::hex(0xE86A3C), false);
    let mark = probe.log_len();
    color.set(Color::hex(0x1E9E86)); // app-initiated (e.g. a "reset to brand" button)
    flush_sync();
    assert!(
        probe
            .log_since(mark)
            .iter()
            .any(|l| l.starts_with("update day.piece.colorpicker")),
        "a signal write patches the native well"
    );
}

#[test]
fn opaque_picker_cannot_clear_alpha() {
    // `.alpha(false)` is the default, and the promise it makes is that the bound color stays
    // opaque no matter what a backend reports — an arm whose native control has a stray alpha
    // channel must not be able to make an app's brand color half-transparent.
    let (probe, color, node) = with_picker(Color::hex(0xE86A3C), false);
    probe.emit(node, Event::custom(PICK_TAG, "0.2 0.4 0.6 0.25"));
    flush_sync();
    assert_eq!(color.get_untracked(), Color::rgba(0.2, 0.4, 0.6, 1.0));
}

// --- the composed idiom -----------------------------------------------------

/// Boot a composed picker and hand back its well's handle (for reading what it draws) and node
/// (for pressing it). Closed, the picker is exactly one canvas — the drawn swatch.
fn with_composed(
    initial: Color,
    alpha: bool,
) -> (MockProbe, Signal<Color>, day_mock::MockHandle, NodeId) {
    let cell: Rc<Cell<Option<Signal<Color>>>> = Rc::new(Cell::new(None));
    let cell2 = cell.clone();
    let probe = boot(move || {
        let color = Signal::new(initial);
        cell2.set(Some(color));
        color_picker(color).composed().alpha(alpha).any()
    });
    let color = cell.get().unwrap();
    let wells = probe.find_by_kind("day.canvas");
    assert_eq!(wells.len(), 1, "the closed picker is just its drawn well");
    (probe, color, wells[0].0, NodeId(wells[0].1.node))
}

/// The hex caption the well draws, read out of its display list.
fn well_caption(probe: &MockProbe, handle: day_mock::MockHandle) -> String {
    probe
        .widget(handle)
        .ops
        .iter()
        .find_map(|op| match op {
            day_spec::DrawOp::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .expect("the well draws its hex")
}

/// Press the well (a drawn canvas takes a tap, not a button press).
fn press_well(probe: &MockProbe, well: NodeId) {
    probe.emit(well, Event::Tap(Point::ZERO));
    flush_sync();
}

/// Press a button by its caption (the panel's Cancel / Done).
fn press_labeled(probe: &MockProbe, text: &str) {
    let buttons = probe.find_by_kind("day.button");
    let target = buttons
        .iter()
        .find(|(_, w)| w.text == text)
        .unwrap_or_else(|| panic!("no button captioned {text:?}"));
    probe.emit(NodeId(target.1.node), Event::Pressed);
    flush_sync();
    // The mock records the dismiss patch but leaves the transition to the test, exactly as a
    // native surface would report it (docs/cover.md): content lives until `CoverHidden`.
    for (_, w) in probe.find_by_kind("day.cover") {
        probe.emit(NodeId(w.node), Event::CoverHidden);
    }
    flush_sync();
}

#[test]
fn composed_realizes_no_native_kind() {
    let (probe, _color, _handle, _well) = with_composed(Color::hex(0xE86A3C), false);
    assert!(
        probe.find_by_kind(KIND).is_empty(),
        "the composed idiom is ordinary pieces — it must not realize the native leaf, or a \
         toolkit with no renderer for it would draw a placeholder behind the panel"
    );
}

#[test]
fn well_shows_the_bound_color_and_follows_it() {
    let (probe, color, handle, _well) = with_composed(Color::hex(0xE86A3C), false);
    assert_eq!(well_caption(&probe, handle), "#e86a3c");
    color.set(Color::hex(0x1E9E86));
    flush_sync();
    assert_eq!(
        well_caption(&probe, handle),
        "#1e9e86",
        "the well re-records when the bound color moves — it is not a build-time snapshot"
    );
}

#[test]
fn pressing_the_well_mounts_the_panel() {
    let (probe, _color, _handle, well) = with_composed(Color::hex(0xE86A3C), true);
    assert!(
        probe.find_by_kind("day.button").is_empty(),
        "nothing but the well before the panel opens"
    );
    press_well(&probe, well);
    // The well, the shade field, the hue strip, the opacity strip, the readout swatch, and one
    // canvas per preset.
    let canvases = probe.find_by_kind("day.canvas").len();
    assert!(
        canvases >= 5,
        "the panel's drawn controls mount with it; found {canvases} canvases"
    );
    assert_eq!(
        probe.find_by_kind("day.button").len(),
        2,
        "Cancel + Done (everything else in the panel is drawn)"
    );
}

#[test]
fn cancel_restores_the_color_the_panel_opened_on() {
    let start = Color::hex(0xE86A3C);
    let (probe, color, _handle, well) = with_composed(start, false);
    press_well(&probe, well);

    // Press the top-leading corner of the shade field: saturation 0, brightness 1 — white,
    // whatever the hue is. That is a change no rounding can mistake for the starting color, and
    // it only lands because `on_tap_at` reports WHERE the press was. The field is the SECOND
    // canvas; the first is the well itself.
    let shade = probe.find_by_kind("day.canvas")[1].1.node;
    probe.emit(NodeId(shade), Event::Tap(Point::new(0.0, 0.0)));
    flush_sync();
    assert_eq!(
        color.get_untracked(),
        Color::WHITE,
        "a press in the shade field's white corner drives the bound color live"
    );

    press_labeled(&probe, &day_l10n::t("day-cancel"));
    assert_eq!(
        color.get_untracked(),
        start,
        "Cancel puts back the color the panel opened on"
    );
    assert_eq!(
        probe.find_by_kind("day.canvas").len(),
        1,
        "and dismisses the panel, leaving only the well"
    );
}

#[test]
fn done_keeps_the_pick() {
    let (probe, color, _handle, well) = with_composed(Color::hex(0xE86A3C), false);
    press_well(&probe, well);
    // The hue strip is the THIRD canvas (well, shade field, hue): press partway along it, which
    // moves the hue while keeping the field's saturation and brightness.
    let hue = probe.find_by_kind("day.canvas")[2].1.node;
    probe.emit(NodeId(hue), Event::Tap(Point::new(90.0, 10.0)));
    flush_sync();
    let picked = color.get_untracked();
    assert_ne!(picked, Color::hex(0xE86A3C), "the hue moved");

    press_labeled(&probe, &day_l10n::t("day-done"));
    assert_eq!(color.get_untracked(), picked, "Done keeps what was picked");
    assert_eq!(
        probe.find_by_kind("day.canvas").len(),
        1,
        "and dismisses the panel, leaving only the well"
    );
}

#[test]
fn readout_label_follows_picks() {
    // The showcase pattern: a hex readout bound to the same signal, which is what the
    // walkthrough asserts cross-platform.
    let probe = boot(|| {
        let color = Signal::new(Color::hex(0xE86A3C));
        column((
            color_picker(color).native(),
            label(move || color.get().to_hex_string()).id("tint-value"),
        ))
        .any()
    });
    let node = picker_node(&probe);
    probe.emit(node, Event::TextChanged("#1e9e86".into()));
    flush_sync();
    let labels = probe.find_by_kind("day.label");
    assert!(
        labels.iter().any(|(_, w)| w.text == "#1e9e86"),
        "the readout shows the picked color; labels = {:?}",
        labels
            .iter()
            .map(|(_, w)| w.text.clone())
            .collect::<Vec<_>>()
    );
}
