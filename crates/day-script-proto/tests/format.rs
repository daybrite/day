// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

use day_script_proto::{
    FailurePolicy, Request, Script, ScriptStep, Step, steps_from_yaml, steps_to_yaml,
};
use serde_json::{Value, json};

#[test]
fn whole_catalog_round_trips_through_canonical_yaml() {
    // Synthetic protocol catalog, not an app resource.
    let fixtures: Vec<Value> = serde_json::from_str(include_str!("fixtures/steps.json")).unwrap();
    let steps: Vec<Step> = fixtures
        .into_iter()
        .map(|v| serde_json::from_value(v).unwrap())
        .collect();
    let yaml = steps_to_yaml(&steps);
    assert_eq!(steps_from_yaml(&yaml).unwrap(), steps);
    let doc: Value = serde_norway::from_str(&yaml).unwrap();
    assert!(
        doc["flow"]
            .as_array()
            .unwrap()
            .iter()
            .all(|v| v.as_object().unwrap().len() == 1)
    );
}

#[test]
fn shorthands_and_mapping_forms_describe_identical_operations() {
    let short = steps_from_yaml(
        "flow:\n- screenshot: fixture\n- pause: 0.125\n- resize: auto\n- nav_back:\n- wait_idle:\n",
    )
    .unwrap();
    let full = steps_from_yaml("flow:\n- screenshot: {name: fixture}\n- pause: {secs: 0.125}\n- resize: {restore: true}\n- nav_back: {native: false}\n- wait_idle: {}\n").unwrap();
    assert_eq!(short, full);
    assert!(matches!(
        short[2],
        Step::Resize {
            restore: true,
            width: None,
            height: None
        }
    ));
    assert!(steps_to_yaml(&short).contains("- nav_back: null"));
}

#[test]
fn bom_comments_crlf_escapes_unicode_and_empty_flows_work() {
    let yaml = "\u{feff}# fixture\r\nflow:\r\n- input:\r\n    id: fixture\r\n    text: |\r\n      Français: \"雪\" # literal text\r\n      ${project}/folder\\file\r\n";
    let steps = steps_from_yaml(yaml).unwrap();
    let Step::Input {
        text: Some(text), ..
    } = &steps[0]
    else {
        panic!("input fixture")
    };
    assert_eq!(
        text,
        "Français: \"雪\" # literal text\n${project}/folder\\file\n"
    );
    assert_eq!(steps_from_yaml(&steps_to_yaml(&steps)).unwrap(), steps);
    assert_eq!(Script::from_yaml("flow: []").unwrap(), Script::default());
    assert_eq!(steps_to_yaml(&[]), "flow: []\n");
}

#[test]
fn host_policy_metadata_and_annotations_survive_document_round_trips() {
    let script = Script::from_yaml("name: fixture\ndescription: Fixture document\non_failure: stop\nflow:\n- screenshot:\n    name: fixture\n    title: {key: fixture-title, args: {name: Ada}}\n    caption: Fixture caption\n    source: controls\n    store: true\n    only_on: [android, flavor:fixture]\n    skip_on: [web-dom]\n    future: {enabled: false}\n- pause: {secs: 0.5, animation: true}\n").unwrap();
    assert_eq!(script.on_failure, FailurePolicy::Stop);
    assert_eq!(script.metadata["name"], "fixture");
    assert_eq!(
        script.steps[0].annotations["only_on"],
        json!(["android", "flavor:fixture"])
    );
    assert_eq!(
        script.steps[0].annotations["future"],
        json!({"enabled":false})
    );
    assert_eq!(Script::from_yaml(&script.to_yaml()).unwrap(), script);
    let request = Request {
        token: "fixture-token".into(),
        step: script.steps[0].step.clone(),
    };
    let wire = serde_json::to_value(request).unwrap();
    for key in [
        "only_on", "skip_on", "title", "caption", "source", "store", "future",
    ] {
        assert!(wire["step"].get(key).is_none(), "annotation leaked: {key}");
    }
    assert_eq!(wire["step"]["name"], "fixture");
}

#[test]
fn optional_nulls_are_omitted_without_losing_false_zero_or_empty_values() {
    let steps = steps_from_yaml("flow:\n- input: {id: fixture, text: '', key: null, args: null}\n- toggle: {id: fixture, value: false}\n- pause: 0\n- navigate: {route: ''}\n").unwrap();
    let yaml = steps_to_yaml(&steps);
    assert!(!yaml.contains("key:"));
    assert!(!yaml.contains("args:"));
    assert_eq!(steps_from_yaml(&yaml).unwrap(), steps);
}

#[test]
fn json_drive_accepts_both_spellings_using_the_same_shorthands() {
    for (entry, wire) in [
        (
            json!({"screenshot":"fixture"}),
            json!({"op":"screenshot","name":"fixture"}),
        ),
        (json!({"pause":0.5}), json!({"op":"pause","secs":0.5})),
        (
            json!({"resize":"auto"}),
            json!({"op":"resize","restore":true}),
        ),
        (json!({"nav_back":null}), json!({"op":"nav_back"})),
    ] {
        assert_eq!(
            ScriptStep::from_value(entry).unwrap(),
            ScriptStep::from_value(wire).unwrap()
        );
    }
}

#[test]
fn failure_policy_is_validated_and_continue_remains_the_default() {
    for yaml in ["flow: []", "on_failure: continue\nflow: []"] {
        assert_eq!(
            Script::from_yaml(yaml).unwrap().on_failure,
            FailurePolicy::Continue
        );
    }
    for policy in ["typo", "true", "null", "4", "[]", "{}"] {
        let error = Script::from_yaml(&format!("on_failure: {policy}\nflow: []")).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("on_failure must be stop or continue")
        );
    }
}

#[test]
fn malformed_documents_are_rejected_before_execution() {
    for yaml in [
        "",
        "[]",
        "null",
        "flow: null",
        "flow: {}",
        "name: fixture",
        "flow: [tap]",
        "flow: [null]",
        "flow: [{}]",
        "flow:\n- tap: {id: fixture}\n  pause: 1",
        "flow:\n- op: tap\n  id: fixture",
        "flow: [",
    ] {
        assert!(Script::from_yaml(yaml).is_err(), "accepted: {yaml}");
    }
}

#[test]
fn duplicate_yaml_keys_are_rejected_in_documents_entries_and_params() {
    for yaml in [
        "flow: []\nflow: []",
        "flow:\n- tap: {id: one}\n  tap: {id: two}",
        "flow:\n- tap: {id: one, id: two}",
    ] {
        assert!(Script::from_yaml(yaml).is_err(), "accepted: {yaml}");
    }
}

#[test]
fn invalid_operations_and_parameter_types_report_the_flow_position() {
    for entry in [
        "unknown:",
        "tap: {}",
        "tap: {id: 7}",
        "tap: {id: fixture, repeat: -1}",
        "tap: {id: fixture, at: [1]}",
        "input: {id: fixture, args: []}",
        "pause: true",
        "pause: text",
        "pause: {}",
        "resize: other",
        "screenshot: [fixture]",
        "dialog_mode: {mode: unknown}",
        "run_tests: {shots: on_failure}",
    ] {
        let yaml = format!("flow:\n- wait_idle:\n- {entry}\n");
        let error = Script::from_yaml(&yaml).unwrap_err().to_string();
        assert!(error.contains("flow step 2:"), "{entry}: {error}");
    }
}

#[test]
fn operation_params_cannot_redirect_the_operation() {
    for entry in [
        json!({"tap":{"op":"wait_idle","id":"fixture"}}),
        json!({"tap":{},"wait_idle":null}),
    ] {
        assert!(ScriptStep::from_value(entry).is_err());
    }
    let error = Script::from_yaml("flow:\n- tap: {id: fixture, op: wait_idle}").unwrap_err();
    assert!(error.to_string().contains("params cannot override `op`"));
}

#[test]
fn malformed_runner_annotations_are_rejected() {
    for params in [
        "skip_on: android",
        "only_on: [true]",
        "skip_on: null",
        "animation: yes",
    ] {
        let error =
            Script::from_yaml(&format!("flow:\n- pause: {{secs: 0.1, {params}}}")).unwrap_err();
        assert!(error.to_string().contains("flow step 1:"));
    }
}

#[test]
fn invalid_host_durations_are_errors_instead_of_panics() {
    for yaml in [
        "flow:\n- pause: -1",
        "flow:\n- pause: .nan",
        "flow:\n- pause: .inf",
        "flow:\n- pause: 1e100",
        "flow:\n- expect_exit: {within: -1}",
        "flow:\n- expect_exit: {within: .inf}",
        "flow:\n- wait_for: {id: fixture, timeout_secs: .nan}",
        "flow:\n- resize: {width: .inf}",
    ] {
        assert!(Script::from_yaml(yaml).is_err(), "accepted: {yaml}");
    }
    assert!(ScriptStep::from_value(json!({"op":"pause","secs":-1})).is_err());
}

#[test]
fn expected_exit_must_be_terminal_even_when_annotated() {
    assert!(Script::from_yaml("flow:\n- expect_exit: {only_on: [android]}\n- wait_idle:").is_err());
    assert!(Script::from_yaml("flow:\n- wait_idle:\n- expect_exit:").is_ok());
}

#[test]
fn annotations_never_override_the_typed_operation() {
    let mut step = ScriptStep::from(Step::NavBack { native: false });
    step.annotations.insert("op".into(), "tap".into());
    step.annotations.insert("native".into(), true.into());
    assert_eq!(step.to_value(), json!({"op":"nav_back"}));
    assert_eq!(
        ScriptStep::from_value(json!({"op":"nav_back","native":false}))
            .unwrap()
            .annotations
            .len(),
        0
    );
}

#[test]
fn null_future_annotations_and_arbitrary_document_metadata_are_retained() {
    let script = Script::from_yaml(
        "name: fixture\nfuture: {versions: [1, 2], optional: null}\nflow:\n- wait_idle: {future: null}\n",
    )
    .unwrap();
    assert_eq!(script.steps[0].annotations["future"], Value::Null);
    assert_eq!(Script::from_yaml(&script.to_yaml()).unwrap(), script);
}
