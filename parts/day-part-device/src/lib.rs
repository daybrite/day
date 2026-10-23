// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Device identity and battery snapshots, queried independently.

pub mod battery;
pub mod info;

#[cfg(target_os = "android")]
mod android;
