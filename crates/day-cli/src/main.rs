// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Day: the command-line tool (DESIGN.md §16). v0: new / build / launch / doctor for the
//! desktop targets, Day.toml manifest, per-target cargo dirs, `--format json` result events.
//! Mobile pipelines (xcodebuild/gradle callbacks) land with the M5 scaffolds.

mod bridge;
mod bump;
mod clean;
mod cli;
mod devices;
mod diagnose;
mod doctor;
mod doctor_verify;
mod documents;
mod drive;
mod external;
mod flavor;
mod git;
mod grant;
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
mod sandbox;
mod screenshot;
mod script;
mod script_report;
mod sessions;
mod shortcuts;
mod sign;
mod signals;
mod starter_l10n;
mod store;
mod targets;
mod template;
mod term;
mod update;
mod url_handlers;
mod web;
mod xcconfig;

fn main() {
    // Before anything else, and while this is the only thread: undo a snap-packaged host's
    // library overrides so every build and launch below inherits the system's (ops.rs).
    let scrubbed = ops::scrub_snap_env();
    let code = cli::run(&scrubbed);
    std::process::exit(code);
}
