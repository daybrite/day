// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! `#[derive(Observable)]`: the day-model derive, and `observable_impls!`, the same expansion
//! for a struct that is described rather than derived.
//!
//! For `struct Item { id: u32, name: String, … }`, `Observable` generates:
//!
//! - `impl day_model::Identified for Item` from the `#[obs(key)]` field, always explicit, never
//!   inferred: a struct that happens to carry an `id` that is not its key would make inference a
//!   trap, and one attribute line is cheap;
//! - an `ItemFields` trait with one accessor per field, implemented for every
//!   `Source<Item>`, so `store.name()`, `store.elem(id).name()` and `item.address().city()`
//!   all work;
//! - `impl day_model::ApplyField for Item`, the typed path undo replays through;
//! - `Item::OBSERVED_FIELDS`, so a test can assert what is observable without reflection.
//!
//! `#[obs(skip)]` leaves a field out entirely: it gets no accessor, path or trigger.
//!
//! `#[model(…)]` attributes are accepted and all but ignored: `#[model(id)]` marks the key, the
//! same as `#[obs(key)]`, and nothing else is read. They belong to day-persistence's `Model`
//! derive; tolerating them here is what lets one struct derive `Model` where it is stored and
//! `Observable` where it is not, behind `cfg_attr`, without restating its fields.
//!
//! `observable_impls! { struct Item { #[obs(key)] id: u32, name: String } }` emits the same
//! impls without the struct itself. It is the hook for a derive layered on `Observable`: such a
//! derive owns its own attribute grammar, restates the struct with only the fields it wants
//! observed, and expands to this macro, so the accessors have one implementation. day-persistence's
//! `#[derive(Model)]` is built that way (docs/persistence.md).
//!
//! Field ids come from field names (`day_model::field_id`), so no index can be duplicated by
//! hand. Generated paths say `day_model::…` unqualified: the crate depends on nothing that could
//! shadow it, and the `day` facade's prelude re-exports the name, so a direct dependency and
//! `use day::prelude::*` both resolve it.
//!
//! Same construction as [`crate::build_path!`], with neither syn nor quote. A derive that needs
//! field names and type tokens never has to understand a type, only re-emit it.

use proc_macro::{Delimiter, TokenStream, TokenTree};

pub(crate) fn observable(input: TokenStream) -> TokenStream {
    finish(expand_observable(input))
}

fn finish(result: Result<String, String>) -> TokenStream {
    match result {
        Ok(out) => out
            .parse()
            .unwrap_or_else(|e| panic!("generated code did not parse: {e}")),
        Err(msg) => format!("compile_error!({msg:?});")
            .parse()
            .expect("compile_error! literal always parses"),
    }
}

#[derive(Default)]
struct FieldDef {
    name: String,
    ty: String,
    key: bool,
    skip: bool,
}

struct StructDef {
    name: String,
    fields: Vec<FieldDef>,
}

fn expand_observable(input: TokenStream) -> Result<String, String> {
    let def = parse_struct(input)?;
    let keys: Vec<&FieldDef> = def.fields.iter().filter(|f| f.key).collect();
    if keys.len() > 1 {
        return Err(format!(
            "Observable: `{}` marks more than one field #[obs(key)]",
            def.name
        ));
    }
    Ok(emit_observable(&def))
}

// ---------------------------------------------------------------------------
// Emission
// ---------------------------------------------------------------------------

fn emit_observable(def: &StructDef) -> String {
    let name = &def.name;
    let observed: Vec<&FieldDef> = def.fields.iter().filter(|f| !f.skip).collect();
    let mut out = String::new();

    // The key, if one was marked. Without one the struct still observes; putting it in a
    // `Keyed` collection is then a compile error naming `Identified` and this attribute.
    if let Some(k) = def.fields.iter().find(|f| f.key) {
        // `AsKey` picks the key's form by the field type (integers pass through as their own
        // handle, `Uuid` and `String` intern), so the derive never interprets the type.
        out.push_str(&format!(
            "impl day_model::Identified for {name} {{\n\
             \x20   fn key(&self) -> day_model::Key {{ day_model::AsKey::as_key(&self.{}) }}\n\
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

    // The `ApplyField` impl: the typed path an undo stack replays through and, since
    // `read_field`, the one label-addressed writes capture priors through and relation wiring
    // reads foreign keys through. One match arm per observed field, downcasting to the field's
    // type.
    out.push_str(&format!(
        "impl day_model::ApplyField for {name} {{\n\
         \x20   fn apply_field(&mut self, label: &str, value: &dyn ::core::any::Any) -> bool {{\n\
         \x20       match label {{\n{}\
         \x20           _ => false,\n\
         \x20       }}\n\
         \x20   }}\n\
         \x20   fn read_field(&self, label: &str) -> \
         Option<::std::rc::Rc<dyn ::core::any::Any>> {{\n\
         \x20       match label {{\n{}\
         \x20           _ => None,\n\
         \x20       }}\n\
         \x20   }}\n\
         }}\n",
        observed
            .iter()
            .map(|f| format!(
                "            \"{}\" => match value.downcast_ref::<{}>() {{\n\
                 \x20               Some(v) => {{\n\
                 \x20                   self.{} = v.clone();\n\
                 \x20                   true\n\
                 \x20               }}\n\
                 \x20               None => false,\n\
                 \x20           }},\n",
                f.name, f.ty, f.name
            ))
            .collect::<String>(),
        observed
            .iter()
            .map(|f| format!(
                "            \"{}\" => Some(::std::rc::Rc::new(self.{}.clone())),\n",
                f.name, f.name
            ))
            .collect::<String>()
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
    out
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// The whole input: the name and the fields. Attributes on the struct itself belong to other
/// derives and are stepped over.
fn parse_struct(input: TokenStream) -> Result<StructDef, String> {
    let tokens: Vec<TokenTree> = input.into_iter().collect();

    let mut i = 0;
    while i < tokens.len() {
        match &tokens[i] {
            TokenTree::Ident(id) if id.to_string() == "struct" => break,
            _ => i += 1,
        }
    }
    if i >= tokens.len() {
        return Err("the derive expects a struct".into());
    }
    let name = match tokens.get(i + 1) {
        Some(TokenTree::Ident(id)) => id.to_string(),
        _ => return Err("the derive expects a named struct".into()),
    };
    let body = match tokens.get(i + 2) {
        Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Brace => g.stream(),
        Some(TokenTree::Punct(p)) if p.as_char() == '<' => {
            return Err("the derive does not support generic structs yet".into());
        }
        _ => return Err("the derive expects a struct with named fields".into()),
    };

    Ok(StructDef {
        name,
        fields: parse_fields(body)?,
    })
}

/// If the bracket group is `name(…)`, return the paren group's items split on top-level commas.
fn attr_items(bracket: &proc_macro::Group, name: &str) -> Option<Vec<Vec<TokenTree>>> {
    let inner: Vec<TokenTree> = bracket.stream().into_iter().collect();
    match (inner.first(), inner.get(1)) {
        (Some(TokenTree::Ident(id)), Some(TokenTree::Group(g)))
            if id.to_string() == name && g.delimiter() == Delimiter::Parenthesis =>
        {
            let mut items = vec![Vec::new()];
            for tt in g.stream() {
                match &tt {
                    TokenTree::Punct(p) if p.as_char() == ',' => items.push(Vec::new()),
                    _ => items.last_mut().expect("never empty").push(tt),
                }
            }
            items.retain(|item| !item.is_empty());
            Some(items)
        }
        _ => None,
    }
}

fn tokens_text(tokens: &[TokenTree]) -> String {
    tokens.iter().cloned().collect::<TokenStream>().to_string()
}

/// Split the brace body on top-level commas and read `#[obs|model(...)] vis name : Type` out of
/// each. "Top-level" means outside `<…>` too, so a `HashMap<u64, usize>` field stays one chunk;
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
    let mut f = FieldDef::default();

    // Attributes and doc comments. `#[obs(…)]` is this derive's own. `#[model(…)]` belongs to
    // day-persistence's `Model` derive and is tolerated here so a struct can swap derives per
    // target (docs/persistence.md): its `id` marks the key, and the rest is ignored. Any other
    // attribute is another macro's business.
    while i < chunk.len() {
        match &chunk[i] {
            TokenTree::Punct(p) if p.as_char() == '#' => {
                if let Some(TokenTree::Group(g)) = chunk.get(i + 1) {
                    if let Some(items) = attr_items(g, "obs") {
                        for item in items {
                            match item.as_slice() {
                                [TokenTree::Ident(id)] if id.to_string() == "key" => f.key = true,
                                [TokenTree::Ident(id)] if id.to_string() == "skip" => {
                                    f.skip = true;
                                }
                                other => {
                                    return Err(format!(
                                        "unknown #[obs] option: `{}` (supported: key, skip)",
                                        tokens_text(other)
                                    ));
                                }
                            }
                        }
                    } else if let Some(items) = attr_items(g, "model") {
                        f.key |= items.iter().any(|item| {
                            matches!(item.as_slice(), [TokenTree::Ident(id)] if id.to_string() == "id")
                        });
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

    f.name = match chunk.get(i) {
        Some(TokenTree::Ident(id)) => id.to_string(),
        _ => return Err("expected a field name".into()),
    };
    i += 1;
    match chunk.get(i) {
        Some(TokenTree::Punct(p)) if p.as_char() == ':' => i += 1,
        _ => return Err(format!("expected `:` after field `{}`", f.name)),
    }

    // The type is whatever is left, re-emitted verbatim, never interpreted.
    f.ty = tokens_text(&chunk[i..]);
    if f.ty.is_empty() {
        return Err(format!("field `{}` has no type", f.name));
    }

    Ok(f)
}
