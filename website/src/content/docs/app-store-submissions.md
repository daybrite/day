---
title: App Store Submissions
description: Prepare store listings, configure signing credentials, and submit iOS and Android releases through GitHub Actions.
order: 33
section: Build & ship
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

Desktop apps can be distributed as downloads from your website or GitHub releases. Mobile apps
are usually distributed through Apple's App Store or Google Play. The shared
[`daybrite/actions`](https://github.com/daybrite/actions) GitHub workflows can build, test,
sign and upload your app when you tag a release. Store account setup and review still take
place in each store's console.

> [!TIP] Publish through App Fair
> Publishing under your own name requires an [Apple Developer Program membership](https://developer.apple.com/programs/enroll/)
> for the App Store and a [Google Play developer account](https://developers.google.com/android-publisher/getting_started)
> for Google Play. Open-source apps submitted through the App Fair process can distribute through
> the Apple App Store and Google Play Store without requiring any developer account. Follow the
> [App Fair submission guide](https://appfair.org/docs/getting-started/) for that process.

This page covers iOS and Android submissions through your own accounts. For desktop downloads,
see [Packaging & distribution](/docs/packaging). For the build workflow, see
[GitHub Actions](/docs/github-actions).

## Prepare the listing

Edit `store/storefront.toml`. If the project has no listing, create it with `day store init`.
Provide the app name, descriptions, release notes, privacy and support URLs, review contact,
and translations for the locales you publish. Store-specific fields belong under that store's
table; for example:

```toml
[storefront.ios-uikit.apple-app-store.submission-info]
apple-category = "PRODUCTIVITY"

[storefront.ios-uikit.apple-app-store.screenshots]
iphone = ["home", "editor", "search"]
ipad = ["home", "editor", "search"]

[storefront.android-mdc.google-play-store.screenshots]
default = ["home", "editor", "search"]
```

Screenshot names refer to captures in your [dayscript tests](/docs/dayscript). Run the tests on
the required phone and tablet profiles, then check the listing and captures:

```sh
day lint
day store screenshots build/day/screenshots/gallery.json
```

The second command needs the `gallery.json` produced by `day screenshot index` or downloaded
from CI. It checks screenshot sizes and locale/device coverage. These checks catch submission
errors; they do not guarantee store approval. See the [store listing reference](/docs/internal/store)
for the schema and screenshot selection rules.

## Create the store records

Keep each app identifier consistent between `Day.toml`, the signing configuration and its store
record. Do not change an identifier when releasing an update.

| Store | One-time setup |
| --- | --- |
| App Store Connect | [Create the app record](https://developer.apple.com/help/app-store-connect/create-an-app-record/add-a-new-app/) for the iOS bundle ID. Configure privacy, age rating, export compliance and availability. Create an App Store Connect API key with access to the app, an Apple distribution certificate, and an App Store provisioning profile. |
| Google Play | Create the app in Play Console and complete its content rating, data safety and audience settings. [Enable the publishing API and grant a service account access](https://developers.google.com/android-publisher/getting_started). Configure Play App Signing and retain the upload keystore. |

Upload the first Android bundle through Play Console before using the API. This is a
[fastlane supply prerequisite](https://docs.fastlane.tools/actions/supply/#setup).
Complete any testing or account-verification requirements shown in the console before requesting
production access.

## Configure CI credentials

Add the following repository secrets under **Settings → Secrets and variables → Actions**.
Configure only the stores you use.

| Purpose | Secrets |
| --- | --- |
| Apple team | `DAY_APPLE_TEAM` — team ID |
| App Store Connect API | `DAY_ASC_KEY_ID`, `DAY_ASC_ISSUER`, `DAY_ASC_KEY_B64` — key ID, issuer ID and base64-encoded `.p8` key |
| iOS signing | `DAY_APPLE_CERT_P12`, `DAY_APPLE_CERT_PASSWORD`, `DAY_IOS_PROFILE_B64` — base64-encoded distribution certificate, its password and base64-encoded provisioning profile |
| Android signing | `DAY_ANDROID_KEYSTORE_B64`, `DAY_ANDROID_KEY_ALIAS`, `DAY_KS_PASS`, `DAY_KEY_PASS` — base64-encoded upload keystore, alias, keystore password and key password |
| Google Play API | `DAY_PLAY_JSON_KEY` — service-account JSON contents |
| HarmonyOS signing | `DAY_OHOS_KEYSTORE_B64`, `DAY_OHOS_CERT_B64`, `DAY_OHOS_PROFILE_B64`, `DAY_OHOS_KEY_ALIAS`, `DAY_OHOS_KS_PASS`, `DAY_OHOS_KEY_PASS` — base64-encoded keystore, certificate and profile, the key alias and the two passwords |

The workflow signs after it builds, and only where it can. The job that compiles the app and
runs its walkthroughs never sees a key. On a tag, it packs unsigned each store package whose
platform has its signing secrets set, and a separate `sign` job, which checks out no code, signs
that package with `day sign apply` and replaces the artifact; the macOS app is signed and
notarized the same way from the environment the `signing-environment` input names. A platform
without secrets packs as on any branch: an unsigned iOS package, a development-signed Android
one. The signing secrets therefore need no `[signing]` table in `Day.toml`, though one still
serves [local release packs](/docs/packaging#signing-configuration). An API key authorizes
uploads; it does not replace the signing certificate or upload key. An upload switched on
without its signing secrets is refused when the workflow decides the uploads, naming the secrets
to set. Unsigned iOS packages and Android packages signed with the development key cannot be
uploaded to these stores.

## Enable store uploads

Add these inputs to the `app` job in the [CI workflow](/docs/github-actions#add-the-workflow):

```yaml
with:
  targets: ios-uikit, android-mdc
  scripts: auto
  locales: all
  themes: light dark
  upload-ios: "true"
  upload-play: "true"
  store-screenshots: true
```

Keep the job's `uses` and `secrets: inherit` entries. Include any desktop or web targets your
existing workflow builds. Remove an upload input, or set it to `"false"`, for a store you do
not use. When omitted, the upload inputs auto-enable only if the listing or matching Fastfile
and the store's API credentials are present.

On a version tag, CI signs the packages and runs the generated fastlane lanes. The default lanes
upload without publishing to production:

| Lane | Result |
| --- | --- |
| `ios upload` | Uploads the build and listing to App Store Connect without submitting for review. |
| `ios submit` | Submits a previously uploaded build for review; does not upload the binary again. |
| `ios release` | Uploads and submits for review. An approved version releases automatically unless the listing sets `apple-release = "manual"`. |
| `android upload` | Uploads to the internal track as a draft. |
| `android release` | Uploads a completed production release, subject to Google Play review and publishing settings. |

To submit for review on each release tag, add:

```yaml
  ios-upload-lane: ios release
  play-upload-lane: android release
```

With `store-screenshots: true`, uploads replace the store screenshots with the captures selected
by the listing. Its default is `false`, which retains the existing screenshots.

## Release and update

Update `version` in `Cargo.toml`, increment `[app] build` in `Day.toml`, and revise the release
notes. Account for platform or [flavor](/docs/flavors) overrides. Commit those changes, then
[push a version tag](/docs/github-actions#create-a-release). Inspect the workflow's upload jobs
and the status in each store console.

A GitHub release and a store release are separate. GitHub's draft or pre-release setting does
not disable store uploads. Set the workflow's `upload-*` inputs explicitly when staging a build
that should not reach the stores.

After publication, add the listing IDs to `Day.toml` so the generated website links to them:

```toml
[store]
apple-app-id = "6802801331"
google-play-id = "com.example.app"
```

## Local uploads and custom lanes

`day pack` builds the packages; `day store stage` writes fastlane projects under
`build/day/store/<target>/`. For local validation and upload commands, see
[Uploading](/docs/internal/store#uploading).

A project with its own `fastlane/Fastfile` uses those lanes instead of generated ones. It must
handle metadata, screenshots and submission policy itself. The generated iOS submission lanes
assume no advertising identifier and exempt encryption; use custom lanes if those declarations
do not describe your app.

Mac App Store uploads also require a custom lane. The workflow supplies macOS build products;
the lane must produce the package signed for Mac App Store distribution. See
[store-upload inputs and limitations](https://github.com/daybrite/actions#store-uploads).
