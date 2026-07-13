//! Day — the command-line tool (DESIGN.md §16). v0: new / build / launch / doctor for the
//! desktop targets, Day.toml manifest, per-target cargo dirs, `--format json` result events.
//! Mobile pipelines (xcodebuild/gradle callbacks) land with the M5 scaffolds.

mod cli;
mod doctor;
mod drive;
mod interactive;
mod lint;
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
mod update;

fn main() {
    let code = cli::run();
    std::process::exit(code);
}
