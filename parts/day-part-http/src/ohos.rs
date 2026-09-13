// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// HarmonyOS: the ArkTS Network Kit (`@ohos.net.http`) through this crate's bridge arm
// (`src/bridge.rs`, docs/bridge.md "Callbacks"). The OpenHarmony NDK has no HTTP C API — only
// websocket and TLS headers — so the request runs in ArkTS on the JS thread and answers through
// the bridge's completion token with the same envelope the Android arm packs. The asynchronous
// entry points are native; the blocking ones wait for that completion, and return `Unsupported`
// on the JS thread itself, where waiting would starve the loop that answers (the web rule).
// ---------------------------------------------------------------------------

use std::path::Path;
use std::sync::mpsc;

use super::{Download, HttpError, Request, Response, Tier};

/// Native when `day build` staged the ArkTS arm; a plain cargo build of this crate for the
/// target compiles the fallback, which reports no HTTP capability rather than a broken one.
pub const TIER: Tier = if cfg!(day_bridge_staged) {
    Tier::NativeStack
} else {
    Tier::Unavailable
};

/// A response envelope from the ArkTS arm as a [`Response`]: `status` i32 BE, `meta` length
/// i32 BE, the `k\nv\n…` header block, then the body — a negative status is a transport
/// sentinel whose message rides in the meta block (`DayEnvelope.error`'s layout).
pub fn response_from_envelope(bytes: &[u8]) -> Result<Response, HttpError> {
    let short = || HttpError::Io("short envelope".into());
    if bytes.len() < 8 {
        return Err(short());
    }
    let status = i32::from_be_bytes(bytes[0..4].try_into().map_err(|_| short())?);
    let meta_len = i32::from_be_bytes(bytes[4..8].try_into().map_err(|_| short())?).max(0) as usize;
    let rest = &bytes[8..];
    if rest.len() < meta_len {
        return Err(short());
    }
    let (meta, payload) = rest.split_at(meta_len);
    if status < 0 {
        let msg =
            String::from_utf8_lossy(if meta.is_empty() { payload } else { meta }).into_owned();
        return Err(match status {
            -1 => HttpError::Timeout,
            -2 => HttpError::Dns,
            -3 => HttpError::Tls(msg),
            -4 => HttpError::Connect,
            -6 => HttpError::BadUrl(msg),
            -7 => HttpError::Cancelled,
            _ => HttpError::Io(msg),
        });
    }
    let meta = String::from_utf8_lossy(meta);
    let mut lines = meta.split('\n');
    let mut headers = Vec::new();
    while let (Some(k), Some(v)) = (lines.next(), lines.next()) {
        if !k.is_empty() {
            headers.push((k.to_string(), v.to_string()));
        }
    }
    Ok(Response {
        status: status as u16,
        headers,
        body: payload.to_vec(),
    })
}

pub fn fetch(req: &Request) -> Result<Response, HttpError> {
    if day_bridge::arkts::on_js_thread() {
        return Err(HttpError::Unsupported);
    }
    let (tx, rx) = mpsc::channel();
    // A start failure has already sent the error through the callback.
    let _ = super::start_bridged(req, move |result| {
        let _ = tx.send(result);
    });
    rx.recv()
        .unwrap_or_else(|_| Err(HttpError::Io("the request never completed".into())))
}

pub fn fetch_to_file(req: &Request, dest: &Path) -> Result<Download, HttpError> {
    let resp = fetch(req)?;
    std::fs::write(dest, &resp.body).map_err(|e| HttpError::Io(e.to_string()))?;
    Ok(Download {
        status: resp.status,
        headers: resp.headers,
        bytes_written: resp.body.len() as u64,
    })
}

/// Buffered, then delivered as one chunk: the Network Kit's streaming form
/// (`requestInStream`) is not bridged yet, so a large body is held in memory here.
pub fn fetch_streamed(
    req: &Request,
    sink: &mut dyn super::StreamSink,
) -> Result<Download, HttpError> {
    let resp = fetch(req)?;
    if !sink.head(resp.status, &resp.headers) {
        return Err(HttpError::Io("aborted".into()));
    }
    if !resp.body.is_empty() {
        sink.chunk(&resp.body)?;
    }
    Ok(Download {
        status: resp.status,
        headers: resp.headers,
        bytes_written: resp.body.len() as u64,
    })
}
