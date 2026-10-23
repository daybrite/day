---
title: "Keychain"
description: "Small secrets in the platform's own secure store via day-part-keychain: protection levels, reading an item that needs the person present, and what each platform stores."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Keychain

A keychain keeps the small secrets an app cannot put in its preferences: a session token, a
password, an API key. `day-part-keychain` stores one under a key of your choosing in the
platform's own secure store and reads it back, with a protection level that says when the item
may be read, including "only after the device verifies the person, every time".

## Add to your app

Defined in [`day-part-keychain`](https://github.com/daybrite/day/tree/main/parts/day-part-keychain). Add this to your app's `Cargo.toml`
`[dependencies]` table, using the same Day Git source as the rest of your app:

```toml
[dependencies]
day-part-keychain = { git = "https://github.com/daybrite/day.git" }
```

The implementation is selected by the compilation target; no toolkit feature flags or dependency
entry in `Day.toml` are needed. An app that stores items at `Protection::UserPresence` also
declares the `biometrics` permission ([docs/permissions](/docs/reference/permissions#what-the-declaration-pipeline-generates)), which carries the
sentence Face ID shows the first time it is used:

```toml
[permissions]
biometrics = true   # the reason is the catalog's `permission_biometrics` message
```

| Platforms / toolkits | Availability |
| --- | --- |
| iOS, macOS | The data protection keychain. An unsigned macOS development build falls back to the login keychain and says so ([`backend()`](https://daybrite.dev/api/day_part_keychain/fn.backend.html) returns `login-keychain`, [`support()`](https://daybrite.dev/api/day_part_keychain/fn.support.html) `Emulated`); user presence is unavailable there. |
| Android | Files in the app's no-backup directory, encrypted with keys the Android Keystore holds; user presence through the system's biometric prompt (API 28 and later). |
| Windows | Files under `%LOCALAPPDATA%`, protected with DPAPI for the signed-in account; user presence gated by Windows Hello (`Emulated`: the check is in front of the item, not bound to it). |
| Linux (GTK / Qt) | The Secret Service (GNOME Keyring, KDE's ksecretd, KeePassXC) through libsecret, loaded at run time. `Unsupported` on a session with none; no user-presence level. |
| HarmonyOS | Files in the app sandbox, encrypted with Universal Keystore keys; user presence through a key bound to user authentication. |
| Web | No store a page may rely on for a secret: `Unsupported`. |

[`backend()`](https://daybrite.dev/api/day_part_keychain/fn.backend.html) identifies the selected storage implementation.
It returns `secret-service` on Linux even when the session has no reachable service. Check
[`support()`](https://daybrite.dev/api/day_part_keychain/fn.support.html) for current availability.

## Authoring

```rust
use day_part_keychain::{KeychainError, Protection, Secret};

// Store. Never shows a dialog, at any level.
day_part_keychain::set("session", &Secret::from(token), Protection::WhenUnlocked)?;

// Read, synchronously. A `UserPresence` item answers `NeedsPresence` here.
let token = day_part_keychain::get("session")?;
println!("{}", token.as_str().unwrap_or("<binary>"));

// Read an item that needs the person: the OS shows its dialog with the reason.
day::task(async move {
    match day_part_keychain::get_future("vault-key", &res::str::unlock_vault_reason().format()).await {
        Ok(secret) => open_vault(secret),
        Err(KeychainError::Cancelled) => {}
        Err(e) => status.set(e.to_string()),
    }
});
```

[`set(key, secret, protection)`](https://daybrite.dev/api/day_part_keychain/fn.set.html) replaces any item under `key`, whatever its level. [`get(key)`](https://daybrite.dev/api/day_part_keychain/fn.get.html) is synchronous
and never shows a dialog. [`get_async(key, reason, callback)`](https://daybrite.dev/api/day_part_keychain/fn.get_async.html) and [`get_future(key, reason)`](https://daybrite.dev/api/day_part_keychain/fn.get_future.html) show the
device's verification dialog when the item needs it and resolve with the secret; an item at
another level resolves without a dialog. [`remove(key)`](https://daybrite.dev/api/day_part_keychain/fn.remove.html), [`contains(key)`](https://daybrite.dev/api/day_part_keychain/fn.contains.html) and [`entries()`](https://daybrite.dev/api/day_part_keychain/fn.entries.html)
(every key with its level, without the secrets) complete the API. A key is at most
[`MAX_KEY_LEN`](https://daybrite.dev/api/day_part_keychain/constant.MAX_KEY_LEN.html) bytes with no control characters; a secret at most [`MAX_SECRET_LEN`](https://daybrite.dev/api/day_part_keychain/constant.MAX_SECRET_LEN.html) (64 KiB).

[`Secret`](https://daybrite.dev/api/day_part_keychain/struct.Secret.html) owns its bytes and zeroes them when dropped, compares in constant time, and prints as
`Secret(<n> bytes)` so a stray `{:?}` never logs it. Build one from a string or bytes; read it
with [`as_str()`](https://daybrite.dev/api/day_part_keychain/struct.Secret.html#method.as_str) or [`as_bytes()`](https://daybrite.dev/api/day_part_keychain/struct.Secret.html#method.as_bytes).

### Protection levels

| [`Protection`](https://daybrite.dev/api/day_part_keychain/enum.Protection.html) | Readable | Use it for |
|---|---|---|
| `WhenUnlocked` (default) | while the device is unlocked | a session token the UI uses |
| `AfterFirstUnlock` | from the first unlock after boot until reboot, locked screen included | a secret background work needs (a sync, a notification handler) |
| `UserPresence` | only after the device verifies the person, each read | a key that opens the app's own vault |

Apple and HarmonyOS distinguish the first two (`WhenUnlocked` items are unreadable while the
device is locked); Android, Windows and Linux keep both readable whenever the app runs, since
their stores unlock with the first sign-in. [`protection_support(level)`](https://daybrite.dev/api/day_part_keychain/fn.protection_support.html) says how a level is
realized here: `Native` where the OS binds the item to it, `Emulated` where the crate gates the
read without a binding (Windows user presence), `Unsupported` where nothing can honor it (Linux
user presence, every level on the web). A `UserPresence` [`set`](https://daybrite.dev/api/day_part_keychain/fn.set.html) needs a device passcode, PIN or
password and fails with [`KeychainError::PasscodeNotSet`](https://daybrite.dev/api/day_part_keychain/enum.KeychainError.html#variant.PasscodeNotSet) without one. Writing never prompts;
only reading does.

### What an item is scoped to

Items belong to the app that wrote them, on the device that holds them. Each platform scopes them
by the app's identity: the bundle on Apple, the package on Android, the signed-in account and the
app id on Windows, the app id as a Secret Service attribute on Linux, the sandbox on HarmonyOS.
Nothing is backed up or synchronized, because the keys that encrypt the items never leave the
device. Apple keeps keychain items across a reinstall of the app; the other platforms remove them
with it.

### Errors

[`KeychainError`](https://daybrite.dev/api/day_part_keychain/enum.KeychainError.html) names the outcomes an app acts on: `NotFound`, `NeedsPresence` (a protected item
read with the synchronous [`get`](https://daybrite.dev/api/day_part_keychain/fn.get.html)), `Cancelled` and `AuthFailed` (the dialog), `Lockout`,
`PasscodeNotSet`, `Unreadable` (the item exists but its key was invalidated or its file damaged;
[`remove`](https://daybrite.dev/api/day_part_keychain/fn.remove.html) clears it), `Locked` (a keyring whose unlock dialog was dismissed, an Apple item read
while the device is locked), `Unavailable` (no Secret Service, no logon session), `Unsupported`,
and `Platform(message)` for the rest.

### Threads

The synchronous calls finish in milliseconds on every platform but Linux, where a locked keyring
makes the Secret Service show its unlock dialog during the call; UI code there prefers [`get_async`](https://daybrite.dev/api/day_part_keychain/fn.get_async.html),
which works on a thread of its own. Completions run where the platform delivers them;
[`get_future`](https://daybrite.dev/api/day_part_keychain/fn.get_future.html) awaited under [`day::task`](https://daybrite.dev/api/day/fn.task.html) resumes on the UI thread ([docs/async](/docs/explanation/async)).

## Per-platform native realization

Platform guidance: [Apple: keychain services](https://developer.apple.com/documentation/security/keychain-services); [Android: the Android Keystore system](https://developer.android.com/privacy-and-security/keystore); [Windows: Data Protection API](https://learn.microsoft.com/en-us/windows/win32/api/dpapi/); [freedesktop: the Secret Service API](https://specifications.freedesktop.org/secret-service-spec/latest/); [HarmonyOS: Universal Keystore Kit](https://github.com/openharmony/docs/blob/master/en/application-dev/security/UniversalKeystoreKit/huks-overview.md).

| OS | API | dependency |
|---|---|---|
| iOS, macOS | [`SecItemAdd`](https://developer.apple.com/documentation/security/secitemadd(_:_:)) / [`SecItemCopyMatching`](https://developer.apple.com/documentation/security/secitemcopymatching(_:_:)) / [`SecItemDelete`](https://developer.apple.com/documentation/security/secitemdelete(_:)) on generic-password items with [`kSecUseDataProtectionKeychain`](https://developer.apple.com/documentation/security/ksecusedataprotectionkeychain); [`SecAccessControlCreateWithFlags`](https://developer.apple.com/documentation/security/secaccesscontrolcreatewithflags(_:_:_:_:)) with `userPresence`; an [`LAContext`](https://developer.apple.com/documentation/localauthentication/lacontext) through [`kSecUseAuthenticationContext`](https://developer.apple.com/documentation/security/ksecuseauthenticationcontext) | `objc2-security`, `objc2-core-foundation`, `objc2` |
| Android | AES-256-GCM under an [`AndroidKeyStore`](https://developer.android.com/privacy-and-security/keystore) key from [`KeyGenParameterSpec`](https://developer.android.com/reference/android/security/keystore/KeyGenParameterSpec); a user-authentication key wraps `UserPresence` items and is used through [`BiometricPrompt.CryptoObject`](https://developer.android.com/reference/android/hardware/biometrics/BiometricPrompt.CryptoObject); files under [`getNoBackupFilesDir()`](https://developer.android.com/reference/android/content/Context#getNoBackupFilesDir()) | a Java arm ([`day-bridge`](/contributing/backend-contracts/bridge)) |
| Windows | [`CryptProtectData`](https://learn.microsoft.com/en-us/windows/win32/api/dpapi/nf-dpapi-cryptprotectdata) / [`CryptUnprotectData`](https://learn.microsoft.com/en-us/windows/win32/api/dpapi/nf-dpapi-cryptunprotectdata) over one file per item; [`day-part-biometrics`](/docs/reference/biometrics) (Windows Hello) in front of `UserPresence` items | `windows` |
| Linux | [`secret_password_storev_sync`](https://gnome.pages.gitlab.gnome.org/libsecret/func.password_storev_sync.html) / [`lookupv_sync`](https://gnome.pages.gitlab.gnome.org/libsecret/func.password_lookupv_sync.html) / [`clearv_sync`](https://gnome.pages.gitlab.gnome.org/libsecret/func.password_clearv_sync.html) / [`searchv_sync`](https://gnome.pages.gitlab.gnome.org/libsecret/func.password_searchv_sync.html) under a schema of the crate's own, `libsecret-1.so.0` loaded with `dlopen` | none at build time |
| HarmonyOS | [`OH_Huks_GenerateKeyItem`](https://github.com/openharmony/docs/blob/master/en/application-dev/reference/apis-universal-keystore-kit/capi-native-huks-api-h.md) / `OH_Huks_InitSession` / `OH_Huks_FinishSession` (AES-256-GCM, [user-authentication-bound keys](https://github.com/openharmony/docs/blob/master/en/application-dev/security/UniversalKeystoreKit/huks-user-identity-authentication.md)) over one file per item; [`day-part-biometrics`](/docs/reference/biometrics) turns the keystore's challenge into the token the key accepts | `libhuks_ndk.z.so` (the public NDK) |
| Web | — | none (`Unsupported`) |

## How each platform realizes the levels

- **iOS and macOS**: `WhenUnlocked` is [`kSecAttrAccessibleWhenUnlockedThisDeviceOnly`](https://developer.apple.com/documentation/security/ksecattraccessiblewhenunlockedthisdeviceonly),
  `AfterFirstUnlock` is [`kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`](https://developer.apple.com/documentation/security/ksecattraccessibleafterfirstunlockthisdeviceonly), and `UserPresence`
  is an access control created with [`kSecAttrAccessibleWhenPasscodeSetThisDeviceOnly`](https://developer.apple.com/documentation/security/ksecattraccessiblewhenpasscodesetthisdeviceonly) and the
  `userPresence` flag: a biometric or the passcode, each read, not invalidated by a change of
  enrollment. The synchronous [`get`](https://daybrite.dev/api/day_part_keychain/fn.get.html) hands the keychain an `LAContext` with `interactionNotAllowed`
  set, so the keychain refuses instead of prompting; [`get_async`](https://daybrite.dev/api/day_part_keychain/fn.get_async.html) hands it one carrying the reason,
  and the system shows its dialog during `SecItemCopyMatching`. The data protection keychain
  needs the `keychain-access-groups` entitlement a signed app has; an unsigned `cargo run` or
  development launch on macOS falls back to the login keychain, which has no access control.
- **Android**: one file per item under the app's no-backup directory, so a backup restored to
  another device does not carry ciphertext nothing can open. Items at the first two levels are
  AES-256-GCM under one keystore key. `UserPresence` items are AES-256-GCM under a per-item key
  that an RSA keystore key pair wraps: encryption with the public key needs no prompt, and
  decryption with the private key is allowed only through a cipher the system prompt authorized,
  with the device credential accepted alongside a biometric. Keys are created by [`set`](https://daybrite.dev/api/day_part_keychain/fn.set.html), never by a
  read, so a wiped keystore reads as `Unreadable` rather than silently starting over.
- **Windows**: DPAPI binds the bytes to the signed-in account on this machine and adds its own
  integrity check; another account, or another machine, cannot open them, and an administrator's
  password reset makes them `Unreadable`. `UserPresence` is Windows Hello in front of the read,
  reported as `Emulated` because DPAPI itself does not know about the check. A [`set`](https://daybrite.dev/api/day_part_keychain/fn.set.html) at that level
  confirms Hello is set up first, so no item is stored that nothing could read.
- **Linux**: one Secret Service item per key in the default collection, under the schema
  `dev.daybrite.day.keychain` with `application` and `key` attributes, the secret stored as base64
  text so every implementation behind the interface keeps it intact. A locked keyring is unlocked
  by the service's own dialog during the call; a dismissed dialog reads as `Locked`. A session with
  no Secret Service (a headless runner, a container) reports `Unsupported`. There is no
  user-presence level: no desktop API lets an app verify the person.
- **HarmonyOS**: three AES-256-GCM keys the Universal Keystore generates on first use, one per
  level: `WhenUnlocked` at the ECE storage level (usable only while unlocked), `AfterFirstUnlock`
  at CE, and `UserPresence` at ECE with user authentication required for decryption only, so a
  [`set`](https://daybrite.dev/api/day_part_keychain/fn.set.html) never prompts. The item's key name is the cipher's associated data. Reading a protected
  item opens a keystore session (which issues a challenge), shows the user-authentication dialog
  through `day-part-biometrics` with that challenge, and finishes the session with the token the
  dialog returned. Removing the lock-screen password invalidates the protected key.

## What it shows about the extension system

Like [`day-part-battery`](/docs/reference/battery) and [`day-part-haptics`](/docs/reference/haptics), this is a headless crate with no UI Piece. It
exercises three of the extension mechanisms at once ([docs/extending](/contributing/backend-contracts/extending)): a daybridge Java arm for Android
([docs/bridge](/contributing/backend-contracts/bridge)), the **manifest-permission overlay** (`USE_BIOMETRIC` and `ACCESS_BIOMETRIC` contributed by
the crate, the Face ID reason declared by the app), and a part depending on a part
([`day-part-biometrics`](/docs/reference/biometrics) on Windows and HarmonyOS, the way `day-part-downloads` rides on `day-part-http`). The Day
Showcase's Security page stores, reads and removes an item at each level.
