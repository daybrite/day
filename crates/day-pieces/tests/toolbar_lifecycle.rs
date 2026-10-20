// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

use day_mock::MockToolkit;
use day_pieces::prelude::*;
use day_reactive::flush_sync;
use day_spec::{ToolbarItemKind, ToolbarValue, WindowOptions};
use std::sync::atomic::{AtomicUsize, Ordering};

static CONTAINED_PANICS: AtomicUsize = AtomicUsize::new(0);

#[test]
fn derived_toolbar_bindings_leave_with_their_view() {
    // Mock UI dispatch contains panics just like native dispatch. A contained panic
    // must fail this regression too, even when the next operation can still run.
    day_core::set_contained_panic_observer(|| {
        CONTAINED_PANICS.fetch_add(1, Ordering::SeqCst);
    });
    let visible = Signal::new(true);
    let title = Signal::new("Original".to_string());
    let on = Signal::new(false);
    let selected = Signal::new(0usize);
    let (mock, probe) = MockToolkit::new();
    day_core::launch_with(mock, WindowOptions::default(), move || {
        when(
            move || visible.get(),
            move || {
                label("editor").toolbar(move || {
                    vec![
                        toolbar_toggle(day_core::id!(toggle), title.get(), on),
                        toolbar_segmented(
                            day_core::id!(mode),
                            vec![segment("One"), segment("Two")],
                            selected,
                        ),
                    ]
                })
            },
        )
        .any()
    });

    // Rebuilding presentation must preserve working value bindings and dispatch slots.
    title.set("Rebuilt".into());
    flush_sync();
    let toggle = day_core::toolbar::toolbar_model()
        .into_iter()
        .find(|item| item.id == "toggle")
        .unwrap();
    assert_eq!(toggle.label, "Rebuilt");
    day_core::toolbar::dispatch_toolbar_value(toggle.action, &ToolbarValue::On(true));
    flush_sync();
    assert!(on.get_untracked());
    assert_eq!(
        probe
            .toolbar_items()
            .iter()
            .find(|i| i.id == "toggle")
            .unwrap()
            .kind,
        ToolbarItemKind::Toggle { on: true }
    );

    visible.set(false);
    flush_sync();
    assert!(day_core::toolbar::toolbar_model().is_empty());
    let patches = probe.toolbar_patches().len();

    // Day-Sketch replaces its editor on document changes. Old bindings must neither
    // read disposed AnyBinding slots nor continue patching the now absent toolbar.
    on.set(false);
    selected.set(1);
    title.set("Remounted".into());
    flush_sync();
    assert_eq!(probe.toolbar_patches().len(), patches);
    day_core::toolbar::dispatch_toolbar_value(toggle.action, &ToolbarValue::On(true));
    flush_sync();
    assert!(
        !on.get_untracked(),
        "removed toolbar actions must be retired"
    );

    visible.set(true);
    flush_sync();
    let items = day_core::toolbar::toolbar_model();
    assert_eq!(
        items.iter().find(|i| i.id == "toggle").unwrap().label,
        "Remounted"
    );
    assert_eq!(
        items.iter().find(|i| i.id == "toggle").unwrap().kind,
        ToolbarItemKind::Toggle { on: false }
    );
    // Also retire a view before its first toolbar rebuild: its initial AnyBinding
    // slots were allocated while entering the view, rather than during a later flush.
    visible.set(false);
    flush_sync();
    on.set(true);
    selected.set(0);
    flush_sync();
    day_core::uninstall_tree();
    assert_eq!(CONTAINED_PANICS.load(Ordering::SeqCst), 0);
}
