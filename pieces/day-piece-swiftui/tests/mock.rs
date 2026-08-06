//! Mock e2e for the swiftui piece's front-end plumbing. The mock toolkit registers no renderer for
//! `day.piece.swiftui` (it realizes as an extension leaf), so these tests cover exactly what the
//! front-end owns: the leaf mounts with its props, a reactive params source patches the node, a
//! constant one never does, and `support()` reports Unsupported off the Apple backends.

use std::cell::Cell;
use std::rc::Rc;

use day_core::AnyPiece;
use day_mock::{MockProbe, MockToolkit};
use day_piece_swiftui::{support, swiftui};
use day_pieces::prelude::*;
use day_reactive::{Signal, flush_sync};
use day_spec::{Size, Support, WindowOptions};

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
fn mounts_one_swiftui_leaf() {
    let probe = boot(|| swiftui("Mod.View").any());
    let leaves = probe.find_by_kind("day.piece.swiftui");
    assert_eq!(leaves.len(), 1, "the swiftui leaf mounted");
}

#[test]
fn reactive_params_patch_the_leaf() {
    let cell: Rc<Cell<Option<Signal<String>>>> = Rc::new(Cell::new(None));
    let cell2 = cell.clone();
    let probe = boot(move || {
        let params = Signal::new(String::from("{\"count\":1}"));
        cell2.set(Some(params));
        swiftui("Mod.View").params(params).any()
    });
    let params = cell.get().expect("signal captured");
    let mark = probe.log_len();

    params.set(String::from("{\"count\":2}"));
    flush_sync();
    let updates: Vec<String> = probe
        .log_since(mark)
        .into_iter()
        .filter(|l| l.starts_with("update day.piece.swiftui"))
        .collect();
    assert_eq!(updates.len(), 1, "one params patch reached the native leaf");
}

#[test]
fn const_params_seed_once_and_never_patch() {
    let probe = boot(|| {
        swiftui("Mod.View")
            .params(String::from("{\"count\":1}"))
            .any()
    });
    flush_sync();
    let updates: Vec<String> = probe
        .log()
        .into_iter()
        .filter(|l| l.starts_with("update day.piece.swiftui"))
        .collect();
    assert_eq!(
        updates,
        Vec::<String>::new(),
        "constant params only seed the realize"
    );
}

#[test]
fn support_is_unsupported_off_the_apple_backends() {
    assert_eq!(support(), Support::Unsupported);
}
