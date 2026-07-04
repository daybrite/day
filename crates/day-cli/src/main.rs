//! day — the command-line tool (DESIGN.md §16). v0: create / build / launch / doctor for the
//! desktop targets, day.yaml manifest, per-target cargo dirs, `--format json` result events.
//! Mobile pipelines (xcodebuild/gradle callbacks) land with the M5 scaffolds.

mod cli;
mod doctor;
mod lint;
mod meta;
mod mobile;
mod ops;
mod pack;
mod pieces;
mod script;
mod signals;
mod targets;

fn main() {
    let code = cli::run();
    std::process::exit(code);
}
