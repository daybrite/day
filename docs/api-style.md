---
title: "API style"
description: "Argument-clarity rules for the day API: when a builder takes a value, a closure, or a signal, and how names stay predictable."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# API style: argument clarity

Rust has no named arguments, so Day emulates their clarity where it pays and keeps
SwiftUI-like terseness where it doesn't. The rule, in priority order:

1. **No bare `bool` (or otherwise unreadable literal) in a public signature.**
   A call site must not read `d.text(…, true)`. Use a two-variant enum
   (`TextAnchor::Centered`, `Boundary::Yes`) or a builder toggle instead.

2. **Required bundles of 3+ concrete-typed values → a struct parameter** with named
   fields at the call site. This is the closest Rust gets to named arguments:

   ```rust
   d.text("40", center, TextStyle { size: 22.0, color: accent, anchor: TextAnchor::Centered });
   ```

   This is already the house style at the spec boundary (`NavProps { title, split }`,
   `TextFieldPatch::Text { text, from_native }`); apply it to app-facing APIs whenever
   the fields are concrete types.

3. **Generic-ergonomic constructors keep ≤3 positional, type-distinct arguments.**
   `route("controls", tr("nav-controls"), controls_page)` stays positional: the three
   types are mutually incompatible, so every mis-ordering is a compile error, and
   funneling `impl IntoText<M>` through struct fields would force `.into()`/`Box::new`
   noise at every call site (struct literals don't do implicit conversion). Names would
   cost more ergonomics than they buy.

4. **Optional configuration → builder methods** (`.spacing(8.0)`, `.align(…)`,
   `.padding(16.0)`). Never grow a constructor's positional list for options.

5. **Conventional-order exemptions.** Universally-fixed orders stay positional even
   with same-typed arguments: `Color::rgba(r, g, b, a)`, `Size::new(w, h)`,
   `.frame(w, h)`, rect `(x, y, w, h)`.

Scope: the rule binds the **app-facing surface** (day-pieces, Day umbrella, day-core's
`BuildCx`/nav API). The engine's internal interfaces (the `Toolkit` trait, `TreeOps`, FFI
shims) prefer the same, but a documented `bool` parameter is acceptable where changing it
would ripple through every backend for internal call sites only.

## Typed builders and erasure

A builder method must not throw away the piece's type. Two rules follow from that:

1. **Generic modifiers return `Decorated<Self>`, never `AnyPiece`.** Every `Decorate` method,
   and every extension trait a toolkit or `day-tweak-*` crate adds (`.gtk(…)`, `.tooltip(…)`,
   `.tickmarks(…)`), returns `Decorated<Self>`. `Decorated` carries an ordered op list beside
   the piece it wraps, and its inherent methods shadow the trait's, so chains stay flat rather
   than nesting `Decorated<Decorated<…>>`. The one exception is `Decorate::modifier`, because
   `Modifier` is defined over `AnyPiece` and cannot preserve a type it never sees.

2. **A piece's own builders go in a `*Builder` trait, forwarded through `Decorated`.**
   `Label`'s inherent methods are the implementation; `LabelBuilder` re-declares them and
   `impl<P: LabelBuilder + Piece> LabelBuilder for Decorated<P>` forwards each through
   `Decorated::map_inner`. That forwarding makes `label(…).padding(8.0).font(…)` resolve, so a
   piece never imposes a "typed modifiers first" ordering rule on its callers. Name the trait
   after the piece (`LabelBuilder`, `ButtonBuilder`, `ColumnBuilder`, `RowBuilder`); `*Style`
   names belong to the value enums (`PickerStyle`, `SelectorStyle`).

Erasure stays explicit and one-way: `.any()` at a boundary that needs a single `AnyPiece` (a
`PieceVec`, an `-> AnyPiece` signature, a stored piece). It is free on a piece
that is already erased (`AnyPiece::any` is inherent and returns `self`). A build-time branch
between two piece types uses `Either<A, B>` rather than erasing both arms; a branch on a
signal uses `when(…).otherwise(…)`.

**Deferring to build time is not such a boundary.** A constructor whose body must wait for the
build (it reads an ambient `environment`, a scope, or the laid-out size) defers through
`piece_fn`, which returns the concrete `PieceFn<F>`, so `canvas`, `frame_clock`, `shape_group`,
`shape_group_fn`, `each` and `with_environment` return `impl Piece` and cost no box. Where the
deferred piece is worth a name, it defers inside its own `build` instead and stays a struct:
`labeled` reads the enclosing form's shared label column at build time and still returns
`Labeled<P>`. **No constructor in day-pieces returns `AnyPiece`.**

`form` and `labeled` were the last two that did, until 2026-08-24. Neither half of the reasoning
justified the exception. The first was that form rows are collected more
often than consumed inline — across day and eleven app repos, 10 of ~170 `labeled` call sites
needed a uniform type, because rows go into `section((…))` tuples and `PieceSeq` accepts
heterogeneous tuples. The second was that erasing bounds monomorphization, which is true but
cheap to give up: un-erasing both cost +90 KB of machine code on Day-Showcase's macos-appkit
release binary (+0.81% of `__text`, +0.40% stripped) and +10% on that crate's compile time.

There is no `From<P> for AnyPiece`, and there cannot be a blanket one: `AnyPiece` implements
`Piece`, so a blanket impl collides with core's reflexive `impl<T> From<T> for T`. `.any()`
is the single spelling.
