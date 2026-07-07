// Desktop Linux: there is NO toolkit-independent native clipboard API — the clipboard lives in the
// display server, and GDK's accessor needs GTK initialized (which would break day-qt binaries). So
// this shells out to the session's standard clipboard tools instead: `wl-copy`/`wl-paste`
// (wl-clipboard) on Wayland, `xclip` on X11 — zero dependencies beyond std::process. The session
// type (WAYLAND_DISPLAY) picks which to try first; the other is the fallback.

use std::io::Write;
use std::process::{Command, Stdio};

/// Whether this looks like a Wayland session (X11 otherwise).
fn wayland_session() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
}

/// Run `cmd args…`, feeding `text` on stdin. True when the tool exists and exits 0.
fn pipe_in(cmd: &str, args: &[&str], text: &str) -> bool {
    let Ok(mut child) = Command::new(cmd)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false; // tool not installed
    };
    if let Some(mut stdin) = child.stdin.take()
        && stdin.write_all(text.as_bytes()).is_err()
    {
        let _ = child.kill();
        let _ = child.wait();
        return false;
    }
    child.wait().map(|s| s.success()).unwrap_or(false)
}

/// Run `cmd args…` and capture stdout. None when the tool is missing, fails, or the clipboard is
/// empty (both wl-paste and xclip exit non-zero on an empty clipboard).
fn read_out(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn set_text(text: &str) -> bool {
    let wl = || pipe_in("wl-copy", &["--type", "text/plain"], text);
    let x = || pipe_in("xclip", &["-selection", "clipboard", "-in"], text);
    if wayland_session() {
        wl() || x()
    } else {
        x() || wl()
    }
}

pub fn get_text() -> Option<String> {
    // --no-newline: wl-paste appends one otherwise; xclip -out is verbatim.
    let wl = || read_out("wl-paste", &["--no-newline"]);
    let x = || read_out("xclip", &["-selection", "clipboard", "-out"]);
    if wayland_session() {
        wl().or_else(x)
    } else {
        x().or_else(wl)
    }
}

pub fn has_text() -> bool {
    // No cheaper probe than reading (the tools exit non-zero when the clipboard is empty).
    get_text().is_some()
}
