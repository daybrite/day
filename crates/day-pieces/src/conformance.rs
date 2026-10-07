// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The built-in pieces' conformance cases, gathered (docs/testing.md). Each piece's cases live
//! in a `mod conformance` at the end of the file that defines it, next to the constructors they
//! prove; this module lists those modules for the targets with no link-time registry (wasm),
//! holds the cases that belong to no piece, and builds the conformance app's browsing page.
//! Compiled only under the `conformance` feature, so a shipping app links none of it.

use day_core::conformance::{Case, Drive, TestFn};
use day_core::*;
use day_reactive::Signal;

use crate::*;

/// The cases that belong to no single piece.
mod general {
    use super::*;

    /// A headless test: no page, app state checked on the app's own thread, in its own process.
    #[day_macros::test(day_core)]
    fn headless_sanity() -> Case {
        Case::headless().run(|t: Drive| async move {
            let count = Signal::new(1i64);
            count.update(|n| *n += 1);
            t.check_eq(count.get(), 2)?;
            day_core::sleep(1).await;
            t.check_eq(count.get(), 2)
        })
    }

    day_core::tests! { headless_sanity }
}

/// Register every module's cases where there is no link-time registry (wasm); a no-op
/// elsewhere. A new piece module's `conformance` joins both lists below.
pub fn register_tests() {
    general::register_tests();
    crate::leaves::conformance::register_tests();
    crate::inputs::conformance::register_tests();
    crate::image::conformance::register_tests();
    crate::forms::conformance::register_tests();
    crate::canvas::conformance::register_tests();
}

/// Every module's roster as written, for the lint that holds it equal to the link-time slice.
pub fn roster() -> Vec<TestFn> {
    [
        general::roster(),
        crate::leaves::conformance::roster(),
        crate::inputs::conformance::roster(),
        crate::image::conformance::roster(),
        crate::forms::conformance::roster(),
        crate::canvas::conformance::roster(),
    ]
    .concat()
}

/// The conformance app's content: a nav over every registered GUI case, one route per case,
/// so a person can open any case on any toolkit and look at it. A run does not go through it:
/// the app roots it in [`test_host`](crate::test_host), which shows each driven case's page
/// alone while it runs. An app with its own tests can browse them the same way.
pub fn test_pages() -> impl Piece {
    let selection = Signal::new(String::new());
    nav(selection)
        .items(
            || {
                day_core::conformance::cases()
                    .into_iter()
                    .filter(|c| c.kind() == day_core::conformance::TestKind::Gui)
                    .map(|c| c.name().to_owned())
                    .collect::<Vec<String>>()
            },
            |name: &String| item(name.clone(), name.clone()),
        )
        .destination(
            |key: &String| match day_core::conformance::case_named(key) {
                Some(case) => guarded_page(case),
                None => label(format!("no test {key:?}")).any(),
            },
        )
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    /// The roster (what wasm registers) names exactly the functions the link-time slice holds,
    /// under the same names.
    #[test]
    fn roster_matches_the_link_time_registry() {
        let entries = |tests: Vec<day_core::conformance::TestFn>| {
            let mut v: Vec<(String, usize)> = tests
                .iter()
                .map(|t| (t.name(), t.build_fn() as usize))
                .collect();
            v.sort();
            v
        };
        assert_eq!(
            entries(day_core::conformance::TESTS.to_vec()),
            entries(super::roster()),
            "every #[day::test] here must also be in `day_core::tests!`"
        );
    }
}
