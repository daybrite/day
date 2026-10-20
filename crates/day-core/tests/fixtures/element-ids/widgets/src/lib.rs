// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
#![allow(dead_code)]

fn unused<T>(_: Option<T>) {
    let _ = kernel::id!(dependency::unused);
}

mod otherwise_unreferenced {
    pub fn helper() {
        let _ = kernel::id!(dependency::unreferenced_module);
    }
}

pub fn touch() -> u32 {
    let _ = kernel::id!(dependency::live);
    1
}
