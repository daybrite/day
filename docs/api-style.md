# API style: argument clarity

Rust has no named arguments, so day emulates their clarity where it pays and keeps
SwiftUI-like terseness where it doesn't. The rule, in priority order:

1. **No bare `bool` (or otherwise unreadable literal) in a public signature.**
   A call site must not read `d.text(…, true)`. Use a two-variant enum
   (`TextAnchor::Centered`, `Boundary::Yes`) or a builder toggle instead.

2. **Required bundles of 3+ concrete-typed values → a struct parameter** with named
   fields at the call site — the closest Rust gets to named arguments:

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
   `.padding(16.0)`) — never grow a constructor's positional list for options.

5. **Conventional-order exemptions.** Universally-fixed orders stay positional even
   with same-typed arguments: `Color::rgba(r, g, b, a)`, `Size::new(w, h)`,
   `.frame(w, h)`, rect `(x, y, w, h)`.

Scope: the rule binds the **app-facing surface** (day-pieces, day umbrella, day-core's
`BuildCx`/nav API). Engine seams (the `Toolkit` trait, `TreeOps`, FFI shims) prefer the
same, but a documented `bool` parameter is acceptable where changing it would ripple
through every backend for internal call sites only.
