// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! `#[derive(Observable)]` and `#[derive(Model)]` — the day-model and day-persistence derives.
//!
//! For `struct Item { id: u32, name: String, … }`, `Observable` generates:
//!
//! - `impl day_model::Identified for Item` from the `#[obs(key)]` field — always explicit, never
//!   inferred: a struct that happens to carry an `id` that is not its key would make inference a
//!   trap, and one attribute line is cheap;
//! - an `ItemFields` trait with one accessor per field, implemented for EVERY
//!   `Source<Item>` — so `store.name()`, `store.elem(id).name()` and `item.address().city()`
//!   all work;
//! - `Item::OBSERVED_FIELDS`, so a test can assert what is observable without reflection.
//!
//! `#[obs(skip)]` leaves a field out entirely: no accessor, no path, no trigger — and, under
//! `Model`, no column: a field the change log cannot name could never mark its row dirty, so
//! persisting it would silently lose edits.
//!
//! `Model` implies `Observable` and adds the schema half (docs/persistence.md): `impl
//! day_persistence::Model` with the table name, column list, row↔struct mappers and the default
//! row. `#[model(id)]` marks the key (it is `#[obs(key)]` too); field options are `column =
//! "…"`, `unique`, `index`, `transient` (observable, never stored), `with = Codec` and `json`;
//! struct options are `table = "…"` and `index("a", "b")` for composites.
//!
//! Field ids come from field NAMES (`day_model::field_id`), so no index can be duplicated by
//! hand. Generated paths say `day_model::…` / `day_persistence::…` unqualified: the crates
//! depend on nothing that could shadow them, and the `day` facade's prelude re-exports both
//! names, so a direct dependency and `use day::prelude::*` both resolve them.
//!
//! Same construction as [`crate::build_path!`]: no syn, no quote. A derive that needs field
//! NAMES and type TOKENS never has to understand a type, only re-emit it.

use proc_macro::{Delimiter, TokenStream, TokenTree};

pub(crate) fn observable(input: TokenStream) -> TokenStream {
    finish(expand_observable(input))
}

pub(crate) fn model(input: TokenStream) -> TokenStream {
    finish(expand_model(input))
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
    // The #[model(…)] half; inert under a bare Observable.
    column: Option<String>,
    unique: bool,
    indexed: bool,
    transient: bool,
    with: Option<String>,
    json: bool,
    /// `#[model(relation(…))]` — a `Many<T>` marker field: no column, no plain accessor;
    /// the Model derive emits a `RelationRef` accessor and the wiring instead.
    relation: Option<RelationAttr>,
}

/// The parsed `relation(target = T, inverse = "…", delete = "…", ordered = "…", join = "…")`.
struct RelationAttr {
    target: String,
    inverse: Option<String>,
    delete: String,
    ordered: Option<String>,
    join: Option<String>,
}

impl FieldDef {
    fn column_name(&self) -> &str {
        self.column.as_deref().unwrap_or(&self.name)
    }
    fn persisted(&self) -> bool {
        !self.skip && !self.transient && self.relation.is_none()
    }
    fn nullable(&self) -> bool {
        self.ty == "Option" || self.ty.starts_with("Option <")
    }
}

struct StructDef {
    name: String,
    table: Option<String>,
    composites: Vec<Vec<String>>,
    fts: Vec<String>,
    spatial: Option<(String, String)>,
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

fn expand_model(input: TokenStream) -> Result<String, String> {
    let def = parse_struct(input)?;
    let keys: Vec<&FieldDef> = def.fields.iter().filter(|f| f.key).collect();
    let key = match keys.as_slice() {
        [k] => *k,
        [] => {
            return Err(format!(
                "Model: `{}` needs exactly one field marked #[model(id)]",
                def.name
            ));
        }
        _ => {
            return Err(format!(
                "Model: `{}` marks more than one field as the id",
                def.name
            ));
        }
    };
    if key.transient || key.skip {
        return Err(format!(
            "Model: `{}`'s id field cannot be transient or skipped",
            def.name
        ));
    }
    if key.with.is_some() || key.json {
        // The key's stored form must match its `Key` form — the fold's WHERE parameters and
        // external-merge decoding both derive it from the key kind, not from a codec.
        return Err(format!(
            "Model: `{}`'s id field takes no codec — integer, Uuid and String keys store \
             in their canonical forms",
            def.name
        ));
    }

    let mut out = emit_observable(&def);
    out.push_str(&emit_model(&def, key)?);
    Ok(out)
}

// ---------------------------------------------------------------------------
// Emission
// ---------------------------------------------------------------------------

fn emit_observable(def: &StructDef) -> String {
    let name = &def.name;
    // Relation marker fields observe through their RelationRef accessor (Model-emitted), not
    // a plain Field — a `Many` holds nothing a Field could usefully bind.
    let observed: Vec<&FieldDef> = def
        .fields
        .iter()
        .filter(|f| !f.skip && f.relation.is_none())
        .collect();
    let mut out = String::new();

    // The key, if one was marked. Without one the struct still observes; putting it in a
    // `Keyed` collection is then a compile error naming `Identified` and this attribute.
    if let Some(k) = def.fields.iter().find(|f| f.key) {
        // `AsKey` picks the key's form by the FIELD TYPE — integers pass through as their own
        // handle, `Uuid` and `String` intern — so the derive never interprets the type.
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

    // The typed seam an undo stack replays through — and, since `read_field`, the seam
    // label-addressed writes capture priors through and relation wiring reads foreign keys
    // through: one match arm per observed field, downcasting to the field's own type.
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

/// The schema half. Every generated expression goes through `day_persistence::` paths and the
/// field's own type — the derive never guesses a SQL type, it asks `ColumnValue::SQL_TYPE` (or
/// the named codec's) at compile time.
fn emit_model(def: &StructDef, key: &FieldDef) -> Result<String, String> {
    let name = &def.name;
    let table = def.table.clone().unwrap_or_else(|| snake_case(name));
    let persisted: Vec<&FieldDef> = def.fields.iter().filter(|f| f.persisted()).collect();

    let mut columns = String::new();
    for f in &persisted {
        let sql = sql_type_expr(f);
        columns.push_str(&format!(
            "        day_persistence::ColumnDef {{ name: \"{}\", field: \"{}\", sql: {sql}, \
             not_null: {}, unique: {}, indexed: {} }},\n",
            f.column_name(),
            f.name,
            !f.nullable(),
            f.unique,
            f.indexed,
        ));
    }

    // fts()/spatial() name COLUMNS; a typo would silently index nothing, so check now.
    let column_names: Vec<&str> = persisted.iter().map(|f| f.column_name()).collect();
    for c in &def.fts {
        if !column_names.contains(&c.as_str()) {
            return Err(format!(
                "Model: fts(…) names `{c}`, which is not a persisted column of `{name}`"
            ));
        }
    }
    if let Some((lat, lon)) = &def.spatial {
        for c in [lat, lon] {
            if !column_names.contains(&c.as_str()) {
                return Err(format!(
                    "Model: spatial(…) names `{c}`, which is not a persisted column of `{name}`"
                ));
            }
        }
    }

    let composites = def
        .composites
        .iter()
        .map(|cols| {
            format!(
                "&[{}]",
                cols.iter()
                    .map(|c| format!("\"{c}\""))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
        .collect::<Vec<_>>()
        .join(", ");

    let mut to_row = String::new();
    for f in &persisted {
        to_row.push_str(&format!("            {},\n", encode_expr(f)));
    }

    let mut from_row = String::new();
    for f in &def.fields {
        if let Some(i) = persisted.iter().position(|p| p.name == f.name) {
            from_row.push_str(&format!("            {}: {},\n", f.name, decode_expr(f, i)));
        } else {
            from_row.push_str(&format!(
                "            {}: ::core::default::Default::default(),\n",
                f.name
            ));
        }
    }

    // Typed column refs: `Trip::name()` in a predicate, beside `trip.name()` the binding.
    // Inherent fns WITHOUT a receiver never collide with the Fields trait's methods (which
    // take self) — method-call syntax finds the trait, path syntax finds these.
    let mut cols = String::new();
    for f in &persisted {
        let encode = match codec(f) {
            Some(codec) => format!(
                "<{codec} as day_persistence::ValueCodec<{}>>::to_sqlite_value",
                f.ty
            ),
            None => format!("day_persistence::encode_column::<{}>", f.ty),
        };
        cols.push_str(&format!(
            "    pub fn {}() -> day_persistence::Col<{}> {{\n\
             \x20       day_persistence::Col::new(\"{}\", \"{}\", \"{table}\", {encode})\n\
             \x20   }}\n",
            f.name,
            f.ty,
            f.column_name(),
            f.name,
        ));
    }
    // The STATIC relation reference, for predicates. The instance accessor of the same name
    // lives on the Relations trait and takes `self`, so path syntax finds this one and
    // method-call syntax finds that one — the rule `Trip::name()` already relies on.
    for f in def.fields.iter().filter(|f| f.relation.is_some()) {
        let r = f.relation.as_ref().expect("filtered");
        cols.push_str(&format!(
            "    pub fn {}() -> day_persistence::RelationCol<{name}, {}> {{\n\
             \x20       day_persistence::RelationCol::new(\"{}\", \"{table}\", \
             <{} as day_persistence::Model>::TABLE)\n\
             \x20   }}\n",
            f.name, r.target, f.name, r.target,
        ));
    }
    if !def.fts.is_empty() {
        cols.push_str(&format!(
            "    pub fn fts() -> day_persistence::FtsRef {{\n\
             \x20       day_persistence::FtsRef {{ columns: <{name} as day_persistence::Model>::FTS_COLUMNS }}\n\
             \x20   }}\n"
        ));
    }
    if let Some((lat, lon)) = &def.spatial {
        cols.push_str(&format!(
            "    pub fn geo() -> day_persistence::GeoRef {{\n\
             \x20       day_persistence::GeoRef {{ lat: \"{lat}\", lon: \"{lon}\" }}\n\
             \x20   }}\n"
        ));
    }
    let col_impl = format!("#[allow(dead_code)]\nimpl {name} {{\n{cols}}}\n");

    // Relations: the const table, the wiring, and the RelationRef accessor trait.
    let rels: Vec<(&FieldDef, &RelationAttr)> = def
        .fields
        .iter()
        .filter_map(|f| f.relation.as_ref().map(|r| (f, r)))
        .collect();
    let delete_expr = |d: &str| match d {
        "cascade" => "day_persistence::DeleteRule::Cascade",
        "deny" => "day_persistence::DeleteRule::Deny",
        _ => "day_persistence::DeleteRule::Nullify",
    };
    let opt_str = |o: &Option<String>| match o {
        Some(s) => format!("Some(\"{s}\")"),
        None => "None".into(),
    };
    let relations_const = if rels.is_empty() {
        String::new()
    } else {
        format!(
            "    const RELATIONS: &'static [day_persistence::RelationDef] = &[\n{}    ];\n",
            rels.iter()
                .map(|(f, r)| format!(
                    "        day_persistence::RelationDef {{ field: \"{}\", target_table: \
                     <{} as day_persistence::Model>::TABLE, inverse: \"{}\", delete: {}, \
                     ordered: {}, join: {} }},\n",
                    f.name,
                    r.target,
                    r.inverse.as_deref().unwrap_or(""),
                    delete_expr(&r.delete),
                    opt_str(&r.ordered),
                    opt_str(&r.join),
                ))
                .collect::<String>()
        )
    };
    let wire_fn = if rels.is_empty() {
        String::new()
    } else {
        format!(
            "    fn wire(reg: &mut day_persistence::Registrar<'_>) {{\n{}    }}\n",
            rels.iter()
                .map(|(f, r)| match (&r.join, &r.inverse) {
                    (Some(join), _) => format!(
                        "        day_persistence::wire_join::<Self, {}>(reg, \"{}\", \
                         \"{join}\", {}, {});\n",
                        r.target,
                        f.name,
                        opt_str(&r.ordered),
                        delete_expr(&r.delete),
                    ),
                    (None, Some(inverse)) => format!(
                        "        day_persistence::wire_to_many::<Self, {}>(reg, \"{}\", \
                         \"{inverse}\", {}, {});\n",
                        r.target,
                        f.name,
                        delete_expr(&r.delete),
                        opt_str(&r.ordered),
                    ),
                    (None, None) => String::new(), // refused at parse time
                })
                .collect::<String>()
        )
    };
    let rel_trait = if rels.is_empty() {
        String::new()
    } else {
        format!(
            "#[allow(non_camel_case_types)]\n\
             pub trait {name}Relations: day_model::Source<{name}> + Sized {{\n{}}}\n\
             impl<S: day_model::Source<{name}>> {name}Relations for S {{}}\n",
            rels.iter()
                .map(|(f, r)| format!(
                    "    fn {}(self) -> day_persistence::RelationRef<Self, {name}, {}> {{\n\
                     \x20       day_persistence::RelationRef::new(day_model::project(\
                     self, \"{}\", |s| &s.{}, |s| &mut s.{}))\n\
                     \x20   }}\n",
                    f.name, r.target, f.name, f.name, f.name
                ))
                .collect::<String>()
        )
    };

    let fts_const = if def.fts.is_empty() {
        String::new()
    } else {
        format!(
            "    const FTS_COLUMNS: &'static [&'static str] = &[{}];\n",
            def.fts
                .iter()
                .map(|c| format!("\"{c}\""))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let spatial_const = match &def.spatial {
        None => String::new(),
        Some((lat, lon)) => format!(
            "    const SPATIAL: Option<day_persistence::SpatialCols> = \
             Some(day_persistence::SpatialCols {{ lat: \"{lat}\", lon: \"{lon}\" }});\n"
        ),
    };

    let key_sql = sql_type_expr(key);
    Ok(col_impl
        + &rel_trait
        + &format!(
            "impl day_persistence::Model for {name} {{\n\
         \x20   const TABLE: &'static str = \"{table}\";\n\
         \x20   const KEY: &'static str = \"{key_col}\";\n\
         \x20   const KEY_SQL: day_persistence::SqlType = {key_sql};\n\
         \x20   const COLUMNS: &'static [day_persistence::ColumnDef] = &[\n{columns}    ];\n\
         \x20   const COMPOSITE_INDEXES: &'static [&'static [&'static str]] = &[{composites}];\n\
         {fts_const}{spatial_const}{relations_const}\
         \x20   fn to_row(&self) -> Vec<day_persistence::Value> {{\n\
         \x20       vec![\n{to_row}        ]\n\
         \x20   }}\n\
         \x20   fn from_row(row: &dyn day_persistence::Row) -> \
         Result<Self, day_persistence::DbError> {{\n\
         \x20       Ok(Self {{\n{from_row}        }})\n\
         \x20   }}\n\
         \x20   fn default_row() -> Vec<day_persistence::Value> {{\n\
         \x20       day_persistence::Model::to_row(&<Self as ::core::default::Default>::default())\n\
         \x20   }}\n\
         {wire_fn}\
         }}\n",
            key_col = key.column_name(),
        ))
}

fn sql_type_expr(f: &FieldDef) -> String {
    match codec(f) {
        Some(codec) => format!(
            "<{codec} as day_persistence::ValueCodec<{}>>::SQL_TYPE",
            f.ty
        ),
        None => format!("<{} as day_persistence::ColumnValue>::SQL_TYPE", f.ty),
    }
}

fn encode_expr(f: &FieldDef) -> String {
    match codec(f) {
        Some(codec) => format!(
            "<{codec} as day_persistence::ValueCodec<{}>>::to_sqlite_value(&self.{})",
            f.ty, f.name
        ),
        None => format!(
            "day_persistence::ColumnValue::to_sqlite_value(&self.{})",
            f.name
        ),
    }
}

/// A stored NULL decodes as the field's `Default` — what makes an added column's old rows
/// readable before their backfill, matching the model's own deleted-row semantics.
fn decode_expr(f: &FieldDef, i: usize) -> String {
    let read = format!("day_persistence::Row::get(row, {i}usize)");
    match codec(f) {
        Some(codec) => format!(
            "match {read} {{\n\
             \x20               day_persistence::Value::Null => ::core::default::Default::default(),\n\
             \x20               v => <{codec} as day_persistence::ValueCodec<{}>>::from_sqlite_value(v)?,\n\
             \x20           }}",
            f.ty
        ),
        None => format!(
            "match {read} {{\n\
             \x20               day_persistence::Value::Null => ::core::default::Default::default(),\n\
             \x20               v => <{} as day_persistence::ColumnValue>::from_sqlite_value(v)?,\n\
             \x20           }}",
            f.ty
        ),
    }
}

fn codec(f: &FieldDef) -> Option<String> {
    if let Some(with) = &f.with {
        Some(with.clone())
    } else if f.json {
        Some("day_persistence::Json".into())
    } else {
        None
    }
}

fn snake_case(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    for (i, ch) in name.chars().enumerate() {
        if ch.is_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.extend(ch.to_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// The whole derive input: struct-level `#[model(…)]` options, the name, and the fields.
fn parse_struct(input: TokenStream) -> Result<StructDef, String> {
    let tokens: Vec<TokenTree> = input.into_iter().collect();

    let mut table = None;
    let mut composites = Vec::new();
    let mut fts = Vec::new();
    let mut spatial = None;
    let mut i = 0;
    while i < tokens.len() {
        match &tokens[i] {
            TokenTree::Ident(id) if id.to_string() == "struct" => break,
            TokenTree::Punct(p) if p.as_char() == '#' => {
                if let Some(TokenTree::Group(g)) = tokens.get(i + 1) {
                    if let Some(items) = attr_items(g, "model") {
                        parse_struct_options(
                            items,
                            &mut table,
                            &mut composites,
                            &mut fts,
                            &mut spatial,
                        )?;
                    }
                    i += 2;
                    continue;
                }
                return Err("malformed attribute on the struct".into());
            }
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
        table,
        composites,
        fts,
        spatial,
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

fn parse_struct_options(
    items: Vec<Vec<TokenTree>>,
    table: &mut Option<String>,
    composites: &mut Vec<Vec<String>>,
    fts: &mut Vec<String>,
    spatial: &mut Option<(String, String)>,
) -> Result<(), String> {
    for item in items {
        match item.as_slice() {
            [TokenTree::Ident(id), TokenTree::Group(g)]
                if id.to_string() == "fts" && g.delimiter() == Delimiter::Parenthesis =>
            {
                for tt in g.stream() {
                    match &tt {
                        TokenTree::Literal(lit) => fts.push(string_literal(lit)?),
                        TokenTree::Punct(p) if p.as_char() == ',' => {}
                        other => {
                            return Err(format!("fts(…) expects string literals, found `{other}`"));
                        }
                    }
                }
            }
            [TokenTree::Ident(id), TokenTree::Group(g)]
                if id.to_string() == "spatial" && g.delimiter() == Delimiter::Parenthesis =>
            {
                let mut lat = None;
                let mut lon = None;
                let inner: Vec<TokenTree> = g.stream().into_iter().collect();
                let mut j = 0;
                while j + 2 < inner.len() + 1 {
                    match (&inner.get(j), &inner.get(j + 1), &inner.get(j + 2)) {
                        (
                            Some(TokenTree::Ident(k)),
                            Some(TokenTree::Punct(eq)),
                            Some(TokenTree::Literal(lit)),
                        ) if eq.as_char() == '=' => {
                            match k.to_string().as_str() {
                                "lat" => lat = Some(string_literal(lit)?),
                                "lon" => lon = Some(string_literal(lit)?),
                                other => {
                                    return Err(format!(
                                        "spatial(…) takes lat = \"…\" and lon = \"…\", found `{other}`"
                                    ));
                                }
                            }
                            j += 3;
                            if matches!(inner.get(j), Some(TokenTree::Punct(p)) if p.as_char() == ',')
                            {
                                j += 1;
                            }
                        }
                        _ => return Err("spatial(…) takes lat = \"…\" and lon = \"…\"".into()),
                    }
                }
                match (lat, lon) {
                    (Some(lat), Some(lon)) => *spatial = Some((lat, lon)),
                    _ => return Err("spatial(…) needs BOTH lat = \"…\" and lon = \"…\"".into()),
                }
            }
            [
                TokenTree::Ident(id),
                TokenTree::Punct(eq),
                TokenTree::Literal(lit),
            ] if id.to_string() == "table" && eq.as_char() == '=' => {
                *table = Some(string_literal(lit)?);
            }
            [TokenTree::Ident(id), TokenTree::Group(g)]
                if id.to_string() == "index" && g.delimiter() == Delimiter::Parenthesis =>
            {
                let mut cols = Vec::new();
                for tt in g.stream() {
                    match &tt {
                        TokenTree::Literal(lit) => cols.push(string_literal(lit)?),
                        TokenTree::Punct(p) if p.as_char() == ',' => {}
                        other => {
                            return Err(format!(
                                "index(…) expects string literals, found `{other}`"
                            ));
                        }
                    }
                }
                composites.push(cols);
            }
            other => {
                return Err(format!(
                    "unknown #[model] option on the struct: `{}` (supported: table = \"…\", index(\"a\", \"b\"), fts(\"a\", \"b\"), spatial(lat = \"…\", lon = \"…\"))",
                    tokens_text(other)
                ));
            }
        }
    }
    Ok(())
}

fn parse_field_options(items: Vec<Vec<TokenTree>>, f: &mut FieldDef) -> Result<(), String> {
    for item in items {
        match item.as_slice() {
            [TokenTree::Ident(id)] => match id.to_string().as_str() {
                "id" => f.key = true,
                "unique" => f.unique = true,
                "index" => f.indexed = true,
                "transient" => f.transient = true,
                "json" => f.json = true,
                other => {
                    return Err(format!(
                        "unknown #[model] option on field `{}`: `{other}` (supported: id, unique, index, transient, json, column = \"…\", with = Codec)",
                        f.name
                    ));
                }
            },
            [
                TokenTree::Ident(id),
                TokenTree::Punct(eq),
                TokenTree::Literal(lit),
            ] if id.to_string() == "column" && eq.as_char() == '=' => {
                f.column = Some(string_literal(lit)?);
            }
            [TokenTree::Ident(id), TokenTree::Punct(eq), rest @ ..]
                if id.to_string() == "with" && eq.as_char() == '=' && !rest.is_empty() =>
            {
                f.with = Some(tokens_text(rest));
            }
            [TokenTree::Ident(id), TokenTree::Group(g)]
                if id.to_string() == "relation" && g.delimiter() == Delimiter::Parenthesis =>
            {
                f.relation = Some(parse_relation(g, &f.name)?);
            }
            other => {
                return Err(format!(
                    "unknown #[model] option on field `{}`: `{}`",
                    f.name,
                    tokens_text(other)
                ));
            }
        }
    }
    Ok(())
}

/// `relation(target = Type, inverse = "field", delete = "…", ordered = "field", join = "table")`.
fn parse_relation(g: &proc_macro::Group, field: &str) -> Result<RelationAttr, String> {
    let mut items = vec![Vec::new()];
    for tt in g.stream() {
        match &tt {
            TokenTree::Punct(p) if p.as_char() == ',' => items.push(Vec::new()),
            _ => items.last_mut().expect("never empty").push(tt),
        }
    }
    items.retain(|i| !i.is_empty());

    let (mut target, mut inverse, mut delete, mut ordered, mut join) =
        (None, None, None, None, None);
    for item in items {
        match item.as_slice() {
            [TokenTree::Ident(k), TokenTree::Punct(eq), rest @ ..]
                if eq.as_char() == '=' && !rest.is_empty() =>
            {
                match k.to_string().as_str() {
                    "target" => target = Some(tokens_text(rest)),
                    "inverse" => inverse = Some(literal_str(rest, field)?),
                    "delete" => delete = Some(literal_str(rest, field)?),
                    "ordered" => ordered = Some(literal_str(rest, field)?),
                    "join" => join = Some(literal_str(rest, field)?),
                    other => {
                        return Err(format!(
                            "relation(…) on `{field}`: unknown option `{other}` (supported: \
                             target = Type, inverse = \"field\", delete = \"…\", \
                             ordered = \"field\", join = \"table\")"
                        ));
                    }
                }
            }
            // Bare `ordered`, the join form: the position lives on the membership row, so
            // there is no child field to name (one child can sit at different positions
            // under different parents).
            [TokenTree::Ident(k)] if k.to_string() == "ordered" => {
                ordered = Some("position".into());
            }
            other => {
                return Err(format!(
                    "relation(…) on `{field}`: expected `name = …`, found `{}`",
                    tokens_text(other)
                ));
            }
        }
    }
    let Some(target) = target else {
        return Err(format!("relation(…) on `{field}` needs target = Type"));
    };
    let delete = delete.unwrap_or_else(|| "nullify".into());
    if !["nullify", "cascade", "deny"].contains(&delete.as_str()) {
        return Err(format!(
            "relation(…) on `{field}`: delete = {delete:?} — nullify, cascade, or deny"
        ));
    }
    if inverse.is_none() && join.is_none() {
        return Err(format!(
            "relation(…) on `{field}` needs inverse = \"field\" (the target's One<…> field) \
             — or join = \"table\" for a many-to-many"
        ));
    }
    Ok(RelationAttr {
        target,
        inverse,
        delete,
        ordered,
        join,
    })
}

fn literal_str(rest: &[TokenTree], field: &str) -> Result<String, String> {
    match rest {
        [TokenTree::Literal(l)] => string_literal(l),
        other => Err(format!(
            "relation(…) on `{field}`: expected a string literal, found `{}`",
            tokens_text(other)
        )),
    }
}

fn string_literal(lit: &proc_macro::Literal) -> Result<String, String> {
    let s = lit.to_string();
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        Ok(s[1..s.len() - 1].to_string())
    } else {
        Err(format!("expected a string literal, found `{s}`"))
    }
}

fn tokens_text(tokens: &[TokenTree]) -> String {
    tokens
        .iter()
        .map(|t| t.to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Split the brace body on top-level commas and read `#[obs|model(...)] vis name : Type` out of
/// each. "Top-level" means outside `<…>` too, so a `HashMap<u64, usize>` field stays one chunk —
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
    let mut deferred: Vec<Vec<Vec<TokenTree>>> = Vec::new();

    // Attributes and doc comments. `#[model(…)]` items need the field NAME for their errors,
    // so they are parsed after it is known.
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
                        deferred.push(items);
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

    // The type is whatever is left — re-emitted verbatim, never interpreted.
    f.ty = tokens_text(&chunk[i..]);
    if f.ty.is_empty() {
        return Err(format!("field `{}` has no type", f.name));
    }

    for items in deferred {
        parse_field_options(items, &mut f)?;
    }
    Ok(f)
}
