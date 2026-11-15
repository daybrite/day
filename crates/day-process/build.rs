// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Present only because Cargo requires a build script of a package that declares `links`
//! (Cargo.toml explains why this one does). It builds nothing.

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
}
