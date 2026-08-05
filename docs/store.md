# Store listings (App Store, Google Play)

> **Status: implemented** as `store/` in a project, `day store init` / `day store stage`, the
> `day::lint::store-*` checks, and a `distribute` job in day's own CI. What is verified: the
> generated trees parse under real fastlane 2.237 (`fastlane lanes` lists the lanes), the artifact
> globs resolve to `build/day/dist`, and the lint rules are unit-tested. What is NOT verified: an
> actual upload — no App Store Connect or Play credentials exist yet, so no listing has been
> accepted by either store. Screenshots are not generated or uploaded yet.

An app's store listing is localized user-facing copy, so it lives beside the app's other localized
copy, as plain text a translator can edit:

```
store/app.toml            # not localized: category, copyright, contacts, review notes
store/<locale>/name.txt   # one directory per locale, keyed the same as resource/locales/
```

`day store stage` turns that into the two layouts the stores expect, under
`build/day/store/<target>/` — generated, never checked in, because a build must not write into a
tracked directory ([§20.3](../DESIGN.md#203-reproducible-build-verification)).

## Why one source and not two fastlane trees

The stores agree on almost nothing. They disagree about what the fields are called
(`name` / `title`, `description` / `full_description`), how long they may be (release notes: 4000
characters on the App Store, **500** on Play), which fields exist at all (keywords are Apple-only,
the short description is Google-only), and how a locale is spelled — `zh-CN` here is `zh-Hans` to
Apple and `zh-CN` to Google, and Google still writes Hebrew with the pre-1989 code `iw`.

Authoring two parallel trees means writing the 4000-character description twice, in two spellings of
every locale, and keeping them in step by hand. That is the same argument that makes `resource/` fan
out to per-platform resources instead of being authored per platform, and `[permissions]` fan out to
manifests and plists. One source, generated outward.

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
`contact-email`, `review-notes`. There is deliberately no Play category — Google Play's category is
set in the Play Console and `supply` cannot write it, so recording one here would be a value that
silently never reached the store.

## What `day lint` checks

| code | what it catches |
| --- | --- |
| `store-missing` | the app ships to a store and has no `store/` at all |
| `store-missing-locale` | the app is translated into a locale the listing is not |
| `store-orphan-locale` | a listing for a locale the app is not translated into |
| `store-unmapped-locale` | a tag neither store knows — an upload under it is dropped silently |
| `store-default-locale` | no listing in the app's default locale, which both stores require |
| `store-missing-field` | a field the targeted store rejects the listing without |
| `store-too-long` | over the limit, naming the store whose limit binds |
| `store-placeholder` | still the scaffold's `TODO`, which would upload verbatim |
| `store-bad-url` | a URL field that is not `https://` |
| `store-bad-keywords` | spaces after the commas — Apple counts them against the 100 |
| `store-whitespace` | leading or trailing whitespace |

The locale checks compare against `resource/locales/`, so the listing and the app cannot drift
apart: translating the app into a new language makes `day lint` ask for the listing to follow.

## Uploading

`day store stage` writes a normal fastlane project per target:

```
build/day/store/ios-uikit/fastlane/{Appfile,Fastfile,metadata/…}
build/day/store/android-mdc/fastlane/{Appfile,Fastfile,metadata/android/…}
```

Two lanes each. `validate` asks the store to check the build and the listing and rolls back;
`upload` sends it. Neither submits for review or releases to users: iOS uploads a build, Android
uploads to the internal track as an unreleased draft. Promotion stays a human decision in the
console, which is where the consequences are visible.

```sh
day pack -p ios-uikit --profile release
cd build/day/store/ios-uikit && fastlane ios validate
```

Credentials come from the environment, never from a checked-in file:

| | variables |
| --- | --- |
| App Store | `DAY_ASC_KEY_ID`, `DAY_ASC_ISSUER`, `DAY_ASC_KEY` (path to the `.p8`) |
| Google Play | `SUPPLY_JSON_KEY` (path to the service-account JSON) |

Note that the Fastfile finds the artifact by glob rather than by name: `day pack` names an unsigned
device build `<app>-unsigned.ipa` and a signed one `<app>.ipa`, and a lane that hardcoded either
would break on the day signing was configured.

## In CI

day's own workflow has a `distribute` job (tag pushes only) that stages the listing, then runs
`validate` followed by `upload`. Each leg **skips itself** when its credentials are absent rather
than failing — the secrets are optional by design, so a fork still gets a green run — and always
uploads the generated tree as an artifact, so what was sent to the store is reviewable after the
fact.

## Not done yet

- **Screenshots:** both stores take them per locale and per device class, and the dayscript
  walkthrough already captures exactly that shape (`build/day/screenshots/<target>/<variant>/`).
  Wiring those into `fastlane/screenshots/` is the obvious next step and is not built.
- **Review information** beyond notes and an email: no demo account fields, no phone number.
- **Age rating / content declarations**, which both stores require before a first submission and
  neither accepts from `supply`/`deliver` in full.
- **No listing has been uploaded.** Everything here is verified up to the point where a credential
  would be needed.
