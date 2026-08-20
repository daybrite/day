// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! `#[derive(Observable)]` — the day-model derive.
//!
//! For `struct Item { id: u32, name: String, … }` it generates:
//!
//! - `impl day_model::Identified for Item` from the `#[obs(key)]` field — always explicit, never
//!   inferred: a struct that happens to carry an `id` that is not its key would make inference a
//!   trap, and one attribute line is cheap;
//! - an `ItemFields` trait with one accessor per field, implemented for EVERY
//!   `Source<Item>` — so `store.name()`, `store.elem(id).name()` and `item.address().city()`
//!   all work;
//! - `Item::OBSERVED_FIELDS`, so a test can assert what is observable without reflection.
//!
//! `#[obs(skip)]` leaves a field out entirely: no accessor, no path, no trigger.
//!
//! Field ids come from field NAMES (`day_model::field_id`), so no index can be duplicated by
//! hand. Generated paths say `day_model::…` unqualified: day-model depends on nothing that could
//! shadow it, and the `day` facade's prelude re-exports the crate under that name, so both a
//! direct dependency and `use day::prelude::*` resolve it.
//!
//! Same construction as [`crate::build_path!`]: no syn, no quote. A derive that needs field
//! NAMES and type TOKENS never has to understand a type, only re-emit it.

use proc_macro::{Delimiter, TokenStream, TokenTree};

pub(crate) fn observable(input: TokenStream) -> TokenStream {
    match expand(input) {
        Ok(ts) => ts,
        Err(msg) => format!("compile_error!({msg:?});")
            .parse()
            .expect("compile_error! literal always parses"),
    }
}

struct FieldDef {
    name: String,
    ty: String,
    key: bool,
    skip: bool,
}

fn expand(input: TokenStream) -> Result<TokenStream, String> {
    let tokens: Vec<TokenTree> = input.into_iter().collect();

    // struct <Name> { … } — generics are rejected rather than mis-handled.
    let mut i = 0;
    while i < tokens.len() {
        if matches!(&tokens[i], TokenTree::Ident(id) if id.to_string() == "struct") {
            break;
        }
        i += 1;
    }
    if i >= tokens.len() {
        return Err("Observable expects a struct".into());
    }
    let name = match tokens.get(i + 1) {
        Some(TokenTree::Ident(id)) => id.to_string(),
        _ => return Err("Observable expects a named struct".into()),
    };
    let body = match tokens.get(i + 2) {
        Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Brace => g.stream(),
        Some(TokenTree::Punct(p)) if p.as_char() == '<' => {
            return Err("Observable does not support generic structs yet".into());
        }
        _ => return Err("Observable expects a struct with named fields".into()),
    };

    let fields = parse_fields(body)?;
    let observed: Vec<&FieldDef> = fields.iter().filter(|f| !f.skip).collect();
    let keys: Vec<&FieldDef> = fields.iter().filter(|f| f.key).collect();
    if keys.len() > 1 {
        return Err(format!(
            "Observable: `{name}` marks more than one field #[obs(key)]"
        ));
    }

    let mut out = String::new();

    // The key, if one was marked. Without one the struct still observes; putting it in a
    // `Keyed` collection is then a compile error naming `Identified` and this attribute.
    if let Some(k) = keys.first() {
        out.push_str(&format!(
            "impl day_model::Identified for {name} {{\n\
             \x20   fn obs_key(&self) -> u64 {{ self.{} as u64 }}\n\
             }}\n",
            k.name
        ));
    }

    // One accessor per observed field, for every Source of this type.
    let trait_name = format!("{name}Fields");
    out.push_str(&format!(
        "#[allow(non_camel_case_types)]\npub trait {trait_name}: day_model::Source<{name}> + Sized {{\n"
    ));
    for f in &observed {
        out.push_str(&format!(
            "    fn {}(self) -> day_model::Field<Self, {name}, {}> {{\n\
             \x20       day_model::project(self, \"{}\", |s| &s.{}, |s| &mut s.{})\n\
             \x20   }}\n",
            f.name, f.ty, f.name, f.name, f.name
        ));
    }
    out.push_str("}\n");
    out.push_str(&format!(
        "impl<S: day_model::Source<{name}>> {trait_name} for S {{}}\n"
    ));

    // A names list, so a test can assert what is observable without reflection.
    out.push_str(&format!(
        "impl {name} {{\n    pub const OBSERVED_FIELDS: &'static [&'static str] = &[{}];\n}}\n",
        observed
            .iter()
            .map(|f| format!("\"{}\"", f.name))
            .collect::<Vec<_>>()
            .join(", ")
    ));

    out.parse()
        .map_err(|e| format!("generated code did not parse: {e}"))
}

/// Split the brace body on top-level commas and read `#[obs(...)] vis name : Type` out of each.
/// "Top-level" means outside `<…>` too, so a `HashMap<u64, usize>` field stays one chunk —
/// brackets and parens are already single `Group` tokens, but angles arrive as bare puncts.
fn parse_fields(body: TokenStream) -> Result<Vec<FieldDef>, String> {
    let mut fields = Vec::new();
    let mut chunk: Vec<TokenTree> = Vec::new();
    let mut angle_depth = 0i32;
    let mut prev_dash = false;

    for tt in body {
        match &tt {
            TokenTree::Punct(p) if p.as_char() == '<' => {
                angle_depth += 1;
                chunk.push(tt);
            }
            // `->` in a fn-pointer type must not close an angle bracket.
            TokenTree::Punct(p) if p.as_char() == '>' && !prev_dash => {
                angle_depth = (angle_depth - 1).max(0);
                chunk.push(tt);
            }
            TokenTree::Punct(p) if p.as_char() == ',' && angle_depth == 0 => {
                if !chunk.is_empty() {
                    fields.push(parse_one(&chunk)?);
                    chunk.clear();
                }
            }
            _ => chunk.push(tt),
        }
        prev_dash = matches!(
            chunk.last(),
            Some(TokenTree::Punct(p)) if p.as_char() == '-'
        );
    }
    if !chunk.is_empty() {
        fields.push(parse_one(&chunk)?);
    }
    Ok(fields)
}

fn parse_one(chunk: &[TokenTree]) -> Result<FieldDef, String> {
    let mut i = 0;
    let mut key = false;
    let mut skip = false;

    // Attributes and doc comments.
    while i < chunk.len() {
        match &chunk[i] {
            TokenTree::Punct(p) if p.as_char() == '#' => {
                if let Some(TokenTree::Group(g)) = chunk.get(i + 1) {
                    let text = g.stream().to_string();
                    if text.starts_with("obs") {
                        if text.contains("key") {
                            key = true;
                        }
                        if text.contains("skip") {
                            skip = true;
                        }
                    }
                    i += 2;
                    continue;
                }
                return Err("malformed attribute on a field".into());
            }
            _ => break,
        }
    }

    // Visibility.
    if let Some(TokenTree::Ident(id)) = chunk.get(i)
        && id.to_string() == "pub"
    {
        i += 1;
        if let Some(TokenTree::Group(g)) = chunk.get(i)
            && g.delimiter() == Delimiter::Parenthesis
        {
            i += 1;
        }
    }

    let name = match chunk.get(i) {
        Some(TokenTree::Ident(id)) => id.to_string(),
        _ => return Err("expected a field name".into()),
    };
    i += 1;
    match chunk.get(i) {
        Some(TokenTree::Punct(p)) if p.as_char() == ':' => i += 1,
        _ => return Err(format!("expected `:` after field `{name}`")),
    }

    // The type is whatever is left — re-emitted verbatim, never interpreted.
    let ty: String = chunk[i..]
        .iter()
        .map(|t| t.to_string())
        .collect::<Vec<_>>()
        .join(" ");
    if ty.is_empty() {
        return Err(format!("field `{name}` has no type"));
    }

    Ok(FieldDef {
        name,
        ty,
        key,
        skip,
    })
}
