// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The conformance app (docs/testing.md): every `#[day::test]` case the built-in pieces
//! register, one route per case, so a person can open any case on any toolkit and `day test`
//! can drive it. Most cases live beside the pieces in day-pieces; cases that need bundled
//! resources, such as vector icons, live in this app.

use day::prelude::*;

mod vector_icons;

#[cfg(target_env = "ohos")]
mod native_dialogs;
#[cfg(target_env = "ohos")]
mod native_links;
#[cfg(target_env = "ohos")]
mod native_long_press;
#[cfg(target_env = "ohos")]
mod native_textarea;

// Entry point for the mobile hosts; a desktop build enters through src/main.rs.
day::day_start!(options: window(), root);

/// Options for every window. The catalog and title go to `launch`, which installs them.
pub fn window() -> day::WindowOptions {
    day::WindowOptions {
        locales: Some((res::locales::DEFAULT, res::locales::CATALOG)),
        title_fn: Some(|| res::str::app_title().format()),
        // Desktop only; phones fill the screen.
        size: day::prelude::Size::new(960.0, 640.0),
        // Named in Day's exit line (https://daybrite.dev/docs/lifecycle).
        version: Some(env!("CARGO_PKG_VERSION").into()),
        ..Default::default()
    }
}

// Typed names for everything under `resource/` (https://daybrite.dev/docs/resources).
day::resources!();

/// The first window's content: the registered GUI cases to browse, inside the test host that
/// shows each case alone while `day test` drives it.
pub fn root() -> impl Piece {
    #[cfg(target_env = "ohos")]
    if std::env::var_os("DAY_NATIVE_LINK_TEST").is_some() {
        return native_links::page().any();
    }
    #[cfg(target_env = "ohos")]
    if std::env::var_os("DAY_NATIVE_TEXTAREA_SUBMIT_TEST").is_some() {
        return native_textarea::submission_page().any();
    }
    #[cfg(target_env = "ohos")]
    if std::env::var_os("DAY_NATIVE_TEXTAREA_TEST").is_some() {
        return native_textarea::page().any();
    }
    #[cfg(target_env = "ohos")]
    if std::env::var_os("DAY_NATIVE_DIALOG_TEST").is_some() {
        return native_dialogs::page().any();
    }
    #[cfg(target_env = "ohos")]
    if std::env::var_os("DAY_NATIVE_LONG_PRESS_TEST").is_some() {
        return native_long_press::page().any();
    }
    // The web has no link-time registry; each crate's roster registers its tests at launch
    // (a no-op everywhere else).
    day::builtin_tests::register_tests();
    vector_icons::register_tests();
    day_piece_activity::conformance::register_tests();
    day_piece_colorpicker::conformance::register_tests();
    day_piece_combobox::conformance::register_tests();
    day_piece_datetime::conformance::register_tests();
    day_piece_map::conformance::register_tests();
    day_piece_pager::conformance::register_tests();
    day_piece_pullrefresh::conformance::register_tests();
    day_piece_rating::conformance::register_tests();
    day_piece_remote_image::conformance::register_tests();
    day_piece_searchfield::conformance::register_tests();
    day_piece_settings::conformance::register_tests();
    day_piece_texteditor::conformance::register_tests();
    day_piece_stepper::conformance::register_tests();
    day::test_host(day::builtin_tests::test_pages).any()
}
