// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
#![allow(dead_code)]

fn unused<T>(_: Option<T>) {
    let _ = day_core::id!(fixture::unused);
}

macro_rules! repeated {
    () => {{
        let _ = day_core::id!(fixture::repeated);
        let _ = day_core::id!(fixture::repeated);
    }};
}

fn unused_expansion() {
    repeated!();
}

#[cfg(any())]
fn excluded() {
    let _ = day_core::id!(fixture::excluded);
}

#[unsafe(no_mangle)]
pub extern "C" fn day_id_fixture() -> u32 {
    let _ = day_core::id!(fixture::live);
    widgets::touch()
}
