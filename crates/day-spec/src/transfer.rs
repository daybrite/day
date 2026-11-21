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
    #[must_use]
    pub fn new(representations: Vec<Representation>) -> Self {
        Self { representations }
    }
    #[must_use]
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
    #[must_use]
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
impl Drop {
    /// The local files among the dropped items, in order: every `text/uri-list`
    /// representation read through [`file_paths`]. Empty when the drop carried none.
    #[must_use]
    pub fn file_paths(&self) -> Vec<std::path::PathBuf> {
        self.items
            .iter()
            .filter_map(|item| item.get("text/uri-list"))
            .filter_map(file_paths)
            .flatten()
            .collect()
    }
}
/// Called on the UI thread at native drag start, outside any tree borrow.
pub type Source = Rc<dyn Fn(Point) -> Option<Offer>>;
type Policy = Rc<dyn Fn(&Location) -> Operation>;
type OnEnter = Rc<dyn Fn(&Location)>;
/// What [`Target::guarded`] runs each callback through: it is handed the callback as a thunk
/// and decides whether, and in which scope, to run it.
pub type Guard = Rc<dyn Fn(&mut dyn FnMut())>;

/// A drop destination: the types it takes, its acceptance policy, and what runs as a drag
/// enters, leaves and drops (docs/drag-and-drop.md "Accepting drops").
///
/// ```ignore
/// DropTarget::new(["text/uri-list"])
///     .on_enter(move |_| hovering.set(true))
///     .on_leave(move || hovering.set(false))
///     .on_drop(move |drop| { files.set(drop.file_paths()); true })
/// ```
///
/// The policy ([`Target::accept`]) is consulted during hover AND again at drop; without one a
/// target takes a copy of any offer that carries one of its types. Every `on_enter` is
/// followed by exactly one `on_leave`: when the drag moves off the target, when the session
/// is cancelled, or right after the drop is delivered, so a highlight needs the pair and
/// nothing else. `on_drop` returns whether the owned data was accepted. Do not delete a
/// source based on asynchronous work started there: external transfers currently advertise
/// Copy only.
///
/// A backend holds the target it was given and calls [`Target::proposal`] as the drag moves,
/// [`Target::deliver`] at the drop and [`Target::leave`] when the session leaves the view or
/// is cancelled; the enter and leave callbacks are derived from those three calls.
#[derive(Clone)]
pub struct Target {
    /// The MIME types the target registers for with the platform.
    pub types: Vec<String>,
    accept: Option<Policy>,
    enter: Option<OnEnter>,
    leave: Option<Rc<dyn Fn()>>,
    receive: Rc<dyn Fn(Drop) -> bool>,
    /// Whether a drag is over the target, shared by every clone a backend keeps.
    hovering: Rc<std::cell::Cell<bool>>,
}
impl Target {
    /// A target for offers that carry any of `types`. It accepts nothing until
    /// [`Target::on_drop`] gives it a receiver.
    pub fn new<S: Into<String>>(types: impl IntoIterator<Item = S>) -> Self {
        Self {
            types: types.into_iter().map(Into::into).collect(),
            accept: None,
            enter: None,
            leave: None,
            receive: Rc::new(|_| false),
            hovering: Rc::new(std::cell::Cell::new(false)),
        }
    }
    /// A target for file references (`text/uri-list`).
    #[must_use]
    pub fn files() -> Self {
        Self::new(["text/uri-list"])
    }
    /// Decide what a drag at `location` would do: called during hover and again at the drop.
    /// [`Operation::None`] refuses this location. Without a policy the target proposes
    /// [`Operation::Copy`] for an offer that carries one of its types.
    #[must_use]
    pub fn accept(mut self, policy: impl Fn(&Location) -> Operation + 'static) -> Self {
        self.accept = Some(Rc::new(policy));
        self
    }
    /// Run when a drag the target would accept comes over it.
    #[must_use]
    pub fn on_enter(mut self, f: impl Fn(&Location) + 'static) -> Self {
        self.enter = Some(Rc::new(f));
        self
    }
    /// Run when the hover ends: the drag left, the session was cancelled, or the drop was
    /// delivered. Paired with [`Target::on_enter`].
    #[must_use]
    pub fn on_leave(mut self, f: impl Fn() + 'static) -> Self {
        self.leave = Some(Rc::new(f));
        self
    }
    /// Take the dropped items. Return whether they were accepted.
    #[must_use]
    pub fn on_drop(mut self, f: impl Fn(Drop) -> bool + 'static) -> Self {
        self.receive = Rc::new(f);
        self
    }
    /// This target with every callback run through `guard`, sharing the hover state: how the
    /// pieces layer scopes the callbacks to the piece that declared them. `guard` is handed
    /// each callback as a thunk and decides whether, and in which scope, to run it.
    #[must_use]
    pub fn guarded(self, guard: Guard) -> Self {
        let Target {
            types,
            accept,
            enter,
            leave,
            receive,
            hovering,
        } = self;
        let g = guard.clone();
        let accept = accept.map(|f| -> Policy {
            Rc::new(move |location| {
                let mut op = Operation::None;
                g(&mut || op = f(location));
                op
            })
        });
        let g = guard.clone();
        let enter = enter.map(|f| -> OnEnter { Rc::new(move |location| g(&mut || f(location))) });
        let g = guard.clone();
        let leave = leave.map(|f| -> Rc<dyn Fn()> { Rc::new(move || g(&mut || f())) });
        let receive: Rc<dyn Fn(Drop) -> bool> = Rc::new(move |drop| {
            let mut taken = false;
            let mut drop = Some(drop);
            guard(&mut || {
                if let Some(drop) = drop.take() {
                    taken = receive(drop);
                }
            });
            taken
        });
        Target {
            types,
            accept,
            enter,
            leave,
            receive,
            hovering,
        }
    }
    /// Whether a drag is over the target right now.
    #[must_use]
    pub fn is_hovering(&self) -> bool {
        self.hovering.get()
    }
    /// What a drag at `location` would do, for the platform's hover feedback. Marks the
    /// hover: the first accepting proposal of a session runs `on_enter`, and a proposal that
    /// turns to [`Operation::None`] runs `on_leave`.
    #[must_use]
    pub fn proposal(&self, location: &Location) -> Operation {
        let op = self.decide(location);
        if op == Operation::None {
            self.leave();
        } else if !self.hovering.replace(true)
            && let Some(enter) = &self.enter
        {
            crate::ffi_guard::contain((), || enter(location));
        }
        op
    }
    /// The drag left the target or the session was cancelled. A backend calls it from the
    /// platform's drag-exit and drag-end callbacks; it is idempotent, and it runs `on_leave`
    /// only when a hover was under way.
    pub fn leave(&self) {
        if self.hovering.replace(false)
            && let Some(leave) = &self.leave
        {
            crate::ffi_guard::contain((), || leave());
        }
    }
    /// The drop: consult the policy once more, hand the items to `on_drop` when it accepts,
    /// then end the hover. Returns whether the data was taken.
    #[must_use]
    #[expect(
        clippy::needless_pass_by_value,
        reason = "public signature; toolkits pass the location they built for this drop"
    )]
    pub fn deliver(&self, location: Location, offer: Offer) -> bool {
        let op = self.decide(&location);
        let taken = op != Operation::None
            && crate::ffi_guard::contain(false, || {
                (self.receive)(Drop {
                    local: location.local,
                    position: location.position,
                    operation: op,
                    items: offer.items,
                })
            });
        self.leave();
        taken
    }
    fn decide(&self, location: &Location) -> Operation {
        let op = match &self.accept {
            Some(accept) => crate::ffi_guard::contain(Operation::None, || accept(location)),
            None if self.types.iter().any(|t| location.has(t)) => Operation::Copy,
            None => Operation::None,
        };
        if location.allowed.contains(&op) {
            op
        } else {
            Operation::None
        }
    }
}

/// Bounded, versioned transport for multiple items through native MIME providers. Standard
/// image and file formats are published alongside this, for applications that do not know Day.
impl Offer {
    #[must_use]
    pub fn types(&self) -> Vec<String> {
        let mut out = Vec::new();
        for r in self.items.iter().flat_map(|i| &i.representations) {
            if !out.contains(&r.mime) {
                out.push(r.mime.clone());
            }
        }
        out
    }
    #[must_use]
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
    #[must_use]
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
#[must_use]
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
        let target = Target::new(Vec::<String>::new())
            .accept(|p| {
                if p.position.x < 50.0 {
                    Operation::Copy
                } else {
                    Operation::Move
                }
            })
            .on_drop(|_| true);
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
    #[test]
    fn a_target_without_a_policy_copies_its_own_types_and_reads_dropped_files() {
        use std::cell::RefCell;
        let got = Rc::new(RefCell::new(Vec::new()));
        let sink = got.clone();
        let target = Target::files().on_drop(move |drop| {
            *sink.borrow_mut() = drop.file_paths();
            true
        });
        let at = |types: &[&str]| Location {
            position: Point::ZERO,
            types: types.iter().map(|t| (*t).to_owned()).collect(),
            allowed: vec![Operation::Copy, Operation::Move],
            local: false,
        };
        assert_eq!(target.proposal(&at(&["text/plain"])), Operation::None);
        assert!(!target.is_hovering());
        assert_eq!(target.proposal(&at(&["text/uri-list"])), Operation::Copy);
        assert!(target.is_hovering());
        let offer = Offer {
            items: vec![Item::new(vec![Representation::new(
                "text/uri-list",
                "file:///tmp/a.txt\r\nfile:///tmp/b.txt\r\n",
            )])],
        };
        assert!(target.deliver(at(&["text/uri-list"]), offer));
        assert!(!target.is_hovering(), "a drop ends the hover");
        assert_eq!(
            *got.borrow(),
            vec![
                std::path::PathBuf::from("/tmp/a.txt"),
                std::path::PathBuf::from("/tmp/b.txt")
            ]
        );
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
