//! Centralized terminal styling for the day CLI.
//!
//! A single palette, referenced everywhere instead of inline `\x1b[..m` escape codes. Built on
//! [`anstyle`] — the styling vocabulary clap itself uses, already in our dependency tree — so the
//! CLI's own output shares clap's color system. Print styled text with the [`anstream`]
//! `eprintln!`/`println!` macros in the `{STYLE}text{STYLE:#}` form (the `:#` alternate flag emits
//! the reset):
//!
//! ```ignore
//! use anstream::eprintln;
//! use crate::term::{ERROR, WARN};
//! eprintln!("  {ERROR}✗{ERROR:#} {WARN}{}{WARN:#}", msg);
//! ```
//!
//! `anstream` strips the escapes automatically when the destination isn't a color-capable terminal
//! (a pipe, a file, `NO_COLOR`, `TERM=dumb`, or a legacy Windows console), so styled call sites
//! stay honest without each one probing the tty. For richer terminal UI later (progress bars,
//! concurrent task spinners) the intended companions are `indicatif` + `console` — see the CLI
//! output notes in AGENTS.md.

use anstyle::{AnsiColor, Color, Effects, Style};

/// A plain foreground color on the default background — the building block for the palette.
const fn fg(color: AnsiColor) -> Style {
    Style::new().fg_color(Some(Color::Ansi(color)))
}

/// cargo-style status header — bold green (`   Launching`, `  Building`); the caller right-aligns.
pub const HEADER: Style = fg(AnsiColor::Green).effects(Effects::BOLD);
/// Success — green (`✓`, "no findings").
pub const SUCCESS: Style = fg(AnsiColor::Green);
/// Emphatic success — bold green (summary "✓ all good").
pub const SUCCESS_BOLD: Style = fg(AnsiColor::Green).effects(Effects::BOLD);
/// Warning / advisory — yellow (`⚠`, "warning", the `▸` keep-alive note, the update banner).
pub const WARN: Style = fg(AnsiColor::Yellow);
/// Failure — red (`✗`).
pub const ERROR: Style = fg(AnsiColor::Red);
/// Emphatic failure — bold red (summary "✗ N error(s)").
pub const ERROR_BOLD: Style = fg(AnsiColor::Red).effects(Effects::BOLD);
/// De-emphasized — dimmed (setup hints, "n/a" lines, the scan preamble).
pub const DIM: Style = Style::new().effects(Effects::DIMMED);
/// Emphasis without color — bold (group labels).
pub const BOLD: Style = Style::new().effects(Effects::BOLD);
/// Forwarded app **stdout** line prefix `[target]` — blue.
pub const LOG_OUT: Style = fg(AnsiColor::Blue);
/// Forwarded app **stderr** line prefix `[target]` — yellow.
pub const LOG_ERR: Style = fg(AnsiColor::Yellow);
