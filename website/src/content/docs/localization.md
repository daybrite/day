---
title: Localization
description: "Fluent-based localization: translation files, arguments and plurals, live locale switching, and per-locale testing."
order: 21
section: Guides
---

Day localizes with [Mozilla Fluent](https://projectfluent.org/) — a message format built for the
grammar problems that `printf`-style formats handle badly: plurals, gender, and languages that
reorder everything. Localization is one of Day's four pillars, which in practice means it isn't
optional plumbing: the locale is a reactive signal, every built-in string mechanism goes through
it, and the test tooling understands it.

## Files and setup

Translations live in `resource/locales/<lang>/app.ftl`, one file per language:

```ftl
# resource/locales/en/app.ftl
app-title = Field Notes
greeting = Hello, { $name }!
unread-count = { $count ->
    [one] You have one unread note
   *[other] You have { $count } unread notes
}
```

Register the catalogs once, at the top of your root function:

```rust
day_fluent::install(
    "en",
    &[
        ("en", include_str!("../resource/locales/en/app.ftl")),
        ("fr", include_str!("../resource/locales/fr/app.ftl")),
    ],
);
```

## Using strings

`tr(key)` produces a localized, *reactive* text value that any text-accepting Piece takes
directly:

```rust
label(tr("app-title")).font(Font::Title)

label(tr("greeting").arg("name", user_name))       // arg can be a value, Signal, or closure
label(tr("unread-count").arg("count", unread))     // Fluent picks the plural form
button(tr("save")).action(save)
```

Because arguments accept signals, a localized message with a live value is one expression — when
`unread` changes, the label re-renders through the same fine-grained binding any other text uses,
and Fluent re-selects the plural category. For a plain `String` in non-UI code, `t("key")`
returns the formatted value once, not reactively.

Missing messages fall back per-message to the default locale, so a half-translated catalog ships
degraded rather than broken. Day's own strings (dialog buttons, menu roles) come from a built-in
core catalog that your app catalog can override key by key.

## Switching locale at runtime

The current locale is a `Signal<String>`, initialized from a CLI override, then the OS
preference, then your default:

```rust
set_locale("fr");          // every tr() binding re-runs; layout reflows for new text sizes
let l = locale().get();    // read (reactively, if inside a binding)
```

A locale switch is an ordinary reactive update — no restart, no tree rebuild. Longer German
strings or shorter Chinese ones change measured text sizes, and
[incremental relayout](/docs/layout#incremental-relayout) handles the reflow.

## Testing what you translated

Two tools make per-locale verification cheap:

```bash
day launch -p macos-appkit --locale fr --script dayscript/walkthrough.yaml
day launch -p macos-appkit --locale en-XA
```

The first runs your [dayscript](/docs/dayscript) walkthrough under French — the CI configuration
does exactly this, so the [gallery](/gallery) screenshots double as translation review. Scripts
can assert by Fluent key rather than literal text, so one script passes in every locale.

`en-XA` is the built-in pseudolocale: it accents every message's text
(`Ĥéĺĺó, Ádá!`) without touching placeables, which makes unlocalized hardcoded strings jump out
visually. `day lint` complements it statically, flagging bare user-facing literals and unused or
missing keys.

## Honest edges

- **What's covered:** your UI strings, and OS-facing metadata (the app's display name and
  similar) conveyed into platform manifests at build time.
- **What isn't:** text inside out-of-process UI (native file dialogs, permission prompts) follows
  the *system* locale, not your in-app override — every framework shares this limit, but you'll
  notice it when testing with `--locale`.
- **RTL:** layout mirroring is designed into the layout engine (leading/trailing resolve at
  placement), but there's no `ar-XB` RTL pseudolocale yet and RTL hasn't had a dedicated CI leg —
  treat right-to-left support as present-but-lightly-exercised, and test with a real RTL locale
  if you ship one.
- **Number/date formatting** follows the locale through Fluent; for formatting outside messages
  you're in ordinary Rust and choose your own crates.

The [localization reference](/docs/internal/localization) covers the mechanics in more depth.
