//! Interactive terminal prompts for `day new` (DESIGN.md §8). These are the *fallback* branch of the
//! `day new` flag resolvers: when a value is not supplied on the command line and a terminal is
//! present, the corresponding question below fills it in. That is the whole flag↔dialog link — there
//! is no separate "wizard" code path that could drift from the flags (see `new.rs`).
//!
//! The prompts are driven by [`inquire`](https://github.com/mikaelmello/inquire): free text
//! ([`Text`]), single-choice ([`Select`]), and a checkbox multi-select ([`MultiSelect`], with
//! space to toggle, arrow keys to move, and type-to-filter). inquire renders to **stderr** (never
//! stdout), so `--format json` result events on stdout stay machine-parseable. When stdin/stderr is
//! not a TTY (CI, pipes), or `--no-input` / `DAY_NO_INPUT` is set, prompting is disabled and callers
//! fall back to defaults or a clean error instead of blocking on a read.

use std::io::{self, IsTerminal};

use inquire::{InquireError, MultiSelect, Select, Text};

/// Resolve an inquire result: Ctrl-C aborts the whole command (like any interrupted CLI); every
/// other outcome (Esc, EOF, a non-tty I/O error) falls back to the caller's default so the flow
/// degrades gracefully instead of failing.
fn resolve<T>(res: Result<T, InquireError>, fallback: impl FnOnce() -> T) -> T {
    match res {
        Ok(value) => value,
        Err(InquireError::OperationInterrupted) => {
            eprintln!();
            std::process::exit(130); // 128 + SIGINT
        }
        Err(_) => fallback(),
    }
}

/// A prompter that is `enabled` only when it is safe to block on interactive input.
pub struct Prompt {
    enabled: bool,
}

impl Prompt {
    /// `no_input` (the `--no-input` flag) or a non-TTY stdin/stderr (or `DAY_NO_INPUT` in the env)
    /// disables prompting.
    pub fn new(no_input: bool) -> Self {
        let enabled = !no_input
            && std::env::var_os("DAY_NO_INPUT").is_none()
            && io::stdin().is_terminal()
            && io::stderr().is_terminal();
        Prompt { enabled }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Free-text answer. With a `default`, empty input accepts it (shown in brackets by inquire); a
    /// required field (no default) re-asks until it gets a non-empty answer. Callers must check
    /// [`enabled`](Self::enabled) first when a value is mandatory and there is no default; disabled
    /// prompts return `default` (or empty).
    pub fn line(&self, question: &str, default: Option<&str>) -> String {
        if !self.enabled {
            return default.unwrap_or_default().to_string();
        }
        let text = match default {
            Some(d) => Text::new(question).with_default(d),
            // No default ⇒ mandatory: inquire re-asks on an empty submission.
            None => Text::new(question).with_validator(inquire::required!()),
        };
        resolve(text.prompt(), || default.unwrap_or_default().to_string())
    }

    /// Single choice among `options` (0-based `default` preselected). Returns the chosen index.
    /// Disabled ⇒ returns `default` without asking.
    pub fn choose(&self, question: &str, options: &[String], default: usize) -> usize {
        if !self.enabled || options.is_empty() {
            return default.min(options.len().saturating_sub(1));
        }
        let start = default.min(options.len() - 1);
        let picked = Select::new(question, options.to_vec())
            .with_starting_cursor(start)
            .raw_prompt();
        resolve(picked.map(|opt| opt.index), || start)
    }

    /// Multi choice — a checkbox list (space toggles, arrows move, typing filters). `preselected`
    /// indices start ticked. Returns the chosen indices (deduped, in menu order). An empty selection
    /// is allowed (mandatory callers reject it themselves). Disabled ⇒ returns `preselected`.
    pub fn choose_multi(
        &self,
        question: &str,
        options: &[String],
        preselected: &[usize],
    ) -> Vec<usize> {
        let normalize = |idxs: &[usize]| -> Vec<usize> {
            (0..options.len()).filter(|i| idxs.contains(i)).collect()
        };
        if !self.enabled || options.is_empty() {
            return normalize(preselected);
        }
        let picked = MultiSelect::new(question, options.to_vec())
            .with_default(preselected)
            .raw_prompt();
        resolve(
            picked.map(|opts| normalize(&opts.iter().map(|opt| opt.index).collect::<Vec<_>>())),
            || normalize(preselected),
        )
    }
}
