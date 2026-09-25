---
title: "Store listings"
description: "store/storefront.toml: the App Store and Google Play listing, submission info, text per locale and screenshots in one file, validated by day lint and packaged by the release pipeline."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Store listings (App Store, Google Play)

> **Status: implemented** as `store/` in a project, `day store init` / `day store stage`, the
> `day::lint::store-*` checks, and a `distribute` job in day's own CI. What is verified: the
> generated trees parse under real fastlane 2.237 (`fastlane lanes` lists the lanes), the artifact
> globs resolve to `build/day/dist`, and the lint rules are unit-tested. What is not verified: an
> actual upload; no App Store Connect or Play credentials exist yet, so no listing has been
> accepted by either store. Screenshots are placed from a gallery index (`stage --screenshots`)
> and go up with the listing; the App Fair's queue is the first pipeline to run that path.

Everything a store listing is made of lives in one file, `store/storefront.toml` (or `store/storefront.yaml`,
the same tree in YAML): the submission info, the listing text in every locale, and the
screenshots each listing shows, each declared once and specialized per target, per store and per
locale with the nearest declaration winning.

```toml
[storefront.submission-info]                          # every store: copyright, contact, review notes
[storefront.metadata]                                 # the listing text in the default locale
[storefront.metadata.fr]                              # French: what differs from the default
[storefront.ios-uikit.apple-app-store.submission-info] # one store's own record
[storefront.ios-uikit.apple-app-store.metadata]       # one store's own wording
[storefront.ios-uikit.screenshots]                    # the website's page, the stores' fallback
[storefront.ios-uikit.apple-app-store.screenshots]    # one store's own set
```

`day store stage` turns that into the two layouts the stores expect, under
`build/day/store/<target>/`, generated and never checked in, because a build must not write into a
tracked directory ([§20.3](../DESIGN.md#203-reproducible-build-verification)). `day store export`
writes the whole listing, resolved, as one JSON document for the app's website and for anything
else that publishes the app. What each store is called, which targets publish to it, its fields
and limits, its locale spellings and its screenshot sizes are data the CLI ships
([the stores' rules](#the-stores-rules)), not code.

The file was `store/app.toml` before 2026-09-25; that name still reads, and `day lint` says to
rename it.

## Why one source feeds both stores

The stores agree on almost nothing. They disagree about what the fields are called
(`name` / `title`, `description` / `full_description`), how long they may be (release notes: 4000
characters on the App Store, **500** on Play), which fields exist at all (keywords are Apple-only,
the short description is Google-only), and how a locale is spelled: `zh-CN` here is `zh-Hans` to
Apple and `zh-CN` to Google, and Google still writes Hebrew with the pre-1989 code `iw`.

Authoring two parallel trees means writing the 4000-character description twice, in two spellings of
every locale, and keeping them in step by hand. That is the same argument that makes `resource/` fan
out to per-platform resources instead of being authored per platform, and `[permissions]` fan out to
manifests and plists. One source is generated outward.

## The fields

The listing text is `[storefront.metadata]`, the default locale's (`en` when the app has it, else
the first of `resource/locales/`), with a table per other locale under it:

```toml
[storefront.metadata]
name = "Example"                                  # ≤30, the App Store name and Play title
subtitle = "Native UI from Rust"                  # ≤30, App Store only
short = "One sentence for Play search results."   # ≤80, Google Play only
description = """
What the app does, who it is for, what it does not do.

Both stores allow 4000 characters and show the first lines before a fold.
"""
keywords = ["rust", "native", "widgets"]          # App Store only; ≤100 characters joined by commas
release-notes = "What changed in this version."   # ≤4000 App Store, ≤500 Play: 500 binds
promo = "…"                                       # ≤170, App Store promotional text (optional)
marketing-url = "https://example.com"             # App Store only
support-url = "https://example.com/support"       # App Store only
privacy-url = "https://example.com/privacy"       # the App Store requires one

[storefront.metadata.fr]                          # French: what differs; the rest is inherited
subtitle = "Interface native en Rust"
short = "Une phrase pour les résultats de recherche Play."
description = """…"""
keywords = ["rust", "natif", "widgets"]
release-notes = "Ce qui a changé."
```

A locale's table carries what differs from the default locale's; every field it leaves out is the
default's, so the name and the URLs an app keeps the same everywhere are written once. A scalar
(or a list, for `keywords`) is a field; a table is a locale. The default locale has no table of
its own: its text is `[storefront.metadata]` itself, and a `[storefront.metadata.en]` is refused.

| key | App Store | limit | Google Play | limit |
| --- | --- | --- | --- | --- |
| `name` | `name.txt` | 30 | `title.txt` | 30 |
| `subtitle` | `subtitle.txt` | 30 | — | |
| `short` | — | | `short_description.txt` | 80 |
| `description` | `description.txt` | 4000 | `full_description.txt` | 4000 |
| `keywords` | `keywords.txt` | 100 | — | |
| `release-notes` | `release_notes.txt` | 4000 | `changelogs/<versionCode>.txt` | **500** |
| `promo` | `promotional_text.txt` | 170 | — | |
| `marketing-url` | `marketing_url.txt` | 255 | — (Play's `video.txt` is a YouTube promo video; the website is a Play Console setting) | |
| `support-url` | `support_url.txt` | 255 | — | |
| `privacy-url` | `privacy_url.txt` | 255 | — | |

An app shipping to both stores is held to the **stricter** limit, which is why release notes are
checked against 500 rather than 4000. Play's changelog is keyed by versionCode, so it is written to
`changelogs/<[app] build>.txt`.

### Per target and per store

The same `metadata` table sits under a target and under a store, for wording that belongs to one
storefront: the App Store's subtitle and promotional text, Play's short description mentioning
Material, the web build's home-screen description (the web target's `name` and `short` are what
the manifest carries, [docs/web.md](web.md) "Home screen and offline"). Each level carries what
differs and inherits the rest:

```toml
[storefront.ios-uikit.apple-app-store.metadata]
subtitle = "Native UIKit, written in Rust"
promo = "Thirty-five live demos, all drawn by UIKit."

[storefront.ios-uikit.apple-app-store.metadata.fr]
subtitle = "UIKit natif, écrit en Rust"

[storefront.android-mdc.google-play-store.metadata]
short = "Live demos of the framework: Material widgets from one Rust codebase."

[storefront.web-dom.metadata]
short = "Try the demos in your browser."
```

A field resolves for one store, one target and one locale by looking at the locale's tables
first, nearest level first (the store's, the target's, the shared), then the default locale's the
same way. A locale's wording therefore outranks a store's: an app that specializes the App Store's
subtitle in the default locale alone still shows French users the French subtitle from
`[storefront.metadata.fr]`, and a store's French table wins over everything.

### Text in files, and a file per locale

Any field may instead be `<field>-ref = "path"`, a project-relative file holding the text, for a
long description a translation tool manages or a release note pasted from elsewhere. The file's
trailing newline is not part of the text; a keywords file holds one keyword per line or a
comma-separated list. A field set both ways is refused, and so is a path that leaves the project
or points outside it (a listing must build from the repository alone).

```toml
[storefront.metadata.zh-CN]
subtitle = "用 Rust 编写的原生界面"
description-ref = "store/metadata/zh-CN/description.txt"
```

A locale's text may also sit in `store/storefront.<tag>.toml` (or `.yaml`) beside the main file: the
same tree with only `metadata` tables in it, each read as that locale's, at the shared level or
under a target or store the main file declares. A translator then edits one file per language.
What the main file already sets for that locale, two files for one tag, or anything but
`metadata` tables in one are refused.

```toml
# store/storefront.ar.toml
[storefront.metadata]
subtitle = "…"
description = """…"""

[storefront.ios-uikit.apple-app-store.metadata]
promo = "…"
```

`day localize add fr` starts a locale's table as the default locale's text (appended to a TOML
main file; as `storefront.fr.yaml` beside a YAML one, which cannot take a table appended at its
end, and the convention for a YAML project is one locale file per language),
and `day localize remove fr` drops it everywhere it sits. `day store init` does the same for
every locale the app ships and the listing lacks, with skeleton text for the default locale.

### Submission info

`[storefront.submission-info]` carries what is not localized, shared, per target, and per store,
the nearest level winning key by key:

```toml
[storefront.submission-info]                          # every store
copyright = "2026 Example"
contact-email = "support@example.com"
review-notes = "How to exercise the app, for the reviewer."

[storefront.ios-uikit.submission-info]                # every store of one target
contact-email = "ios@example.com"

[storefront.ios-uikit.apple-app-store.submission-info] # one store
apple-category = "DEVELOPER_TOOLS"
apple-release = "manual"                              # hold an approved version for the Release button
```

`apple-release` says what happens once App Review approves a version: `automatic`, the default,
releases it to the store on its own; `manual` leaves it in Pending Developer Release until
someone presses Release in App Store Connect. Google Play has no such gate: a completed
production release rolls out when its review passes.

The file may be `storefront.toml` or `storefront.yaml` (`.yml` too), whichever you prefer to write; both carry
the same tree, are parsed into the same value, and resolve identically, so a project can switch
formats at any time. In YAML the example above reads:

```yaml
storefront:
  submission-info:
    copyright: "2026 Example"
    contact-email: support@example.com
    review-notes: |
      How to exercise the app, for the reviewer.
  metadata:
    name: Example
    keywords: [rust, native]
    description: |-
      What the app does.
    fr: { name: Exemple }
  ios-uikit:
    submission-info: { contact-email: ios@example.com }
    apple-app-store:
      submission-info: { apple-category: DEVELOPER_TOOLS }
```

One rule follows from YAML's implicit typing: every value here is text, so a bare `2026` or
`1.2` is refused by name (quote it). A directory with both files is refused rather than one
being preferred, so the two can never drift apart; `day store init` writes TOML. A key the file
does not take (a typo in `submission-info`, a field no store has, a store table with something
other than `submission-info`, `metadata` and `screenshots` in it) is read past and reported by
`day lint` as `store-unknown-key`, with the keys the table takes, so a listing with one typo
still stages once the typo is fixed and never stages while it stands.

The keys are `bundle-id`, `apple-category`, `apple-release`, `copyright`, `contact-email`, `review-notes`, and
the App Review contact as `contact-first-name`, `contact-last-name` and `contact-phone` (with its
country code, `+1 555 555 5555`). `day store stage` resolves them for the store it stages, so the
App Store record and the Play record can differ where they must and share the rest. App Store
Connect refuses a review contact missing any of the three, so the staged `review_information/`
tree is written only when all of them and the email are set; otherwise the contact already
entered in App Store Connect stands and the notes stay home. There is no Play category, because
Google Play's category is set in the Play Console and `supply` cannot write it; recording one
here would be a value that never reached the store. A key at the top level of the file is not
read: `day lint` names it and where it belongs. Store keys are open: a storefront the rules know
by name (`mac-app-store`, `altstore`, `f-droid`, `flathub`, `microsoft-store`, `appgallery`)
declares its own tables the same way and rides through the export and the gallery index for
whatever publishes there; a key the rules do not know does the same, with a `store-unknown-store`
warning, since it is either a storefront nobody has named yet or a typo.

### Moving from the older layout

Before 2026-09-25 the text lived in `store/<locale>/*.txt`, one directory per locale and one
file per field. `day store migrate` folds those directories into `[storefront.metadata]` tables
(appended to a TOML main file; as locale files beside a YAML one) and removes them (`--keep`
leaves them); the directories are not read otherwise, and `day lint` says so. Afterwards, move
the values every locale shares up into `[storefront.metadata]` and drop them from the locale
tables.

## Listed apps

Once a listing is live, say so in `Day.toml`:

```toml
[store]
apple-app-id = "6802801331"               # https://apps.apple.com/app/id6802801331
google-play-id = "dev.daybrite.showcase"  # https://play.google.com/store/apps/details?id=…
```

Each key is independent, and each is the store's own identifier for the listing rather than
a URL, so the URL shape stays the framework's concern: `day metadata --json` reports both the
ids and the `apple-url` / `google-url` they resolve to. The project site
([daysite](https://github.com/daybrite/daysite)) reads the same table into its app index as the
`appleappstore` and `googleplaystore` channels and shows the store's localized badge on the
landing page, linking to the listing; an app with neither key shows its downloads instead.
The template ships the stores' localized badge artwork, so a French page shows the French badge.

## What `day lint` checks

| code | what it catches |
| --- | --- |
| `store-missing` | the app ships to a store and the listing has no text (or only the older `store/<locale>/` layout, which `day store migrate` folds in) |
| `store-legacy-name` | the file is still called `app.toml` (or a locale file `app.<tag>.toml`); rename it to `storefront.toml` |
| `store-unknown-key` | a key the file does not take, named with the keys the table takes |
| `store-unknown-target` | a `[storefront.<target>]` for a target the app does not build |
| `store-unknown-store` | a store key the rules do not know by name (a storefront to be, or a typo) |
| `store-unknown-shot` | a screenshot name no dayscript captures |
| `store-missing-locale` | the app is translated into a locale no `metadata` table specializes |
| `store-orphan-locale` | a locale table for a locale the app is not translated into |
| `store-unmapped-locale` | a tag no store spells; an upload under it is dropped silently |
| `store-default-locale` | no `[storefront.metadata]` table, which both stores require |
| `store-missing-field` | a store's record, in some locale, resolves to none of a field the store's rules list as `required` (`name`, `description`; `short` for Play, `privacy-url` for the App Store) |
| `store-too-long` | over the limit, naming the store whose limit binds; a text under a target or store is measured by that store alone |
| `store-placeholder` | still the scaffold's `TODO`, which would upload verbatim |
| `store-bad-url` | a URL field that is not `https://` |
| `store-whitespace` | leading or trailing whitespace |
| `store-unreadable` | the file does not parse: a field set inline and by `-ref`, a `-ref` file that is missing or outside the project, a locale table for the default locale, a locale file that carries more than text |

`store-unreadable`, `store-unknown-key`, `store-unknown-target`, `store-unknown-shot`,
`store-missing-field`, `store-default-locale`, `store-too-long` and `store-bad-url` are errors;
the rest are warnings. `day store stage` refuses a listing with any error, and with a
`store-placeholder` unless `--allow-placeholders` says the TODO goes up on purpose, so what a
store would reject is refused before an upload.

The locale checks compare against `resource/locales/`, so the listing and the app cannot drift
apart: translating the app into a new language makes `day lint` ask for the listing to follow.
Every finding names its place: `store/storefront.toml [storefront.metadata.fr] description`, or the
referenced file with the key that named it.

Whitespace is the only listing rule `day lint --fix` repairs, and only in a referenced file, which
it rewrites whole; a value inline in the storefront file is reported and left to you. Keywords
are a list, so there are no spaces after commas to strip. Every other code needs someone to write
words, and reports instead.

```
$ day lint --fix
fixed   day::lint::store-whitespace     store/metadata/en/description.txt: Trim the surrounding whitespace
```

## Uploading

`day store stage` holds the listing to `day lint`'s store rules first (every error, and any
placeholder text unless `--allow-placeholders`), then writes a normal fastlane project per
target:

```
build/day/store/ios-uikit/fastlane/{Appfile,Fastfile,metadata/…}
build/day/store/android-mdc/fastlane/{Appfile,Fastfile,metadata/android/…}
```

Two lanes each. `validate` asks the store to check the build and the listing and rolls back;
`upload` sends it. Neither submits for review or releases to users: iOS uploads a build, Android
uploads to the internal track as an unreleased draft. Each has a third lane, `release`: on Android
it uploads to the production track as a completed release, which is Play's submission, and the
rollout starts when Google's review passes; on iOS it
uploads, waits for App Store Connect to process the build, and submits the version for review
with export compliance answered as exempt; once approved it goes live on its own, unless the
listing's `apple-release = "manual"` keeps it for the Release button in App Store Connect
(`day store stage` writes that choice into the tree's `.env.default` as
`DAY_ASC_MANUAL_RELEASE`, which the lanes read). `DAY_IPA` and `DAY_AAB` name the artifact outright, which is how the
release workflow hands each lane the file it downloaded; without them the lanes glob
`build/day/dist/`.

```sh
day pack -p ios-uikit --profile release
cd build/day/store/ios-uikit && fastlane ios validate
```

Credentials come from the environment, never from a checked-in file:

| | variables |
| --- | --- |
| App Store | `DAY_ASC_KEY_ID`, `DAY_ASC_ISSUER`, `DAY_ASC_KEY` (path to the `.p8`) |
| Google Play | `SUPPLY_JSON_KEY` (path to the service-account JSON) |

The Fastfile finds the artifact by glob rather than by name: `day pack` names an unsigned
device build `<stem>-ios-uikit-unsigned.ipa` and a signed one `<stem>-ios-uikit.ipa`, and a lane
that hardcoded either would break on the day signing was configured. `<stem>` is the app's own
(`[app] artifact` in `Day.toml`, else a slug of its title), which is the other reason for the glob.

## In CI

day's own workflow has a `distribute` job (tag pushes only) that stages the listing, then runs
`validate` followed by `upload`. Each leg **skips itself** when its credentials are absent rather
than failing (the secrets are optional, so a fork still gets a green run), and always
uploads the generated tree as an artifact, so what was sent to the store is reviewable after the
fact.

## Screenshots

Both stores take screenshots per locale and per device class, and a dayscript walkthrough
captures far more screens than a listing shows. Which captures each listing shows is declared
in `store/storefront.toml`'s `[storefront]`, apart from the walkthrough that takes them, so one
capture set serves several listings and the website:

```toml
[storefront.ios-uikit.screenshots]                  # the website's ios-uikit page; the stores' fallback
default = ["home", "canvas", "controls"]
ipad = ["home", "canvas", "grid", "layout"]          # the iPad row shows these

[storefront.ios-uikit.apple-app-store.screenshots]
iphone = ["canvas", { name = "controls", theme = "dark" }, "layout"]
# ipad is not named, so the App Store's iPad set is the target's ipad list above

[storefront.android-mdc.google-play-store.screenshots]
default = ["canvas", "controls", "layout"]           # every device kind not named
tablet = ["canvas", "layout", "grid"]

[storefront.macos-appkit.screenshots]
default = ["home", "canvas", "menus"]

[storefront.macos-appkit.mac-app-store.screenshots]
default = ["canvas", "controls", "menus"]
```

Each name is a `screenshot:` step of a dayscript; a bare name takes the light capture and
`{ name = "…", theme = "dark" }` the dark one. Every `screenshots` table lists device kinds, the
slugs the CI device profiles name (`iphone`, `ipad`, `phone`, `tablet`), and `default` for the
kinds it does not name. A target's own table is what the app's website shows on that target's
page, one row per device kind the walkthrough captured on (iPhone and iPad; Handset and Tablet),
and what its stores fall back to: a store's kind → the store's `default` → the target's kind →
the target's `default`.

A list applies to every locale the walkthrough captured; a locale table inside a `screenshots`
table specializes it for captures of that locale, with the same lists inside:

```toml
[storefront.ios-uikit.screenshots.fr]                   # the French page and listings lead with localization
default = ["localization", "canvas", "controls"]

[storefront.ios-uikit.apple-app-store.screenshots.zh-CN]
iphone = ["localization", "text", "canvas"]
```

A list is a device kind, a table is a locale; a locale matches by exact tag, then by primary
language (`fr` covers `fr-CA`). At each level the locale's lists come before the general ones
(kind, then `default`), and the levels keep their order: a store's general list still beats the
target's French one. Store keys
are open: `apple-app-store` and `google-play-store` are what `day store stage` places;
`mac-app-store`, `altstore`, `f-droid` or any other key rides through the index for whatever
publishes there. `day lint` checks every name against the dayscripts and every target against
Day.toml.

`day screenshot index` resolves the declaration into `gallery.json`'s `listings`, per captured
target: `website`, one list per device kind and per locale the target captured, and `stores`,
the same per store, each item `{ "shot", "theme" }`. The stores written are every store the
declaration names and the store [the rules](#the-stores-rules) stage for the target whether or
not it is named, so a list declared on the target alone reaches the store's listing as well as
the website's page. That is what every consumer selects on, so none needs the app's `store/` or
the fallback rules. A store's block carries only the device kinds its rules list (Play's
`phone`, `tablet` and `tablet-7`; the App Store's `iphone` and `ipad`) and any the declaration
names for it; a capture on some other profile, such as an optional API-floor row in a CI matrix,
appears in the website's rows and nowhere a store would refuse it.

`day store stage --screenshots <gallery.json | URL>` reads an index, takes each device kind's
list for the target's store, one capture per locale the store knows, and places them where
fastlane reads them:

| store | where | device |
|---|---|---|
| App Store | `fastlane/screenshots/<locale>/<NN>-<device>-<shot>.png` | deliver reads it from the image's size |
| Google Play | `fastlane/metadata/android/<locale>/images/<kind>Screenshots/<NN>-<shot>.png` | the kind: `phone`, `tablet-7`, or `tablet` for ten inches |

The generated lanes upload screenshots only when some were staged, so a run without the flag
leaves what the store already shows. The index can be the app's published site
(`https://<host>/main/gallery/gallery.json`, the latest build) or a local `gallery.json` beside
its capture tree, in which case the images are read from that tree.

### The stores' rules

`day store screenshots <gallery.json | URL>` holds the declared set to each store's rules and
fails on any refusal; `day store stage --screenshots` runs the same check before it places a
file, so a set a store would refuse never reaches an upload.

Everything the CLI knows about a store is data, one table per store in `store-rules.toml`:

```toml
[apple-app-store]
label = "the App Store"
targets = ["ios-uikit"]                  # which targets the CLI stages to this store
layout = "deliver"                       # the fastlane tree: deliver (Apple) or supply (Google)
default-kind = "iphone"                  # what a capture from a profile-less run counts as
required = ["name", "description", "privacy-url"]

[apple-app-store.fields]                 # deliver's file name and the store's limit
name = { file = "name.txt", limit = 30 }
keywords = { file = "keywords.txt", limit = 100 }
# …

[apple-app-store.locales]                # Day tag → the store's spelling
"zh-CN" = "zh-Hans"
"he" = "he"
# …

[apple-app-store.screenshots.iphone]     # per device kind
label = "iPhone"
required = true
max = 10
sizes = [[1320, 2868], [2868, 1320], [1290, 2796], [2796, 1290], [1260, 2736], [2736, 1260]]

[google-play-store.screenshots.phone]
label = "Handset"
folder = "phoneScreenshots"              # supply's folder for the kind
required = true
max = 8
upscale = true                           # a capture under min-side is scaled up to it, not refused
min-side = 1080
max-side = 7680
max-ratio = 2.3

[mac-app-store]                          # known by name, not staged
label = "the Mac App Store"
targets = ["macos-appkit"]
```

The CLI ships a copy as its default; `--rules FILE` or `DAY_STORE_RULES` names another, and a
project's `store/rules.toml` takes over when present (whole, so it says which store stages
what). The shared CI workflow hands the CLI its own copy (`daybrite/actions`, the `store-rules`
action), so a store changing its limits is a data edit there, in force for every app on its next
run, and `day store export` carries the rules in force, so a consumer of the export (the App
Fair's queue) holds captures to the same limits without a copy of its own. `day lint` reads the
same file: the field limits, the required fields and the locale spellings it checks are the
rules', and a store with a `layout` is one the CLI stages.

| store | device slug | rule |
|---|---|---|
| App Store | `iphone` | 1320×2868, 1290×2796 or 1260×2736, either way up (the 6.9" sizes); at most 10 per locale |
| App Store | `ipad` | 2064×2752 or 2048×2732, either way up (the 13" sizes); at most 10 per locale |
| Google Play | `phone` | 1080 to 7680 px a side, the long side at most 2.3× the short; at most 8 per locale |
| Google Play | `tablet`, `tablet-7` | the same range; the set is optional; a capture under the floor is scaled up (`upscale = true`) |

Play's numbers are what its publishing API enforces ("min size: [1080], max size: [7680], max
aspect ratio: [2.3]"), not the 320 px and 2:1 its help page states. The default CI device
profiles `iPhone * Pro Max` and `iPad Pro 13-inch` produce the Apple sizes, and a 1080-wide
phone such as `medium_phone` or `pixel` clears Play. The default `medium_tablet` does not as
captured: `day devices boot` halves a headless panel past three million pixels, so it captures
1280×800, under the floor. A kind whose rule says `upscale = true` (Play's tablet and phone
kinds in the shipped rules) has such a capture scaled up by the smallest whole factor that
clears the floor, ×2 to 2560×1600 here, before it is placed; the check judges the size the store
receives, and `day store screenshots` reports the factor. A scaled capture is softer than a
native one, which is why the key is per kind and off for the App Store, whose exact sizes a
scale cannot hit. Every locale the index carries that the store knows needs a capture on each
required device, since App Store Connect refuses a version whose localization has none; a
device slug the store has no kind for is refused too.

The shared `dayapp.yml` workflow does all of this on a tag with `store-screenshots: true`: each
upload job takes the run's own `screenshots-<target>` artifact, indexes it, checks it, stages
the listing with it, and uploads, so the set is the tagged version's without a website in
between. The App Fair's queue rebuilds an app from its tag and takes the set from the release
instead, the `gallery.json` and `screenshots.zip` the same workflow attaches to it, and its
pull-request checks hold it to the rules the release's `storefront.json` carries before anything
is built.

## Exporting

`day store export` writes the whole listing as one JSON document, resolved the way `day store
stage` resolves it, so a consumer reads neither the storefront file nor the project:

```sh
day store export --out build/day/store/storefront.json
```

```json
{
  "schema": 1,
  "generator": "day 0.4.8",
  "project": { "name": "…", "id": "…", "title": "…", "version": "…", "build": 7, "artifact": "…",
               "targets": ["ios-uikit", "android-mdc"],
               "store": { "apple-app-id": "…", "google-play-id": "…", "apple-url": "…", "google-url": "…" } },
  "default-locale": "en",
  "locales": ["en", "fr"],
  "storefront": {
    "file": "store/storefront.toml", "files": ["store/storefront.toml", "store/storefront.ar.toml"],
    "submission-info": { "copyright": "…", "contact-email": "…", … },
    "metadata": { "en": { "name": "…", "keywords": ["…"], … }, "fr": { … } },
    "targets": {
      "ios-uikit": {
        "submission-info": { … }, "metadata": { "en": { … }, "fr": { … } },
        "screenshots": { "default": [{ "shot": "home", "theme": "light" }], "fr": { "default": [ … ] } },
        "stores": { "apple-app-store": { "submission-info": { … }, "metadata": { … }, "screenshots": { … } } }
      }
    }
  },
  "permissions": [ … ],
  "rawPermissions": { … },
  "rules": { "apple-app-store": { "label": "…", "fields": { … }, "locales": { … }, "screenshots": { … } }, … }
}
```

Every `metadata` map holds each locale's text fully resolved (a locale that sets nothing of its
own carries the default locale's), `keywords` as a list, and the target and store levels the same
so a store queue reads its record without knowing the fallback rules; `screenshots` is the
declaration as written. The app's website is built from this document alone (daysite's
`generate-appindex`, handed it by the workflow as `--storefront FILE`, or run through `DAY_BIN`),
and a release carries it as `storefront.json` beside `gallery.json`, so a catalog or a store queue
describes the app without a checkout. `rules` is the stores' rules in force for the project
([the stores' rules](#the-stores-rules)), so the same consumer holds the release's captures to
them without a copy of its own. `day metadata --json` describes the project and its permissions;
the storefront is this document's alone.

## Not done yet

- **Review information** beyond notes and an email: the demo-account fields and the phone number
  are missing.
- **Age rating / content declarations**, which both stores require before a first submission and
  neither accepts from `supply`/`deliver` in full.
- **No listing has been uploaded.** Everything here is verified up to the point where a credential
  would be needed.
