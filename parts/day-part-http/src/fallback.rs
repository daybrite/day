// ---------------------------------------------------------------------------
// The Rust fallback tier — Linux desktop and HarmonyOS (neither has an OS-level HTTP API this
// crate can front): ureq + rustls, the tree's blessed minimal synchronous client (day-cli's
// update check). System awareness is limited to the `http_proxy`/`https_proxy`/`no_proxy`
// environment variables (no PAC, no desktop proxy settings) — `Tier::RustFallback` says so.
// An agent is built per request (per-request timeout with zero config gymnastics); connection
// pooling across requests is a non-goal for this tier.
// ---------------------------------------------------------------------------

use std::path::Path;

use super::{Download, HttpError, Request, Response, Tier};

pub const TIER: Tier = Tier::RustFallback;

fn map_err(e: ureq::Error) -> HttpError {
    use ureq::Error as E;
    match e {
        E::Timeout(_) => HttpError::Timeout,
        E::HostNotFound => HttpError::Dns,
        E::ConnectionFailed => HttpError::Connect,
        E::Tls(m) => HttpError::Tls(m.to_string()),
        E::BadUri(u) => HttpError::BadUrl(u.to_string()),
        E::Io(io) if io.kind() == std::io::ErrorKind::ConnectionRefused => HttpError::Connect,
        E::Io(io) if io.kind() == std::io::ErrorKind::TimedOut => HttpError::Timeout,
        other => HttpError::Io(other.to_string()),
    }
}

fn send(req: &Request) -> Result<ureq::http::Response<ureq::Body>, HttpError> {
    if !(req.url.starts_with("http://") || req.url.starts_with("https://")) {
        return Err(HttpError::BadUrl(req.url.clone()));
    }
    let mut cfg = ureq::Agent::config_builder()
        // 4xx/5xx are RESPONSES in this crate's contract, not errors.
        .http_status_as_error(false)
        // Bound resolve/connect/send/response-head, NOT the whole transfer — the native halves'
        // timeout is an idle/per-operation bound, so a multi-minute download must survive here
        // too (the body phase is uncapped; ureq has no per-chunk idle timer).
        .timeout_resolve(Some(req.timeout))
        .timeout_connect(Some(req.timeout))
        .timeout_send_request(Some(req.timeout))
        .timeout_recv_response(Some(req.timeout));
    if let Some(proxy) = ureq::Proxy::try_from_env() {
        cfg = cfg.proxy(Some(proxy));
    }
    let agent: ureq::Agent = cfg.build().into();

    let mut builder = ureq::http::Request::builder()
        .method(req.method.as_str())
        .uri(&req.url);
    for (k, v) in &req.headers {
        builder = builder.header(k, v);
    }
    let request = builder
        .body(req.body.clone().unwrap_or_default())
        .map_err(|e| HttpError::BadUrl(e.to_string()))?;
    agent.run(request).map_err(map_err)
}

fn parts_of(resp: &ureq::http::Response<ureq::Body>) -> (u16, Vec<(String, String)>) {
    let status = resp.status().as_u16();
    let headers = resp
        .headers()
        .iter()
        .map(|(k, v)| {
            (
                k.as_str().to_string(),
                String::from_utf8_lossy(v.as_bytes()).into_owned(),
            )
        })
        .collect();
    (status, headers)
}

pub fn fetch(req: &Request) -> Result<Response, HttpError> {
    let mut resp = send(req)?;
    let (status, headers) = parts_of(&resp);
    let body = resp
        .body_mut()
        .with_config()
        // The caller buffers into memory by contract; don't let ureq's default body cap
        // truncate silently (fetch_to_file is the big-download path).
        .limit(u64::MAX)
        .read_to_vec()
        .map_err(map_err)?;
    Ok(Response {
        status,
        headers,
        body,
    })
}

pub fn fetch_to_file(req: &Request, dest: &Path) -> Result<Download, HttpError> {
    let mut resp = send(req)?;
    let (status, headers) = parts_of(&resp);
    let mut file = std::fs::File::create(dest).map_err(|e| HttpError::Io(e.to_string()))?;
    let mut reader = resp.body_mut().with_config().limit(u64::MAX).reader();
    let bytes_written =
        std::io::copy(&mut reader, &mut file).map_err(|e| HttpError::Io(e.to_string()))?;
    Ok(Download {
        status,
        headers,
        bytes_written,
    })
}

pub fn fetch_streamed(
    req: &Request,
    sink: &mut dyn super::StreamSink,
) -> Result<Download, HttpError> {
    use std::io::Read;

    let mut resp = send(req)?;
    let (status, headers) = parts_of(&resp);
    if !sink.head(status, &headers) {
        return Err(HttpError::Io("aborted".into()));
    }
    let mut reader = resp.body_mut().with_config().limit(u64::MAX).reader();
    let mut chunk = vec![0u8; 65536];
    let mut bytes_written = 0u64;
    loop {
        let n = reader
            .read(&mut chunk)
            .map_err(|e| HttpError::Io(e.to_string()))?;
        if n == 0 {
            break;
        }
        sink.chunk(&chunk[..n])?;
        bytes_written += n as u64;
    }
    Ok(Download {
        status,
        headers,
        bytes_written,
    })
}
