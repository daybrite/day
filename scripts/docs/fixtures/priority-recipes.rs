// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Synthetic mock fixtures for the exact published modules. These tests never open
//! an OS prompt, make a network request, or write outside the validator's workspace.

use day::prelude::*;
use day_mock::{MockProbe, MockToolkit, MockWidget};
use day_spec::{Event, NodeId};
use dayapp::{
    accessibility_recipe, events, layout_recipe, localization_recipe, navigation_recipe,
    permissions, res, resource_recipe,
};

fn boot<P: Piece>(root: impl FnOnce() -> P + 'static) -> MockProbe {
    boot_with_locales(root, res::locales::CATALOG)
}

fn boot_with_locales<P: Piece>(
    root: impl FnOnce() -> P + 'static,
    locales: &[(&str, &str)],
) -> MockProbe {
    day_core::uninstall_tree();
    install_locales("en", locales);
    set_locale("en");
    let (mock, probe) = MockToolkit::new();
    day_core::launch_with(mock, res::window_options(), move || root().any());
    probe
}

fn widget(probe: &MockProbe, kind: &str, id: &str) -> MockWidget {
    let mut widget = probe
        .find_by_kind(kind)
        .into_iter()
        .map(|(_, widget)| widget)
        .find(|widget| widget.a11y.identifier.as_deref() == Some(id))
        .unwrap_or_else(|| panic!("missing fixture node {id}"));
    // Fluent isolates interpolated values for bidirectional text. These assertions
    // compare visible wording; directional rendering needs separate native tests.
    widget.text.retain(|c| c != '\u{2068}' && c != '\u{2069}');
    for value in [&mut widget.a11y.label, &mut widget.a11y.value]
        .into_iter()
        .flatten()
    {
        value.retain(|c| c != '\u{2068}' && c != '\u{2069}');
    }
    widget
}

fn press(probe: &MockProbe, id: &str) {
    let button = widget(probe, "day.button", id);
    probe.emit(NodeId(button.node), Event::Pressed);
    day::reactive::flush_sync();
}

#[test]
fn native_button_action_updates_label_and_saturates() {
    let count = Signal::new(i64::MAX);
    let probe = boot(move || events::counter(count));
    press(&probe, "event_increment");
    assert_eq!(count.get(), i64::MAX);
    press(&probe, "event_reset");
    assert_eq!(widget(&probe, "day.label", "event_count").text, "Count: 0");
    press(&probe, "event_increment");
    assert_eq!(widget(&probe, "day.label", "event_count").text, "Count: 1");
}

#[test]
fn shared_count_survives_sidebar_page_replacement() {
    use navigation_recipe::Section;
    let section = Signal::new(Section::Counter);
    let count = Signal::new(0_i64);
    let probe = boot(move || navigation_recipe::sections(section, count));
    press(&probe, "event_increment");
    section.set(Section::Summary);
    day::reactive::flush_sync();
    assert_eq!(
        widget(&probe, "day.label", "section_total").text,
        "Total: 1"
    );
    section.set(Section::Counter);
    day::reactive::flush_sync();
    assert_eq!(widget(&probe, "day.label", "event_count").text, "Count: 1");
    press(&probe, "event_reset");
    section.set(Section::Summary);
    day::reactive::flush_sync();
    assert_eq!(
        widget(&probe, "day.label", "section_total").text,
        "Total: 0"
    );
}

#[test]
fn labeled_control_has_accessible_name_and_stable_id() {
    let enabled = Signal::new(false);
    let probe = boot(move || layout_recipe::settings(enabled));
    let toggle = widget(&probe, "day.toggle", "layout_notifications");
    assert_eq!(toggle.a11y.label.as_deref(), Some("Notifications"));
    enabled.set(true);
    day::reactive::flush_sync();
    assert!(widget(&probe, "day.toggle", "layout_notifications").flag);
}

#[test]
fn messages_follow_data_and_locale_without_changing_ids() {
    let name = Signal::new(String::from("Fixture reader"));
    let count = Signal::new(1_i64);
    let english = res::locales::CATALOG
        .iter()
        .find(|(locale, _)| *locale == "en")
        .unwrap()
        .1;
    // Synthetic translations test invalidation, not translation quality. Register
    // catalogs before building UI, as the guide requires; switch only the locale later.
    let probe = boot_with_locales(
        move || localization_recipe::greeting(name, count),
        &[
            ("en", english),
            (
                "fr",
                "recipe_greeting = Bonjour, { $name } !\nrecipe_unread = { $name } : { $count } notes\n",
            ),
        ],
    );
    assert_eq!(
        widget(&probe, "day.label", "recipe_unread").text,
        "Fixture reader, you have one unread note."
    );
    name.set(String::from("Second reader"));
    count.set(2);
    day::reactive::flush_sync();
    assert_eq!(
        widget(&probe, "day.label", "recipe_unread").text,
        "Second reader, you have 2 unread notes."
    );
    set_locale("fr");
    day::reactive::flush_sync();
    assert_eq!(
        widget(&probe, "day.label", "recipe_greeting").text,
        "Bonjour, Second reader !"
    );
    assert_eq!(
        widget(&probe, "day.label", "recipe_unread").text,
        "Second reader : 2 notes"
    );
}

#[test]
fn accessibility_value_updates_and_icon_button_has_name() {
    let percent = Signal::new(42_i64);
    let probe = boot(move || accessibility_recipe::progress_controls(percent));
    assert_eq!(
        widget(&probe, "day.progress", "a11y_progress")
            .a11y
            .value
            .as_deref(),
        Some("42 percent complete")
    );
    assert_eq!(
        widget(&probe, "day.button", "a11y_reset")
            .a11y
            .label
            .as_deref(),
        Some("Reset progress")
    );
    press(&probe, "a11y_reset");
    assert_eq!(
        widget(&probe, "day.progress", "a11y_progress")
            .a11y
            .value
            .as_deref(),
        Some("0 percent complete")
    );
}

#[test]
fn permission_policy_does_not_request_granted_or_unavailable_access() {
    use day_part_permissions::Status;
    for state in [
        Status::Granted,
        Status::Restricted,
        Status::Unsupported,
        Status::Unknown,
    ] {
        assert!(!permissions::should_request(state, true));
        assert!(!permissions::should_request(state, false));
    }
    for state in [Status::Prompt, Status::Denied] {
        assert!(permissions::should_request(state, true));
        assert!(!permissions::should_request(state, false));
    }
}

#[test]
fn generated_asset_loads_and_missing_file_has_localized_message() {
    let path = std::path::PathBuf::from(std::env::var_os("DAY_ASSET_ROOT").unwrap())
        .join("recipe_data.json");
    let data = std::fs::read(&path).unwrap();
    let probe = boot(resource_recipe::bundled_data);
    assert_eq!(
        widget(&probe, "day.label", "resource_status").text,
        format!("Bundled data: {} bytes", data.len())
    );
    std::fs::remove_file(&path).unwrap();
    let probe = boot(resource_recipe::bundled_data);
    // Restore the isolated fixture before asserting; never touch app/user files.
    std::fs::write(&path, data).unwrap();
    assert_eq!(
        widget(&probe, "day.label", "resource_status").text,
        "The bundled data could not be opened."
    );
}
