// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

fn main() {
    pkg_config::Config::new()
        .atleast_version("2.52")
        .probe("librsvg-2.0")
        .expect("librsvg development files are required for native SVG rendering");
}
