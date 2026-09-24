---
title: Store submission
description: "Release a Day app to the App Store and Google Play: the listing, the first upload by hand, then updates through CI."
order: 33
section: Build & ship
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

A Day app reaches the App Store and Google Play from one file, `store/storefront.toml`, and one
command, `day store stage`, which writes the fastlane project each store expects. The first
release of an app takes a few steps in each store's console that no tool can do for you; every
release after that can be a tag push. This guide walks the first release by hand, then hands the
rest to the shared CI workflow.

## 1. Write the listing

`day new app` scaffolds `store/storefront.toml` for any app with a mobile target, and `day store
init` adds one to an app that lacks it. Fill in the text under `[storefront.metadata]`, the
default locale's, and a table per other locale for what differs:

```toml
[storefront.submission-info]
copyright = "2026 Example"
contact-email = "support@example.com"
review-notes = "Every screen is reachable from the first page; no account is needed."

[storefront.ios-uikit.apple-app-store.submission-info]
apple-category = "PRODUCTIVITY"

[storefront.metadata]
name = "Example"
subtitle = "Notes that stay on your device"
short = "Notes, lists and reminders that never leave your phone."
description = """
Example keeps your notes on your device …
"""
keywords = ["notes", "lists", "offline"]
release-notes = "First release."
privacy-url = "https://example.com/privacy"
support-url = "https://example.com/support"

[storefront.metadata.fr]
subtitle = "Des notes qui restent sur votre appareil"
short = "Notes, listes et rappels qui ne quittent jamais votre téléphone."
description = """…"""
keywords = ["notes", "listes", "hors ligne"]
release-notes = "Première version."
```

A store's own wording goes under its table (`[storefront.ios-uikit.apple-app-store.metadata]`),
a long text can live in a file (`description-ref = "store/text/description.txt"`), and a locale
can have a file of its own (`store/storefront.fr.toml`). The full schema, the fallback order and
the length limits are in [Store listings](/docs/internal/store).

Then the screenshots. A dayscript walkthrough captures every screen; the listing names the ones
each store shows, per device kind, in order:

```toml
[storefront.ios-uikit.screenshots]                     # the website's iOS page, the stores' fallback
default = ["home", "editor", "search"]
ipad = ["home", "editor", "search", "sidebar"]

[storefront.ios-uikit.apple-app-store.screenshots]
iphone = ["home", { name = "editor", theme = "dark" }, "search"]

[storefront.android-mdc.google-play-store.screenshots]
default = ["home", "editor", "search"]
```

Run `day lint` until it is clean. It holds every field to each store's limit, names the fields a
store's record cannot do without, checks every locale the app is translated into has a listing,
and refuses a screenshot name no dayscript captures. `day store stage` refuses a listing with any
lint error, so nothing below runs on a listing a store would reject.

## 2. Set up the store records

Both stores want the record to exist before anything is uploaded, and both want it created in a
browser.

**App Store Connect.** Create the app under the bundle id `Day.toml` names (`[app] id`, or
`[app.ios] id`). Under Users and Access, Integrations, create an App Store Connect API key with
the Developer role and keep its key id, issuer id and the `.p8` file: the generated lanes take
these as `DAY_ASC_KEY_ID`, `DAY_ASC_ISSUER` and `DAY_ASC_KEY` (the path to the `.p8`), so no
Apple ID password is ever needed. Answer the questions the console asks once per app: age rating,
export compliance, the privacy nutrition labels, and the primary category, which the listing's
`apple-category` also writes.

**Google Play Console.** Create the app under the package name `Day.toml` names (`[app.android]
id` when the bundle id has a hyphen). Create a service account with the Release Manager role
under Setup, API access, and download its JSON key: the lanes take its path as
`SUPPLY_JSON_KEY`. Fill in the store settings the API cannot write: the category, the content
rating questionnaire, the data safety form, and the target audience.

Google Play will not take an app's first bundle through the API. Build it, and upload it once by
hand in the console, under the internal testing track; every bundle after that can come from
`supply`.

## 3. Build and validate

```sh
day pack -p ios-uikit --profile release
day pack -p android-mdc --profile release
```

`day pack` signs the iOS build with the App Store profile installed on the machine (see
[Packaging](/docs/packaging) for the certificate and profile), and the Android bundle with the
keystore `Day.toml` names. Both packs also stage the listing under `build/day/store/<target>/`,
and `day store stage` does the same on its own:

```sh
day store stage                                   # the text, per store
day store stage --screenshots build/day/screenshots/gallery.json   # with the walkthrough's captures
```

The second form reads the gallery index a walkthrough run wrote (`day launch --script
dayscript/walkthrough.yaml --themes light,dark --locales en,fr`, then `day screenshot index`),
checks every capture against the store's sizes and coverage, and places them where fastlane
uploads them. A set a store would refuse is refused here, with the device profile to capture on
named in the message.

Each staged tree is a fastlane project. Validate before anything goes up:

```sh
export DAY_ASC_KEY_ID=… DAY_ASC_ISSUER=… DAY_ASC_KEY=/path/to/AuthKey.p8
cd build/day/store/ios-uikit && fastlane ios validate

export SUPPLY_JSON_KEY=/path/to/play-service-account.json
cd build/day/store/android-mdc && fastlane android validate
```

`validate` asks each store to check the build and the listing and rolls back; it reports the
same refusals the upload would, from the store itself.

## 4. Upload the first release

```sh
cd build/day/store/ios-uikit && fastlane ios upload
cd build/day/store/android-mdc && fastlane android upload
```

`ios upload` sends the build and the listing to App Store Connect and leaves the version in
Prepare for Submission; `ios release` also submits it for review, and `ios submit` submits a
build that is already there. `android upload` sends the bundle to the internal track as a draft;
`android release` sends it to production as a completed release, which is Play's submission, and
the rollout starts when Google's review passes. The Fastfile finds the packed artifact under
`build/day/dist/` by glob; `DAY_IPA` and `DAY_AAB` name one outright, which is how CI hands each
lane the file it downloaded.

Once the first version is live, put its ids in `Day.toml` so the website links the listings:

```toml
[store]
apple-app-id = "6802801331"
google-play-id = "com.example.app"
```

## 5. Every release after that: CI

The shared workflow (`daybrite/actions`, `dayapp.yml`) builds, packs and, on a semantic-version
tag, uploads. Give the repository the store secrets and it does on a tag what you just did by
hand:

| secret | what it is |
|---|---|
| `DAY_ASC_KEY_ID`, `DAY_ASC_ISSUER`, `DAY_ASC_KEY_B64` | the App Store Connect API key: its id, its issuer id, and the `.p8` base64-encoded |
| `DAY_APPLE_CERT_P12`, `DAY_APPLE_CERT_PASSWORD`, `DAY_IOS_PROFILE_B64` | the distribution certificate and the App Store provisioning profile, base64-encoded; the build packs unsigned and a `sign-ios` job signs it |
| `DAY_PLAY_JSON_KEY` | the Play service account's JSON key |

With the secrets set and a `store/storefront.toml` listing in the repository, the `upload-ios`
and `upload-play` inputs left empty turn the uploads on by themselves; `store-screenshots: true`
replaces each store's screenshots on every upload with the tag's own captures, checked against
the stores' rules first:

```yaml
# .github/workflows/ci.yml
jobs:
  app:
    uses: daybrite/actions/.github/workflows/dayapp.yml@main
    with:
      store-screenshots: true
      # ios-upload-lane: ios release    # submit for review too; the default `ios upload` does not
      # play-upload-lane: android release
    secrets: inherit
```

Then a release is:

```sh
# raise `version` in Cargo.toml and `[app] build` in Day.toml, then
git tag v1.0.1 && git push --tags
```

The workflow runs the walkthrough on every target, packs, signs, attaches the packages, the
screenshot bundle, `gallery.json` and `storefront.json` (the listing resolved, with the rules it
was held to) to the GitHub release, and runs `ios upload` and `android upload` with the tag's
listing and captures. Switch the lanes to `ios release` and `android release` once you trust the
walkthrough to produce the screenshots you want to submit, and the tag is the submission.

An app with its own `fastlane/Fastfile` keeps it: the workflow then hands the lane the packed
artifact and nothing else, and the Fastfile owns the store policy.

## What can go wrong

- **"Your account has reached the maximum number of certificates."** An earlier CI setup
  archived with automatic signing, and each runner minted a development certificate. Revoke the
  "Created via API" certificates in the developer portal; the workflow now packs unsigned and
  signs with the certificate you give it.
- **Play refuses the screenshots: "Dimensions out of range".** Its API takes 1080 to 7680 px a
  side and at most 2.3:1; a CI tablet past three million pixels captures halved. Use the `Nexus 7
  2013` profile with `density=240` for the tablet set, or leave the tablet set out.
- **App Store Connect refuses a locale.** A locale the listing carries has no screenshots in it,
  or the store does not know the tag. `day store screenshots gallery.json` names the locale and
  the device kind before an upload does.
- **The listing uploaded a `TODO`.** `day store stage` refuses placeholder text; a Fastfile of
  your own does not. Run `day lint` before a tag either way.
