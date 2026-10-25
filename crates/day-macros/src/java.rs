// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! `java_class!`: a typed binding to one Java class for day-platform-android (docs/android-jni.md).
//!
//! The input is jni's `bind_java_type!` grammar, and the expansion is a `bind_java_type!` call
//! with Day's policies around it. Four things are added:
//!
//! - **Java names, verbatim.** Every method and field is rewritten to the macro's block form
//!   with an explicit `name = "…"`, so `makeCombo` binds `makeCombo` and `speak_native` binds
//!   `speak_native`. jni's own default turns a snake-case name into camel case, which would
//!   make a Rust name and its Java target differ.
//! - **A module of its own** holding the generated items, so the lint allows their Java names
//!   and jni's generated code need (`non_snake_case`, and jni-macros 0.22's `deprecated` and
//!   `clippy::too_many_arguments`) reach them and nothing else. The items are re-exported with
//!   the binding's visibility.
//! - **A startup check entry** in `day_platform_android::JAVA_CLASSES`, which `day_platform_android::init`
//!   resolves so a mismatch with the Java source is named at launch.
//! - **An R8 keep rule** for exactly the class and members bound, as plain text in the
//!   `day_keep` link section of the app's shared library. `day build` reads the section back and
//!   hands the rules to R8, so a release build keeps what Rust calls and nothing more.
//!
//! Parsing walks the token trees by hand, like the rest of this crate. The user's own tokens are
//! spliced through unchanged, so jni's errors about a signature still point at the signature.

use proc_macro::{Delimiter, Group, Ident, Literal, Punct, Spacing, Span, TokenStream, TokenTree};

/// The link section the keep rules ride in. `day build` (day-cli `android_keep`) reads it.
const KEEP_SECTION: &str = "day_keep";

/// What the R8 rule keeps: one class, and the members Rust names.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Keep {
    pub java: String,
    pub methods: Vec<String>,
    pub fields: Vec<String>,
    pub constructors: bool,
}

impl Keep {
    /// The ProGuard rule. `includedescriptorclasses` keeps the classes named in a kept member's
    /// signature too: R8 renaming a Day view class would change the descriptor jni looks the
    /// method up by, and the lookup would fail at startup. Members are kept by name with any
    /// arguments, so an overload costs a few bytes and never a missed method.
    pub fn rule(&self) -> String {
        let mut members = Vec::new();
        if self.constructors {
            members.push("<init>(...);".to_string());
        }
        for m in &self.methods {
            members.push(format!("*** {m}(...);"));
        }
        for f in &self.fields {
            members.push(format!("*** {f};"));
        }
        members.sort();
        members.dedup();
        if members.is_empty() {
            return format!("-keep class {}\n", self.java);
        }
        let mut out = format!("-keep,includedescriptorclasses class {} {{\n", self.java);
        for m in members {
            out.push_str("    ");
            out.push_str(&m);
            out.push('\n');
        }
        out.push_str("}\n");
        out
    }
}

/// A Rust identifier's Java spelling: the text itself, less a raw-identifier prefix (`r#type`
/// binds the Java `type`).
pub(crate) fn java_name(ident: &str) -> &str {
    ident.strip_prefix("r#").unwrap_or(ident)
}

pub(crate) fn java_class(input: TokenStream) -> TokenStream {
    match expand(input) {
        Ok(ts) => ts,
        Err((span, message)) => compile_error_at(span, &message),
    }
}

type Error = (Span, String);

fn expand(input: TokenStream) -> Result<TokenStream, Error> {
    let trees: Vec<TokenTree> = input.into_iter().collect();
    let mut i = 0;

    // [vis] Name => java.path , rest…
    let mut vis: Vec<TokenTree> = Vec::new();
    if is_ident(trees.first(), "pub") {
        vis.push(trees[0].clone());
        i = 1;
        if let Some(TokenTree::Group(g)) = trees.get(1)
            && g.delimiter() == Delimiter::Parenthesis
        {
            vis.push(trees[1].clone());
            i = 2;
        }
    }
    let name = match trees.get(i) {
        Some(TokenTree::Ident(id)) => id.clone(),
        other => {
            return Err((
                span_of(other),
                "java_class! starts `[pub] RustName => java.package.Class`".into(),
            ));
        }
    };
    i += 1;
    if !(is_punct(trees.get(i), '=') && is_punct(trees.get(i + 1), '>')) {
        return Err((
            span_of(trees.get(i)),
            "expected `=>` and the Java class after the Rust name".into(),
        ));
    }
    i += 2;
    let path_start = i;
    while i < trees.len() && !is_punct(trees.get(i), ',') {
        i += 1;
    }
    let path_tokens = &trees[path_start..i];
    let java = java_path(path_tokens)?;
    let rest = if i < trees.len() {
        &trees[i + 1..]
    } else {
        &[][..]
    };

    let mut keep = Keep {
        java: java.clone(),
        ..Keep::default()
    };
    let mut body: Vec<TokenTree> = Vec::new();
    let mut since = String::from("0");
    for item in split_commas(rest) {
        if item.is_empty() {
            continue;
        }
        // `since = 26`: Day's own property, not jni's. The class or a member it binds first exists
        // at that API level, so the startup check skips the binding on an older device.
        if is_ident(item.first(), "since") && is_punct(item.get(1), '=') {
            match item.get(2) {
                Some(TokenTree::Literal(lit)) if item.len() == 3 => since = lit.to_string(),
                other => {
                    return Err((
                        span_of(other),
                        "expected `since = <API level>`, such as `since = 26`".into(),
                    ));
                }
            }
            continue;
        }
        let block = match (item.first(), item.get(1), item.len()) {
            (Some(TokenTree::Ident(kind)), Some(TokenTree::Group(g)), 2)
                if g.delimiter() == Delimiter::Brace =>
            {
                Some((kind.to_string(), kind.clone(), g.clone()))
            }
            _ => None,
        };
        match block {
            Some((kind, ident, group))
                if kind == "methods" || kind == "native_methods" || kind == "fields" =>
            {
                let entries = rewrite_block(&kind, &group, &mut keep)?;
                body.push(TokenTree::Ident(ident));
                let mut g = Group::new(Delimiter::Brace, entries);
                g.set_span(group.span());
                body.push(TokenTree::Group(g));
            }
            Some((kind, _, _)) if kind == "constructors" => {
                keep.constructors = true;
                body.extend(item.iter().cloned());
            }
            _ => body.extend(item.iter().cloned()),
        }
        body.push(TokenTree::Punct(Punct::new(',', Spacing::Alone)));
    }

    let name_s = name.to_string();
    let module = Ident::new(&format!("__day_java_class_{name_s}"), Span::call_site());

    // The module and its bind_java_type! call. Everything spelled out here names day_platform_android by
    // its crate name: bind_java_type! reparses its `jni =` path as text, where `$crate` is no path.
    let mut inner: TokenStream = format!(
        "jni = ::day_platform_android::jni,\n\
         pub {name_s} => {java_tokens},\n\
         native_methods_error_policy = ::day_platform_android::jni::errors::LogErrorAndDefault,\n\
         hooks = {{\n\
             load_class = |env, load_context, initialize| {{\n\
                 ::day_platform_android::load_app_class::<{name_s}>(env, load_context, initialize)\n\
             }},\n\
         }},\n",
        java_tokens = path_text(path_tokens),
    )
    .parse()
    .map_err(|e| {
        (
            name.span(),
            format!("java_class!: internal expansion error: {e}"),
        )
    })?;
    inner.extend(body);

    let mut out: TokenStream = format!(
        "#[doc(hidden)]\n\
         #[allow(non_snake_case, non_camel_case_types, unused_imports, deprecated, \
                 clippy::too_many_arguments, clippy::needless_lifetimes)]\n\
         mod {module} {{ use super::*; ::day_platform_android::jni::bind_java_type! {{ }} }}"
    )
    .parse()
    .map_err(|e| {
        (
            name.span(),
            format!("java_class!: internal expansion error: {e}"),
        )
    })?;
    // Fill the empty `bind_java_type! { }` group with the real body. The module text above leaves
    // it empty so the user's tokens (and their spans) go in as tokens, never through a string.
    out = fill_innermost_brace(out, inner);

    let mut reexport: TokenStream = vis.into_iter().collect();
    reexport.extend(
        format!("use self::{module}::*;")
            .parse::<TokenStream>()
            .map_err(|e| {
                (
                    name.span(),
                    format!("java_class!: internal expansion error: {e}"),
                )
            })?,
    );
    out.extend(reexport);

    let rule = keep.rule();
    let registration: TokenStream = format!(
        "const _: () = {{\n\
             #[::day_platform_android::linkme::distributed_slice(::day_platform_android::JAVA_CLASSES)]\n\
             #[linkme(crate = ::day_platform_android::linkme)]\n\
             static CHECK: ::day_platform_android::JavaClassCheck = ::day_platform_android::JavaClassCheck {{\n\
                 name: {java_lit},\n\
                 since: {since},\n\
                 resolve: |env| {{\n\
                     <{name_s}<'static> as ::day_platform_android::jni::refs::Reference>::lookup_class(\n\
                         env,\n\
                         &::day_platform_android::jni::refs::LoaderContext::None,\n\
                     )\n\
                     .map(|_| ())\n\
                 }},\n\
             }};\n\
             #[used]\n\
             #[unsafe(link_section = {section})]\n\
             static KEEP: [u8; {len}] = *{bytes};\n\
         }};",
        java_lit = Literal::string(&java),
        section = Literal::string(KEEP_SECTION),
        len = rule.len(),
        bytes = Literal::byte_string(rule.as_bytes()),
    )
    .parse()
    .map_err(|e| {
        (
            name.span(),
            format!("java_class!: internal expansion error: {e}"),
        )
    })?;
    out.extend(registration);
    Ok(out)
}

/// Rewrite one `methods`, `native_methods` or `fields` block so every member carries its Java
/// name explicitly, recording the names for the keep rule.
fn rewrite_block(kind: &str, group: &Group, keep: &mut Keep) -> Result<TokenStream, Error> {
    let trees: Vec<TokenTree> = group.stream().into_iter().collect();
    let mut out = TokenStream::new();
    for member in split_commas(&trees) {
        if member.is_empty() {
            continue;
        }
        let rewritten = if kind == "fields" {
            rewrite_field(member, keep)?
        } else {
            rewrite_method(member, keep)?
        };
        out.extend(rewritten);
        out.extend([TokenTree::Punct(Punct::new(',', Spacing::Alone))]);
    }
    Ok(out)
}

/// `[quals] fn name(args) [-> ret]` becomes `[quals] fn name { sig = (args) [-> ret], name =
/// "name" }`. A member already in block form gains `name = "…"` unless it sets one.
fn rewrite_method(member: &[TokenTree], keep: &mut Keep) -> Result<TokenStream, Error> {
    let Some(fn_at) = member.iter().position(|t| is_ident(Some(t), "fn")) else {
        return Err((
            span_of(member.first()),
            "expected `fn name(…)` in a methods block".into(),
        ));
    };
    let mut quals: Vec<TokenTree> = Vec::new();
    let mut non_null = false;
    for t in &member[..fn_at] {
        if is_ident(Some(t), "non_null") {
            non_null = true;
        } else {
            quals.push(t.clone());
        }
    }
    let name = match member.get(fn_at + 1) {
        Some(TokenTree::Ident(id)) => id.clone(),
        other => return Err((span_of(other), "expected the method name after `fn`".into())),
    };
    let java = java_name(&name.to_string()).to_string();
    keep.methods.push(java.clone());
    let after = &member[fn_at + 2..];
    let mut props = TokenStream::new();
    match after.first() {
        Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Parenthesis => {
            props.extend(ts("sig = "));
            props.extend(after.iter().cloned());
            props.extend(ts(&format!(", name = {}", Literal::string(&java))));
            if non_null {
                props.extend(ts(", non_null = true"));
            }
        }
        Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Brace && after.len() == 1 => {
            let inner: Vec<TokenTree> = g.stream().into_iter().collect();
            if !sets_name(&inner) {
                props.extend(ts(&format!("name = {}, ", Literal::string(&java))));
            }
            props.extend(inner);
        }
        other => {
            return Err((
                span_of(other),
                "expected `(args) -> ret` or a `{ sig = … }` block after the method name".into(),
            ));
        }
    }
    let mut out: TokenStream = quals.into_iter().collect();
    out.extend([TokenTree::Ident(Ident::new("fn", name.span()))]);
    out.extend([TokenTree::Ident(name)]);
    out.extend([TokenTree::Group(Group::new(Delimiter::Brace, props))]);
    Ok(out)
}

/// `[quals] name: type` becomes `[quals] name { sig = type, name = "name" }`; a block-form field
/// gains `name = "…"` unless it sets one.
fn rewrite_field(member: &[TokenTree], keep: &mut Keep) -> Result<TokenStream, Error> {
    // The name is the last identifier before `:` (shorthand) or before the `{ … }` block.
    let split = member
        .iter()
        .position(|t| is_punct(Some(t), ':') || is_brace(t))
        .ok_or_else(|| {
            (
                span_of(member.first()),
                "expected `name: type` in a fields block".to_string(),
            )
        })?;
    let (head, tail) = member.split_at(split);
    let Some((TokenTree::Ident(name), quals)) = head.split_last() else {
        return Err((span_of(head.last()), "expected the field name".into()));
    };
    let java = java_name(&name.to_string()).to_string();
    keep.fields.push(java.clone());
    let mut kept: Vec<TokenTree> = Vec::new();
    let mut non_null = false;
    for t in quals {
        if is_ident(Some(t), "non_null") {
            non_null = true;
        } else {
            kept.push(t.clone());
        }
    }
    let mut props = TokenStream::new();
    if is_punct(tail.first(), ':') {
        props.extend(ts("sig = "));
        props.extend(tail[1..].iter().cloned());
        props.extend(ts(&format!(", name = {}", Literal::string(&java))));
        if non_null {
            props.extend(ts(", non_null = true"));
        }
    } else if let Some(TokenTree::Group(g)) = tail.first() {
        let inner: Vec<TokenTree> = g.stream().into_iter().collect();
        if !sets_name(&inner) {
            props.extend(ts(&format!("name = {}, ", Literal::string(&java))));
        }
        props.extend(inner);
    }
    let mut out: TokenStream = kept.into_iter().collect();
    out.extend([TokenTree::Ident(name.clone())]);
    out.extend([TokenTree::Group(Group::new(Delimiter::Brace, props))]);
    Ok(out)
}

/// Whether a block's properties already say `name = …`.
fn sets_name(props: &[TokenTree]) -> bool {
    split_commas(props)
        .iter()
        .any(|p| is_ident(p.first(), "name") && is_punct(p.get(1), '='))
}

/// The Java class in its dotted binary form: `a.b.C`, `a.b.Outer::Inner` as `a.b.Outer$Inner`,
/// or a string literal taken as written.
fn java_path(tokens: &[TokenTree]) -> Result<String, Error> {
    if let [TokenTree::Literal(lit)] = tokens {
        let text = lit.to_string();
        return text
            .strip_prefix('"')
            .and_then(|t| t.strip_suffix('"'))
            .map(str::to_string)
            .ok_or_else(|| {
                (
                    lit.span(),
                    "expected the Java class name as a string".into(),
                )
            });
    }
    let mut out = String::new();
    let mut i = 0;
    while i < tokens.len() {
        match &tokens[i] {
            TokenTree::Ident(id) => out.push_str(&id.to_string()),
            TokenTree::Punct(p) if p.as_char() == '.' => out.push('.'),
            TokenTree::Punct(p) if p.as_char() == ':' && is_punct(tokens.get(i + 1), ':') => {
                out.push('$');
                i += 1;
            }
            other => {
                return Err((
                    other.span(),
                    "expected a Java class such as `dev.daybrite.day.bridge.DayBridge`".into(),
                ));
            }
        }
        i += 1;
    }
    if out.is_empty() {
        return Err((
            Span::call_site(),
            "the Java class is missing after `=>`".into(),
        ));
    }
    Ok(out)
}

/// The class path tokens as text, for the generated `Name => path` (no spans to keep: jni only
/// reads the path).
fn path_text(tokens: &[TokenTree]) -> String {
    tokens.iter().map(|t| t.to_string()).collect::<String>()
}

fn split_commas(trees: &[TokenTree]) -> Vec<&[TokenTree]> {
    let mut out = Vec::new();
    let mut start = 0;
    for (i, t) in trees.iter().enumerate() {
        if is_punct(Some(t), ',') {
            out.push(&trees[start..i]);
            start = i + 1;
        }
    }
    if start < trees.len() {
        out.push(&trees[start..]);
    }
    out
}

/// Put `inner` into the last, empty brace group of `outer` (the `bind_java_type! { }` body).
fn fill_innermost_brace(outer: TokenStream, inner: TokenStream) -> TokenStream {
    fn fill(stream: TokenStream, inner: &mut Option<TokenStream>) -> TokenStream {
        let mut out = TokenStream::new();
        for t in stream {
            match t {
                TokenTree::Group(g) if g.delimiter() == Delimiter::Brace => {
                    let mut filled = if g.stream().is_empty() {
                        Group::new(Delimiter::Brace, inner.take().unwrap_or_default())
                    } else {
                        Group::new(Delimiter::Brace, fill(g.stream(), inner))
                    };
                    filled.set_span(g.span());
                    out.extend([TokenTree::Group(filled)]);
                }
                other => out.extend([other]),
            }
        }
        out
    }
    fill(outer, &mut Some(inner))
}

fn ts(text: &str) -> TokenStream {
    // Every caller passes fixed punctuation and literals built by `Literal`, which always parse.
    text.parse().unwrap_or_default()
}

fn is_ident(t: Option<&TokenTree>, text: &str) -> bool {
    matches!(t, Some(TokenTree::Ident(i)) if i.to_string() == text)
}

fn is_punct(t: Option<&TokenTree>, c: char) -> bool {
    matches!(t, Some(TokenTree::Punct(p)) if p.as_char() == c)
}

fn is_brace(t: &TokenTree) -> bool {
    matches!(t, TokenTree::Group(g) if g.delimiter() == Delimiter::Brace)
}

fn span_of(t: Option<&TokenTree>) -> Span {
    t.map(TokenTree::span).unwrap_or_else(Span::call_site)
}

/// `compile_error!("…")` placed at `span`, so the error underlines the offending tokens.
fn compile_error_at(span: Span, message: &str) -> TokenStream {
    let mut lit = Literal::string(message);
    lit.set_span(span);
    let mut body = TokenStream::new();
    body.extend([TokenTree::Literal(lit)]);
    let mut out: TokenStream = "::core::compile_error!".parse().unwrap_or_default();
    let mut g = Group::new(Delimiter::Parenthesis, body);
    g.set_span(span);
    out.extend([TokenTree::Group(g)]);
    out.extend([TokenTree::Punct(Punct::new(';', Spacing::Alone))]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rule keeps the class by name and each bound member, with the classes in their
    /// descriptors, so neither the lookup by name nor the descriptor jni builds can be broken by
    /// R8 renaming.
    #[test]
    fn the_keep_rule_names_the_class_and_every_bound_member() {
        let keep = Keep {
            java: "dev.daybrite.day.piece.combobox.DayCombo".into(),
            methods: vec!["setComboText".into(), "makeCombo".into()],
            fields: vec!["ctx".into()],
            constructors: false,
        };
        assert_eq!(
            keep.rule(),
            "-keep,includedescriptorclasses class dev.daybrite.day.piece.combobox.DayCombo {\n    \
             *** ctx;\n    *** makeCombo(...);\n    *** setComboText(...);\n}\n"
        );
    }

    #[test]
    fn a_class_bound_for_its_type_alone_keeps_its_name() {
        let keep = Keep {
            java: "android.view.View".into(),
            ..Keep::default()
        };
        assert_eq!(keep.rule(), "-keep class android.view.View\n");
    }

    /// Java names pass through untouched; only a raw-identifier prefix, which Java never spells,
    /// is dropped.
    #[test]
    fn java_names_are_verbatim() {
        assert_eq!(java_name("makeCombo"), "makeCombo");
        assert_eq!(java_name("speak_native"), "speak_native");
        assert_eq!(java_name("r#type"), "type");
    }
}
