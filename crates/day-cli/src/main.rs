//! Day — the command-line tool (DESIGN.md §16). v0: new / build / launch / doctor for the
//! desktop targets, Day.toml manifest, per-target cargo dirs, `--format json` result events.
//! Mobile pipelines (xcodebuild/gradle callbacks) land with the M5 scaffolds.

mod cli;
mod doctor;
mod drive;
mod interactive;
mod intl;
mod lint;
mod lite;
mod mcp;
mod meta;
mod metadata;
mod mobile;
mod new;
mod ohos;
mod ops;
mod pack;
mod pieces;
mod resources;
mod script;
mod sessions;
mod sign;
mod signals;
mod targets;
mod template;
mod term;
mod update;

fn main() {
    // Before any thread spawns (the update check): point icu4x's source cache at ~/.day/icu/src.
    intl::init_source_cache();
    let code = cli::run();
    std::process::exit(code);
}
