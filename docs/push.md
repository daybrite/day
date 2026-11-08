---
title: "Push notifications"
description: "Register devices and receive remote notifications through APNs and Firebase Cloud Messaging."
navTitle: "Push notifications"
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Push notifications

Push notifications let a server alert a device about new information, even when the app is not
in the foreground. The app registers with a provider, shares its device token with the server,
and handles incoming messages or notification taps.

## Add to your app

```sh
cargo add 'day-part-push@0.*'
```

Use the [Day CLI](/docs/reference/cli) to stage the native integrations and permission metadata.
Configure the app's Apple capabilities or Firebase client resources as described below. Keep
sender credentials on the server.

[API reference](https://docs.rs/day-part-push/latest/day_part_push/) ·
[Source and demo](https://github.com/daybrite/day-part-push).

## Choose providers

| Cargo feature | Provider crate | Platform |
| --- | --- | --- |
| `ios-apns` (default) | `day-part-push-ios-apns` | iOS / UIKit |
| `android-fcm` (default) | `day-part-push-android-fcm` | Android / Material with Google Play services |
| `relay` | Optional client for the demo relay | A configured HTTPS relay |

Each native provider is restricted to its target OS. Desktop, web, and HarmonyOS do not have
native providers in this package.

For an Android application that excludes Google dependencies:

```toml
[dependencies]
day-part-push = { version = "0.*", default-features = false, features = ["ios-apns"] }
```

This excludes the FCM provider, Firebase dependency, messaging service, and Java bridge from
Android. Android push remains unsupported until another provider is added. Cargo features are
additive: another dependency enabling `android-fcm` enables it for the app. Inspect the final
APK as well as the Cargo graph. The demo's `fdroid` flavor demonstrates dependency exclusion;
a microG-specific provider is not implemented.

## Register and handle events

Subscribe with [`watch`](https://docs.rs/day-part-push/latest/day_part_push/fn.watch.html) before
requesting permission and registering. Keep its returned watch while events are needed.

```rust
use day_part_push::{self as push, Event};

async fn register() -> Result<push::Watch, push::Error> {
    let watch = push::watch(|event| match event {
        Event::Token { token } => {
            // Send this token to your app server over HTTPS, including rotations.
            let _ = token;
        }
        Event::Message { payload, opened } => {
            // Hand data to the UI through a Day Setter; opened identifies a tap.
            let _ = (payload, opened);
        }
        Event::Error { message } => { let _ = message; }
    })?;
    push::request_permission().await?;
    push::register().await?;
    Ok(watch)
}
```

[`register`](https://docs.rs/day-part-push/latest/day_part_push/fn.register.html) starts native
registration. Token events carry the result and later rotations; a device token can change
without the user signing out. Treat tokens as installation-specific delivery addresses, not
account identities.

Callbacks run on native delivery threads and must return promptly. Send values through a
[Setter](/docs/explanation/state), then format [localized text](/docs/reference/localization) on
the UI thread. Events are serialized for each subscriber. A new subscriber receives the last
token and up to 32 recent messages or errors, including taps received before its page opened.
Dropping the watch unsubscribes; an executing callback may finish.

[`unregister`](https://docs.rs/day-part-push/latest/day_part_push/fn.unregister.html) stops native
registration and clears the cached token. Remove the installation from your server as well.

## Native configuration

| Platform | Setup and delivery behavior |
| --- | --- |
| iOS / UIKit | Enable Push Notifications for the app identifier and provision it with that capability. Include `aps-environment` in signing entitlements. Development tokens use the APNs sandbox; distribution tokens use production. |
| Android / Material | Register the exact application ID in Firebase and provide its client resources. Foreground messages reach the app; background notification payloads are displayed by the OS and returned when tapped. |

On iOS, the provider shares Day's notification delegate with [local notifications](/docs/reference/notify).
Silent-push background execution and notification service extensions are outside this API.
See Apple's [remote notification registration guide](https://developer.apple.com/documentation/usernotifications/registering-your-app-with-apns).

On Android, supply `google_app_id`, `gcm_defaultSenderId`, `google_api_key`, and `project_id` in
Android XML resources under `platform/android/res/values/`. The provider reads these with
Firebase's resource-based configuration; the Google Services Gradle plugin is not required.
These values are client configuration, not the service-account private key. Automatic messaging
initialization stays disabled until registration. See Firebase's
[Android client setup](https://firebase.google.com/docs/cloud-messaging/android/client).

OS background limits, permissions, connectivity, and force-stop affect delivery. Provider
acceptance of a send does not establish that the device received or displayed it.

## Demos and automated checks

Push Lab exposes registration, token changes, and incoming events. Configure its identifier,
`dev.daybrite.push.demo`, separately from Day-Showcase. The portable walkthrough checks UI
controls without requiring a provider account.

The repository also contains a Cloudflare Worker relay for Day-Showcase. It issues short-lived
enrollment codes and per-device credentials, stores registrations in D1, and sends through
APNs or FCM. Sender keys stay in Worker secrets; the app stores its device credential in the
[keychain](/docs/reference/keychain). See the
[relay guide](https://github.com/daybrite/day-part-push/blob/main/relay/README.md).

CI tests the Rust workspace, the relay with synthetic credentials, and both Android flavors.
An APK audit checks that the Google-free build contains neither Firebase nor Google Play
services. On-device delivery tests remain separate from those checks.
