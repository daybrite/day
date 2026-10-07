// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

use day_script_proto::{DialogMode, Reply, Request, ShotPolicy, Step};
use serde_json::{Value, json};

// Synthetic protocol fixtures, not bundled app resources. Every shipped operation appears
// once, with optional fields populated to detect lost fields during extraction or serialization.
fn catalog() -> Vec<Value> {
    serde_json::from_str(include_str!("fixtures/steps.json")).unwrap()
}

#[test]
fn operation_catalog_round_trips_requests_without_changing_wire_fields() {
    let catalog = catalog();
    assert_eq!(catalog.len(), 53);
    let mut operations = std::collections::BTreeSet::new();
    for fixture in catalog {
        let step: Step = serde_json::from_value(fixture.clone()).unwrap();
        assert_eq!(fixture["op"], step.op());
        assert!(operations.insert(step.op()));
        let request = Request {
            token: "fixture-token".into(),
            step,
        };
        let wire = serde_json::to_value(&request).unwrap();
        assert_eq!(wire["token"], "fixture-token");
        for (key, value) in fixture.as_object().unwrap() {
            assert_eq!(wire["step"][key], *value, "{}: {key}", request.step.op());
        }
        assert_eq!(serde_json::from_value::<Request>(wire).unwrap(), request);
    }
}

#[test]
fn old_requests_keep_their_behavioral_defaults() {
    let screenshot: Step =
        serde_json::from_value(json!({"op":"screenshot","name":"fixture"})).unwrap();
    assert!(matches!(
        screenshot,
        Step::Screenshot {
            in_process: true,
            window: None,
            ..
        }
    ));
    let expand: Step =
        serde_json::from_value(json!({"op":"expand","id":"fixture","row":"child"})).unwrap();
    assert!(matches!(expand, Step::Expand { expanded: true, .. }));
    let back: Step = serde_json::from_value(json!({"op":"nav_back"})).unwrap();
    assert_eq!(back, Step::NavBack { native: false });
    assert_eq!(
        serde_json::to_value(back).unwrap(),
        json!({"op":"nav_back"})
    );
    let tap: Step = serde_json::from_value(json!({"op":"tap","id":"fixture"})).unwrap();
    assert!(
        matches!(tap, Step::Tap { if_present: false, repeat: None, at: None, modifiers, .. } if modifiers.is_empty())
    );
    let run: Step = serde_json::from_value(json!({"op":"run_tests"})).unwrap();
    assert!(
        matches!(run, Step::RunTests { shots: ShotPolicy::OnFailure, filter, timeout_secs: None, case_timeout_secs: None } if filter.is_empty())
    );
}

#[test]
fn enum_spellings_remain_wire_compatible() {
    for (mode, name) in [
        (DialogMode::Native, "native"),
        (DialogMode::Scripted, "scripted"),
    ] {
        assert_eq!(serde_json::to_value(mode).unwrap(), name);
        assert_eq!(
            serde_json::from_value::<DialogMode>(name.into()).unwrap(),
            mode
        );
    }
    for (policy, name) in [
        (ShotPolicy::Never, "never"),
        (ShotPolicy::OnFailure, "on-failure"),
        (ShotPolicy::Always, "always"),
    ] {
        assert_eq!(serde_json::to_value(policy).unwrap(), name);
        assert_eq!(
            serde_json::from_value::<ShotPolicy>(name.into()).unwrap(),
            policy
        );
    }
    assert!(serde_json::from_value::<ShotPolicy>(json!("on_failure")).is_err());
}

#[test]
fn old_replies_default_new_fields_and_ignore_future_fields() {
    let reply: Reply =
        serde_json::from_value(json!({"ok":true,"future_field":{"version":2}})).unwrap();
    assert_eq!(reply, Reply::ok());
    assert_eq!(reply.capture_revision, None);
    assert_eq!(reply.fast_animations, None);
    assert!(!reply.capture_pending);
    assert_eq!(
        serde_json::to_value(reply).unwrap(),
        json!({"ok":true,"retryable":false,"screenshot_unsupported":false})
    );
    let failure: Reply =
        serde_json::from_value(json!({"ok":false,"error":"fixture failure","retryable":true}))
            .unwrap();
    assert_eq!(failure, Reply::fail("fixture failure", true));
    assert!(serde_json::from_value::<Reply>(json!({"error":"missing verdict"})).is_err());
}

#[test]
fn populated_reply_round_trips_and_runtime_hint_never_crosses_the_wire() {
    let reply = Reply {
        ok: true,
        png_base64: Some("Zml4dHVyZQ==".into()),
        screenshot_unsupported: true,
        capture_revision: Some(17),
        fast_animations: Some(true),
        capture_pending: true,
        data: Some(json!({"tests":[{"name":"fixture","verdict":"pass"}]})),
        ..Reply::default()
    };
    let wire = serde_json::to_value(&reply).unwrap();
    assert!(wire.get("capture_pending").is_none());
    let mut expected = reply;
    expected.capture_pending = false;
    assert_eq!(serde_json::from_value::<Reply>(wire).unwrap(), expected);
    let inbound: Reply = serde_json::from_value(json!({"ok":true,"capture_pending":true})).unwrap();
    assert!(!inbound.capture_pending);
}

#[test]
fn future_request_fields_are_ignored_but_invalid_protocol_fields_fail() {
    let request: Request = serde_json::from_value(
        json!({"token":"fixture","extra":true,"step":{"op":"wait_idle","future":true}}),
    )
    .unwrap();
    assert_eq!(request.step, Step::WaitIdle);
    for fixture in [
        json!({"step":{"op":"wait_idle"}}),
        json!({"token":"fixture","step":{"op":"unknown"}}),
        json!({"token":7,"step":{"op":"wait_idle"}}),
        json!({"token":"fixture","step":{"op":"tap","id":false}}),
    ] {
        assert!(serde_json::from_value::<Request>(fixture).is_err());
    }
}

#[test]
fn wait_budgets_share_defaults_and_per_operation_overrides() {
    for (op, fields) in [
        ("wait_for", json!({"id":"fixture"})),
        ("assert_text", json!({"id":"fixture"})),
        ("web_eval", json!({"id":"fixture","script":"true"})),
        ("run_tests", json!({})),
    ] {
        for (timeout, expected) in [
            (None, 5.0),
            (Some(0.0), 5.0),
            (Some(-1.0), 5.0),
            (Some(13.0), 13.0),
        ] {
            let mut step = fields.clone();
            step["op"] = op.into();
            if let Some(timeout) = timeout {
                step["timeout_secs"] = json!(timeout);
            }
            let step: Step = serde_json::from_value(step).unwrap();
            assert_eq!(step.wait_budget_secs(), expected, "{op}");
        }
    }
    assert_eq!(Step::WaitIdle.wait_budget_secs(), 5.0);
}
