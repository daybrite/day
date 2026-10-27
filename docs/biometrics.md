---
title: "Biometrics"
description: "The platform's own authentication prompt via day-part-biometrics: Touch ID, Face ID, Android's biometric prompt, Windows Hello and HarmonyOS user authentication, with the device credential as the way in."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Biometrics

An app that guards something (a vault, a payment, a settings page) asks the device to verify the
person before opening it. `day-part-biometrics` shows the platform's own authentication dialog
(Touch ID, Face ID, Android's biometric prompt, Windows Hello, HarmonyOS user authentication) with a
reason in the person's language, lets the device credential pass where the app allows it, and
reports one answer: verified, or why not.

To sign in to an account service or request a passkey, use
[sign-in and passkeys](/docs/reference/auth). A local biometric check does not establish a
server session.

## Add to your app

Add [`day-part-biometrics`](https://github.com/daybrite/day/tree/main/crates/day-part-biometrics) to your app’s Cargo dependencies, using the same Day version as the rest of your app:

```toml
[dependencies]
day-part-biometrics = "0.6"
```

The implementation is selected by the compilation target; no toolkit feature flags are needed. The
crate declares that it uses the portable `biometrics` permission, so `day build` asks the app to
declare it too ([docs/permissions](/docs/reference/permissions#what-the-declaration-pipeline-generates)): `USE_BIOMETRIC` on Android and `ACCESS_BIOMETRIC` on
HarmonyOS are granted at install with no dialog, and iOS shows the reason the first time Face ID is
used.

```toml
[permissions]
biometrics = true   # the reason is the catalog's `permission_biometrics` message
```

| Platforms / toolkits | Availability |
| --- | --- |
| iOS, macOS | Touch ID, Face ID, Optic ID, and the passcode or account password when allowed. |
| Android | The system's biometric prompt on API 28 and later, with the device credential when allowed; `Unsupported` below. |
| Windows | Windows Hello (PIN, face, fingerprint), owned by the active window. Hello decides itself what it asks for. |
| HarmonyOS | The User Authentication Kit's dialog (face, fingerprint, and the PIN when allowed); a passed prompt carries the kit's token. |
| Linux (GTK / Qt), Web | No API an application may use: `Unsupported`. |

## Authoring

```rust
use day_part_biometrics::{Availability, BiometricError, Prompt};

let ready = day_part_biometrics::availability() == Availability::Available;

day::task(async move {
    let prompt = Prompt::new(res::str::unlock_reason().format())
        .fallback_title(res::str::use_passcode().format());
    match day_part_biometrics::authenticate_future(prompt).await {
        Ok(verified) => open_vault(verified.method),
        Err(BiometricError::UserCancelled) => {}
        Err(e) => status.set(e.to_string()),
    }
});
```

[`availability()`](https://daybrite.dev/api/day_part_biometrics/fn.availability.html) asks whether a prompt can succeed now ([`Availability`](https://daybrite.dev/api/day_part_biometrics/enum.Availability.html): `Available`, `NotEnrolled`,
`NoHardware`, `PasscodeNotSet`, `Lockout`, `DisabledByPolicy`, `Busy`, `Unsupported`), and
[`biometry()`](https://daybrite.dev/api/day_part_biometrics/fn.biometry.html) which sensor the device offers ([`Biometry`](https://daybrite.dev/api/day_part_biometrics/enum.Biometry.html): `Fingerprint`, `Face`, `Iris`, `Unknown`, `None`).
Both are probes of the moment; [`support()`](https://daybrite.dev/api/day_part_biometrics/fn.support.html) is the fixed fact of whether this target has an API at
all.

A [`Prompt`](https://daybrite.dev/api/day_part_biometrics/struct.Prompt.html) carries the reason the dialog shows (required; localize it), an optional
[`fallback_title`](https://daybrite.dev/api/day_part_biometrics/struct.Prompt.html#method.fallback_title) for the button that leads away from the biometric, whether the
[`device_credential`](https://daybrite.dev/api/day_part_biometrics/struct.Prompt.html#method.device_credential) (passcode, PIN, pattern, password) may pass the prompt (it may, by default),
and a [`challenge`](https://daybrite.dev/api/day_part_biometrics/struct.Prompt.html#method.challenge) HarmonyOS binds its token to. [`authenticate(prompt, callback)`](https://daybrite.dev/api/day_part_biometrics/fn.authenticate.html) and
[`authenticate_future(prompt)`](https://daybrite.dev/api/day_part_biometrics/fn.authenticate_future.html) show the dialog and answer exactly once with a [`Verified`](https://daybrite.dev/api/day_part_biometrics/struct.Verified.html)
(how the person passed, and the platform token where one exists) or a [`BiometricError`](https://daybrite.dev/api/day_part_biometrics/enum.BiometricError.html).
[`cancel()`](https://daybrite.dev/api/day_part_biometrics/fn.cancel.html) dismisses the dialog on screen.

### One prompt at a time

Every platform shows one authentication dialog at a time. A second [`authenticate`](https://daybrite.dev/api/day_part_biometrics/fn.authenticate.html) while one is on
screen fails at once with `BiometricError::Busy`. A prompt whose reason is empty fails with
`InvalidPrompt` before reaching the platform (Apple aborts on an empty reason; the others show a
blank dialog).

### What a pass proves

A `Verified` answer means the OS showed its dialog and the person passed it. It is a fact about the
moment, not a key: nothing is unlocked by it except what the app does next. Where a stored secret
should be bound to the same check, use [`day-part-keychain`](/docs/reference/keychain)'s `Protection::UserPresence`, whose
read performs this prompt itself on the platforms that need one and lets the keychain or keystore
do the binding.

### Errors

[`BiometricError`](https://daybrite.dev/api/day_part_biometrics/enum.BiometricError.html) distinguishes what the app should do next: `UserCancelled` and `SystemCancelled`
(nothing), `UserFallback` (the person chose the fallback button of a biometric-only prompt: show the
app's own alternative), `Failed` (the attempt did not match and the dialog closed), `Lockout` and
`LockoutPermanent` (the device credential clears them), `NotEnrolled`, `NoHardware`,
`PasscodeNotSet`, `Busy`, `DisabledByPolicy`, `NotInteractive` (no window to attach the dialog to),
`InvalidPrompt`, `Unsupported`, and `Platform { code, message }` for the rest.

### Threads

Completions run where the platform delivers them (a LocalAuthentication reply queue, an Android
executor, a WinRT thread-pool thread, the HarmonyOS JS thread), which is why the callback is
`Send`. [`authenticate_future`](https://daybrite.dev/api/day_part_biometrics/fn.authenticate_future.html) awaited under [`day::task`](https://daybrite.dev/api/day/fn.task.html) resumes on the UI thread, so the code after
the `.await` writes signals directly ([docs/async](/docs/explanation/async)). Nothing here posts to a main loop, so the
crate works in a plain `main` and under `cargo test`.

## Platform implementations

Platform guidance: [Apple: Local Authentication](https://developer.apple.com/documentation/localauthentication); [Android: show a biometric authentication dialog](https://developer.android.com/identity/sign-in/biometric-auth); [Windows Hello](https://learn.microsoft.com/en-us/windows/apps/develop/security/windows-hello); [HarmonyOS: User Authentication Kit](https://github.com/openharmony/docs/blob/master/en/application-dev/security/UserAuthenticationKit/user-authentication-overview.md).

| OS | API | dependency |
|---|---|---|
| iOS, macOS | [`LAContext`](https://developer.apple.com/documentation/localauthentication/lacontext): [`canEvaluatePolicy(_:error:)`](https://developer.apple.com/documentation/localauthentication/lacontext/canevaluatepolicy(_:error:)) and [`biometryType`](https://developer.apple.com/documentation/localauthentication/lacontext/biometrytype) for the probes, [`evaluatePolicy(_:localizedReason:reply:)`](https://developer.apple.com/documentation/localauthentication/lacontext/evaluatepolicy(_:localizedreason:reply:)) for the dialog, [`invalidate()`](https://developer.apple.com/documentation/localauthentication/lacontext/invalidate()) to cancel | `objc2`, `block2`, `objc2-foundation` |
| Android | [`BiometricManager.canAuthenticate`](https://developer.android.com/reference/android/hardware/biometrics/BiometricManager#canAuthenticate(int)) for the probe, [`BiometricPrompt`](https://developer.android.com/reference/android/hardware/biometrics/BiometricPrompt) for the dialog, a [`CancellationSignal`](https://developer.android.com/reference/android/os/CancellationSignal) to cancel | a Java arm ([`day-bridge`](https://github.com/daybrite/day/blob/main/docs/bridge.md)) |
| Windows | [`UserConsentVerifier.CheckAvailabilityAsync`](https://learn.microsoft.com/en-us/uwp/api/windows.security.credentials.ui.userconsentverifier.checkavailabilityasync) for the probe, [`IUserConsentVerifierInterop::RequestVerificationForWindowAsync`](https://learn.microsoft.com/en-us/windows/win32/api/userconsentverifierinterop/nf-userconsentverifierinterop-iuserconsentverifierinterop-requestverificationforwindowasync) for the dialog, owned by the active window | `windows` |
| HarmonyOS | [`userAuth.getAvailableStatus`](https://github.com/openharmony/docs/blob/master/en/application-dev/reference/apis-user-authentication-kit/js-apis-useriam-userauth.md#userauthgetavailablestatus9) for the probes, [`getUserAuthInstance(...).start()`](https://github.com/openharmony/docs/blob/master/en/application-dev/reference/apis-user-authentication-kit/js-apis-useriam-userauth.md#userauthgetuserauthinstance10) for the dialog, [`cancel()`](https://daybrite.dev/api/day_part_biometrics/fn.cancel.html) to cancel | an ArkTS arm ([`day-bridge`](https://github.com/daybrite/day/blob/main/docs/bridge.md)) |
| Linux, Web | — | none (`Unsupported`) |

## How each platform realizes the prompt

- **iOS and macOS**: a fresh `LAContext` per prompt (one that has evaluated once answers the next
  evaluation from its cache without showing anything), kept alive until the reply so [`cancel`](https://daybrite.dev/api/day_part_biometrics/fn.cancel.html) can
  invalidate it. `device_credential` selects the policy: `deviceOwnerAuthentication` falls back to
  the passcode or account password on its own; `deviceOwnerAuthenticationWithBiometrics` asks for
  a biometric only and reports the fallback button as `UserFallback`. [`availability`](https://daybrite.dev/api/day_part_biometrics/fn.availability.html) always asks
  about the biometric policy. A pass through the mixed policy reports `Method::Unknown`, because
  the reply does not say which credential passed.
- **Android**: the platform prompt in `android.hardware.biometrics`, which takes an application
  context and needs no library. On API 30 and later the prompt is built with
  `BIOMETRIC_STRONG`, plus `DEVICE_CREDENTIAL` when allowed; on API 28 and 29 the device
  credential is offered where the OS supports it, and a biometric-only prompt carries the
  fallback title as its negative button. Each miss is shown in the dialog and not reported until
  the dialog ends. Below API 28 there is no prompt an app may show without the deprecated
  fingerprint API and its own permission, so the crate reports `Unsupported` there.
- **Windows**: `CheckAvailabilityAsync` first, because a request on a machine with no Hello
  credential does not fail, it waits. The dialog is owned by the window handle captured on the
  calling thread (the active window of a Day action) and the verification is awaited on a worker
  thread with its own apartment, so the UI keeps pumping. Hello never says which credential
  passed, so a pass reports `Method::Unknown` and [`biometry()`](https://daybrite.dev/api/day_part_biometrics/fn.biometry.html) reports `Unknown` when anything is
  set up. The `device_credential` flag is ignored: Hello offers its PIN itself.
- **HarmonyOS**: the dialog lists the modalities it is given and refuses one the device lacks, so
  the arm probes face, fingerprint and (when allowed) the PIN with `getAvailableStatus` and lists
  only those with an executor, enrolled or not; a device with nothing enrolled then fails at
  `start()` with `NotEnrolled`. A passed prompt returns the kit's token (up to 1024 bytes), bound
  to the prompt's `challenge`, which a Universal Keystore key generated with user authentication
  accepts once; `day-part-keychain` uses this for its protected items. `ACCESS_BIOMETRIC` is
  required even for a PIN-only prompt.

## Extension integration

A headless crate with no UI Piece, like [`day-part-haptics`](/docs/reference/haptics). Its Android and HarmonyOS halves are
daybridge arms ([docs/bridge](https://github.com/daybrite/day/blob/main/docs/bridge.md)) in Java and ArkTS, and its `[package.metadata.day.permissions] uses`
line is what makes `day build` insist on the app's `biometrics` declaration ([docs/extending](https://github.com/daybrite/day/blob/main/docs/extending.md)): the
crate names the machine-facing half, the app writes the reason. The Day Showcase's Security page
shows the probes and the prompt beside the keychain's protected item.
