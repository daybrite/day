// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Explicit conditional GET/HEAD for applications that store their own representations.
use crate::{Method, Request, Response};

/// Origin-supplied validators for one stored URL/representation. Keep these with the body:
/// a 304 has no replacement body. ETags are opaque, including quotes and the `W/` prefix.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CacheValidators {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

impl CacheValidators {
    pub fn is_empty(&self) -> bool {
        self.etag.as_deref().and_then(valid).is_none()
            && self.last_modified.as_deref().and_then(valid).is_none()
    }

    /// Learn validators after successfully storing a 2xx representation, or merge those on
    /// a 304 with the existing ones. Other statuses leave the cached validators unchanged.
    /// A new 2xx without validators clears old ones; they described the previous body.
    pub fn updated(&self, response: &Response) -> Self {
        let etag = response.header("etag").and_then(valid).map(str::to_owned);
        let modified = response
            .header("last-modified")
            .and_then(valid)
            .map(str::to_owned);
        match response.status {
            304 => Self {
                etag: etag.or_else(|| self.etag.clone()),
                last_modified: modified.or_else(|| self.last_modified.clone()),
            },
            200..=299 => Self {
                etag,
                last_modified: modified,
            },
            _ => self.clone(),
        }
    }
}

fn valid(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty() && value.len() <= 8192 && !value.bytes().any(|b| b < 0x20 || b == 0x7f))
        .then_some(value)
}

impl Request {
    /// Revalidate a locally stored GET/HEAD representation on every backend. Installs
    /// `If-None-Match` and `If-Modified-Since` (ETag takes precedence at compliant servers).
    /// Replaces existing copies of these headers. Does not add preconditions to mutations.
    /// This is independent of the platform HTTP cache and does not turn a 304 into a 200.
    pub fn conditional(mut self, validators: &CacheValidators) -> Self {
        if !matches!(self.method, Method::Get | Method::Head) {
            return self;
        }
        self.headers.retain(|(name, _)| {
            !name.eq_ignore_ascii_case("if-none-match")
                && !name.eq_ignore_ascii_case("if-modified-since")
        });
        if let Some(value) = validators.etag.as_deref().and_then(valid) {
            self = self.header("If-None-Match", value);
        }
        if let Some(value) = validators.last_modified.as_deref().and_then(valid) {
            self = self.header("If-Modified-Since", value);
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_opaque_etags_replaces_headers_and_keeps_304_metadata() {
        let old = CacheValidators {
            etag: Some("W/\"revision-1\"".into()),
            last_modified: Some("Wed, 30 Sep 2026 10:00:00 GMT".into()),
        };
        let req = Request::get("https://fixture.example/feed")
            .header("if-none-match", "old")
            .conditional(&old);
        assert_eq!(req.headers().len(), 2);
        assert_eq!(req.headers()[0].1, "W/\"revision-1\"");
        let response = Response::new(304, vec![("ETag".into(), "\"revision-2\"".into())], vec![]);
        let next = old.updated(&response);
        assert_eq!(next.etag.as_deref(), Some("\"revision-2\""));
        assert_eq!(next.last_modified, old.last_modified);
        assert!(old.updated(&Response::new(200, vec![], vec![])).is_empty());
        assert_eq!(old.updated(&Response::new(503, vec![], vec![])), old);
    }
    #[test]
    fn ignores_unsafe_values_and_does_not_change_mutating_requests() {
        let validators = CacheValidators {
            etag: Some("\"tag\"\r\nX-Injected: value".into()),
            last_modified: Some(String::new()),
        };
        assert!(validators.is_empty());
        assert!(
            Request::get("https://fixture.example")
                .conditional(&validators)
                .headers()
                .is_empty()
        );
        let mut req = Request::get("https://fixture.example");
        req.method = Method::Post;
        assert!(
            req.conditional(&CacheValidators {
                etag: Some("\"tag\"".into()),
                last_modified: None
            })
            .headers()
            .is_empty()
        );
    }
}
