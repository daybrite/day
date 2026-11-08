// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Run the authored tutorial checks against its exact app sources on the mock toolkit.
//! Test inputs are generated from the tutorial recipe, not shipped application assets.

use day::prelude::*;
use day_mock::MockToolkit;
use day_script::Step;
use std::path::Path;

fn boot() {
    day_core::uninstall_tree();
    install_locales("en", dayapp::res::locales::CATALOG);
    set_locale("en");
    let (mock, probe) = MockToolkit::new();
    probe.set_native_behavior(20);
    day_core::launch_with(mock, dayapp::res::window_options(), || dayapp::root().any());
}

fn replay(steps: Vec<Step>) {
    boot();
    for (index, step) in steps.into_iter().enumerate() {
        let label = format!("{step:?}");
        let reply = day_script::step(step);
        assert!(reply.ok, "step {index} {label}: {:?}", reply.error);
        day::reactive::flush_sync();
    }
    day_core::uninstall_tree();
}

#[test]
fn authored_interactions() {
    let source = std::fs::read_to_string("tests/steps.json").unwrap();
    let script = day_script_protocol::Script::from_yaml(&source).unwrap();
    replay(script.steps.into_iter().map(|entry| entry.step).collect());
    let path = Path::new("dayscript/session-counter.yaml");
    if path.exists() {
        let source = std::fs::read_to_string(path).unwrap();
        let script = day_script_protocol::Script::from_yaml(&source).unwrap();
        replay(script.steps.into_iter().map(|entry| entry.step).collect());
    }
}
