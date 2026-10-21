// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// The consuming crate never calls anything here. In particular these nested modules have
// no ordinary public symbol to pull their archive members into the link.
mod unused {
    #[day_macros::test(day_core)]
    fn dependency_unused() -> day_core::conformance::Case {
        day_core::conformance::Case::new()
    }
    mod deeper {
        #[day_macros::test(day_core)]
        fn dependency_deep() -> day_core::conformance::Case {
            day_core::conformance::Case::new()
        }
    }
}

#[allow(dead_code)]
fn uncalled_generic<T>() {
    #[day_macros::test(day_core)]
    fn dependency_inside_generic() -> day_core::conformance::Case {
        day_core::conformance::Case::new()
    }
}
