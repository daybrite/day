---
title: App Store Submissions
description: Prepare store listings, configure signing credentials, and submit iOS, Mac App Store and Android releases through GitHub Actions.
order: 33
section: How-to guides
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

This page covers iOS, Mac App Store and Android submissions through your own accounts. For
desktop downloads, see [Packaging & distribution](/docs/how-to/packaging). For the build workflow, see
[GitHub Actions](/docs/how-to/github-actions).

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

[storefront.android-material.google-play-store.screenshots]
default = ["home", "editor", "search"]

[storefront.macos-appkit.mac-app-store.submission-info]
apple-category = "PRODUCTIVITY"

[storefront.macos-appkit.mac-app-store.screenshots]
default = ["home", "editor", "search"]
```

The Mac App Store is opt-in: an app publishes there only when its listing has a
`[storefront.macos-appkit.mac-app-store.*]` table, as above.

Screenshot names refer to captures in your [dayscript tests](/docs/how-to/dayscript). Run the tests on
the required phone and tablet profiles, then check the listing and captures:

```sh
day lint
day store screenshots build/day/screenshots/gallery.json
```

The second command needs the `gallery.json` produced by `day screenshot index` or downloaded
from CI. It checks screenshot sizes and locale/device coverage. These checks catch submission
errors; they do not guarantee store approval. See the [store listing reference](/docs/how-to/store)
for the schema and screenshot selection rules.

## Create the store records

Keep each app identifier consistent between `Day.toml`, the signing configuration and its store
record. Do not change an identifier when releasing an update.

| Store | One-time setup |
| --- | --- |
| App Store Connect | [Create the app record](https://developer.apple.com/help/app-store-connect/create-an-app-record/add-a-new-app/) for the iOS bundle ID. Configure privacy, age rating, export compliance and availability. Create an App Store Connect API key with access to the app, an Apple distribution certificate, and an App Store provisioning profile. |
| Mac App Store | Add the macOS platform to the same app record, so one bundle ID covers iPhone, iPad and Mac. Create a Mac Installer Distribution certificate and a Mac App Store Connect provisioning profile for the bundle ID; the Apple Distribution certificate signs both platforms. |
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
| Mac App Store signing | `DAY_MAS_CERT_P12`, `DAY_MAS_CERT_PASSWORD`, `DAY_MAS_PROFILE_B64` — one base64-encoded `.p12` holding both the Apple Distribution and the Mac Installer Distribution identities, its password, and the base64-encoded Mac App Store profile (optional: without it, the workflow has Apple issue one through the API key and `DAY_APPLE_TEAM`) |
| HarmonyOS signing | `DAY_OHOS_KEYSTORE_B64`, `DAY_OHOS_CERT_B64`, `DAY_OHOS_PROFILE_B64`, `DAY_OHOS_KEY_ALIAS`, `DAY_OHOS_KS_PASS`, `DAY_OHOS_KEY_PASS` — base64-encoded keystore, certificate and profile, the key alias and the two passwords |

The workflow signs after it builds, and only where it can. The job that compiles the app and
runs its walkthroughs never sees a key. On a tag, it packs unsigned each store package whose
platform has its signing secrets set, and a separate `sign` job, which checks out no code, signs
that package with `day sign apply` and replaces the artifact; the macOS app is signed and
notarized the same way from the environment the `signing-environment` input names. The Mac App
Store upload job signs its own `.pkg` from the same unsigned app with the `DAY_MAS_*` secrets. A platform
without secrets packs as on any branch: an unsigned iOS package, a development-signed Android
one. The signing secrets therefore need no `[signing]` table in `Day.toml`, though one still
serves [local release packs](/docs/how-to/packaging#signing-configuration). An API key authorizes
uploads; it does not replace the signing certificate or upload key. An upload switched on
without its signing secrets is refused when the workflow decides the uploads, naming the secrets
to set. Unsigned iOS packages and Android packages signed with the development key cannot be
uploaded to these stores.

## Enable store uploads

Add these inputs to the `app` job in the [CI workflow](/docs/how-to/github-actions#the-workflow):

```yaml
with:
  targets: ios-uikit, macos-appkit, android-material
  scripts: auto
  locales: all
  themes: light dark
  upload-ios: "true"
  upload-macos: "true"
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
| `mac upload`, `mac submit`, `mac release` | The same three for the Mac App Store, over the signed `.pkg`. |
| `android upload` | Uploads to the internal track as a draft. |
| `android release` | Uploads a completed production release, subject to Google Play review and publishing settings. |

To submit for review on each release tag, add:

```yaml
  ios-upload-lane: ios release
  macos-upload-lane: mac release
  play-upload-lane: android release
```

With `store-screenshots: true`, uploads replace the store screenshots with the captures selected
by the listing. Its default is `false`, which retains the existing screenshots.

## Release and update

Update `version` in `Cargo.toml`, increment `[app] build` in `Day.toml`, and revise the release
notes. Account for platform or [flavor](/docs/how-to/flavors) overrides. Commit those changes, then
[push a version tag](/docs/how-to/github-actions#create-a-release). Inspect the workflow's upload jobs
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
[Uploading](/docs/how-to/store#uploading).

A project with its own `fastlane/Fastfile` uses those lanes instead of generated ones. It must
handle metadata, screenshots and submission policy itself. The generated iOS submission lanes
assume no advertising identifier and exempt encryption; use custom lanes if those declarations
do not describe your app.

## The Mac App Store

The Mac App Store takes a `.pkg` rather than the `.dmg` you offer as a download
([Packaging](/docs/how-to/packaging#the-mac-app-store)). Try the first upload from your Mac before a
release tag does it:

1. In the Apple Developer portal, create a **Mac Installer Distribution** certificate and a
   **Mac App Store Connect** provisioning profile for the app's bundle ID, using your Apple
   Distribution certificate. Install the certificate and download the profile.
2. In App Store Connect, add the macOS platform to the app record.
3. Set the signing variables `[signing.macos.app-store]` reads, then pack, stage and validate:

   ```sh
   export DAY_MAS_IDENTITY="Apple Distribution: Example Inc (ABCDE12345)"
   export DAY_MAS_INSTALLER_IDENTITY="3rd Party Mac Developer Installer: Example Inc (ABCDE12345)"
   export DAY_MAS_PROFILE="$HOME/Downloads/Example_Mac_App_Store.provisionprofile"
   day pack -t macos-appkit --formats pkg
   day store stage -t macos-appkit
   xcrun altool --validate-app build/day/dist/<stem>-macos-appkit.pkg \
     --apiKey <key id> --apiIssuer <issuer id>
   ```

   `altool` reads the API key from `~/.appstoreconnect/private_keys/AuthKey_<key id>.p8`.

4. Upload with the generated lane, from `build/day/store/macos-appkit/`:

   ```sh
   DAY_ASC_KEY_ID=… DAY_ASC_ISSUER=… DAY_ASC_KEY=/path/to/AuthKey.p8 fastlane mac upload
   ```

   Apple's Transporter app also takes the `.pkg` by drag and drop.

The build appears under the record's macOS platform once App Store Connect has processed it.
