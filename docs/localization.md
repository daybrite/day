---
title: "Localization"
description: "Catalog generation, private namespaces, locale selection, fallback, formatting, and translation checks."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Localization (§12)

Day embeds Fluent catalogs and generates Rust accessors for their messages. The runtime
resolves each message using a shared locale signal and the catalog attached to the accessor.
The [localization guide](https://daybrite.dev/docs/localization) covers setup and translation
workflows. This reference defines the generation, lookup, and tooling contracts.

## Catalog generators

| Entry point in `build.rs` | Output in `OUT_DIR` | Include macro | Lookup |
| --- | --- | --- | --- |
| `day_build::generate_resources()` | `day_resources.rs` | `day::resources!()` | Global app/core catalogs |
| `day_build::generate_locales()` | `day_locales.rs` | `day_fluent::locales!()` | Private crate/file catalog |

Both generators normally read `resource/locales/` relative to the crate and emit embedded
Fluent sources. The app resource generator can read a merged flavor tree through
`DAY_RESOURCE_ROOT`. Cargo tracks the resource inputs, so changes regenerate the code on the
next build.
The generated files belong in Cargo's output directory and are not edited by hand.

`generate_resources()` also emits image, vector, asset, and font names. Its localization API
is unchanged: files within a locale concatenate into one source, and `res::str` accessors use
the global `tr` lookup. The app supplies `res::locales::CATALOG` through
`WindowOptions::locales`. See [resources](resources.md) for the other generated modules.

`generate_locales()` requires `day-fluent` at runtime. It reads the crate's resource directory
even when an app build sets `DAY_RESOURCE_ROOT`. An app's flavor overlay therefore does not
replace its dependencies' catalogs. Private catalog generation does not currently apply
flavor overlays.

## Private catalogs for reusable crates and source modules

The private generator accepts `resource/locales/<locale>/<file>.ftl`. A filename can correspond
to a Rust source module, but there is no source-file discovery step.

| Source | Accessor | Catalog descriptor | Script key |
| --- | --- | --- | --- |
| `app.ftl`, message `game_title` | `res::str::game_title()` | `res::locales::SCOPE` | `blockblast::game_title` |
| `board.ftl`, message `moves` | `res::board::str::moves(count)` | `res::board::locales::SCOPE` | `blockblast::board::moves` |

These examples assume the Cargo package is named `blockblast`. Script namespaces use the
package name with hyphens replaced by underscores, even if a dependent crate uses a Cargo
alias for that package.

Each named file imports the same locale's `app.ftl` messages and terms. The generated module
also exposes accessors for imported messages. A missing `app.ftl` is allowed. Duplicate
entries within a file, or collisions between a named file and its imports, fail generation.
Sibling files can reuse message names and use different argument lists.

File stems must be Rust identifiers. `app` selects the crate-wide catalog. `str`, `locales`,
`self`, `Self`, `super`, and `crate` are reserved. Nested directories beneath a locale are
rejected. Fluent syntax errors in private catalogs fail the build.

Each generated `locales` module contains:

| Item | Meaning |
| --- | --- |
| `DEFAULT` | `en` when present; otherwise the first locale in sorted order |
| `CATALOG` | Embedded `(tag, source)` pairs for this namespace |
| `ALL` | `(tag, display name)` pairs; names come from a literal `language_name` message, falling back to the tag |
| `SCOPE` | A static `day_fluent::Catalog` containing the namespace, default, and sources |
| `register()` | Makes this catalog available to qualified script lookups without changing the app locale |

The global resource generator emits `DEFAULT`, `CATALOG`, and `ALL`, plus `install()` instead
of `SCOPE` and `register()`. Normal app startup uses `WindowOptions::locales`; the explicit
install function remains available for code that manages installation itself.

### Combining resource and private catalog generation

Both include macros declare `pub mod res`. A crate using both generators must place the
outputs in separate modules. For example:

```rust
// build.rs
fn main() {
    day_build::generate_resources().expect("resource codegen");
    day_build::generate_locales().expect("localization codegen");
}
```

```rust
// src/lib.rs
day::resources!();

pub mod messages {
    include!(concat!(env!("OUT_DIR"), "/day_locales.rs"));
}
```

This exposes global resources under `res` and private translations under `messages`.
The resource generator still scans the same Fluent files as one global catalog. Files with
colliding names or different parameter lists that are valid in separate private namespaces
cannot also pass that global scan. Apps with reusable components can keep the global root
catalog and use private generation in the component crates.

## Generated message functions

Generated functions return `LocalizedText`. Function parameters follow the message's
`$variables`, sorted by name. Variable names must agree across translations. Arguments accept
strings, numbers, or supported signals through `IntoFArg`. A variable used numerically in any
translation requires `IntoNumberFArg`, which accepts `i64`, `f64`, `Signal<i64>`, and
`Signal<f64>`.

Plural selectors and `NUMBER()` arguments are numeric. String selectors, such as a selection
on `$gender`, remain string-capable. The generator unions message keys across locales;
translation completeness is a separate lint check. Generated documentation prefers the
English message when available.

A Fluent attribute such as `menu_group.key` generates `menu_group_key()`. Generation fails if
another key produces the same Rust name. A locale may omit an attribute and inherit the
catalog default's value; coverage lint requires message keys, not every attribute.

`LocalizedText` implements the text conversion used by pieces. A text binding tracks both
the locale signal and signal arguments. Calling `.format()` resolves a `String` immediately;
it remains reactive only when the call runs inside a reactive computation.

## Runtime implementation

[`day-build`](https://github.com/daybrite/day/tree/main/crates/day-build) parses Fluent sources
and generates the accessors. [`day-l10n`](https://github.com/daybrite/day/tree/main/crates/day-l10n)
manages Fluent bundles, locale selection, formatting, and the core catalog.
[`day-fluent`](https://github.com/daybrite/day/tree/main/crates/day-fluent) adds `LocalizedText`
and the `tr_in` constructor used by private accessors.

A private accessor calls `tr_in(&SCOPE, key)`. Registration parses its catalog on first use
and caches the bundles in thread-local runtime state. The static descriptor's address
identifies the catalog; its diagnostic name does not determine typed lookup. Two versions of
a crate can therefore keep separate translations even when their script namespace is the same.
Reinstalling the global app catalog preserves private caches and the shared locale signal.

The registration path uses ordinary Rust calls on native targets and WebAssembly. It does not
require platform initializers or an app-maintained dependency registration list. Catalog
sources are embedded in the binary; registration does not read translation files from disk.

## Lookup and fallback

For a requested locale, bundle selection tries the exact tag, the tag without a `-u-…`
extension, and then its language subtag. Once it selects a bundle, a missing message or
attribute follows the relevant default-catalog fallback below.

| Lookup | Resolution order |
| --- | --- |
| Private accessor | Selected private bundle → that private catalog's default bundle |
| Global `tr(key)` | Selected app bundle → app default → selected core bundle → English core |

Private lookup never searches the app, core, or another private catalog for a matching key.
An unresolved global key renders as `⟨key⟩`; a private key renders as
`⟨package::key⟩` or `⟨package::file::key⟩`. Fallback permits partial translations to render,
but a project that requires complete translations must also enforce coverage checks.

Message lookup recognizes these Chinese aliases when the earlier candidates are absent:

| Requested tag | Catalog tag |
| --- | --- |
| `zh-Hans`, `zh-Hans-CN`, `zh-SG` | `zh-CN` |
| `zh-Hant`, `zh-Hant-TW` | `zh-TW` |

Launch negotiation checks the app's registered tags separately. An app shipping `zh-CN` may
need a compatible root-catalog alias, such as `zh`, to accept the platform's script-based
preference before message lookup runs. Day Games registers that alias using the same source;
it keeps one translation file and one store listing per language.

## Which language an app opens in

`day::launch` collects backend locale hints, installs `WindowOptions::locales`, computes
`title_fn`, and then builds the UI. The app catalog declares supported launch languages;
private catalogs do not add languages to that declaration.

If `DAY_LOCALE` is set, it supplies the launch candidate. Otherwise Day considers the ordered
host preferences, including an explicit web `?locale=` value before browser preferences.
It selects the first candidate the app catalog can serve, or the configured default if none
matches. Launch matching accepts exact tags, tags without Unicode extensions, language
subtags, and pseudolocales. Core catalogs determine availability only when the app registers
no catalog entries.

`set_locale` from `day::prelude` changes the shared signal at runtime. It does not persist a
preference. An app that saves a selection must restore it through its startup flow. Locale
strings passed to `set_locale` normalize underscores to hyphens.

## Core strings the framework provides

The core catalog supplies standard dialog buttons, menu roles, window commands, and settings
labels. It ships `en`, `fr`, `es`, `de`, `ja`, `zh`, and `ar`. Examples include `day-ok`,
`day-cancel`, `day-copy`, and `day-about-app`; the last accepts the app name as `$app`.
Framework code resolves these through the global lookup, so an installed app catalog can
override them. Private catalog messages do not override core strings.

Native dialogs and permission UI may also contain text supplied by the operating system.
Those strings follow the system's language rules and may not follow an in-app locale switch.

## Formatted values: NUMBER() and DATETIME()

Every Day Fluent bundle registers ICU4X-backed `NUMBER()` and `DATETIME()` functions. Plain
numeric interpolations also use locale-aware decimal formatting.

```ftl
amount = { NUMBER($value, minimumFractionDigits: 2) }
percentage = { NUMBER($value, style: "percent") }
saved = Saved { DATETIME($when, dateStyle: "long", timeStyle: "short") }
```

| Function | Supported behavior |
| --- | --- |
| `NUMBER` | Grouping, minimum integer digits, minimum/maximum fraction digits, minimum/maximum significant digits, and decimal or percent style |
| `DATETIME` | ISO date, time, or date/time strings; numeric Unix seconds as UTC; `dateStyle` and `timeStyle` values `full`, `long`, `medium`, `short`, or `none` |

Decimal formatting defaults to at most three fraction digits unless options request more.
Percent formatting multiplies by 100 and adds a localized percent sign. Currency style is
not implemented; it renders as decimal and produces a lint finding.

Date/time formatting uses the Gregorian calendar. Civil strings carry no time zone. Numeric
timestamps use seconds, not milliseconds. Defaults depend on the input: medium date style
and short time style where those parts exist. An unparseable input remains visible as text.

`day lint` checks function names and options in each catalog. Findings distinguish unknown
functions, invalid option names or values, and options that Day does not support.

## Numbers outside a message

`day::format_decimal(value, fraction_digits)` formats a number using the selected locale.
`day::format_decimal_in(locale, value, fraction_digits)` takes an explicit locale. The first
tracks the locale signal, so it can update a readout inside a reactive closure:

```rust
label(move || day::format_decimal(total.get(), 2))
```

Grouping, decimal separators, and digits come from locale data. Non-finite values or missing
data fall back to Rust formatting.

## Sorting: locale-aware collation

`day::compare`, `day::compare_in`, and `day::sort_localized` use ICU4X collation. The first and
last track the selected locale. `compare_in` accepts locale extensions, such as
`zh-u-co-stroke`, for an alternate collation. Sort translated display strings; keep stable
IDs unchanged when the display order changes.

## Searching: localized match

`day::matches_search(text, query)` performs a case-insensitive prefix match at word starts.
`matches_search_in` takes an explicit locale. An empty query matches everything, and the
start of the text is always a candidate. Multi-word prefixes are supported.

ICU4X segmentation supplies word boundaries for scripts that do not separate words with
spaces. Unicode case folding handles cases such as `Straße` and `STRASSE`; Turkish and
Azerbaijani use Turkic folding. Matching does not remove accents: `é` and `e` remain distinct.

## Locale data

ICU4X components use compiled locale data independently of the app's Fluent locale list.
Changing that list changes the app's translations, not the embedded ICU data selection.
Unused components can be removed by the linker. Builds that supply custom baked data can
use `ICU4X_DATA_DIR`; the Day CLI does not run ICU data generation as part of app builds.

## Keyboard shortcuts

Fluent attributes can provide localized shortcut characters:

```ftl
menu_group = Group
    .key = g
```

This generates `res::str::menu_group()` and `res::str::menu_group_key()`. A missing translated
attribute falls back to the default locale. Modifiers remain command behavior in Rust, and
standard menu roles retain platform shortcuts. See [menus](menus.md).

## Permission reasons and store metadata

Platform metadata tools read the app root's catalogs. Keep permission reasons there using
`permission_<name>` keys, including raw platform keys where required. These messages are
consumed during platform generation and are exempt from the app's unused-key lint.
See [permissions](permissions.md) for the key mapping and generated platform files.

Store copy lives in `store/<locale>/`, separate from UI messages. `day store stage` maps
project locale tags to each store's spelling and generates fastlane metadata. Website locale
configuration lives in `website/site.toml`; the app site uses store text and localized
screenshot metadata. See [store listings](store.md) and [DayScript](https://daybrite.dev/docs/dayscript).

## Adding and removing locales

`day localize list` surveys the app root's Fluent directories, store directories, Xcode
`knownRegions`, and website locale list. `day localize add <tags>` copies the default Fluent
files and updates the other surfaces when present. It translates recognized scaffold
messages where a starter translation exists; the remaining messages need translation.
Store text is copied verbatim. Existing locale directories are left in place.

`day localize remove <tags>` removes the corresponding root surfaces. It refuses to remove
the default locale. Neither command edits private catalogs in dependency crates or the CI
locale matrix. Those changes belong in the app's translation workflow.

Project tags use a lowercase language, optional titlecase script, and optional uppercase or
numeric region, such as `fr`, `zh-Hans`, or `es-419`. Store support is checked separately.
Metadata mappings such as `zh-CN` to Apple's `zh-Hans` do not rename the source directory.

## Lint coverage and stale translations

`day lint --strict` fails when findings remain; `day localize list` is informational.
The compiler and generator check typed call sites and parameter consistency. The linter
adds coverage and metadata checks:

| Finding | Scope |
| --- | --- |
| `unknown-key`, `unused-key` | Heuristic source-reference checks for the global app catalog |
| `missing-translation` | Default-locale message keys absent from another existing locale |
| `invalid-catalog` | Private catalog generation errors, including invalid syntax and collisions |
| `unknown-function`, `bad-format-option`, `unsupported-format-option` | Fluent formatting calls |

Private catalog lint recognizes workspace source roots whose `build.rs` calls
`generate_locales()`. It checks files separately and does not report unused public accessors.
It does not audit arbitrary downloaded dependencies or require every crate to ship the app's
full locale set. Project checks can enforce exact locale, filename, key, and argument parity.

Coverage checks detect missing messages, not outdated wording. Day does not store a source
revision or review state for each translation. An unchanged key with changed English text
can pass every structural check. Copied text can pass too. Review reference-catalog diffs,
resolve translation TODOs, and check the rendered screens. Remove obsolete keys from all
locales; extra translated keys are not a reliable signal of a current catalog.

## Right-to-left locales

Day chooses layout direction at launch. Its layout engine mirrors horizontal placement,
while toolkits configure widget text and internal behavior for the selected direction.
Leading and trailing layout values follow that direction. Canvas drawings and other leaf
content are not automatically mirrored.

Runtime locale changes update strings but do not recompute the launch-time layout direction.
A direction change requires relaunching. Real right-to-left locales, such as Arabic, are used
for layout verification; Day does not provide an `ar-XB` pseudolocale.

## Pseudolocale

The `-XA` suffix accents and expands messages from the locale beneath it. `en-XA` exercises
the English catalog, and `fr-XA` exercises French. The transformation applies to formatted
output, including interpolated values, in global and private catalogs. It is a layout test,
not a translation-completeness check.

## DayScript catalog keys

A private assertion uses `package::key` or `package::file::key`. The catalog must already be
registered by an accessor call or by the generated `register()` function. Bare keys continue
to resolve globally. Two registered descriptors with the same script namespace make the
qualified lookup ambiguous; it returns an unresolved marker. Typed accessor calls remain
isolated by descriptor identity.

Use the same element IDs across locales and pass message arguments through the assertion's
`args` field. `day launch --locales 'en fr ar' --script dayscript/walkthrough.yaml` runs the
script per locale. A reusable `dayapp.yml` job takes the locale list in its `locales` input.
The CLI's locale-add command does not update that input.
