---
title: "Store listings"
description: "store/: localized App Store and Google Play listing sources, validated by day lint and packaged by the release pipeline."
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
> accepted by either store. Screenshots are not generated or uploaded yet.

An app's store listing is localized user-facing copy, so it lives beside the app's other localized
copy, as plain text a translator can edit:

```
store/app.toml            # not localized: category, copyright, contacts, review notes
store/<locale>/name.txt   # one directory per locale, keyed the same as resource/locales/
```

`day store stage` turns that into the two layouts the stores expect, under
`build/day/store/<target>/`, generated and never checked in, because a build must not write into a
tracked directory ([§20.3](../DESIGN.md#203-reproducible-build-verification)).

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

| `store/<locale>/…` | App Store | limit | Google Play | limit |
| --- | --- | --- | --- | --- |
| `name.txt` | `name.txt` | 30 | `title.txt` | 30 |
| `subtitle.txt` | `subtitle.txt` | 30 | — | |
| `short.txt` | — | | `short_description.txt` | 80 |
| `description.txt` | `description.txt` | 4000 | `full_description.txt` | 4000 |
| `keywords.txt` | `keywords.txt` | 100 | — | |
| `release-notes.txt` | `release_notes.txt` | 4000 | `changelogs/<versionCode>.txt` | **500** |
| `promo.txt` | `promotional_text.txt` | 170 | — | |
| `marketing-url.txt` | `marketing_url.txt` | 255 | `video.txt` | 255 |
| `support-url.txt` | `support_url.txt` | 255 | — | |
| `privacy-url.txt` | `privacy_url.txt` | 255 | — | |

An app shipping to both stores is held to the **stricter** limit, which is why release notes are
checked against 500 rather than 4000. Play's changelog is keyed by versionCode, so it is written to
`changelogs/<[app] build>.txt`.

`store/app.toml` carries what is not localized: `bundle-id`, `apple-category`, `copyright`,
`contact-email`, `review-notes`, and the App Review contact as `contact-first-name`,
`contact-last-name` and `contact-phone` (with its country code, `+1 555 555 5555`). App Store
Connect refuses a review contact missing any of the three, so the staged
`review_information/` tree is written only when all of them and the email are set; otherwise
the contact already entered in App Store Connect stands and the notes stay home. There is no Play category, because Google Play's category is
set in the Play Console and `supply` cannot write it; recording one here would be a value that
never reached the store.

The listing's name and short description are also what the web build's home-screen manifest
carries ([docs/web.md](web.md) "Home screen and offline"), so an app is called the same thing
on a store page and on a phone's home screen.

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
| `store-missing` | the app ships to a store and has no `store/` at all |
| `store-missing-locale` | the app is translated into a locale the listing is not |
| `store-orphan-locale` | a listing for a locale the app is not translated into |
| `store-unmapped-locale` | a tag neither store knows; an upload under it is dropped silently |
| `store-default-locale` | no listing in the app's default locale, which both stores require |
| `store-missing-field` | a field the targeted store rejects the listing without |
| `store-too-long` | over the limit, naming the store whose limit binds |
| `store-placeholder` | still the scaffold's `TODO`, which would upload verbatim |
| `store-bad-url` | a URL field that is not `https://` |
| `store-bad-keywords` | spaces after the commas; Apple counts them against the 100 |
| `store-whitespace` | leading or trailing whitespace |

The locale checks compare against `resource/locales/`, so the listing and the app cannot drift
apart: translating the app into a new language makes `day lint` ask for the listing to follow.

The last two are the only listing rules `day lint --fix` will repair, because they are the only
ones with a single right answer that invents no copy. Both rewrite the file whole:

```
$ day lint --fix
fixed   day::lint::store-whitespace     store/en/name.txt: Trim the surrounding whitespace
fixed   day::lint::store-bad-keywords   store/en/keywords.txt: Remove the spaces around commas
```

The keyword repair splits on commas and trims each entry, so `a,  b ,c` and `a, b, c` both land on
`a,b,c` in one pass. Every other code needs someone to write words, and reports instead.

## Uploading

`day store stage` writes a normal fastlane project per target:

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
with export compliance answered as exempt; the release itself still waits for the Release button
in App Store Connect. `DAY_IPA` names the artifact outright, which is how the release workflow
hands the lane the `.ipa` it downloaded.

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

## Not done yet

- **Screenshots:** both stores take them per locale and per device class, and the dayscript
  walkthrough already captures exactly that shape (`build/day/screenshots/<target>/<variant>/`).
  Wiring those into `fastlane/screenshots/` is the obvious next step and is not built.
- **Review information** beyond notes and an email: the demo-account fields and the phone number
  are missing.
- **Age rating / content declarations**, which both stores require before a first submission and
  neither accepts from `supply`/`deliver` in full.
- **No listing has been uploaded.** Everything here is verified up to the point where a credential
  would be needed.
