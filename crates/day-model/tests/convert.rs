// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Conversions: the store's shape and the control's type need not agree.

use day_core::AnyPiece;
use day_macros::Observable;
use day_mock::{MockProbe, MockToolkit};
use day_model::{Keyed, Store};
use day_pieces::prelude::*;
use day_reactive::flush_sync;
use day_spec::{Event, NodeId, Size, WindowOptions};

#[derive(Observable, Clone, Default, PartialEq, Debug)]
pub struct Item {
    #[obs(key)]
    pub id: u32,
    /// `#RRGGBB` in the store, a `Color` in the UI.
    pub color: String,
    /// Minutes in the store, an index into `SNOOZES` in the picker.
    pub snooze: i64,
}

const SNOOZES: [i64; 5] = [5, 10, 15, 20, 30];

fn boot(root: impl FnOnce() -> AnyPiece + 'static) -> MockProbe {
    day_core::uninstall_tree();
    let (mock, probe) = MockToolkit::new();
    day_core::launch_with(
        mock,
        WindowOptions {
            title: "t".into(),
            size: Size::new(400.0, 400.0),
            ..Default::default()
        },
        root,
    );
    probe
}

#[test]
fn a_hex_string_binds_as_a_color() {
    let store = Store::new(Keyed::new(vec![Item {
        id: 1,
        color: "#3B82F6".into(),
        snooze: 10,
    }]));
    boot(move || {
        let color = store.elem(1).color().map(
            |s| Color::hex(u32::from_str_radix(s.trim_start_matches('#'), 16).unwrap_or(0)),
            |c| {
                format!(
                    "#{:02X}{:02X}{:02X}",
                    (c.r * 255.0).round() as u8,
                    (c.g * 255.0).round() as u8,
                    (c.b * 255.0).round() as u8
                )
            },
        );
        assert_eq!(color.peek(), Color::hex(0x3B82F6));
        color.write(Color::hex(0x10B981));
        label("x").any()
    });
    flush_sync();
    assert_eq!(
        store.with_untracked(|k| k.get(1).unwrap().color.clone()),
        "#10B981"
    );
}

#[test]
fn a_lookup_table_binds_as_a_picker_index() {
    let store = Store::new(Keyed::new(vec![Item {
        id: 1,
        color: String::new(),
        snooze: 15,
    }]));
    let probe = boot(move || {
        let idx = store.elem(1).snooze().map(
            |m| SNOOZES.iter().position(|s| s == m).unwrap_or(0),
            |i| SNOOZES[(*i).min(SNOOZES.len() - 1)],
        );
        // A slider stands in for `picker`, which still takes a concrete Signal today.
        slider(idx.map_to_f64()).range(0.0..=4.0).any()
    });
    let h = probe.find_by_kind("day.slider")[0].0;
    assert_eq!(probe.widget(h).value, 2.0, "15 minutes is index 2");
    let node = NodeId(probe.find_by_kind("day.slider")[0].1.node);
    probe.emit(node, Event::ValueChanged(4.0));
    flush_sync();
    assert_eq!(store.with_untracked(|k| k.get(1).unwrap().snooze), 30);
}
