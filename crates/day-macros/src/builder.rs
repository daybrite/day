// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! `#[builder(TraitName)]`: a piece's builder trait, generated from its inherent impl
//! (docs/api-style.md "Typed builders and erasure").
//!
//! The attribute goes on the inherent `impl` block that holds a piece's builder methods. The
//! block is emitted unchanged, followed by three generated items:
//!
//! - `pub trait TraitName: Sized`, declaring every `pub fn` of the block that takes `self` by
//!   value and returns `Self`;
//! - `impl TraitName for <the piece>`, forwarding each method to the inherent one;
//! - `impl<Inner: TraitName + Piece> TraitName for Decorated<Inner>`, forwarding through
//!   `Decorated::map_inner`, which is what lets `label(…).padding(8.0).font(…)` resolve.
//!
//! A trait that carries one of the impl's type parameters names it in the attribute
//! (`#[builder(NavBuilder<K>)]`); the parameter's bounds are copied from the impl's generic list.
//! `Decorated` and `Piece` must be in scope where the attribute is used, as they are under
//! `use day::prelude::*`.
//!
//! Methods that take `&self`, `&mut self` or no receiver, return anything other than `Self`,
//! or are not `pub` are left out: they are not builder steps. `#[doc]`, `#[cfg]` and `#[allow]`
//! attributes are carried onto the generated declarations.
//!
//! Same construction as the crate's other macros: a token walk, with neither syn nor quote.

use proc_macro::{Delimiter, Spacing, TokenStream, TokenTree};

/// One builder method of the impl block, as the pieces the generated items need.
struct Method {
    /// `#[doc]`, `#[cfg]` and `#[allow]` attributes, verbatim.
    attrs: String,
    name: String,
    /// The method's own generic parameter list, `<…>` included, or empty.
    generics: String,
    /// The parameters after `self`, each as `name: Type`, verbatim.
    params: Vec<String>,
    /// The parameter names, in order.
    args: Vec<String>,
    /// The method's `where` clause, `where` included, or empty.
    where_clause: String,
}

/// The attribute's argument: the trait name and the impl type parameters it carries.
struct TraitSpec {
    name: String,
    params: Vec<String>,
}

pub fn builder(attr: TokenStream, item: TokenStream) -> TokenStream {
    match expand(attr, item.clone()) {
        Ok(generated) => {
            let mut out = item;
            out.extend(generated);
            out
        }
        Err(message) => {
            let mut out = item;
            out.extend(crate::compile_error(&format!("#[builder]: {message}")));
            out
        }
    }
}

fn expand(attr: TokenStream, item: TokenStream) -> Result<TokenStream, String> {
    let spec = parse_attr(attr)?;
    let tokens: Vec<TokenTree> = item.into_iter().collect();
    let mut i = 0;
    // Outer attributes and visibility before `impl` are allowed and skipped.
    while i < tokens.len() {
        match &tokens[i] {
            TokenTree::Punct(p) if p.as_char() == '#' => i += 2,
            TokenTree::Ident(id) if id.to_string() == "impl" => break,
            _ => i += 1,
        }
    }
    if i >= tokens.len() {
        return Err("expected an inherent `impl` block".into());
    }
    i += 1;
    // The impl's generic parameters.
    let mut impl_generics: Vec<TokenTree> = Vec::new();
    if matches!(&tokens.get(i), Some(TokenTree::Punct(p)) if p.as_char() == '<') {
        let end = matching_angle(&tokens, i)?;
        impl_generics = tokens[i + 1..end].to_vec();
        i = end + 1;
    }
    // The self type, up to `where` or the body.
    let mut self_ty: Vec<TokenTree> = Vec::new();
    while i < tokens.len() {
        match &tokens[i] {
            TokenTree::Group(g) if g.delimiter() == Delimiter::Brace => break,
            TokenTree::Ident(id) if id.to_string() == "where" => break,
            t => self_ty.push(t.clone()),
        }
        i += 1;
    }
    if self_ty
        .iter()
        .any(|t| matches!(t, TokenTree::Ident(id) if id.to_string() == "for"))
    {
        return Err("the attribute goes on an inherent impl, not a trait impl".into());
    }
    let mut impl_where: Vec<TokenTree> = Vec::new();
    while i < tokens.len() {
        match &tokens[i] {
            TokenTree::Group(g) if g.delimiter() == Delimiter::Brace => break,
            t => impl_where.push(t.clone()),
        }
        i += 1;
    }
    let Some(TokenTree::Group(body)) = tokens.get(i) else {
        return Err("expected the impl block's body".into());
    };
    let methods = parse_methods(body.stream())?;
    if methods.is_empty() {
        return Err("the impl block has no `pub fn …(self, …) -> Self` builder methods".into());
    }

    // The trait's own generic parameters, with the bounds the impl gives them.
    let impl_params = split_top_level(&impl_generics, ',');
    let mut trait_params_bounded: Vec<String> = Vec::new();
    for p in &spec.params {
        let found = impl_params.iter().find(
            |group| matches!(group.first(), Some(TokenTree::Ident(id)) if id.to_string() == *p),
        );
        match found {
            Some(group) => trait_params_bounded.push(render(group)),
            None => {
                return Err(format!(
                    "`{p}` is named in the attribute but is not a type parameter of the impl"
                ));
            }
        }
    }
    let trait_generics_decl = if trait_params_bounded.is_empty() {
        String::new()
    } else {
        format!("<{}>", trait_params_bounded.join(", "))
    };
    let trait_generics_use = if spec.params.is_empty() {
        String::new()
    } else {
        format!("<{}>", spec.params.join(", "))
    };
    let trait_path = format!("{}{}", spec.name, trait_generics_use);
    let impl_generics_src = if impl_generics.is_empty() {
        String::new()
    } else {
        format!("<{}>", render(&impl_generics))
    };
    let self_ty_src = render(&self_ty);
    let impl_where_src = render(&impl_where);

    let mut out = String::new();
    // The trait.
    out.push_str(&format!(
        "#[doc = \"Builder methods of [`{}`], reachable through a [`Decorated`] (docs/api-style.md \\\"Typed builders and erasure\\\"): generic modifiers and typed ones chain in any order.\"]\n",
        first_ident(&self_ty)
    ));
    out.push_str(&format!(
        "pub trait {}{}: Sized {{\n",
        spec.name, trait_generics_decl
    ));
    for m in &methods {
        out.push_str(&format!(
            "{}fn {}{}(self{}) -> Self {};\n",
            m.attrs,
            m.name,
            m.generics,
            params_src(&m.params),
            m.where_clause
        ));
    }
    out.push_str("}\n");
    // The impl for the piece: each method forwards to the inherent one. `Self::name` resolves
    // to the inherent method, since inherent items take precedence over trait items.
    out.push_str(&format!(
        "impl{} {} for {} {} {{\n",
        impl_generics_src, trait_path, self_ty_src, impl_where_src
    ));
    for m in &methods {
        out.push_str(&format!(
            "{}fn {}{}(self{}) -> Self {} {{ Self::{}(self{}) }}\n",
            cfg_only(&m.attrs),
            m.name,
            m.generics,
            params_src(&m.params),
            m.where_clause,
            m.name,
            args_src(&m.args)
        ));
    }
    out.push_str("}\n");
    // The impl for `Decorated`: through `map_inner`, so the modifier chain stays flat.
    let decorated_generics = {
        let mut g = trait_params_bounded.clone();
        g.push(format!("DecoratedInner: {trait_path} + Piece"));
        format!("<{}>", g.join(", "))
    };
    out.push_str(&format!(
        "impl{} {} for Decorated<DecoratedInner> {{\n",
        decorated_generics, trait_path
    ));
    for m in &methods {
        out.push_str(&format!(
            "{}fn {}{}(self{}) -> Self {} {{ self.map_inner(|inner| inner.{}({})) }}\n",
            cfg_only(&m.attrs),
            m.name,
            m.generics,
            params_src(&m.params),
            m.where_clause,
            m.name,
            m.args.join(", ")
        ));
    }
    out.push_str("}\n");

    out.parse()
        .map_err(|e| format!("generated code did not parse: {e}\n{out}"))
}

fn parse_attr(attr: TokenStream) -> Result<TraitSpec, String> {
    let tokens: Vec<TokenTree> = attr.into_iter().collect();
    let Some(TokenTree::Ident(name)) = tokens.first() else {
        return Err("expected the builder trait's name, e.g. `#[builder(LabelBuilder)]`".into());
    };
    let mut params = Vec::new();
    if tokens.len() > 1 {
        if !matches!(&tokens[1], TokenTree::Punct(p) if p.as_char() == '<') {
            return Err("expected `TraitName` or `TraitName<K, …>`".into());
        }
        let end = matching_angle(&tokens, 1)?;
        for group in split_top_level(&tokens[2..end], ',') {
            match group.as_slice() {
                [TokenTree::Ident(id)] => params.push(id.to_string()),
                _ => return Err("trait parameters must be bare type parameter names".into()),
            }
        }
        if end + 1 != tokens.len() {
            return Err("unexpected tokens after the trait name".into());
        }
    }
    Ok(TraitSpec {
        name: name.to_string(),
        params,
    })
}

fn parse_methods(body: TokenStream) -> Result<Vec<Method>, String> {
    let tokens: Vec<TokenTree> = body.into_iter().collect();
    let mut methods = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        // Attributes.
        let mut attrs = String::new();
        while let (Some(TokenTree::Punct(p)), Some(TokenTree::Group(g))) =
            (tokens.get(i), tokens.get(i + 1))
            && p.as_char() == '#'
            && g.delimiter() == Delimiter::Bracket
        {
            let first = g.stream().into_iter().next();
            if let Some(TokenTree::Ident(id)) = first
                && matches!(
                    id.to_string().as_str(),
                    "doc" | "cfg" | "allow" | "cfg_attr"
                )
            {
                attrs.push_str(&format!("#[{}]\n", g.stream()));
            }
            i += 2;
        }
        // Visibility.
        let mut is_pub = false;
        if matches!(tokens.get(i), Some(TokenTree::Ident(id)) if id.to_string() == "pub") {
            i += 1;
            if matches!(tokens.get(i), Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Parenthesis)
            {
                // `pub(crate)` and friends: not part of the public builder surface.
                i += 1;
            } else {
                is_pub = true;
            }
        }
        // Qualifiers before `fn` (`const`, `async`, `unsafe`) disqualify a method; `const`
        // and `type` items are skipped to their `;`.
        let Some(TokenTree::Ident(kw)) = tokens.get(i) else {
            i += 1;
            continue;
        };
        match kw.to_string().as_str() {
            "fn" => {}
            "const" | "type" | "static" => {
                while i < tokens.len()
                    && !matches!(&tokens[i], TokenTree::Punct(p) if p.as_char() == ';')
                {
                    i += 1;
                }
                i += 1;
                continue;
            }
            _ => {
                // `async fn`, `unsafe fn`, macro invocations, …: skip to the end of the item.
                i = skip_item(&tokens, i);
                continue;
            }
        }
        i += 1;
        let Some(TokenTree::Ident(name)) = tokens.get(i) else {
            return Err("expected a method name after `fn`".into());
        };
        let name = name.to_string();
        i += 1;
        let mut generics = String::new();
        if matches!(tokens.get(i), Some(TokenTree::Punct(p)) if p.as_char() == '<') {
            let end = matching_angle(&tokens, i)?;
            generics = format!("<{}>", render(&tokens[i + 1..end]));
            i = end + 1;
        }
        let Some(TokenTree::Group(params)) = tokens.get(i) else {
            return Err(format!("expected `{name}`'s parameter list"));
        };
        if params.delimiter() != Delimiter::Parenthesis {
            return Err(format!("expected `{name}`'s parameter list"));
        }
        let param_tokens: Vec<TokenTree> = params.stream().into_iter().collect();
        i += 1;
        // The return type: tokens between `->` and `where` or the body.
        let mut ret: Vec<TokenTree> = Vec::new();
        let mut where_tokens: Vec<TokenTree> = Vec::new();
        let mut has_arrow = false;
        if let (Some(TokenTree::Punct(a)), Some(TokenTree::Punct(b))) =
            (tokens.get(i), tokens.get(i + 1))
            && a.as_char() == '-'
            && b.as_char() == '>'
        {
            has_arrow = true;
            i += 2;
            while i < tokens.len() {
                match &tokens[i] {
                    TokenTree::Group(g) if g.delimiter() == Delimiter::Brace => break,
                    TokenTree::Ident(id) if id.to_string() == "where" => break,
                    t => ret.push(t.clone()),
                }
                i += 1;
            }
        }
        while i < tokens.len() {
            match &tokens[i] {
                TokenTree::Group(g) if g.delimiter() == Delimiter::Brace => break,
                TokenTree::Punct(p) if p.as_char() == ';' => break,
                t => where_tokens.push(t.clone()),
            }
            i += 1;
        }
        // The body (or `;` for a declaration, which an inherent impl never has).
        i += 1;

        let Some((params, args)) = split_params(&param_tokens) else {
            continue; // no by-value `self` receiver: not a builder step
        };
        let returns_self = has_arrow
            && ret.len() == 1
            && matches!(&ret[0], TokenTree::Ident(id) if id.to_string() == "Self");
        if !is_pub || !returns_self {
            continue;
        }
        methods.push(Method {
            attrs,
            name,
            generics,
            params,
            args,
            where_clause: render(&where_tokens),
        });
    }
    Ok(methods)
}

/// Skip a non-`fn` item starting at `i`: to the end of its brace group or its `;`.
fn skip_item(tokens: &[TokenTree], mut i: usize) -> usize {
    while i < tokens.len() {
        match &tokens[i] {
            TokenTree::Group(g) if g.delimiter() == Delimiter::Brace => return i + 1,
            TokenTree::Punct(p) if p.as_char() == ';' => return i + 1,
            _ => i += 1,
        }
    }
    i
}

/// Split a parameter list into the parameters after a by-value `self` (`name: Type`, verbatim)
/// and their names. `None` when the first parameter is not `self` or `mut self`.
fn split_params(tokens: &[TokenTree]) -> Option<(Vec<String>, Vec<String>)> {
    let groups = split_top_level(tokens, ',');
    let receiver = groups.first()?;
    let is_self = match receiver.as_slice() {
        [TokenTree::Ident(s)] => s.to_string() == "self",
        [TokenTree::Ident(m), TokenTree::Ident(s)] => {
            m.to_string() == "mut" && s.to_string() == "self"
        }
        _ => false,
    };
    if !is_self {
        return None;
    }
    let mut params = Vec::new();
    let mut args = Vec::new();
    for group in &groups[1..] {
        if group.is_empty() {
            continue; // trailing comma
        }
        let colon = group
            .iter()
            .position(|t| matches!(t, TokenTree::Punct(p) if p.as_char() == ':'))?;
        let name = match &group[..colon] {
            [TokenTree::Ident(n)] => n.to_string(),
            [TokenTree::Ident(m), TokenTree::Ident(n)] if m.to_string() == "mut" => n.to_string(),
            _ => return None, // a pattern parameter: not forwardable by name
        };
        params.push(format!("{}: {}", name, render(&group[colon + 1..])));
        args.push(name);
    }
    Some((params, args))
}

/// Index of the `>` closing the `<` at `open`, with `->` and nested angles accounted for.
fn matching_angle(tokens: &[TokenTree], open: usize) -> Result<usize, String> {
    let mut depth = 0usize;
    let mut i = open;
    while i < tokens.len() {
        if let TokenTree::Punct(p) = &tokens[i] {
            match p.as_char() {
                '<' => depth += 1,
                '>' => {
                    // The `>` of `->` is not a closing angle.
                    let arrow = i > 0
                        && matches!(&tokens[i - 1], TokenTree::Punct(q)
                            if q.as_char() == '-' && q.spacing() == Spacing::Joint);
                    if !arrow {
                        depth -= 1;
                        if depth == 0 {
                            return Ok(i);
                        }
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
    Err("unbalanced `<` in a generic parameter list".into())
}

/// Split at every `sep` punct that sits outside angle brackets (groups are already single
/// tokens, so parentheses and braces need no tracking).
fn split_top_level(tokens: &[TokenTree], sep: char) -> Vec<Vec<TokenTree>> {
    let mut out = vec![Vec::new()];
    let mut depth = 0usize;
    for (i, t) in tokens.iter().enumerate() {
        if let TokenTree::Punct(p) = t {
            match p.as_char() {
                '<' => depth += 1,
                '>' => {
                    let arrow = i > 0
                        && matches!(&tokens[i - 1], TokenTree::Punct(q)
                            if q.as_char() == '-' && q.spacing() == Spacing::Joint);
                    if !arrow {
                        depth = depth.saturating_sub(1);
                    }
                }
                c if c == sep && depth == 0 => {
                    out.push(Vec::new());
                    continue;
                }
                _ => {}
            }
        }
        if let Some(last) = out.last_mut() {
            last.push(t.clone());
        }
    }
    out
}

fn render(tokens: &[TokenTree]) -> String {
    tokens.iter().cloned().collect::<TokenStream>().to_string()
}

fn first_ident(tokens: &[TokenTree]) -> String {
    tokens
        .iter()
        .find_map(|t| match t {
            TokenTree::Ident(id) => Some(id.to_string()),
            _ => None,
        })
        .unwrap_or_default()
}

fn params_src(params: &[String]) -> String {
    params.iter().map(|p| format!(", {p}")).collect()
}

fn args_src(args: &[String]) -> String {
    args.iter().map(|a| format!(", {a}")).collect()
}

/// Only the `#[cfg]` attributes of a method, for the generated impls (docs stay on the trait).
fn cfg_only(attrs: &str) -> String {
    attrs
        .lines()
        .filter(|l| l.starts_with("#[cfg"))
        .map(|l| format!("{l}\n"))
        .collect()
}
