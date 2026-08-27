// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Bake a rich version string into the `day` binary: `<version>[*] (<profile>[, <git ref>])`, where the
//! trailing `*` marks a debug build and the git ref names what HEAD was at build time — always
//! including the COMMIT, and the tag or branch as well when there is one.
//!
//! This is purely additive metadata — it never affects the binary at runtime. Off a git checkout (e.g.
//! a crates.io build), the git lookups fail and the ref is simply omitted (`0.0.3 (release)`), so the CLI
//! stays fully portable.

use std::process::Command;

/// Run `git <args>` and return its trimmed stdout, or `None` if git is absent / the command failed /
/// output is empty (e.g. building outside a git checkout).
fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// The ref HEAD points at, ALWAYS ending in the commit: an exact tag or a named branch when there
/// is one, and the short SHA either way.
///
/// The name is a hint; the SHA is the fact. Two things made that distinction matter. A branch name
/// does not identify a build — every commit on `main` used to bake the same
/// `(release, branch main)`, so `day rebuild`, which compares recorded tool versions strictly,
/// read two different CLIs as the same one. And the name is not always even the right one: cargo
/// names the local branch of its own git checkout, so `cargo install --git --branch main` reported
/// `branch master` on a Windows runner while being a correct build of main.
///
/// No SHA means no git, which is a crates.io build — the ref is then omitted entirely, as before.
fn git_ref() -> Option<String> {
    let sha = git(&["rev-parse", "--short", "HEAD"])?;
    if let Some(tag) = git(&["describe", "--tags", "--exact-match", "HEAD"]) {
        return Some(format!("tag {tag}, {sha}"));
    }
    if let Some(branch) = git(&["symbolic-ref", "--short", "-q", "HEAD"]) {
        return Some(format!("branch {branch}, {sha}"));
    }
    Some(format!("commit {sha}"))
}

fn main() {
    // Rebuild when the checked-out ref changes so the baked ref stays accurate (best-effort; these
    // paths simply don't exist for a crates.io tarball, which has no .git).
    if let Some(gitdir) = git(&["rev-parse", "--absolute-git-dir"]) {
        println!("cargo:rerun-if-changed={gitdir}/HEAD");
        println!("cargo:rerun-if-changed={gitdir}/packed-refs");
        if let Some(head_ref) = git(&["symbolic-ref", "-q", "HEAD"]) {
            println!("cargo:rerun-if-changed={gitdir}/{head_ref}");
        }
    }
    println!("cargo:rerun-if-env-changed=PROFILE");

    let profile = std::env::var("PROFILE").unwrap_or_else(|_| "debug".into());
    let star = if profile == "debug" { "*" } else { "" };
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let git_suffix = git_ref().map(|r| format!(", {r}")).unwrap_or_default();
    // e.g. "0.0.3* (debug, branch main, 9b8c387)" · "0.0.3 (release, tag v0.0.3, 1f2e3d4)"
    //    · "0.0.3 (release, commit 9b8c387)" (detached) · "0.0.3 (release)" (no git checkout)
    println!("cargo:rustc-env=DAY_VERSION_LONG={version}{star} ({profile}{git_suffix})");
}
