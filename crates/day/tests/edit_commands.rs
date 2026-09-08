// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! `install_edit_commands` (docs/menus.md): the app-level Cut/Copy/Paste wiring, booted on the
//! mock toolkit so the bridge has a tree to install into.

use std::cell::RefCell;
use std::rc::Rc;

use day_mock::MockToolkit;
use day_pieces::prelude::*;
use day_spec::{EditOp, Size, WindowOptions};

fn boot() {
    day_core::uninstall_tree();
    let (mock, _probe) = MockToolkit::new();
    day_core::launch_with(
        mock,
        WindowOptions {
            title: "test".into(),
            size: Size::new(400.0, 600.0),
            ..Default::default()
        },
        || label("main").any(),
    );
}

/// An app's OWN Copy ▸ Paste round-trips even when the platform hands the payload back.
/// Reading the clipboard is a privilege Android grants only to the app holding input focus
/// (docs/clipboard.md), and writing to it raises a system overlay that can take that focus —
/// so a Paste moments after a Copy can be refused the very payload the app just wrote. An
/// emptied clipboard is what that refusal looks like from here: `get_text` answers `None`
/// either way.
#[test]
fn own_copy_then_paste_round_trips_when_the_platform_read_answers_nothing() {
    // Be polite: the transport is the user's real system clipboard on a desktop host.
    let previous = day_part_clipboard::get_text();
    boot();
    let pasted: Rc<RefCell<Vec<String>>> = Rc::default();
    let seen = pasted.clone();
    day::install_edit_commands(
        || true,
        || Some("<svg>copied</svg>".into()),
        || Some("<svg>cut</svg>".into()),
        move |text| seen.borrow_mut().push(text.into()),
        || {},
    );
    day_reactive::flush_sync();

    assert!(day::invoke_edit(EditOp::Copy));
    day_part_clipboard::set_text("");
    assert!(
        day_part_clipboard::get_text().is_none_or(|t| t.is_empty()),
        "the emptied clipboard still reads back — this test is not exercising the refusal"
    );
    assert!(day::invoke_edit(EditOp::Paste));
    assert_eq!(
        pasted.borrow().as_slice(),
        ["<svg>copied</svg>".to_string()],
        "Paste dropped the payload this app had just copied"
    );

    // Cut replaces what the fallback holds, so it is never a stale payload.
    assert!(day::invoke_edit(EditOp::Cut));
    day_part_clipboard::set_text("");
    assert!(day::invoke_edit(EditOp::Paste));
    assert_eq!(pasted.borrow().len(), 2);
    assert_eq!(pasted.borrow()[1], "<svg>cut</svg>");

    if let Some(prev) = previous {
        day_part_clipboard::set_text(&prev);
    }
}
