// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Native drag data and synchronous destination policy. No clipboard or process-local transport.
use crate::Point;
use std::{rc::Rc, sync::Arc};

pub const BUNDLE_MIME: &str = "application/vnd.day.transfer";
pub const MAX_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_ITEMS: usize = 256;

/// One representation of an item. File references use standard `text/uri-list` bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Representation {
    pub mime: String,
    pub bytes: Arc<Vec<u8>>,
}
impl Representation {
    pub fn new(mime: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            mime: mime.into(),
            bytes: Arc::new(bytes.into()),
        }
    }
}
/// Representations in an item are alternatives; items in an offer are separate objects.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Item {
    pub representations: Vec<Representation>,
}
impl Item {
    pub fn new(representations: Vec<Representation>) -> Self {
        Self { representations }
    }
    pub fn get(&self, mime: &str) -> Option<&[u8]> {
        self.representations
            .iter()
            .find(|r| r.mime == mime)
            .map(|r| r.bytes.as_slice())
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Operation {
    #[default]
    None,
    Copy,
    Move,
    Link,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Offer {
    pub items: Vec<Item>,
}

#[derive(Clone, Debug)]
pub struct Location {
    /// Target-local logical coordinates, not screen pixels.
    pub position: Point,
    pub types: Vec<String>,
    pub allowed: Vec<Operation>,
    /// Provenance supplied by the native session, never decoded from a payload.
    pub local: bool,
}
impl Location {
    pub fn has(&self, mime: &str) -> bool {
        self.types.iter().any(|m| m == mime)
    }
}
#[derive(Clone, Debug)]
pub struct Drop {
    pub local: bool,
    pub position: Point,
    pub operation: Operation,
    pub items: Vec<Item>,
}
/// Called on the UI thread at native drag start, outside any tree borrow.
pub type Source = Rc<dyn Fn(Point) -> Option<Offer>>;
/// Policy is consulted during hover AND again at drop. None rejects this location.
/// `receive` returns whether the owned data was accepted. Do not delete a source based on
/// asynchronous work started here: external transfers currently advertise Copy only.
#[derive(Clone)]
pub struct Target {
    pub types: Vec<String>,
    pub accept: Rc<dyn Fn(&Location) -> Operation>,
    pub receive: Rc<dyn Fn(Drop) -> bool>,
}
impl Target {
    pub fn proposal(&self, location: &Location) -> Operation {
        let op = crate::ffi_guard::contain(Operation::None, || (self.accept)(location));
        if location.allowed.contains(&op) {
            op
        } else {
            Operation::None
        }
    }
    pub fn deliver(&self, location: Location, offer: Offer) -> bool {
        let op = self.proposal(&location);
        op != Operation::None
            && crate::ffi_guard::contain(false, || {
                (self.receive)(Drop {
                    local: location.local,
                    position: location.position,
                    operation: op,
                    items: offer.items,
                })
            })
    }
}

/// Bounded, versioned transport for multiple items through native MIME providers. Standard
/// image and file formats are published alongside this, for applications that do not know Day.
impl Offer {
    pub fn types(&self) -> Vec<String> {
        let mut out = Vec::new();
        for r in self.items.iter().flat_map(|i| &i.representations) {
            if !out.contains(&r.mime) {
                out.push(r.mime.clone());
            }
        }
        out
    }
    pub fn encode(&self) -> Option<Vec<u8>> {
        if self.items.is_empty() || self.items.len() > MAX_ITEMS {
            return None;
        }
        let mut out = b"DAYDND\0\x01".to_vec();
        out.extend_from_slice(&(self.items.len() as u32).to_le_bytes());
        for item in &self.items {
            if item.representations.is_empty() || item.representations.len() > 32 {
                return None;
            }
            out.extend_from_slice(&(item.representations.len() as u32).to_le_bytes());
            let mut seen = std::collections::HashSet::new();
            for r in &item.representations {
                if !valid_mime(&r.mime) || r.mime == BUNDLE_MIME || !seen.insert(&r.mime) {
                    return None;
                }
                if out
                    .len()
                    .checked_add(r.mime.len() + 8)?
                    .checked_add(r.bytes.len())?
                    > MAX_BYTES
                {
                    return None;
                }
                out.extend_from_slice(&(r.mime.len() as u32).to_le_bytes());
                out.extend_from_slice(&(r.bytes.len() as u32).to_le_bytes());
                out.extend_from_slice(r.mime.as_bytes());
                out.extend_from_slice(&r.bytes);
            }
        }
        Some(out)
    }
    pub fn decode(mut bytes: &[u8]) -> Option<Self> {
        fn take<'a>(b: &mut &'a [u8], n: usize) -> Option<&'a [u8]> {
            let (a, rest) = b.split_at_checked(n)?;
            *b = rest;
            Some(a)
        }
        fn number(b: &mut &[u8]) -> Option<usize> {
            Some(u32::from_le_bytes(take(b, 4)?.try_into().ok()?) as usize)
        }
        if bytes.len() > MAX_BYTES || take(&mut bytes, 8)? != b"DAYDND\0\x01" {
            return None;
        }
        let count = number(&mut bytes)?;
        if count == 0 || count > MAX_ITEMS {
            return None;
        }
        let mut items = Vec::new();
        for _ in 0..count {
            let count = number(&mut bytes)?;
            if count == 0 || count > 32 {
                return None;
            }
            let mut representations = Vec::new();
            for _ in 0..count {
                let m = number(&mut bytes)?;
                let n = number(&mut bytes)?;
                if m > 255 {
                    return None;
                }
                let mime = std::str::from_utf8(take(&mut bytes, m)?).ok()?;
                if !valid_mime(mime)
                    || mime == BUNDLE_MIME
                    || representations
                        .iter()
                        .any(|r: &Representation| r.mime == mime)
                {
                    return None;
                }
                representations.push(Representation::new(mime, take(&mut bytes, n)?));
            }
            items.push(Item { representations });
        }
        if !bytes.is_empty() {
            return None;
        }
        Some(Self { items })
    }
}
pub fn valid_mime(m: &str) -> bool {
    m.len() <= 255
        && m.split_once('/')
            .is_some_and(|(a, b)| !a.is_empty() && !b.is_empty() && !b.contains('/'))
        && m.bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"/!#$&^_.+-".contains(&c))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn items_and_alternatives_survive_untrusted_transport() {
        let offer = Offer {
            items: vec![
                Item::new(vec![
                    Representation::new("image/png", [0, 255]),
                    Representation::new("application/x-test", [9, 0]),
                ]),
                Item::new(vec![Representation::new(
                    "text/uri-list",
                    b"file:///tmp/a%20b\r\n",
                )]),
            ],
        };
        let encoded = offer.encode().unwrap();
        assert_eq!(Offer::decode(&encoded), Some(offer));
        for n in 0..encoded.len() {
            assert!(Offer::decode(&encoded[..n]).is_none());
        }
        let mut extra = encoded;
        extra.push(0);
        assert!(Offer::decode(&extra).is_none());
    }
    #[test]
    fn location_guard_is_rechecked_and_cannot_escalate_operations() {
        let target = Target {
            types: vec![],
            accept: Rc::new(|p| {
                if p.position.x < 50.0 {
                    Operation::Copy
                } else {
                    Operation::Move
                }
            }),
            receive: Rc::new(|_| true),
        };
        let mut p = Location {
            position: Point::new(5., 5.),
            types: vec![],
            allowed: vec![Operation::Copy],
            local: true,
        };
        assert_eq!(target.proposal(&p), Operation::Copy);
        p.position.x = 80.;
        assert!(!target.deliver(p, Offer::default()));
    }
}

/// Parse native URI-list records without treating remote or opaque URIs as filesystem paths.
/// Callers still need the platform's access lease for sandboxed/provider-backed references.
pub fn file_paths(bytes: &[u8]) -> Option<Vec<std::path::PathBuf>> {
    let text = std::str::from_utf8(bytes).ok()?;
    let mut paths = Vec::new();
    for line in text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        let uri = line.strip_prefix("file://")?;
        let path = if uri.starts_with('/') {
            uri
        } else {
            uri.strip_prefix("localhost/")
                .map(|p| &uri[uri.len() - p.len() - 1..])?
        };
        if path.contains(['?', '#']) {
            return None;
        }
        let mut decoded = Vec::with_capacity(path.len());
        let mut raw = path.bytes();
        while let Some(b) = raw.next() {
            let b = if b == b'%' {
                let hi = (raw.next()? as char).to_digit(16)?;
                let lo = (raw.next()? as char).to_digit(16)?;
                (hi * 16 + lo) as u8
            } else {
                b
            };
            if b == 0 {
                return None;
            }
            decoded.push(b);
        }
        #[cfg(unix)]
        let path = {
            use std::os::unix::ffi::OsStringExt;
            std::path::PathBuf::from(std::ffi::OsString::from_vec(decoded))
        };
        #[cfg(not(unix))]
        let path = {
            let s = String::from_utf8(decoded).ok()?;
            #[cfg(windows)]
            let s = if s.as_bytes().get(2) == Some(&b':') {
                s[1..].to_string()
            } else {
                s
            };
            std::path::PathBuf::from(s)
        };
        paths.push(path);
        if paths.len() > MAX_ITEMS {
            return None;
        }
    }
    (!paths.is_empty()).then_some(paths)
}

#[cfg(test)]
mod file_tests {
    use super::*;
    #[test]
    fn uri_references_decode_without_fetching_remote_resources() {
        assert_eq!(
            file_paths(b"# files\r\nfile:///tmp/a%20b%23c.png\r\nfile://localhost/tmp/two\r\n")
                .unwrap(),
            vec![
                std::path::PathBuf::from("/tmp/a b#c.png"),
                std::path::PathBuf::from("/tmp/two")
            ]
        );
        for s in [
            "https://example.com/a",
            "file://remote/a",
            "file:///tmp/%00",
            "file:///tmp/%xx",
            "file:///tmp/a#fragment",
        ] {
            assert!(file_paths(s.as_bytes()).is_none());
        }
    }
}
