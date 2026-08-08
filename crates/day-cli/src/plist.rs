// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! A narrow XML-plist editor: read and rewrite a MANAGED SET of top-level `<key>`/`<string>` pairs,
//! preserving every other byte of the file.
//!
//! # Why not `plutil`
//!
//! `plutil` is the obvious tool and it is not the right one here, for three reasons:
//!
//! 1. **`day lint` must READ the plist on every host** to report that a checked-in manifest has
//!    drifted from `Day.toml`. `plutil` is macOS-only, so the reader has to be pure Rust regardless
//!    — and having a Rust reader with a `plutil` writer means two models of the same file.
//! 2. **`plutil -replace` reserializes the whole document.** The plist is checked in, so a build
//!    that reformats it churns the diff on every run — which would destroy the main argument for
//!    writing into a checked-in file at all: that a human can see and review what Day changed.
//! 3. It is testable on Linux CI, where the iOS leg cannot run at all.
//!
//! On macOS `plutil -lint` still verifies the result after a write (see `mobile.rs`), so Apple's own
//! parser has the last word — `plutil` is demoted from mutator to verifier, and its absence
//! elsewhere costs checking rather than correctness.
//!
//! # Depth awareness
//!
//! The scan tracks container depth so only `<key>`s directly inside the root `<dict>` count. The
//! real scaffold has `CFBundleURLName` nested inside `CFBundleURLTypes`' array-of-dict, and a
//! depth-blind editor would happily mistake it for a top-level key and rewrite the wrong element.

use std::collections::{BTreeMap, BTreeSet};

/// Every top-level `<key>` whose value is a `<string>`, mapped to that string.
///
/// Keys whose value is any other type (`<array>`, `<dict>`, `<true/>`) are skipped: this editor
/// only ever manages string-valued keys, and skipping them here means it can never be asked to
/// rewrite one.
pub fn read_string_keys(text: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for (key, value, _, _) in scan(text) {
        if let Some(v) = value {
            out.insert(key, v);
        }
    }
    out
}

/// One top-level entry: `(key, Some(string value) | None, byte range of the whole pair)`.
type Entry = (String, Option<String>, usize, usize);

/// Walk the root `<dict>`, yielding its immediate `<key>` entries with their byte ranges.
fn scan(text: &str) -> Vec<Entry> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0usize;
    // Depth inside the root dict: 0 = not there yet, 1 = the root dict's own children.
    let mut depth = 0usize;
    let mut seen_root = false;

    while i < bytes.len() {
        let Some(lt) = text[i..].find('<').map(|p| i + p) else {
            break;
        };
        let Some(gt) = text[lt..].find('>').map(|p| lt + p) else {
            break;
        };
        let tag = &text[lt + 1..gt];
        let name = tag
            .trim_start_matches('/')
            .split(|c: char| c.is_whitespace() || c == '/' || c == '>')
            .next()
            .unwrap_or("");
        let closing = tag.starts_with('/');
        let self_closing = tag.ends_with('/');

        match name {
            "dict" | "array" => {
                if closing {
                    depth = depth.saturating_sub(1);
                } else if !self_closing {
                    if !seen_root && name == "dict" {
                        seen_root = true;
                    }
                    depth += 1;
                }
            }
            "key" if depth == 1 && !closing => {
                // <key>NAME</key> then the value element.
                let Some(key_end) = text[gt..].find("</key>").map(|p| gt + p) else {
                    break;
                };
                let key = unescape(text[gt + 1..key_end].trim());
                let after = key_end + "</key>".len();
                let (value, end) = read_value(text, after);
                out.push((key, value, lt, end));
                i = end;
                continue;
            }
            _ => {}
        }
        i = gt + 1;
    }
    out
}

/// Read the value element that follows a key, returning its string content (when it is a
/// `<string>`) and the byte offset just past it.
fn read_value(text: &str, from: usize) -> (Option<String>, usize) {
    let Some(lt) = text[from..].find('<').map(|p| from + p) else {
        return (None, from);
    };
    let Some(gt) = text[lt..].find('>').map(|p| lt + p) else {
        return (None, from);
    };
    let tag = &text[lt + 1..gt];
    let name = tag
        .split(|c: char| c.is_whitespace() || c == '/' || c == '>')
        .next()
        .unwrap_or("");
    if tag.ends_with('/') {
        return (None, gt + 1); // <true/>, <dict/>
    }
    let close = format!("</{name}>");
    // A nested container needs balanced matching; a leaf can take the next close tag.
    let end = if name == "dict" || name == "array" {
        match balanced_end(text, gt + 1, name) {
            Some(e) => e,
            None => return (None, gt + 1),
        }
    } else {
        match text[gt..].find(&close).map(|p| gt + p + close.len()) {
            Some(e) => e,
            None => return (None, gt + 1),
        }
    };
    if name == "string" {
        let inner = &text[gt + 1..end - close.len()];
        (Some(unescape(inner)), end)
    } else {
        (None, end)
    }
}

/// The byte offset just past the balanced close of a container opened before `from`.
fn balanced_end(text: &str, from: usize, name: &str) -> Option<usize> {
    let open = format!("<{name}");
    let close = format!("</{name}>");
    let mut depth = 1usize;
    let mut i = from;
    while depth > 0 {
        let next_open = text[i..].find(&open).map(|p| i + p);
        let next_close = text[i..].find(&close).map(|p| i + p)?;
        match next_open {
            Some(o) if o < next_close => {
                // Ignore a self-closing <dict/> — it opens nothing.
                let tag_end = text[o..].find('>').map(|p| o + p)?;
                if !text[o..=tag_end].ends_with("/>") {
                    depth += 1;
                }
                i = tag_end + 1;
            }
            _ => {
                depth -= 1;
                i = next_close + close.len();
            }
        }
    }
    Some(i)
}

/// Rewrite a managed set of top-level string keys.
///
/// `set` are replaced in place (or inserted in sorted position); `remove` are deleted with their
/// value. Every other byte — arrays, nested dicts, the DOCTYPE, the file's tab indentation — is
/// preserved exactly, so the checked-in plist's diff shows only what Day actually changed.
///
/// Returns `Err` for a file this editor does not recognize (a binary or JSON plist), rather than
/// guessing: a corrupted scaffold is far worse than a build that says what to do.
pub fn apply_string_keys(
    text: &str,
    set: &BTreeMap<String, String>,
    remove: &BTreeSet<String>,
) -> Result<String, String> {
    if !text.trim_start().starts_with("<?xml") {
        return Err(
            "Info.plist is not XML (a binary or JSON plist). Convert it with \
             `plutil -convert xml1 <path>`, or let `day new` regenerate the scaffold."
                .to_string(),
        );
    }
    let entries = scan(text);
    if entries.is_empty() && !text.contains("<dict") {
        return Err("Info.plist has no root <dict>".to_string());
    }

    let mut out = String::with_capacity(text.len() + 256);
    let mut cursor = 0usize;
    let mut written: BTreeSet<&str> = BTreeSet::new();

    for (key, value, start, end) in &entries {
        // Only string-valued keys are ever managed, so a key whose value is an array or dict is
        // passed through even if it shares a name with a managed one.
        let manageable = value.is_some();
        if !manageable {
            continue;
        }
        // Copy up to the START OF THE LINE, not to the tag: the line's leading indentation belongs
        // to the entry being rewritten, and copying it here as well would double it on every pass
        // (which is what breaks byte-for-byte idempotency).
        let line_start = text[..*start].rfind('\n').map(|p| p + 1).unwrap_or(0);
        if remove.contains(key.as_str()) {
            out.push_str(&text[cursor..line_start]);
            cursor = skip_to_next_line(text, *end);
            continue;
        }
        if let Some(new) = set.get(key.as_str()) {
            out.push_str(&text[cursor..line_start]);
            out.push_str(&entry_xml(key, new, indent_of(text, *start)));
            written.insert(key.as_str());
            cursor = skip_to_next_line(text, *end);
        }
    }
    out.push_str(&text[cursor..]);

    // Insert the keys that were not already present, before the root dict's closing tag.
    let missing: Vec<(&String, &String)> = set
        .iter()
        .filter(|(k, _)| !written.contains(k.as_str()))
        .collect();
    if !missing.is_empty() {
        let anchor = out
            .rfind("</dict>")
            .ok_or_else(|| "Info.plist has no closing </dict>".to_string())?;
        let indent = indent_of(&out, anchor) + "\t";
        let mut block = String::new();
        for (k, v) in missing {
            block.push_str(&entry_xml(k, v, indent.clone()));
        }
        let line_start = out[..anchor].rfind('\n').map(|p| p + 1).unwrap_or(0);
        out.insert_str(line_start, &block);
    }
    Ok(out)
}

/// Set (or remove, with `None`) a top-level array of strings — `UIAppFonts`.
///
/// This exists so that `UIAppFonts` and the permission keys go through ONE writer. When
/// `sync_uiappfonts` still used `plutil -replace`, every build moved that key to the end of the
/// file while this editor kept the permission keys in place, so the two writers swapped their
/// relative order on every run and the checked-in plist churned forever.
pub fn apply_array_key(text: &str, key: &str, values: Option<&[String]>) -> Result<String, String> {
    if !text.trim_start().starts_with("<?xml") {
        return Err("Info.plist is not XML".to_string());
    }
    let entry = scan(text).into_iter().find(|(k, ..)| k == key);
    let rendered = values.map(|v| (v, ()));

    match (entry, rendered) {
        // Replace in place, preserving position.
        (Some((_, _, start, end)), Some((v, _))) => {
            let line_start = text[..start].rfind('\n').map(|p| p + 1).unwrap_or(0);
            let indent = indent_of(text, start);
            let mut out = String::with_capacity(text.len() + 128);
            out.push_str(&text[..line_start]);
            out.push_str(&array_xml(key, v, &indent));
            out.push_str(&text[skip_to_next_line(text, end)..]);
            Ok(out)
        }
        // Remove.
        (Some((_, _, start, end)), None) => {
            let line_start = text[..start].rfind('\n').map(|p| p + 1).unwrap_or(0);
            let mut out = String::with_capacity(text.len());
            out.push_str(&text[..line_start]);
            out.push_str(&text[skip_to_next_line(text, end)..]);
            Ok(out)
        }
        // Insert before the root dict's close.
        (None, Some((v, _))) => {
            let anchor = text
                .rfind("</dict>")
                .ok_or_else(|| "Info.plist has no closing </dict>".to_string())?;
            let indent = indent_of(text, anchor) + "\t";
            let line_start = text[..anchor].rfind('\n').map(|p| p + 1).unwrap_or(0);
            let mut out = String::with_capacity(text.len() + 128);
            out.push_str(&text[..line_start]);
            out.push_str(&array_xml(key, v, &indent));
            out.push_str(&text[line_start..]);
            Ok(out)
        }
        (None, None) => Ok(text.to_string()),
    }
}

/// Set (or remove, with `None`) a top-level array of flat string dicts —
/// `UIApplicationShortcutItems`. Same placement rules as [`apply_array_key`], and the same
/// single-writer rationale: every managed Info.plist key goes through this editor so their
/// relative order never churns.
pub fn apply_dict_array_key(
    text: &str,
    key: &str,
    dicts: Option<&[Vec<(String, String)>]>,
) -> Result<String, String> {
    if !text.trim_start().starts_with("<?xml") {
        return Err("Info.plist is not XML".to_string());
    }
    let entry = scan(text).into_iter().find(|(k, ..)| k == key);

    match (entry, dicts) {
        // Replace in place, preserving position.
        (Some((_, _, start, end)), Some(d)) => {
            let line_start = text[..start].rfind('\n').map(|p| p + 1).unwrap_or(0);
            let indent = indent_of(text, start);
            let mut out = String::with_capacity(text.len() + 256);
            out.push_str(&text[..line_start]);
            out.push_str(&dict_array_xml(key, d, &indent));
            out.push_str(&text[skip_to_next_line(text, end)..]);
            Ok(out)
        }
        // Remove.
        (Some((_, _, start, end)), None) => {
            let line_start = text[..start].rfind('\n').map(|p| p + 1).unwrap_or(0);
            let mut out = String::with_capacity(text.len());
            out.push_str(&text[..line_start]);
            out.push_str(&text[skip_to_next_line(text, end)..]);
            Ok(out)
        }
        // Insert before the root dict's close.
        (None, Some(d)) => {
            let anchor = text
                .rfind("</dict>")
                .ok_or_else(|| "Info.plist has no closing </dict>".to_string())?;
            let indent = indent_of(text, anchor) + "\t";
            let line_start = text[..anchor].rfind('\n').map(|p| p + 1).unwrap_or(0);
            let mut out = String::with_capacity(text.len() + 256);
            out.push_str(&text[..line_start]);
            out.push_str(&dict_array_xml(key, d, &indent));
            out.push_str(&text[line_start..]);
            Ok(out)
        }
        (None, None) => Ok(text.to_string()),
    }
}

fn dict_array_xml(key: &str, dicts: &[Vec<(String, String)>], indent: &str) -> String {
    let mut s = format!("{indent}<key>{}</key>\n{indent}<array>\n", escape(key));
    for pairs in dicts {
        s.push_str(&format!("{indent}\t<dict>\n"));
        for (k, v) in pairs {
            s.push_str(&format!(
                "{indent}\t\t<key>{}</key>\n{indent}\t\t<string>{}</string>\n",
                escape(k),
                escape(v)
            ));
        }
        s.push_str(&format!("{indent}\t</dict>\n"));
    }
    s.push_str(&format!("{indent}</array>\n"));
    s
}

fn array_xml(key: &str, values: &[String], indent: &str) -> String {
    let mut s = format!("{indent}<key>{}</key>\n{indent}<array>\n", escape(key));
    for v in values {
        s.push_str(&format!("{indent}\t<string>{}</string>\n", escape(v)));
    }
    s.push_str(&format!("{indent}</array>\n"));
    s
}

/// One `<key>`/`<string>` pair, indented to match its neighbours.
fn entry_xml(key: &str, value: &str, indent: String) -> String {
    format!(
        "{indent}<key>{}</key>\n{indent}<string>{}</string>\n",
        escape(key),
        escape(value)
    )
}

/// The whitespace at the start of the line containing `pos`.
fn indent_of(text: &str, pos: usize) -> String {
    let line_start = text[..pos].rfind('\n').map(|p| p + 1).unwrap_or(0);
    text[line_start..pos]
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect()
}

fn skip_to_next_line(text: &str, from: usize) -> usize {
    match text[from..].find('\n') {
        Some(p) => from + p + 1,
        None => text.len(),
    }
}

/// XML-escape a value. A permission reason is prose: it WILL contain an ampersand or an apostrophe
/// sooner or later, and an unescaped one makes the plist unparseable.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        // `&amp;` last, so "&amp;lt;" round-trips to "&lt;" rather than "<".
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The scaffold TEMPLATE — a real plist, including the nested dict that a depth-blind scanner
    /// gets wrong, and stable: `day build` rewrites an app's plist, never the template. (The
    /// showcase's own plist was the obvious fixture and the wrong one: once the permission writer
    /// started adding keys to it, these tests were asserting against their own side effects.)
    const SHOWCASE: &str = include_str!("../templates/app/platform/ios/Runner/Info.plist");

    fn set(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    #[test]
    fn reads_top_level_string_keys() {
        let keys = read_string_keys(SHOWCASE);
        // The template's value is still a handlebars placeholder — what matters is that the key is
        // seen at all, and that whatever text it holds round-trips.
        assert_eq!(
            keys.get("CFBundleDisplayName").map(String::as_str),
            Some("{{title}}")
        );
        assert_eq!(
            keys.get("CFBundlePackageType").map(String::as_str),
            Some("APPL")
        );
        // Nested inside CFBundleURLTypes' array-of-dict — must NOT be seen as top-level.
        assert!(!keys.contains_key("CFBundleURLName"));
        assert!(!keys.contains_key("CFBundleURLSchemes"));
        // Non-string values are not managed.
        assert!(!keys.contains_key("UIAppFonts"));
        assert!(!keys.contains_key("LSRequiresIPhoneOS"));
    }

    #[test]
    fn inserts_and_replaces() {
        let once = apply_string_keys(
            SHOWCASE,
            &set(&[("NSCameraUsageDescription", "Scan.")]),
            &BTreeSet::new(),
        )
        .expect("apply");
        assert!(once.contains("<key>NSCameraUsageDescription</key>"));
        assert_eq!(
            read_string_keys(&once)
                .get("NSCameraUsageDescription")
                .map(String::as_str),
            Some("Scan.")
        );
        // Everything else survived untouched.
        assert!(
            once.contains("<string>DayPieces_DayPieces.bundle/fonts/Pacifico-Regular.ttf</string>")
        );
        assert!(once.contains("<key>CFBundleURLName</key>"));

        let twice = apply_string_keys(
            &once,
            &set(&[("NSCameraUsageDescription", "Different.")]),
            &BTreeSet::new(),
        )
        .expect("apply");
        assert_eq!(
            read_string_keys(&twice)
                .get("NSCameraUsageDescription")
                .map(String::as_str),
            Some("Different.")
        );
        assert_eq!(
            twice.matches("<key>NSCameraUsageDescription</key>").count(),
            1
        );
    }

    /// Writing the same thing twice must be a byte-for-byte no-op — this is the property that makes
    /// mutating a checked-in file defensible.
    #[test]
    fn applying_twice_is_identical() {
        let keys = set(&[
            ("NSCameraUsageDescription", "Scan a document."),
            (
                "NSLocationWhenInUseUsageDescription",
                "Show nearby stations.",
            ),
        ]);
        let once = apply_string_keys(SHOWCASE, &keys, &BTreeSet::new()).expect("first");
        let twice = apply_string_keys(&once, &keys, &BTreeSet::new()).expect("second");
        assert_eq!(once, twice);
    }

    #[test]
    fn removes_only_what_it_is_told_to() {
        let with = apply_string_keys(
            SHOWCASE,
            &set(&[("NSCameraUsageDescription", "Scan.")]),
            &BTreeSet::new(),
        )
        .expect("apply");
        let mut remove = BTreeSet::new();
        remove.insert("NSCameraUsageDescription".to_string());
        let without = apply_string_keys(&with, &BTreeMap::new(), &remove).expect("remove");
        assert!(!without.contains("NSCameraUsageDescription"));
        // Removing the only managed key must restore the original file exactly.
        assert_eq!(without, SHOWCASE);
    }

    /// A hand-added key Day does not manage must survive forever — that is the escape hatch.
    #[test]
    fn unmanaged_keys_are_never_touched() {
        let hand_edited = SHOWCASE.replace(
            "\t<key>LSRequiresIPhoneOS</key>",
            "\t<key>NSContactsUsageDescription</key>\n\t<string>Find friends.</string>\n\t<key>LSRequiresIPhoneOS</key>",
        );
        let out = apply_string_keys(
            &hand_edited,
            &set(&[("NSCameraUsageDescription", "Scan.")]),
            &BTreeSet::new(),
        )
        .expect("apply");
        assert_eq!(
            read_string_keys(&out)
                .get("NSContactsUsageDescription")
                .map(String::as_str),
            Some("Find friends.")
        );
    }

    #[test]
    fn escapes_xml_in_reasons() {
        let out = apply_string_keys(
            SHOWCASE,
            &set(&[("NSCameraUsageDescription", "Scan Tom & Jerry's <docs>")]),
            &BTreeSet::new(),
        )
        .expect("apply");
        assert!(out.contains("Scan Tom &amp; Jerry&apos;s &lt;docs&gt;"));
        // …and it round-trips back to the original text.
        assert_eq!(
            read_string_keys(&out)
                .get("NSCameraUsageDescription")
                .map(String::as_str),
            Some("Scan Tom & Jerry's <docs>")
        );
    }

    #[test]
    fn refuses_a_file_it_does_not_understand() {
        assert!(apply_string_keys("bplist00\u{0}", &BTreeMap::new(), &BTreeSet::new()).is_err());
        assert!(apply_string_keys("{\"a\": 1}", &BTreeMap::new(), &BTreeSet::new()).is_err());
    }

    #[test]
    fn dict_array_inserts_replaces_and_removes() {
        let items = vec![vec![
            (
                "UIApplicationShortcutItemType".to_string(),
                "app://menus".to_string(),
            ),
            (
                "UIApplicationShortcutItemTitle".to_string(),
                "Menus & dialogs".to_string(),
            ),
        ]];
        let once = apply_dict_array_key(SHOWCASE, "UIApplicationShortcutItems", Some(&items))
            .expect("insert");
        assert!(once.contains("<key>UIApplicationShortcutItems</key>"));
        assert!(once.contains("<string>Menus &amp; dialogs</string>"));
        // Everything else survived untouched.
        assert!(once.contains("<key>CFBundleURLName</key>"));

        let twice = apply_dict_array_key(&once, "UIApplicationShortcutItems", Some(&items))
            .expect("replace");
        assert_eq!(once, twice, "re-applying the same items must be a no-op");

        let gone =
            apply_dict_array_key(&twice, "UIApplicationShortcutItems", None).expect("remove");
        assert!(!gone.contains("UIApplicationShortcutItems"));
        assert!(gone.contains("<key>CFBundleURLName</key>"));
    }
}
