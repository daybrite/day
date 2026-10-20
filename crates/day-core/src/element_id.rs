// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Declared element identities. `id!` is the only declaration surface; the catalog is
//! collected at link time, including declarations in helpers which have never mounted.
use std::borrow::Cow;
use std::collections::BTreeMap;

/// An application-owned element identity, declared with [`crate::id!`].
#[derive(Clone, Debug)]
pub struct ElementId {
    declaration: &'static Declaration,
    text: Cow<'static, str>,
}
impl PartialEq for ElementId {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text
    }
}
impl Eq for ElementId {}

impl ElementId {
    #[doc(hidden)]
    pub fn from_declaration(declaration: &'static Declaration) -> Self {
        Self {
            declaration,
            text: Cow::Borrowed(declaration.family),
        }
    }
    #[doc(hidden)]
    pub fn with_key(declaration: &'static Declaration, key: impl IntoElementKey) -> Self {
        let mut text = String::from(declaration.family);
        text.push(':');
        for byte in key.into_element_key().bytes() {
            if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
                text.push(char::from(byte));
            } else {
                const HEX: &[u8] = b"0123456789ABCDEF";
                text.push('%');
                text.push(char::from(HEX[(byte >> 4) as usize]));
                text.push(char::from(HEX[(byte & 15) as usize]));
            }
        }
        Self {
            declaration,
            text: Cow::Owned(text),
        }
    }
    pub fn as_str(&self) -> &str {
        &self.text
    }
    pub fn family(&self) -> &'static str {
        self.declaration.family
    }
    pub fn is_keyed(&self) -> bool {
        self.declaration.keyed
    }
}

/// Explicit, stable canonical text for an instance key. No blanket Display conversion.
/// Integer 42 and text "42" intentionally denote the same key. Composite keys must
/// define an unambiguous app-owned encoding before the framework percent-escapes it.
pub trait IntoElementKey {
    fn into_element_key(self) -> String;
}
impl IntoElementKey for String {
    fn into_element_key(self) -> String {
        self
    }
}
impl IntoElementKey for &str {
    fn into_element_key(self) -> String {
        self.to_owned()
    }
}
impl IntoElementKey for &String {
    fn into_element_key(self) -> String {
        self.clone()
    }
}
macro_rules! integer_keys {
    ($($ty:ty),*) => { $(impl IntoElementKey for $ty {
        fn into_element_key(self) -> String { self.to_string() }
    })* };
}
integer_keys!(
    u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize
);

#[doc(hidden)]
#[derive(Debug, PartialEq, Eq)]
pub struct Declaration {
    pub family: &'static str,
    pub keyed: bool,
    pub package: &'static str,
    pub version: &'static str,
    pub file: &'static str,
    pub line: u32,
}
/// Target-specific, normalized declaration. Provenance is omitted in release builds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegisteredId {
    pub family: String,
    pub keyed: bool,
    pub package: String,
    pub version: String,
    pub locations: Vec<String>,
}
#[cfg(not(target_arch = "wasm32"))]
#[linkme::distributed_slice]
#[doc(hidden)]
pub static DECLARATIONS: [Declaration];

/// Return the linked app's vocabulary, independent of visited pages. Reject conflicting
/// shapes and owners; repeated declarations within one package are permitted.
pub fn registry() -> Result<&'static [RegisteredId], String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        static CATALOG: std::sync::OnceLock<Result<Vec<RegisteredId>, String>> =
            std::sync::OnceLock::new();
        CATALOG
            .get_or_init(|| {
                normalize(
                    DECLARATIONS
                        .iter()
                        .map(|d| RegisteredId {
                            family: d.family.into(),
                            keyed: d.keyed,
                            package: d.package.into(),
                            version: d.version.into(),
                            locations: if d.file.is_empty() {
                                vec![]
                            } else {
                                vec![format!("{}:{}", d.file, d.line)]
                            },
                        })
                        .collect(),
                )
            })
            .as_ref()
            .map(Vec::as_slice)
            .map_err(Clone::clone)
    }
    #[cfg(target_arch = "wasm32")]
    WEB_REGISTRY
        .get()
        .ok_or("missing day_ids custom section handoff")?
        .as_ref()
        .map(Vec::as_slice)
        .map_err(Clone::clone)
}
/// The exact family portion of a canonical locator; used only for mounted diagnostics.
pub fn family_of(id: &str) -> &str {
    let mut chars = id.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c == ':' {
            if chars.peek().is_some_and(|(_, c)| *c == ':') {
                chars.next();
            } else {
                return &id[..i];
            }
        }
    }
    id
}

fn normalize(records: Vec<RegisteredId>) -> Result<Vec<RegisteredId>, String> {
    let mut catalog: BTreeMap<String, RegisteredId> = BTreeMap::new();
    for record in records {
        if let Some(prior) = catalog.get_mut(&record.family) {
            if (prior.keyed, &prior.package, &prior.version)
                != (record.keyed, &record.package, &record.version)
            {
                let owner = |r: &RegisteredId| {
                    let shape = if r.keyed { "keyed" } else { "static" };
                    match r.locations.first() {
                        Some(at) => format!("{shape} in {} {} at {at}", r.package, r.version),
                        None => format!("{shape} in {} {}", r.package, r.version),
                    }
                };
                return Err(format!(
                    "conflicting element ID declaration: {} ({}; {})",
                    record.family,
                    owner(prior),
                    owner(&record)
                ));
            }
            prior.locations.extend(record.locations);
            prior.locations.sort();
            prior.locations.dedup();
        } else {
            catalog.insert(record.family.clone(), record);
        }
    }
    Ok(catalog.into_values().collect())
}

#[doc(hidden)]
pub const fn validate_name(name: &str) {
    let bytes = name.as_bytes();
    assert!(!bytes.is_empty(), "element ID must not be empty");
    let mut start = 0;
    let mut i = 0;
    while i <= bytes.len() {
        if i == bytes.len() || bytes[i] == b':' {
            assert!(i > start, "empty element ID namespace segment");
            assert!(
                bytes[start] >= b'a' && bytes[start] <= b'z',
                "element ID must start with a lowercase ASCII letter"
            );
            assert!(
                bytes[i - 1] != b'_',
                "element ID must not end with underscore"
            );
            // Rust ident fragments accept keywords, including raw identifiers. Keep the
            // protocol vocabulary independent of the edition used by the calling crate.
            let keywords = [
                "as",
                "async",
                "await",
                "break",
                "const",
                "continue",
                "crate",
                "dyn",
                "else",
                "enum",
                "extern",
                "false",
                "fn",
                "for",
                "if",
                "impl",
                "in",
                "let",
                "loop",
                "match",
                "mod",
                "move",
                "mut",
                "pub",
                "ref",
                "return",
                "self",
                "static",
                "struct",
                "super",
                "trait",
                "true",
                "type",
                "unsafe",
                "use",
                "where",
                "while",
                "abstract",
                "become",
                "box",
                "do",
                "final",
                "gen",
                "macro",
                "override",
                "priv",
                "try",
                "typeof",
                "unsized",
                "virtual",
                "yield",
                "union",
                "macro_rules",
            ];
            let mut k = 0;
            while k < keywords.len() {
                let keyword = keywords[k].as_bytes();
                let mut equal = keyword.len() == i - start;
                let mut j = 0;
                while equal && j < keyword.len() {
                    equal = bytes[start + j] == keyword[j];
                    j += 1;
                }
                assert!(!equal, "Rust keyword is not an element ID");
                k += 1;
            }
            if i == bytes.len() {
                break;
            }
            assert!(
                i + 1 < bytes.len() && bytes[i + 1] == b':',
                "invalid namespace separator"
            );
            i += 2;
            start = i;
        } else {
            let b = bytes[i];
            assert!(
                (b >= b'a' && b <= b'z') || (b >= b'0' && b <= b'9') || b == b'_',
                "element IDs use lowercase ASCII snake_case, not raw identifiers"
            );
            assert!(
                b != b'_' || i == start || bytes[i - 1] != b'_',
                "double underscore in element ID"
            );
            i += 1;
        }
    }
}

/// Declare an ID at its piece, including a link-time catalog entry.
///
/// ```
/// let save = day_core::id!(account::save);
/// let row = day_core::id!(query_row, 42_u64);
/// assert_eq!(save.as_str(), "account::save");
/// assert_eq!(row.as_str(), "query_row:42");
/// ```
///
/// Names are declarations using identifier tokens, with no string or case-conversion path.
///
/// ```compile_fail
/// let _ = day_core::id!("save");
/// ```
/// ```compile_fail
/// let _ = day_core::id!(Save);
/// ```
/// ```compile_fail
/// let _ = day_core::id!(r#save);
/// ```
/// ```compile_fail
/// let _ = day_core::id!(account__save);
/// ```
/// ```compile_fail
/// let _ = day_core::id!(type);
/// ```
#[macro_export]
macro_rules! id {
    ($first:ident $(:: $rest:ident)* $(,)?) => {{
        $crate::__declare_element_id!(false, "s", $first $(:: $rest)*);
        $crate::ElementId::from_declaration(&__DAY_ID)
    }};
    ($first:ident $(:: $rest:ident)*, $key:expr $(,)?) => {{
        $crate::__declare_element_id!(true, "k", $first $(:: $rest)*);
        $crate::ElementId::with_key(&__DAY_ID, $key)
    }};
}
#[doc(hidden)]
#[macro_export]
macro_rules! __declare_element_id {
    ($keyed:expr, $flag:literal, $first:ident $(:: $rest:ident)*) => {
        const __NAME: &str = concat!(stringify!($first), $("::", stringify!($rest)),*);
        const _: () = $crate::element_id::validate_name(__NAME);
        #[cfg(not(target_arch = "wasm32"))]
        #[$crate::linkme::distributed_slice($crate::element_id::DECLARATIONS)]
        #[linkme(crate = $crate::linkme)]
        static __DAY_ID: $crate::element_id::Declaration = $crate::element_id::Declaration {
            family: __NAME, keyed: $keyed, package: env!("CARGO_PKG_NAME"), version: env!("CARGO_PKG_VERSION"),
            file: if cfg!(debug_assertions) { file!() } else { "" },
            line: if cfg!(debug_assertions) { line!() } else { 0 },
        };
        #[cfg(target_arch = "wasm32")]
        static __DAY_ID: $crate::element_id::Declaration = $crate::element_id::Declaration {
            family: __NAME, keyed: $keyed, package: env!("CARGO_PKG_NAME"), version: env!("CARGO_PKG_VERSION"), file: "", line: 0,
        };
        #[cfg(target_arch = "wasm32")]
        const __PAYLOAD: &str = concat!("D1\0", $flag, "\0", stringify!($first), $("::", stringify!($rest),)* "\0", env!("CARGO_PKG_NAME"), "\0", env!("CARGO_PKG_VERSION"));
        #[cfg(target_arch = "wasm32")]
        #[$crate::__retain_element_id]
        #[used]
        #[unsafe(link_section = "day_ids")]
        static __DAY_ID_BYTES: [u8; __PAYLOAD.len()+4] = $crate::element_id::record_bytes(__PAYLOAD);
    };
}

#[doc(hidden)]
pub const fn record_bytes<const N: usize>(payload: &str) -> [u8; N] {
    let mut out = [0; N];
    let length = (payload.len() as u32).to_le_bytes();
    let mut i = 0;
    while i < 4 {
        out[i] = length[i];
        i += 1;
    }
    i = 0;
    while i < payload.len() {
        out[i + 4] = payload.as_bytes()[i];
        i += 1;
    }
    out
}
#[cfg(target_arch = "wasm32")]
#[crate::__retain_element_id]
#[used]
#[unsafe(link_section = "day_ids")]
static SENTINEL: [u8; 6] = record_bytes("D1");
#[cfg(target_arch = "wasm32")]
static WEB_REGISTRY: std::sync::OnceLock<Result<Vec<RegisteredId>, String>> =
    std::sync::OnceLock::new();
/// Install all concatenated custom sections once, before app startup. Invalid metadata
/// remains an error at the protocol boundary; it never becomes an empty catalog.
#[cfg(target_arch = "wasm32")]
pub fn install_web_registry(bytes: &[u8]) -> Result<(), String> {
    WEB_REGISTRY
        .set(decode_records(bytes))
        .map_err(|_| "element registry already installed".to_owned())
}
#[cfg(any(test, target_arch = "wasm32"))]
fn decode_records(mut bytes: &[u8]) -> Result<Vec<RegisteredId>, String> {
    let mut records = Vec::new();
    let mut sentinel = false;
    while !bytes.is_empty() {
        let length = bytes.get(..4).ok_or("truncated day_ids length")?;
        let length = u32::from_le_bytes(length.try_into().unwrap()) as usize;
        bytes = &bytes[4..];
        let payload = bytes.get(..length).ok_or("truncated day_ids record")?;
        bytes = &bytes[length..];
        let text = std::str::from_utf8(payload).map_err(|_| "non UTF-8 day_ids record")?;
        if text == "D1" {
            sentinel = true;
            continue;
        }
        let fields: Vec<_> = text.split('\0').collect();
        if fields.len() != 5
            || fields[0] != "D1"
            || !matches!(fields[1], "s" | "k")
            || fields[2].is_empty()
        {
            return Err("invalid day_ids schema/record".into());
        }
        records.push(RegisteredId {
            family: fields[2].into(),
            keyed: fields[1] == "k",
            package: fields[3].into(),
            version: fields[4].into(),
            locations: vec![],
        });
    }
    if !sentinel {
        return Err("missing day_ids registry sentinel".into());
    }
    normalize(records)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_names_and_injective_keys() {
        assert_eq!(crate::id!(identity), crate::id!(identity));
        assert_eq!(crate::id!(account::save).as_str(), "account::save");
        assert_eq!(
            crate::id!(query_row, ":% café").as_str(),
            "query_row:%3A%25%20caf%C3%A9"
        );
        assert_eq!(
            crate::id!(query_row, 42).as_str(),
            crate::id!(query_row, "42").as_str()
        );
        let calls = std::cell::Cell::new(0);
        let _ = crate::id!(once, {
            calls.set(calls.get() + 1);
            3
        });
        assert_eq!(calls.get(), 1);
    }
    #[allow(dead_code)]
    fn unused<T>(_: Option<T>) {
        let _ = crate::id!(never_mounted);
    }
    #[test]
    fn registry_includes_unused_generic() {
        assert!(
            registry()
                .unwrap()
                .iter()
                .any(|d| d.family == "never_mounted")
        );
    }
    #[test]
    fn framed_metadata_and_fail_closed() {
        let mut data = record_bytes::<6>("D1").to_vec();
        let payload = concat!("D1\0k\0row\0app\0", "1");
        data.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        data.extend_from_slice(payload.as_bytes());
        assert_eq!(decode_records(&data).unwrap()[0].family, "row");
        assert!(decode_records(&data[6..]).is_err());
        assert!(decode_records(&data[..data.len() - 1]).is_err());
    }
    #[test]
    fn normalization_rejects_owner_and_shape_conflicts() {
        let record = RegisteredId {
            family: "save".into(),
            keyed: false,
            package: "fixture".into(),
            version: "1".into(),
            locations: vec!["fixture.rs:1".into()],
        };
        let mut duplicate = record.clone();
        duplicate.locations = vec!["fixture.rs:2".into()];
        let normalized = normalize(vec![record.clone(), duplicate.clone()]).unwrap();
        assert_eq!(normalized.len(), 1);
        assert_eq!(normalized[0].locations.len(), 2);
        duplicate.keyed = true;
        assert!(normalize(vec![record.clone(), duplicate.clone()]).is_err());
        duplicate.keyed = false;
        duplicate.package = "other_fixture".into();
        // The message names both owners, so a collision between two crates is traceable.
        assert_eq!(
            normalize(vec![record, duplicate]).unwrap_err(),
            "conflicting element ID declaration: save (static in fixture 1 at fixture.rs:1; \
             static in other_fixture 1 at fixture.rs:2)"
        );
    }
}
