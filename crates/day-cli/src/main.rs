// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Day — the command-line tool (DESIGN.md §16). v0: new / build / launch / doctor for the
//! desktop targets, Day.toml manifest, per-target cargo dirs, `--format json` result events.
//! Mobile pipelines (xcodebuild/gradle callbacks) land with the M5 scaffolds.

mod bridge;
mod checkup;
mod cli;
mod devices;
mod diagnose;
mod doctor;
mod drive;
mod external;
mod icon;
mod interactive;
mod json5;
mod lint;
mod localize;
mod mcp;
mod meta;
mod metadata;
mod mobile;
mod new;
mod ohos;
mod ops;
mod pack;
mod patch;
mod permissions;
mod pieces;
mod plist;
mod provenance;
mod rebuild;
mod resources;
mod screenshot;
mod script;
mod sessions;
mod shortcuts;
mod sign;
mod signals;
mod starter_l10n;
mod store;
mod swift;
mod targets;
mod template;
mod term;
mod update;
mod web;
mod xcconfig;

fn main() {
    let code = cli::run();
    std::process::exit(code);
}
