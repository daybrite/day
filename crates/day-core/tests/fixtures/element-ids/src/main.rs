// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
fn main() {
    day_id_registry_fixture::day_id_fixture();
    for id in day_core::element_id::registry().unwrap() {
        println!("{}", id.family);
    }
}
