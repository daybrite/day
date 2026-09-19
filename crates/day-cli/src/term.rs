// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Centralized terminal styling for the day CLI.
//!
//! A single palette, referenced everywhere instead of inline `\x1b[..m` escape codes. Built on
//! [`anstyle`] (the styling vocabulary clap itself uses, already in our dependency tree), so the
//! CLI's output shares clap's color system. Print styled text with the [`anstream`]
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
//! need not each probe the tty. For richer terminal UI later (progress bars, concurrent task
//! spinners) the intended companions are `indicatif` + `console`; see the CLI output notes in
//! AGENTS.md.

use anstyle::{AnsiColor, Color, Effects, Style};

/// A plain foreground color on the default background: the building block for the palette.
const fn fg(color: AnsiColor) -> Style {
    Style::new().fg_color(Some(Color::Ansi(color)))
}

// Daybreak: warm sunlight for structure, sky blue for commands, and cyan for values.
// Use the terminal's ANSI palette so the theme works without truecolor support or a fixed
// background. Keep body text unstyled and errors/successes in their familiar semantic colors.
const SUN: Style = fg(AnsiColor::BrightYellow).bold();
const SKY_BOLD: Style = fg(AnsiColor::BrightCyan).bold();
const SKY: Style = fg(AnsiColor::Cyan);

/// Day's help and usage palette, inherited by every subcommand. Clap handles color detection.
pub const fn help_styles() -> clap::builder::Styles {
    clap::builder::Styles::styled()
        .header(SUN)
        .usage(SUN)
        .literal(SKY_BOLD)
        .placeholder(SKY.italic())
        .error(ERROR_BOLD)
        .valid(SUCCESS_BOLD)
        .invalid(WARN.bold())
}

/// A compact sunrise signature for the root help page. No emoji or background-color blocks;
/// it also reads cleanly when clap strips styling for a pipe or NO_COLOR.
pub fn help_banner() -> String {
    format!(
        "{SUN}   \\ | /{SUN:#}    {SKY_BOLD}d a y{SKY_BOLD:#}\n\
         {SUN}  ── ◒ ──{SUN:#}   {SKY}Rise and shine.{SKY:#}"
    )
}

/// A short, styled way into the command tree, shown only on the root help page.
pub fn help_footer() -> String {
    format!(
        "{SUN}Create an app:{SUN:#}  {SKY_BOLD}day new app{SKY_BOLD:#}\n\
         {SUN}Command help:{SUN:#}   {SKY_BOLD}day help{SKY_BOLD:#} {SKY}<COMMAND>{SKY:#}\n\n\
         Docs: https://daybrite.dev/docs/cli/"
    )
}

/// Daybreak status header: bold yellow (`   Launching`, `  Building`); the caller right-aligns.
pub const HEADER: Style = SUN;
/// Success: green (`✓`, "no findings").
pub const SUCCESS: Style = fg(AnsiColor::Green);
/// Emphatic success: bold green (summary "✓ all good").
pub const SUCCESS_BOLD: Style = fg(AnsiColor::Green).effects(Effects::BOLD);
/// Warning / advisory: yellow (`⚠`, "warning", the `▸` keep-alive note, the update banner).
pub const WARN: Style = fg(AnsiColor::Yellow);
/// Failure: red (`✗`).
pub const ERROR: Style = fg(AnsiColor::Red);
/// Emphatic failure: bold red (summary "✗ N error(s)").
pub const ERROR_BOLD: Style = fg(AnsiColor::Red).effects(Effects::BOLD);
/// De-emphasized: dimmed (setup hints, "n/a" lines, the scan preamble).
pub const DIM: Style = Style::new().effects(Effects::DIMMED);
/// Emphasis without color: bold (group labels).
pub const BOLD: Style = Style::new().effects(Effects::BOLD);
/// Forwarded app **stdout** line prefix `[target]`: blue. Used only for a line that isn't in
/// Day's `LEVEL target: message` format; anything that is gets colored by level instead.
pub const LOG_OUT: Style = fg(AnsiColor::Blue);
/// Forwarded app **stderr** line prefix `[target]`: yellow. Same fallback role as [`LOG_OUT`].
pub const LOG_ERR: Style = fg(AnsiColor::Yellow);

// Forwarded app log lines, colored by level (`ops::format_log`). This is `env_logger`'s palette
// rather than a new one, so a Day app's terminal reads like any other Rust app's.
/// A forwarded `ERROR` line: red.
pub const LOG_ERROR: Style = fg(AnsiColor::Red);
/// A forwarded `WARN` line: yellow.
pub const LOG_WARN: Style = fg(AnsiColor::Yellow);
/// A forwarded `INFO` line: green.
pub const LOG_INFO: Style = fg(AnsiColor::Green);
/// A forwarded `DEBUG` line: blue.
pub const LOG_DEBUG: Style = fg(AnsiColor::Blue);
/// A forwarded `TRACE` line: cyan.
pub const LOG_TRACE: Style = fg(AnsiColor::Cyan);
