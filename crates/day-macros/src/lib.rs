// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Compile-time macros for Day.
//!
//! [`build_path!`]: SVG path data in, a `PathBuilder` method chain out. [`Observable`]
//! (`#[derive]`): a struct in, typed per-field accessors for day-model out.

mod obs;
mod svg;

use proc_macro::TokenStream;

/// Turn SVG path data into a `PathBuilder` chain, at compile time.
///
/// ```ignore
/// let heart = build_path!("M 12,21 C 2,14 2,7 7,5 c 3,-1 5,1 5,3 …").build();
/// d.fill(heart, CORAL);
/// ```
///
/// The whole SVG 1.1 path grammar is accepted: absolute and relative commands, `H`/`V`, the
/// smooth forms `S`/`T`, elliptical arcs, implicit command repetition, and SVG's number syntax
/// (`1e2`, `.5.5`, `10-5`). Malformed data is a COMPILE error naming the offending character,
/// not a path that silently draws nothing.
///
/// The result is a `PathBuilder`, so the fill rule and any further segments still chain on:
///
/// ```ignore
/// build_path!("M0,0 h10 v10 h-10 Z").rule(FillRule::EvenOdd).build()
/// ```
///
/// Arcs are converted to cubics here, once, rather than by each of the nine backends at draw
/// time — an arc is the one command with no counterpart in the 2-D APIs Day draws through.
///
/// Everything is evaluated at compile time, so a path costs the same at runtime as writing the
/// method chain by hand: there is no string left in the binary and no parsing on the draw path.
#[proc_macro]
pub fn build_path(input: TokenStream) -> TokenStream {
    let literal = match string_literal(input) {
        Ok(s) => s,
        Err(e) => return compile_error(&e),
    };
    let segs = match svg::parse(&literal) {
        Ok(s) => s,
        Err(e) => {
            return compile_error(&format!(
                "build_path!: {} at offset {} in {literal:?}",
                e.message, e.at
            ));
        }
    };
    let mut code = String::from("day::prelude::PathBuilder::new()");
    for seg in &segs {
        match seg {
            svg::Seg::Move(x, y) => {
                code.push_str(&format!(".move_to({})", point(*x, *y)));
            }
            svg::Seg::Line(x, y) => {
                code.push_str(&format!(".line_to({})", point(*x, *y)));
            }
            svg::Seg::Quad(cx, cy, x, y) => {
                code.push_str(&format!(".quad_to({}, {})", point(*cx, *cy), point(*x, *y)));
            }
            svg::Seg::Cubic(ax, ay, bx, by, x, y) => {
                code.push_str(&format!(
                    ".cubic_to({}, {}, {})",
                    point(*ax, *ay),
                    point(*bx, *by),
                    point(*x, *y)
                ));
            }
            svg::Seg::Close => code.push_str(".close()"),
        }
    }
    match code.parse() {
        Ok(ts) => ts,
        Err(e) => compile_error(&format!("build_path!: generated code did not parse: {e}")),
    }
}

/// A `Point::new(x, y)` literal. Coordinates are emitted with full `f64` precision so the
/// generated chain is exactly the path that was parsed.
fn point(x: f64, y: f64) -> String {
    format!("day::prelude::Point::new({x:?}, {y:?})")
}

/// Pull the single string literal out of the macro input, rejecting anything else.
///
/// Done by hand rather than with `syn`: the accepted input is one literal, and a parser
/// framework would be the crate's only dependency.
fn string_literal(input: TokenStream) -> Result<String, String> {
    let mut trees = input.into_iter();
    let first = trees.next().ok_or_else(|| {
        "build_path! takes an SVG path string, e.g. build_path!(\"M0,0 L10,10\")".to_string()
    })?;
    if trees.next().is_some() {
        return Err("build_path! takes exactly one string literal".to_string());
    }
    let text = match first {
        proc_macro::TokenTree::Literal(l) => l.to_string(),
        other => return Err(format!("expected a string literal, found `{other}`")),
    };
    // `Literal::to_string` gives the SOURCE spelling, quotes and escapes included, so a plain
    // literal has to be unescaped by hand. A raw string does not (that is what raw means).
    let trimmed = text.trim();
    if let Some(rest) = trimmed.strip_prefix('r') {
        let hashes = rest.len() - rest.trim_start_matches('#').len();
        let inner = rest
            .trim_start_matches('#')
            .strip_prefix('"')
            .ok_or_else(|| "expected a string literal".to_string())?;
        let end = inner
            .len()
            .checked_sub(hashes + 1)
            .ok_or_else(|| "expected a string literal".to_string())?;
        return Ok(inner[..end].to_string());
    }
    let body = trimmed
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .ok_or_else(|| format!("expected a string literal, found `{trimmed}`"))?;
    unescape(body)
}

/// Expand Rust's string escapes.
///
/// The LINE CONTINUATION is the one that matters here: `\` at end of line eats the newline and
/// the next line's leading whitespace, which is how a long path stays readable across several
/// source lines. Without this the backslash reaches the path parser and fails the build on
/// exactly the paths most worth writing that way.
fn unescape(s: &str) -> Result<String, String> {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('\n') => {
                // Line continuation: skip the newline and any indentation after it.
                while it.peek().is_some_and(|c| c.is_whitespace()) {
                    it.next();
                }
            }
            Some('\r') => {
                if it.peek() == Some(&'\n') {
                    it.next();
                }
                while it.peek().is_some_and(|c| c.is_whitespace()) {
                    it.next();
                }
            }
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('0') => out.push('\0'),
            Some('\\') => out.push('\\'),
            Some('"') => out.push('"'),
            Some('\'') => out.push('\''),
            Some(other) => {
                return Err(format!("unsupported escape `\\{other}` in the path string"));
            }
            None => return Err("path string ends in a lone backslash".to_string()),
        }
    }
    Ok(out)
}

/// A `compile_error!` invocation, so a bad path fails the build where it is written.
fn compile_error(message: &str) -> TokenStream {
    format!("compile_error!({message:?});")
        .parse()
        .unwrap_or_else(|_| {
            // Unreachable in practice: the message is a formatted Rust string literal.
            TokenStream::new()
        })
}

/// Per-property observation for a struct — see day-model's crate docs and `docs/model.md`.
///
/// Generates typed field accessors on every `Source` of the struct (`store.name()`,
/// `store.elem(id).name()`, nested `item.address().city()`), `Identified` from the field marked
/// `#[obs(key)]` (always explicit), and `OBSERVED_FIELDS`. `#[obs(skip)]` leaves a field out:
/// no accessor, no path, no trigger.
#[proc_macro_derive(Observable, attributes(obs, model))]
pub fn observable(input: TokenStream) -> TokenStream {
    obs::observable(input)
}

/// A persistable model — everything [`macro@Observable`] generates, plus `impl
/// day_persistence::Model`: table name, column list, row↔struct mappers and the default row.
/// See day-persistence's crate docs and `docs/persistence.md`.
///
/// `#[model(id)]` marks the key (it is `#[obs(key)]` too). Field options: `column = "…"`,
/// `unique`, `index`, `transient` (observable, never stored), `with = Codec`, `json`. Struct
/// options: `table = "…"` (snake_cased struct name otherwise), `index("a", "b")` for
/// composites. `#[obs(skip)]` removes a field from both halves.
#[proc_macro_derive(Model, attributes(model, obs))]
pub fn model(input: TokenStream) -> TokenStream {
    obs::model(input)
}
