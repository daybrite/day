// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// Desktop Linux: there is no toolkit-independent native clipboard API. The clipboard lives in the
// display server, and GDK's accessor needs GTK initialized (which would break day-qt binaries). So
// this shells out to the session's standard clipboard tools instead: `wl-copy`/`wl-paste`
// (wl-clipboard) on Wayland, `xclip` on X11, with zero dependencies beyond std::process. The
// session type (WAYLAND_DISPLAY) picks which to try first; the other is the fallback.

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
    // Try the session's native tool first, the other as fallback. Selecting the order up front,
    // rather than a bare `wl()||x()` vs `x()||wl()` in each branch, keeps
    // clippy::if_same_then_else, which normalizes commutative `||`, from collapsing the two arms
    // into "identical blocks".
    let (first, second): (&dyn Fn() -> bool, &dyn Fn() -> bool) = if wayland_session() {
        (&wl, &x)
    } else {
        (&x, &wl)
    };
    first() || second()
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

use crate::{Content, Error, MAX_BYTES, Representation};
pub fn write_content(content: &Content) -> Result<Vec<String>, Error> {
    // Session tools own one target; prefer a standard image representation when offered.
    let r = content
        .0
        .iter()
        .find(|r| r.mime == "image/png")
        .or_else(|| content.0.iter().find(|r| r.mime == "image/svg+xml"))
        .or_else(|| content.0.iter().find(|r| r.mime == "text/plain"))
        .unwrap_or(&content.0[0]);
    let put = |cmd: &str, args: &[&str]| {
        let Ok(mut child) = Command::new(cmd)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            return false;
        };
        let ok = child
            .stdin
            .take()
            .is_some_and(|mut s| s.write_all(&r.bytes).is_ok());
        if !ok {
            let _ = child.kill();
        }
        child.wait().is_ok_and(|s| ok && s.success())
    };
    let wl = || put("wl-copy", &["--type", &r.mime]);
    let x = || {
        put(
            "xclip",
            &["-selection", "clipboard", "-in", "-target", &r.mime],
        )
    };
    let ok = if wayland_session() {
        wl() || x()
    } else {
        x() || wl()
    };
    if ok {
        Ok(vec![r.mime.clone()])
    } else {
        Err(Error::Unavailable)
    }
}
pub fn read_content(preferred: &[&str]) -> Result<Option<Representation>, Error> {
    use std::io::Read;
    for mime in preferred {
        let read = |cmd: &str, args: &[&str]| -> Result<Option<Vec<u8>>, Error> {
            let Ok(mut child) = Command::new(cmd)
                .args(args)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
            else {
                return Ok(None);
            };
            let mut bytes = Vec::new();
            let ok = child
                .stdout
                .take()
                .unwrap()
                .take(MAX_BYTES as u64 + 1)
                .read_to_end(&mut bytes)
                .is_ok();
            if bytes.len() > MAX_BYTES {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Error::TooLarge);
            }
            Ok(child
                .wait()
                .is_ok_and(|s| ok && s.success())
                .then_some(bytes))
        };
        let wl = || read("wl-paste", &["--no-newline", "--type", mime]);
        let x = || {
            read(
                "xclip",
                &["-selection", "clipboard", "-out", "-target", mime],
            )
        };
        let bytes = if wayland_session() {
            match wl()? {
                some @ Some(_) => some,
                None => x()?,
            }
        } else {
            match x()? {
                some @ Some(_) => some,
                None => wl()?,
            }
        };
        if let Some(bytes) = bytes {
            return Ok(Some(Representation::new(*mime, bytes)));
        }
    }
    Ok(None)
}
