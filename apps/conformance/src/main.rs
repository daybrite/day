// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

fn main() {
    // The same window and root the mobile hosts open through `day_start!` in src/lib.rs.
    day::launch(dayapp::window(), dayapp::root);
}
