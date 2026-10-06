// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The conformance app (docs/testing.md): every `#[day::test]` case the built-in pieces
//! register, one route per case, so a person can open any case on any toolkit and `day test`
//! can drive it. Nothing here is written per piece: the cases live beside the pieces in
//! day-pieces, and `day::conformance` lists them.

use day::prelude::*;

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
    // The web has no link-time registry; each crate's roster registers its tests at launch
    // (a no-op everywhere else).
    day::builtin_tests::register_tests();
    day::test_host(day::builtin_tests::test_pages)
}
