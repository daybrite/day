//! Interactive terminal prompts for `day new` (DESIGN.md §8). These are the *fallback* branch of the
//! `day new` flag resolvers: when a value is not supplied on the command line and a terminal is
//! present, the corresponding question below fills it in. That is the whole flag↔dialog link — there
//! is no separate "wizard" code path that could drift from the flags (see `new.rs`).
//!
//! Everything is written to **stderr** (never stdout) so `--format json` result events on stdout stay
//! machine-parseable, and reads from stdin. When stdin/stderr is not a TTY (CI, pipes) prompting is
//! disabled and callers fall back to defaults or a clean error instead of blocking on a read.

use std::io::{self, BufRead, IsTerminal, Write};

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

    fn read_line() -> Option<String> {
        let mut s = String::new();
        match io::stdin().lock().read_line(&mut s) {
            Ok(0) => None, // EOF
            Ok(_) => Some(s.trim().to_string()),
            Err(_) => None,
        }
    }

    /// Free-text answer. Empty input accepts `default` (shown in brackets). A required field (no
    /// default) re-asks until it gets a non-empty answer. Callers must check [`enabled`] first when a
    /// value is mandatory and there is no default; disabled prompts return `default` (or empty).
    pub fn line(&self, question: &str, default: Option<&str>) -> String {
        if !self.enabled {
            return default.unwrap_or_default().to_string();
        }
        loop {
            match default {
                Some(d) => eprint!("{question} [{d}]: "),
                None => eprint!("{question}: "),
            }
            let _ = io::stderr().flush();
            match Self::read_line() {
                None => return default.unwrap_or_default().to_string(), // EOF: stop asking
                Some(s) if s.is_empty() => {
                    if let Some(d) = default {
                        return d.to_string();
                    }
                    // required — re-ask
                }
                Some(s) => return s,
            }
        }
    }

    /// Single choice among `options` (0-based `default` preselected). Returns the chosen index.
    /// Disabled ⇒ returns `default` without asking.
    pub fn choose(&self, question: &str, options: &[String], default: usize) -> usize {
        if !self.enabled || options.is_empty() {
            return default.min(options.len().saturating_sub(1));
        }
        eprintln!("{question}");
        for (i, opt) in options.iter().enumerate() {
            eprintln!("  {}. {opt}", i + 1);
        }
        loop {
            eprint!("Choose [{}]: ", default + 1);
            let _ = io::stderr().flush();
            match Self::read_line() {
                None => return default,
                Some(s) if s.is_empty() => return default,
                Some(s) => match s.parse::<usize>() {
                    Ok(n) if (1..=options.len()).contains(&n) => return n - 1,
                    _ => eprintln!("  please enter a number from 1 to {}", options.len()),
                },
            }
        }
    }

    /// Multi choice. Accepts a comma/space-separated list of numbers, `all`, or `none`; empty input
    /// accepts `preselected`. Returns the chosen indices (deduped, in menu order). Disabled ⇒ returns
    /// `preselected`.
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
        let default_label = if preselected.is_empty() {
            "none".to_string()
        } else {
            preselected
                .iter()
                .map(|i| (i + 1).to_string())
                .collect::<Vec<_>>()
                .join(",")
        };
        eprintln!("{question}");
        for (i, opt) in options.iter().enumerate() {
            let mark = if preselected.contains(&i) { "x" } else { " " };
            eprintln!("  [{mark}] {}. {opt}", i + 1);
        }
        eprintln!("  (enter numbers separated by commas, or `all` / `none`)");
        loop {
            eprint!("Select [{default_label}]: ");
            let _ = io::stderr().flush();
            let ans = match Self::read_line() {
                None => return normalize(preselected),
                Some(s) => s,
            };
            if ans.is_empty() {
                return normalize(preselected);
            }
            let lower = ans.to_ascii_lowercase();
            if lower == "all" {
                return (0..options.len()).collect();
            }
            if lower == "none" {
                return Vec::new();
            }
            let mut picked = Vec::new();
            let mut ok = true;
            for tok in ans.split([',', ' ']).filter(|t| !t.is_empty()) {
                match tok.parse::<usize>() {
                    Ok(n) if (1..=options.len()).contains(&n) => {
                        if !picked.contains(&(n - 1)) {
                            picked.push(n - 1);
                        }
                    }
                    _ => {
                        eprintln!("  `{tok}` is not a choice from 1 to {}", options.len());
                        ok = false;
                        break;
                    }
                }
            }
            if ok {
                if picked.is_empty() {
                    // A non-empty answer that parsed to nothing (e.g. a lone `,`) — re-ask rather
                    // than return an empty selection, which mandatory callers treat as fatal.
                    eprintln!("  enter at least one number, or `all` / `none`");
                    continue;
                }
                return normalize(&picked);
            }
        }
    }
}
