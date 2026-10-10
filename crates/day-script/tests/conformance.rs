// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The mock harness for `#[day::test]` cases (docs/testing.md): every case the built-in
//! pieces register, driven by the real engine against the mock toolkit. What `day test` runs on
//! a toolkit, this runs under `cargo test`, so a broken binding or id fails here first.

use day_core::conformance::{Case, Drive, TestFn};
use day_mock::MockToolkit;
use day_pieces::Decorate;
use day_script::conformance::{Outcome, RunReport, ShotPolicy};
use day_script::{Reply, Step};
use day_spec::{Size, WindowOptions};

fn boot() -> day_mock::MockProbe {
    boot_with(|| {
        day_core::AnyPiece::new(day_pieces::test_host(day_pieces::conformance::test_pages))
    })
}

fn boot_with(root: impl FnOnce() -> day_core::AnyPiece + 'static) -> day_mock::MockProbe {
    day_core::uninstall_tree();
    let (mock, probe) = MockToolkit::new();
    // No test drives the mock's native side here (binding rows, reporting covers), so it does
    // that by itself.
    probe.set_native_behavior(20);
    // The mock groups windows like AppKit does (recorded, never drawn), so the document tabs'
    // window-group case runs here rather than skipping on `Cap::WindowTabbing`.
    probe.state.borrow_mut().native_window_tabs = true;
    day_core::launch_with(
        mock,
        WindowOptions {
            title: "conformance".into(),
            size: Size::new(400.0, 600.0),
            ..Default::default()
        },
        root,
    );
    probe
}

/// Drive `run_tests` to its report: the first answer starts the run, and on the mock the whole
/// run completes inside it, so the next answer carries the report.
fn run(filter: &[&str]) -> RunReport {
    // No time limits: the mock's timers fire the moment they are set, so a limit would trip
    // on the first retry. `a_case_past_its_limit_fails_as_timed_out` runs with them on.
    run_with(filter, Some(0.0))
}

fn run_with(filter: &[&str], case_timeout_secs: Option<f64>) -> RunReport {
    let step = Step::RunTests {
        filter: filter.iter().map(|s| s.to_string()).collect(),
        shots: ShotPolicy::Never,
        timeout_secs: None,
        case_timeout_secs,
    };
    for _ in 0..100 {
        let reply: Reply = day_script::step(step.clone());
        if reply.ok {
            let data = reply.data.expect("a finished run answers its report");
            return serde_json::from_value(data).expect("the report decodes");
        }
        assert!(reply.retryable, "{:?}", reply.error);
    }
    panic!("the run never finished");
}

#[test]
fn every_built_in_case_passes_on_the_mock() {
    let _probe = boot();
    let listing = day_script::step(Step::Tests)
        .data
        .expect("tests lists the registry");
    let names: Vec<&str> = listing
        .as_array()
        .expect("a list")
        .iter()
        .filter_map(|t| t.get("name").and_then(|n| n.as_str()))
        .collect();
    assert!(names.contains(&"button-press"), "{names:?}");

    let report = run(&[]);
    assert_eq!(report.tests.len(), names.len());
    let failed: Vec<String> = report
        .tests
        .iter()
        .filter(|t| t.verdict != "pass")
        .map(|t| {
            format!(
                "{}: {} {}",
                t.name,
                t.verdict,
                t.message.clone().or(t.reason.clone()).unwrap_or_default()
            )
        })
        .collect();
    assert!(failed.is_empty(), "{failed:#?}");
}

#[test]
fn a_filter_selects_by_glob() {
    let _probe = boot();
    let report = run(&["text-field-*"]);
    let names: Vec<&str> = report.tests.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "text-field-binding",
            "text-field-disabled",
            "text-field-max-length",
            "text-field-read-only",
            "text-field-secure",
            "text-field-submit"
        ]
    );
    assert!(
        report.tests.iter().all(|t| t.verdict == "pass"),
        "{report:?}"
    );
}

fn only(report: &RunReport, name: &str) -> Outcome {
    let found: Vec<&Outcome> = report.tests.iter().filter(|t| t.name == name).collect();
    assert_eq!(found.len(), 1, "{report:?}");
    found[0].clone()
}

// Harness-only cases, registered at run time on this thread (the registry is per thread, so
// they never reach the built-in run above).

fn panics_in_its_drive() -> Case {
    Case::new()
        .page(|| day_pieces::label("here").id("pd-label"))
        .drive(|d: Drive| async move {
            d.assert_text("pd-label", "here").await?;
            panic!("the drive gave up");
        })
}

fn panics_in_its_page() -> Case {
    Case::new()
        .page(|| -> day_pieces::Label { panic!("no page today") })
        .drive(|_d: Drive| async move { Ok(()) })
}

fn panics_headless() -> Case {
    Case::headless().run(|_t: Drive| async move { panic!("headless boom") })
}

fn never_finishes() -> Case {
    Case::headless().timeout(0.05).run(|_t: Drive| async move {
        std::future::pending::<day_core::conformance::TestResult>().await
    })
}

fn shares_a_name() -> Case {
    Case::headless()
}

mod elsewhere {
    pub fn shares_a_name() -> day_core::conformance::Case {
        day_core::conformance::Case::headless()
    }
}

#[test]
fn a_panic_fails_its_case_and_the_run_goes_on() {
    let _probe = boot();
    for t in [
        TestFn::new("panics_in_its_drive", panics_in_its_drive),
        TestFn::new("panics_in_its_page", panics_in_its_page),
        TestFn::new("panics_headless", panics_headless),
    ] {
        day_core::conformance::register(t);
    }
    let report = run(&["panics-*", "button-press"]);
    let drive = only(&report, "panics-in-its-drive");
    assert_eq!(drive.verdict, "fail");
    assert!(
        drive
            .message
            .as_deref()
            .unwrap_or("")
            .contains("the drive gave up"),
        "{drive:?}"
    );
    let page = only(&report, "panics-in-its-page");
    assert_eq!(page.verdict, "fail");
    assert!(
        page.message
            .as_deref()
            .unwrap_or("")
            .contains("no page today"),
        "{page:?}"
    );
    let headless = only(&report, "panics-headless");
    assert!(
        headless
            .message
            .as_deref()
            .unwrap_or("")
            .contains("headless boom"),
        "{headless:?}"
    );
    // The cases after a panic still run, on a page built fresh for them.
    assert_eq!(only(&report, "button-press").verdict, "pass");
}

#[test]
fn a_case_past_its_limit_fails_as_timed_out() {
    let _probe = boot();
    day_core::conformance::register(TestFn::new("never_finishes", never_finishes));
    let report = run_with(&["never-finishes"], None);
    let t = only(&report, "never-finishes");
    assert_eq!(t.verdict, "fail");
    assert!(
        t.message.as_deref().unwrap_or("").contains("timed out"),
        "{t:?}"
    );
}

#[test]
fn two_tests_with_one_name_both_fail() {
    let _probe = boot();
    day_core::conformance::register(TestFn::new("shares_a_name", shares_a_name));
    day_core::conformance::register(TestFn::new(
        "elsewhere::shares_a_name",
        elsewhere::shares_a_name,
    ));
    let report = run(&["shares-a-name"]);
    assert_eq!(report.tests.len(), 2, "{report:?}");
    assert!(
        report.tests.iter().all(|t| t.verdict == "fail"),
        "{report:?}"
    );
}

#[test]
fn a_gui_case_needs_a_test_host() {
    let _probe = boot_with(|| day_core::AnyPiece::new(day_pieces::label("no host here")));
    let report = run(&["button-press"]);
    let t = only(&report, "button-press");
    assert_eq!(t.verdict, "fail");
    assert!(
        t.message.as_deref().unwrap_or("").contains("test host"),
        "{t:?}"
    );
}

#[test]
fn test_names_come_from_the_function() {
    assert_eq!(
        TestFn::new("button_press", shares_a_name).name(),
        "button-press"
    );
    assert_eq!(
        TestFn::new("a :: b :: text_field_secure", shares_a_name).name(),
        "text-field-secure"
    );
}

fn reads_what_the_mock_cannot() -> Case {
    Case::new()
        .page(|| day_pieces::label("here").id("unread-label"))
        .drive(|d: Drive| async move {
            d.assert_native(
                "unread-label",
                day_core::conformance::NativeExpect {
                    visible: Some(true),
                    ..Default::default()
                },
            )
            .await
        })
}

#[test]
fn a_native_field_the_toolkit_cannot_read_is_recorded_not_failed() {
    let _probe = boot();
    day_core::conformance::register(TestFn::new(
        "reads_what_the_mock_cannot",
        reads_what_the_mock_cannot,
    ));
    let t = only(
        &run(&["reads-what-the-mock-cannot"]),
        "reads-what-the-mock-cannot",
    );
    assert_eq!(t.verdict, "pass", "{t:?}");
    assert_eq!(t.native_unread, ["unread-label visible"]);
    // The default native follow-up ran on every built-in case without leaving gaps on the
    // mock, whose reader answers every field the leaves assert.
    let report = run(&["button-*", "text-field-*", "toggle-*", "slider-*"]);
    for t in &report.tests {
        assert!(t.native_unread.is_empty(), "{t:?}");
    }
}
