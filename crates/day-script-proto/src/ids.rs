// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
//! Compiled declaration catalogs and family-level dayscript evidence.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct IdDeclaration {
    pub family: String,
    pub keyed: bool,
    pub package: String,
    pub version: String,
    pub locations: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct IdCatalog {
    pub schema_version: u32,
    pub declarations: Vec<IdDeclaration>,
}
impl IdCatalog {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 {
            return Err("unsupported element registry version".into());
        }
        let mut names = BTreeSet::new();
        for d in &self.declarations {
            if !names.insert(&d.family) {
                return Err(format!("duplicate registry family {}", d.family));
            }
        }
        Ok(())
    }
    pub fn resolve(&self, id: &str) -> Result<&IdDeclaration, String> {
        let mut split = None;
        let mut chars = id.char_indices().peekable();
        while let Some((i, c)) = chars.next() {
            if c == ':' {
                if chars.peek().is_some_and(|(_, c)| *c == ':') {
                    chars.next();
                } else {
                    split = Some(i);
                    break;
                }
            }
        }
        let family = split.map_or(id, |i| &id[..i]);
        let d = self
            .declarations
            .iter()
            .find(|d| d.family == family)
            .ok_or_else(|| format!("undeclared element ID {family:?}"))?;
        if d.keyed != split.is_some() {
            return Err(format!("element ID {id:?} has wrong static/keyed shape"));
        }
        if let Some(i) = split {
            validate_key(&id[i + 1..])?;
        }
        Ok(d)
    }
}
fn unreserved(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"-._~".contains(&b)
}
fn validate_key(key: &str) -> Result<(), String> {
    let b = key.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if unreserved(b[i]) {
            out.push(b[i]);
            i += 1;
            continue;
        }
        if b[i] != b'%' || i + 2 >= b.len() {
            return Err("noncanonical element key; use percent-encoded UTF-8".into());
        }
        let digit = |x: u8| match x {
            b'0'..=b'9' => Some(x - b'0'),
            b'A'..=b'F' => Some(x - b'A' + 10),
            _ => None,
        };
        let (Some(hi), Some(lo)) = (digit(b[i + 1]), digit(b[i + 2])) else {
            return Err("element key escapes require uppercase hexadecimal".into());
        };
        let byte = (hi << 4) | lo;
        if unreserved(byte) {
            return Err("unnecessary element key escape".into());
        }
        out.push(byte);
        i += 3;
    }
    String::from_utf8(out).map_err(|_| "element key is not UTF-8".to_owned())?;
    Ok(())
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceClass {
    Interaction,
    Assertion,
    Observed,
    Negative,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct IdEvidence {
    pub family: String,
    pub class: EvidenceClass,
}

impl crate::Step {
    /// Element references only: model row keys, routes, strings and screenshot names are
    /// intentionally excluded. Secondary geometry references are validated, not credited.
    pub fn element_references(&self) -> Vec<String> {
        let value = serde_json::to_value(self).expect("serializable step");
        let mut ids = Vec::new();
        for field in ["id", "relative_to", "item_id"] {
            if let Some(id) = value.get(field).and_then(|v| v.as_str()) {
                ids.push(id.to_owned());
            }
        }
        if let Self::Toolbar { item, .. } = self {
            ids.push(item.clone());
        }
        ids
    }
    pub fn evidence_class(&self) -> Option<EvidenceClass> {
        use EvidenceClass::*;
        match self {
            Self::WaitFor { .. } => Some(Observed),
            Self::AssertMissing { .. } | Self::AssertHidden { .. } => Some(Negative),
            Self::AssertVisible { .. }
            | Self::AssertText { .. }
            | Self::AssertValue { .. }
            | Self::AssertFrame { .. }
            | Self::AssertNative { .. }
            | Self::AssertFocused { .. } => Some(Assertion),
            Self::WebEval { text, contains, .. } => {
                if text.is_some() || contains.is_some() {
                    Some(Assertion)
                } else {
                    Some(Observed)
                }
            }
            Self::Tap { .. }
            | Self::Key { .. }
            | Self::ScrollTo { .. }
            | Self::Drag { .. }
            | Self::Hover { .. }
            | Self::Pan { .. }
            | Self::Pinch { .. }
            | Self::Submit { .. }
            | Self::Input { .. }
            | Self::SetValue { .. }
            | Self::Toggle { .. }
            | Self::Select { .. }
            | Self::Focus { .. }
            | Self::Reorder { .. }
            | Self::Expand { .. }
            | Self::TreeMove { .. }
            | Self::DeleteRow { .. }
            | Self::SwipeRow { .. }
            | Self::Menu { .. }
            | Self::Toolbar { .. }
            | Self::ContextMenu { .. } => Some(Interaction),
            _ => None,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn namespace_and_canonical_keys() {
        let c = IdCatalog {
            schema_version: 1,
            declarations: vec![IdDeclaration {
                family: "account::row".into(),
                keyed: true,
                package: "fixture".into(),
                version: "0".into(),
                locations: vec![],
            }],
        };
        for s in ["account::row:42", "account::row:", "account::row:%3Ab"] {
            assert!(c.resolve(s).is_ok(), "{s}");
        }
        for s in [
            "account::row",
            "account::row:%3ab",
            "account::row:%41",
            "account::row:%FF",
            "unknown:1",
        ] {
            assert!(c.resolve(s).is_err(), "{s}");
        }
    }
}
