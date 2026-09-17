// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Parse HarmonyOS JSON5 with json-five. Its round-trip AST retains comments, whitespace and
//! quoting; edits touch actual properties, never matching text inside comments or strings.
//! Permission entries remain in a managed region within module.requestPermissions.

use json_five::rt::parser::{ArrayValueContext, KeyValuePairContext};
pub use json_five::rt::parser::{JSONText as Document, JSONValue as Value};
use json_five::tokenize::TokType;

pub fn parse(text: &str) -> Result<Document, String> {
    let mut tokens =
        json_five::tokenize::tokenize_rt_str(text).map_err(|e| format!("JSON5: {e}"))?;
    // json-five 0.3.1 reports block-comment ends at the final slash instead of just after
    // it. Correct that token span before round-trip parsing, or rendering drops the slash.
    for (_, kind, end) in &mut tokens.tok_spans {
        if *kind == TokType::BlockComment && text.as_bytes().get(*end) == Some(&b'/') {
            *end += 1;
        }
    }
    json_five::rt::parser::from_tokens(&tokens).map_err(|e| format!("JSON5: {e}"))
}

pub fn string(value: &Value) -> Option<String> {
    match value {
        Value::DoubleQuotedString(_) | Value::SingleQuotedString(_) => {
            json_five::from_str(&value.to_string()).ok()
        }
        Value::Identifier(name) => {
            // Decode escaped identifier names using the parser's object-key handling.
            let object: std::collections::BTreeMap<String, bool> =
                json_five::from_str(&format!("{{{name}:true}}")).ok()?;
            object.into_keys().next()
        }
        _ => None,
    }
}

pub fn get<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    let Value::JSONObject {
        key_value_pairs, ..
    } = value
    else {
        return None;
    };
    key_value_pairs
        .iter()
        .find(|p| string(&p.key).as_deref() == Some(key))
        .map(|p| &p.value)
}

pub fn get_mut<'a>(value: &'a mut Value, key: &str) -> Option<&'a mut Value> {
    let Value::JSONObject {
        key_value_pairs, ..
    } = value
    else {
        return None;
    };
    key_value_pairs
        .iter_mut()
        .find(|p| string(&p.key).as_deref() == Some(key))
        .map(|p| &mut p.value)
}

pub fn array(value: &Value) -> impl Iterator<Item = &Value> {
    let values = match value {
        Value::JSONArray { values, .. } => values.as_slice(),
        _ => &[],
    };
    values.iter().map(|v| &v.value)
}

pub fn set_string(value: &mut Value, new: &str) -> Result<(), String> {
    if string(value).as_deref() != Some(new) {
        *value = parse(&serde_json::to_string(new).map_err(|e| e.to_string())?)?.value;
    }
    Ok(())
}

/// Append without reformatting or dropping existing members, comments or trailing commas.
pub fn push(array: &mut Value, value: Value) -> Result<(), String> {
    let Value::JSONArray { values, .. } = array else {
        return Err("expected JSON5 array".into());
    };
    if let Some(last) = values.last_mut() {
        let ctx = last.context.get_or_insert(ArrayValueContext {
            wsc: (String::new(), None),
        });
        ctx.wsc.1.get_or_insert_with(|| "\n".into());
    }
    values.push(json_five::rt::parser::JSONArrayValue {
        value,
        context: None,
    });
    Ok(())
}

pub fn insert(object: &mut Value, key: &str, value: Value) -> Result<(), String> {
    let Value::JSONObject {
        key_value_pairs, ..
    } = object
    else {
        return Err("expected JSON5 object".into());
    };
    if let Some(last) = key_value_pairs.last_mut() {
        let ctx = last.context.get_or_insert(KeyValuePairContext {
            wsc: (String::new(), String::new(), String::new(), None),
        });
        ctx.wsc.3.get_or_insert_with(|| "\n".into());
    }
    key_value_pairs.push(json_five::rt::parser::JSONKeyValuePair {
        key: parse(&serde_json::to_string(key).map_err(|e| e.to_string())?)?.value,
        value,
        context: None,
    });
    Ok(())
}

fn permissions<'a>(doc: &'a mut Document, key: &str) -> Result<&'a mut Value, String> {
    get_mut(&mut doc.value, "module")
        .and_then(|m| get_mut(m, key))
        .filter(|v| matches!(v, Value::JSONArray { .. }))
        .ok_or_else(|| format!("module.json5 has no module.{key} array"))
}

/// Only actual, standalone line-comment tokens may delimit a managed region.
fn markers(text: &str, tag: &str) -> Result<Option<(usize, usize)>, String> {
    let tokens = json_five::source_to_tokens(text).map_err(|e| e.to_string())?;
    let mut begin = None;
    let mut end = None;
    let mut depth = 0usize;
    for t in tokens {
        match t.tok_type {
            TokType::LeftBracket | TokType::LeftBrace => depth += 1,
            TokType::RightBracket | TokType::RightBrace => depth = depth.saturating_sub(1),
            _ => {}
        }
        if t.tok_type != TokType::LineComment || depth != 1 {
            continue;
        }
        let pos = t
            .context
            .ok_or("missing JSON5 token position")?
            .start_byte_offset;
        let line = text[..pos].rfind('\n').map_or(0, |p| p + 1);
        if !text[line..pos].trim().is_empty() {
            continue;
        }
        let Some(marker) = t.lexeme.split_whitespace().nth(1) else {
            continue;
        };
        if marker == format!("day:{tag}-begin") {
            if begin.is_some() {
                return Err("duplicate managed-region begin marker".into());
            }
            begin = Some(
                text[pos..]
                    .find('\n')
                    .ok_or("managed-region begin marker has no newline")?
                    + pos
                    + 1,
            );
        } else if marker == format!("day:{tag}-end") {
            if end.is_some() {
                return Err("duplicate managed-region end marker".into());
            }
            end = Some(line);
        }
    }
    match (begin, end) {
        (None, None) => Ok(None),
        (Some(b), Some(e)) if b <= e => Ok(Some((b, e))),
        _ => Err("unpaired or reversed managed-region markers".into()),
    }
}

pub fn has_region(text: &str, tag: &str) -> Result<bool, String> {
    let doc = parse(text)?;
    let Some(value) = get(&doc.value, "module").and_then(|m| get(m, "requestPermissions")) else {
        return Ok(false);
    };
    if !matches!(value, Value::JSONArray { .. }) {
        return Err("module.requestPermissions must be an array".into());
    }
    Ok(markers(&value.to_string(), tag)?.is_some())
}

pub fn replace_region(text: &str, tag: &str, body: &str) -> Option<String> {
    let mut doc = parse(text).ok()?;
    let value = permissions(&mut doc, "requestPermissions").ok()?;
    let source = value.to_string();
    let (b, e) = markers(&source, tag).ok()??;
    let edited = format!("{}{body}{}", &source[..b], &source[e..]);
    *value = parse(&edited).ok()?.value;
    Some(doc.to_string())
}

pub fn ensure_region(text: &str, key: &str, tag: &str) -> Result<String, String> {
    let mut doc = parse(text)?;
    let value = permissions(&mut doc, key)?;
    let source = value.to_string();
    if markers(&source, tag)?.is_some() {
        return Ok(text.into());
    }
    let tokens = json_five::source_to_tokens(&source).map_err(|e| e.to_string())?;
    let significant: Vec<_> = tokens
        .iter()
        .filter(|t| {
            !matches!(
                t.tok_type,
                TokType::Whitespace | TokType::LineComment | TokType::BlockComment | TokType::EOF
            )
        })
        .collect();
    let close = significant
        .last()
        .ok_or("empty JSON5 array tokens")?
        .context
        .as_ref()
        .unwrap()
        .start_byte_offset;
    let prior = significant[significant.len() - 2];
    let mut edited = source[..close].to_string();
    if !matches!(prior.tok_type, TokType::Comma | TokType::LeftBracket) {
        // Put the comma immediately after the last value, before any trailing line comment.
        edited.insert(prior.context.as_ref().unwrap().end_byte_offset, ',');
    }
    let indent = source[..close]
        .rsplit('\n')
        .next()
        .unwrap_or("")
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect::<String>();
    edited.push_str(&format!("\n{indent}  // day:{tag}-begin — generated by `day build` from [permissions] in Day.toml.\n{indent}  // Everything between these markers is rewritten every build; edit Day.toml, not here.\n{indent}  // day:{tag}-end\n{indent}]"));
    *value = parse(&edited)?.value;
    Ok(doc.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    // The scaffold template, not the showcase's copy: `include_str!` may not reach outside this
    // package (see web.rs), and a fixture the CLI's writers can edit would drift. Same
    // reasoning as plist.rs's `SHOWCASE` fixture; `day new` copies this file verbatim.
    const MODULE: &str =
        include_str!("../templates/app/platform/harmony/entry/src/main/module.json5");

    #[test]
    fn permissions_ignore_comment_examples_and_other_objects() {
        let src = r#"// "requestPermissions": []
{other: {requestPermissions: []}, module: {
  'requestPermissions': [ {name: 'manual'} // keep this explanation
  ],
  note: '// day:permissions-begin'
}}"#;
        let once = ensure_region(src, "requestPermissions", "permissions").unwrap();
        assert!(
            once.starts_with("// \"requestPermissions\": []\n{other: {requestPermissions: []}")
        );
        assert!(once.contains("{name: 'manual'}, // keep this explanation"));
        let out = replace_region(&once, "permissions", "    {name:'generated'},\n").unwrap();
        let mut doc = parse(&out).unwrap();
        assert_eq!(
            array(permissions(&mut doc, "requestPermissions").unwrap()).count(),
            2
        );
        assert!(out.contains("note: '// day:permissions-begin'"));
        assert_eq!(
            ensure_region(&out, "requestPermissions", "permissions").unwrap(),
            out
        );
        assert_eq!(
            replace_region(&out, "permissions", "    {name:'generated'},\n").unwrap(),
            out
        );
    }

    #[test]
    fn region_markers_must_be_paired_and_belong_to_the_array() {
        let broken = "{module:{requestPermissions:[\n// day:permissions-begin\n]}}";
        assert!(ensure_region(broken, "requestPermissions", "permissions").is_err());
        assert!(!has_region("{module:{name:'entry'}}", "permissions").unwrap());
        let nested = "{module:{requestPermissions:[{\n// day:permissions-begin\nname:'manual'\n// day:permissions-end\n}]}}";
        assert!(!has_region(nested, "permissions").unwrap());
        let once = ensure_region(nested, "requestPermissions", "permissions").unwrap();
        let out = replace_region(&once, "permissions", "{name:'generated'},\n").unwrap();
        assert!(out.contains("name:'manual'"));
        let mut doc = parse(&out).unwrap();
        assert_eq!(
            array(permissions(&mut doc, "requestPermissions").unwrap()).count(),
            2
        );
    }

    #[test]
    fn invalid_documents_and_wrong_permission_types_are_rejected() {
        assert!(
            ensure_region(
                "{module:{requestPermissions: [}",
                "requestPermissions",
                "permissions"
            )
            .is_err()
        );
        assert!(
            ensure_region(
                "{module:{requestPermissions: '[]'}}",
                "requestPermissions",
                "permissions"
            )
            .is_err()
        );
        assert!(
            ensure_region(
                "{other:{requestPermissions: []}}",
                "requestPermissions",
                "permissions"
            )
            .is_err()
        );
    }

    #[test]
    fn roundtrip_preserves_json5_comments_and_quoting() {
        let src = "/* before */ { 'm\\u006fdule': {requestPermissions: [], n: 0x20, s: 'a\\\"b'} } // after\n";
        assert_eq!(parse(src).unwrap().to_string(), src);
        let mut doc = parse(src).unwrap();
        assert!(permissions(&mut doc, "requestPermissions").is_ok());
    }

    #[test]
    fn inserts_a_region_then_replaces_it() {
        let once = ensure_region(MODULE, "requestPermissions", "permissions").expect("insert");
        assert!(once.contains("// day:permissions-begin"));
        assert!(once.contains("// day:permissions-end"));
        // The hand-managed entry and the comment explaining it both survive.
        assert!(once.contains("\"ohos.permission.INTERNET\""));
        assert!(once.contains("Required even for the LOOPBACK dayscript engine socket"));

        // Inserting again is a no-op.
        assert_eq!(
            ensure_region(&once, "requestPermissions", "permissions").unwrap(),
            once
        );

        let filled = replace_region(
            &once,
            "permissions",
            "      { \"name\": \"ohos.permission.CAMERA\" },\n",
        )
        .expect("replace");
        assert!(filled.contains("ohos.permission.CAMERA"));
        assert!(filled.contains("\"ohos.permission.INTERNET\""));

        // Replacing with the same body is byte-identical; replacing with a new body drops the old.
        assert_eq!(
            replace_region(
                &filled,
                "permissions",
                "      { \"name\": \"ohos.permission.CAMERA\" },\n"
            )
            .unwrap(),
            filled
        );
        let changed = replace_region(&filled, "permissions", "").unwrap();
        assert!(!changed.contains("ohos.permission.CAMERA"));
        assert!(changed.contains("\"ohos.permission.INTERNET\""));
    }

    #[test]
    fn replace_reports_missing_markers() {
        assert!(replace_region(MODULE, "permissions", "x").is_none());
    }

    #[test]
    fn adds_a_comma_only_when_needed() {
        let with_entries = "{\n  \"requestPermissions\": [\n    { \"name\": \"a\" }\n  ]\n}\n";
        let wrapped = format!("{{module: {with_entries}}}");
        let out = ensure_region(&wrapped, "requestPermissions", "permissions").unwrap();
        assert!(
            out.contains("{ \"name\": \"a\" },"),
            "needs a separating comma:\n{out}"
        );

        let empty = "{\n  \"requestPermissions\": [\n  ]\n}\n";
        let wrapped = format!("{{module: {empty}}}");
        let out = ensure_region(&wrapped, "requestPermissions", "permissions").unwrap();
        assert!(
            !out.contains("[,"),
            "an empty array must not gain a leading comma:\n{out}"
        );
    }
}
