// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("ios") {
        println!("cargo:rerun-if-changed=src/context.m");
        cc::Build::new()
            .file("src/context.m")
            .flag("-fobjc-arc")
            .compile("day_context");
    }
}
