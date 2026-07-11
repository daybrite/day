//! Day — the command-line tool (DESIGN.md §16). v0: new / build / launch / doctor for the
//! desktop targets, day.yaml manifest, per-target cargo dirs, `--format json` result events.
//! Mobile pipelines (xcodebuild/gradle callbacks) land with the M5 scaffolds.

mod cli;
mod doctor;
mod interactive;
mod lint;
mod meta;
mod mobile;
mod new;
mod ohos;
mod ops;
mod pack;
mod pieces;
mod resources;
mod script;
mod sign;
mod signals;
mod targets;
mod template;

fn main() {
    let code = cli::run();
    std::process::exit(code);
}
