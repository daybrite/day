---
title: "API style"
description: "Argument-clarity rules for the day API: when a builder takes a value, a closure, or a binding, how coupled configuration and items are spelled, and how names stay predictable."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# API style: argument clarity

Rust has no named arguments, so Day emulates their clarity where it pays and keeps
SwiftUI-like terseness where it doesn't. The rules, in priority order:

1. **No bare `bool` (or otherwise unreadable literal) as a configuration flag in a public
   signature.** A call site must not read [`d.text(…, true)`](https://docs.rs/day-pieces/latest/day_pieces/struct.Draw.html#method.text) or [`.reorderable(true)`](https://docs.rs/day-pieces/latest/day_pieces/trait.ListBuilder.html#tymethod.reorderable). Use a
   two-variant enum (`TextAlign::Center`, `Boundary::Yes`), a presence builder ([`.multi_select()`](https://docs.rs/day-pieces/latest/day_pieces/trait.ListBuilder.html#tymethod.multi_select)
   / [`.no_separators()`](https://docs.rs/day-pieces/latest/day_pieces/trait.ListBuilder.html#tymethod.no_separators)), or a feature value (rule 6). A positional control-flow condition is
   different: in [`when(cond, build)`](https://docs.rs/day-pieces/latest/day_pieces/fn.when.html) and [`apply_if(cond, f)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.apply_if) the condition is the call's subject,
   not a flag on something else, and it stays positional.

2. **Required bundles of 3+ concrete-typed values → a struct parameter** with named
   fields at the call site. This is the closest Rust gets to named arguments:

   ```rust
   d.text(&res::str::canvas_value(40_i64).format(), center, TextStyle { size: 22.0, color: accent, anchor: TextAnchor::CENTERED, ..Default::default() });
   ```

   This is already the house style at the spec boundary (`NavProps { title, split }`,
   `TextFieldPatch::Text { text, from_native }`); apply it to app-facing APIs whenever
   the fields are concrete types. The generated `res::str` functions follow it for messages
   with two or more variables (`res::str::items_moved(res::str::ItemsMoved { count, folder })`).

3. **Generic-ergonomic constructors keep ≤3 positional, type-distinct arguments.**
   [`route("controls", res::str::nav_controls(), controls_page)`](https://docs.rs/day-pieces/latest/day_pieces/fn.route.html) and
   [`Command::new(id!(add), res::str::add(), add_item)`](https://docs.rs/day-pieces/latest/day_pieces/struct.Command.html#method.new) stay positional: the three types are mutually
   incompatible, so every mis-ordering is a compile error, and funneling `impl IntoText<M>`
   through struct fields would force `.into()`/`Box::new` noise at every call site (struct
   literals don't do implicit conversion). Names would cost more ergonomics than they buy.

4. **Optional configuration → builder methods** ([`.spacing(Space::S)`](https://docs.rs/day-pieces/latest/day_pieces/trait.ColumnBuilder.html#tymethod.spacing), [`.align(…)`](https://docs.rs/day-pieces/latest/day_pieces/trait.ColumnBuilder.html#tymethod.align),
   [`.padding(Space::L)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.padding)). Never grow a constructor's positional list for options.

5. **Conventional-order exemptions.** Universally-fixed orders stay positional even
   with same-typed arguments: [`Color::rgba(r, g, b, a)`](https://docs.rs/day-geometry/latest/day_geometry/struct.Color.html#method.rgba), [`Size::new(w, h)`](https://docs.rs/day-geometry/latest/day_geometry/struct.Size.html#method.new),
   [`.frame(w, h)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.frame), rect `(x, y, w, h)`.

6. **Coupled configuration is one value with its own builder.** A feature whose settings only
   make sense together (search: the query, the prompt, the placement, the reveal policy; reorder:
   the handler, the guard, the availability) is one value — [`Search::new`](https://docs.rs/day-pieces/latest/day_pieces/struct.Search.html#method.new)`(query)`[`.prompt`](https://docs.rs/day-pieces/latest/day_pieces/struct.Search.html#method.prompt)`(..)`,
   [`Reorder::new`](https://docs.rs/day-pieces/latest/day_pieces/struct.Reorder.html#method.new)`(f)`[`.guard`](https://docs.rs/day-pieces/latest/day_pieces/struct.Reorder.html#method.guard)`(g)`, [`ContentList::new`](https://docs.rs/day-pieces/latest/day_pieces/struct.ContentList.html#method.new)`(build)`[`.width`](https://docs.rs/day-pieces/latest/day_pieces/struct.ContentList.html#method.width)`(w)`, [`Sections::restored(key)`](https://docs.rs/day-pieces/latest/day_pieces/struct.Sections.html#method.restored) —
   handed to one method ([`.searchable(search)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.searchable), [`.reorderable(reorder)`](https://docs.rs/day-pieces/latest/day_pieces/trait.ListBuilder.html#tymethod.reorderable)). Never a prefixed-method
   family (`search_prompt`, `search_placement`, …), and never a method whose effect depends on a
   prior call ("call after [`searchable`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.searchable)", "applies to the item just added"). The piece is in the
   feature state exactly when it carries the value, so the feature cannot be half configured.

7. **Three spellings for a reactive input.** Two-way state is `impl Binding<T>` (a `Signal<T>`,
   a [day-model](/docs/explanation/model) `Field`, a `Mapped` view; stored as `AnyBinding<T>`). A read is
   `impl IntoReactive<T, M>` (a constant, a signal, or a closure, all accepted by one method:
   [`.enabled(false)`](https://docs.rs/day-pieces/latest/day_pieces/trait.ButtonBuilder.html#tymethod.enabled), [`.enabled(busy)`](https://docs.rs/day-pieces/latest/day_pieces/trait.ButtonBuilder.html#tymethod.enabled), [`.enabled`](https://docs.rs/day-pieces/latest/day_pieces/trait.ButtonBuilder.html#tymethod.enabled)`(move || !busy`[`.get`](https://docs.rs/day-reactive/latest/day_reactive/struct.Signal.html#method.get)`())`). An event is `impl Fn`.
   Text follows rule 8. Static configuration is a plain value, and a collection is an
   `IntoIterator` or a closure that derives it. A bare `Signal<T>` parameter, or a `foo` /
   `foo_when` / `foo_reactive` pair, is a violation: one method, one marker-generic parameter.

8. **Anything a person reads is `impl IntoText<M>`**: a label, a title, a prompt, a tooltip, a
   menu entry, a delete affordance, an accessibility string. The caller passes a literal, a
   `LocalizedText` (`res::str::x()`, no [`.format()`](https://docs.rs/day-localization-ui/latest/day_localization_ui/struct.LocalizedText.html#method.format)), a `Signal<String>` or a closure, and the
   text is live. Filenames, ids and keys are the only `Into<String>` text.

9. **Modifiers are classified**, and a piece's docs name its semantic target. Identity modifiers
   ([`id`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.id), [`id_of`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.id_of)) name the piece's own node wherever they sit in the chain;
   outer-layout facts (`grid_span`, [`grid_align`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.grid_align)) land on the outermost node whatever follows;
   wrapping modifiers are ordered, as they read. See "Modifier classes" below. No rule may ask
   the caller to place [`.id`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.id) before [`.padding`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.padding).

10. **Items are values.** A nav item, a toolbar entry, a menu entry, a form row, a picker option
    is a value — [`item`](https://docs.rs/day-pieces/latest/day_pieces/fn.item.html)`(key, title)`[`.icon`](https://docs.rs/day-pieces/latest/day_pieces/struct.NavItem.html#method.icon)`(..)`[`.page`](https://docs.rs/day-pieces/latest/day_pieces/struct.NavItem.html#method.page)`(..)`, [`toolbar_button(id, label)`](https://docs.rs/day-pieces/latest/day_pieces/fn.toolbar_button.html),
    [`menu_item(title)`](https://docs.rs/day-pieces/latest/day_pieces/fn.menu_item.html), `(value, title)` — taken as a value, a tuple of values, an `Option`, a
    `Vec`, or a derived list, never as positional arguments to the container plus trailing
    "decorates the last added" calls. Decorations go on the item.

11. **Builder traits are generated** with `#[day::builder(FooBuilder)]` on the piece's inherent
    `impl` block. Hand-written `*Builder` traits and `map_inner` forwards are not accepted.

Scope: the rules bind the **app-facing surface** (day-pieces, the Day umbrella, day-core's
`BuildCx`/nav API, the piece crates). The engine's internal interfaces (the `Toolkit` trait,
`TreeOps`, FFI shims) prefer the same, but a documented `bool` parameter is acceptable where
changing it would ripple through every backend for internal call sites only.

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](/docs/reference/localization#reference-example-messages) and [catalog setup](/docs/how-to/localization).

## Typed builders and erasure

A builder method must not throw away the piece's type. Two rules follow from that:

1. **Generic modifiers return `Decorated<Self>`, never `AnyPiece`.** Every `Decorate` method,
   and every extension trait a toolkit or `day-tweak-*` crate adds (`.gtk(…)`, `.tooltip(…)`,
   `.tickmarks(…)`), returns `Decorated<Self>`. `Decorated` carries an ordered op list beside
   the piece it wraps, and its inherent methods shadow the trait's, so chains stay flat rather
   than nesting `Decorated<Decorated<…>>`. The one exception is [`Decorate::modifier`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.modifier), because
   `Modifier` is defined over `AnyPiece` and cannot preserve a type it never sees.

2. **A piece's own builders are reachable after a modifier through a generated `*Builder`
   trait.** `Label`'s inherent methods are the implementation; `#[day::builder(LabelBuilder)]`
   on that `impl` block emits the trait, its impl for `Label`, and the forwarding impl for
   `Decorated<Label>`, which is what makes [`label`](https://docs.rs/day-pieces/latest/day_pieces/fn.label.html)`(…)`[`.padding`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.padding)`(8.0)`[`.font`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.font)`(…)` resolve, so a piece
   never imposes a "typed modifiers first" ordering rule on its callers.

   ```rust
   #[day::builder(GaugeBuilder)]   // `Decorated` and `Piece` in scope (use day::prelude::*)
   impl Gauge {
       pub fn ticks(mut self, n: usize) -> Self { self.ticks = n; self }
   }
   ```

   Every `pub fn` that takes `self` by value and returns `Self` becomes a trait method; `&self`
   accessors, non-`pub` helpers and methods returning anything else are left out. A generic
   piece names the parameters the trait carries (`#[day::builder(NavBuilder<K>)]`), and their
   bounds are copied from the impl. Name the trait after the piece (`LabelBuilder`,
   `ButtonBuilder`, `ColumnBuilder`, `RowBuilder`); `*Style` names belong to the value enums
   (`PickerStyle`, `NavStyle`). Skip any method whose name `Decorate` already defines: the
   inherent method on `Decorated` wins resolution, so a forwarded twin would never be called.

Erasure stays explicit and one-way: [`.any()`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.any) at a boundary that needs a single `AnyPiece` (a
`Vec<AnyPiece>` of differently typed children, an `-> AnyPiece` signature, a stored piece). It is
free on a piece that is already erased ([`AnyPiece::any`](https://docs.rs/day-core/latest/day_core/struct.AnyPiece.html#method.any) is inherent and returns `self`). A
runtime-sized list of one piece type needs no erasure: `Vec<P>`, `[P; N]` and `Option<P>` are
child sequences ([`row(stars)`](https://docs.rs/day-pieces/latest/day_pieces/fn.row.html), [`row((icon, cond.then(|| badge)))`](https://docs.rs/day-pieces/latest/day_pieces/fn.row.html); `None` builds nothing), and
`Option<P>` is a `Piece` in its own right. A build-time branch between two piece types uses
`Either<A, B>` rather than erasing both arms; a branch on a signal uses [`when`](https://docs.rs/day-pieces/latest/day_pieces/fn.when.html)`(…)`[`.otherwise`](https://docs.rs/day-pieces/latest/day_pieces/trait.WhenBuilder.html#tymethod.otherwise)`(…)`,
[`keyed`](https://docs.rs/day-pieces/latest/day_pieces/fn.keyed.html), or [`switch`](https://docs.rs/day-pieces/latest/day_pieces/fn.switch.html).

**Deferring to build time is not such a boundary.** A constructor whose body must wait for the
build (it reads an ambient [`environment`](https://docs.rs/day-pieces/latest/day_pieces/fn.environment.html), a scope, or the laid-out size) defers through
[`piece_fn`](https://docs.rs/day-core/latest/day_core/fn.piece_fn.html), which returns the concrete `PieceFn<F>`, so [`canvas`](https://docs.rs/day-pieces/latest/day_pieces/fn.canvas.html), [`frame_clock`](https://docs.rs/day-pieces/latest/day_pieces/fn.frame_clock.html), [`shape_group`](https://docs.rs/day-pieces/latest/day_pieces/fn.shape_group.html),
[`shape_group_fn`](https://docs.rs/day-pieces/latest/day_pieces/fn.shape_group_fn.html), [`each`](https://docs.rs/day-pieces/latest/day_pieces/fn.each.html) and [`with_environment`](https://docs.rs/day-pieces/latest/day_pieces/fn.with_environment.html) return `impl Piece` and cost no box. Where the
deferred piece is worth a name, it defers inside its own [`build`](https://docs.rs/day-core/latest/day_core/trait.Piece.html#tymethod.build) instead and stays a struct:
[`labeled`](https://docs.rs/day-pieces/latest/day_pieces/fn.labeled.html) reads the enclosing form's shared label column at build time and still returns
`Labeled<P>`. **No constructor in day-pieces returns `AnyPiece`.**

[`form`](https://docs.rs/day-pieces/latest/day_pieces/fn.form.html) and [`labeled`](https://docs.rs/day-pieces/latest/day_pieces/fn.labeled.html) were the last two that did, until 2026-08-24. Neither half of the reasoning
justified the exception. The first was that form rows are collected more
often than consumed inline — across day and eleven app repos, 10 of ~170 [`labeled`](https://docs.rs/day-pieces/latest/day_pieces/fn.labeled.html) call sites
needed a uniform type, because rows go into [`section((…))`](https://docs.rs/day-pieces/latest/day_pieces/fn.section.html) tuples and `PieceSeq` accepts
heterogeneous tuples. The second was that erasing bounds monomorphization, which is true but
cheap to give up: un-erasing both cost +90 KB of machine code on Day-Showcase's macos-appkit
release binary (+0.81% of `__text`, +0.40% stripped) and +10% on that crate's compile time. The
`Option`/`Vec`/array child impls and the generated builder traits are to be measured the same
way (a stripped release build of Day-Showcase, `__text` and total); no figure is recorded yet.

There is no `From<P> for AnyPiece`, and there cannot be a blanket one: `AnyPiece` implements
`Piece`, so a blanket impl collides with core's reflexive `impl<T> From<T> for T`. [`.any()`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.any)
is the single spelling.

## Modifier classes

A `Decorated<P>` chain is a piece plus its modifiers, and only one class of modifier is ordered:

- **Identity** ([`id`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.id), [`id_of`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.id_of)) names the piece's own node, the one its [`build`](https://docs.rs/day-core/latest/day_core/trait.Piece.html#tymethod.build)
  returns, wherever it sits in the chain: [`label`](https://docs.rs/day-pieces/latest/day_pieces/fn.label.html)`(t)`[`.padding`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.padding)`(8.0)`[`.id`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.id)`(id!(x))` and
  [`label`](https://docs.rs/day-pieces/latest/day_pieces/fn.label.html)`(t)`[`.id`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.id)`(id!(x))`[`.padding`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.padding)`(8.0)` are the same tree. A dayscript step addressed to that id
  reaches the handler a wrapping modifier above it registered ([`.padding`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.padding)`(..)`[`.on_tap`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.on_tap)`(..)`),
  through every wrapper the same chain built, a native container such as [`.background(..)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.background)
  included, as a native gesture on an inner view reaches the recognizer on its superview. The
  id never has to move to make a tap land, and the step stops at the chain's outermost node, so
  a tappable card around a button leaves the button's tap alone.
- **Wrapping** modifiers ([`padding`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.padding), [`frame`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.frame), [`background`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.background), [`grow`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.grow), [`a11y`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.a11y), [`on_tap`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.on_tap), …) each
  apply to what came before, so they compose outward in call order, as they read. This is the
  class an ordering rule may still apply to: [`.a11y`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.a11y) annotates the node built so far, and a
  tweak goes after a modifier that rebuilds the backing widget ([docs/tweaks](tweaks.md)).
- **Outer-layout facts** (`grid_span`, [`grid_align`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.grid_align)) describe the piece to the container it
  sits in, so they land on the outermost node whatever wrappers follow them
  ([docs/grid](grid.md)).

[`wrapper_id`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.wrapper_id) is the one way to name a wrapper deliberately: it tags whatever node is outermost
at the point it is called, for a frame assertion on a padded box. A piece whose own node is not
the obvious target (a stepper's field, a list's scroller) says so in its docs; [`.id`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.id) goes there.

## Tokens

Two value vocabularies keep configuration semantic where a literal would be a magic number.

- **`Space`** is the spacing scale: `XS` (4), `S` (8), `M` (12), `L` (16), `XL` (24) points,
  accepted wherever a container or modifier takes a gap or an inset ([`.spacing(Space::S)`](https://docs.rs/day-pieces/latest/day_pieces/trait.ColumnBuilder.html#tymethod.spacing),
  [`.padding(Space::L)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.padding), [`Insets::symmetric(Space::L.into(), Space::XS.into())`](https://docs.rs/day-geometry/latest/day_geometry/struct.Insets.html#method.symmetric)). Fonts are
  semantic (`Font::Headline`) and the house rule forbids point sizes; spacing is the same kind of
  thing, with room for the platforms to resolve a step differently. A raw `f64` stays accepted,
  and `Insets::{`[`all`](https://docs.rs/day-geometry/latest/day_geometry/struct.Insets.html#method.all), [`symmetric`](https://docs.rs/day-geometry/latest/day_geometry/struct.Insets.html#method.symmetric), [`horizontal`](https://docs.rs/day-geometry/latest/day_geometry/struct.Insets.html#method.horizontal), [`vertical`](https://docs.rs/day-geometry/latest/day_geometry/struct.Insets.html#method.vertical), [`top`](https://docs.rs/day-geometry/latest/day_geometry/struct.Insets.html#method.top), [`bottom`](https://docs.rs/day-geometry/latest/day_geometry/struct.Insets.html#method.bottom), [`leading`](https://docs.rs/day-geometry/latest/day_geometry/struct.Insets.html#method.leading), [`trailing`](https://docs.rs/day-geometry/latest/day_geometry/struct.Insets.html#method.trailing)`}` build an
  inset without a struct literal ([`horizontal_total()`](https://docs.rs/day-geometry/latest/day_geometry/struct.Insets.html#method.horizontal_total) / [`vertical_total()`](https://docs.rs/day-geometry/latest/day_geometry/struct.Insets.html#method.vertical_total) sum a pair).
- **`TextPreset`** names a text style once: `const CAPTION: TextPreset =
`[`TextPreset::new`](https://docs.rs/day-pieces/latest/day_pieces/struct.TextPreset.html#method.new)`(Font::Caption)`[`.secondary`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.secondary)`();` then [`label`](https://docs.rs/day-pieces/latest/day_pieces/fn.label.html)`(t)`[`.preset`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.preset)`(&CAPTION)`. Every
  `TextPreset` builder is `const`, so a preset is a constant, and [`.preset`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.preset) applies the same
  `LabelBuilder` steps the equivalent chain would ([docs/text](text.md)). [`.card()`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.card) is the
  same move for a surface: the platform's grouped-content surface with rounded corners and a
  `Space::M` inset, in place of a [`padding`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.padding)/[`background`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.background)/[`corner_radius`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.corner_radius) idiom.
