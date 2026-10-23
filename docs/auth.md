---
navTitle: "Authentication"
title: "Sign-in and passkeys"
description: "Connect a Day app to an account provider, receive native credentials, and request passkeys, with token ownership and platform requirements."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Sign-in and passkeys

Sign-in connects an app to a person's account so it can retrieve their data and call services on
their behalf. `day-part-auth` opens provider sign-in pages in the system browser, runs OAuth 2.0
and OpenID Connect exchanges, and requests Apple, Google, and passkey credentials from native
dialogs. The app remains responsible for its session, token storage, and server authorization.

Use [biometrics](/docs/reference/biometrics) to ask whether the person at the device can unlock
protected content. Use sign-in to establish an account identity with a service. A successful
biometric prompt alone does not sign the person in to that service.

> [!WARNING]
> **Experimental account integration.**
> The current client has unresolved endpoint and callback validation, refresh, and browser
> limitations described below. Use it for local evaluation; it is not ready to protect production
> accounts. Native credential requests also need server verification before granting access.

## Add to your app

The defining crate, `day-part-auth`, lives in a separate source checkout. It is not available on
crates.io, and its public Git repository and hosted Rustdoc are not available yet. With the crate
beside the app directory, add this dependency to the app's `Cargo.toml`:

```toml
[dependencies]
day-part-auth = { path = "../day-part-auth" }
```

This path requires that checkout on every machine that builds the app. The crate uses the same
Day Git source as other [parts](/docs/reference/parts). Build the app with the
[Day CLI](/docs/reference/cli), which generates and packages its native bridge code. There is no
`[parts]` entry or toolkit feature to add to `Day.toml`. Provider registration, callback schemes,
and platform entitlements are separate requirements.

For local API browsing, run `cargo doc --no-deps --open` in the crate checkout. The tables below
describe the public entry points; hosted item links can be added when the crate is published.

<!-- Publication follow-up: replace the local dependency only after the external repository is
available. Add the crate to the API build and replace API names with verified root-relative
Rustdoc links. Do not add /api/day_part_auth links before the publication artifact contains it. -->

| Platform / toolkit | Browser sign-in path | Native credentials |
| --- | --- | --- |
| macOS / AppKit | Default browser and a loopback callback by default; system authentication session for configured scheme or HTTPS callbacks. | Apple sign-in; platform passkeys on macOS 13+. |
| iOS / UIKit | System authentication session and an app callback. | Apple sign-in on iOS 13+; platform passkeys on iOS 16+. |
| Android | Custom Tab and an app callback. | Google through Credential Manager; passkeys on Android 9+ with a credential provider. |
| Windows / WinUI | Default browser and a loopback callback. | Windows WebAuthn on Windows 10 1903+. |
| Linux / GTK / Qt | Default browser and a loopback callback. | Passkeys and the Apple/Google native helpers return `Unsupported`. |
| HarmonyOS / ArkUI | Default browser and an app callback; the browser tab remains open. | Passkeys and the Apple/Google native helpers return `Unsupported`. |
| Web / DOM | Popup and a callback to the app's origin; the token client has the clock limitation below. | Browser WebAuthn. |

These are implementation paths, not a promise that an account, associated domain, credential
provider, or signing entitlement is configured. Handle failure when opening every dialog.

## Choose an authentication flow

| App requirement | API | Result and responsibility |
| --- | --- | --- |
| Open a provider page and receive its callback | `session::open(url, callback_prefix, &options)` | `Result<String, AuthError>` containing a URL; this layer does not verify identity. |
| Run an OAuth / OpenID Connect code flow | `Client::sign_in()` | `Result<TokenSet, AuthError>` after the code exchange and applicable ID-token checks. |
| Supply a custom browser integration | `Client::start()` and `Client::complete(request, callback_url)` | Keep the authorization request with the same client until completion. |
| Request an Apple credential | `apple::sign_in(SignInWithApple)` | `AppleCredential`; send its token/code to the account service for verification. |
| Request a Google credential on Android | `google::sign_in(GoogleSignIn)` | `GoogleCredential`; verify the ID token at the account service. |
| Register or use a passkey | `passkeys::create(&options)` / `passkeys::get(&options)` | Credential JSON; the relying-party server must verify it. |

OAuth authorizes access to APIs. OpenID Connect adds an ID token describing the account that
signed in. A plain OAuth access token does not establish an identity for the app. See
[OpenID Connect Core](https://openid.net/specs/openid-connect-core-1_0.html) for the protocol.

## Configure an OAuth or OpenID Connect client

`Client::new(provider, client_id)` defaults to the `openid` scope, S256 PKCE, `Redirect::Auto`,
and default `SessionOptions`. PKCE binds the authorization code to the request that started the
flow. Leave it enabled for public clients such as installed apps.

| Configuration | Contract |
| --- | --- |
| `Provider::google()` | Built-in Google endpoint configuration; register a client appropriate to the app and redirect. |
| `Provider::apple()` | Apple endpoint configuration, not a complete Apple web integration; see [Apple requirements](#sign-in-with-apple). |
| `Provider::discover(issuer).await` | Fetches the issuer's discovery document and compares its issuer after trimming trailing slashes. |
| `Provider::new(authorization_endpoint, token_endpoint)` | Plain OAuth endpoint configuration. For OAuth without identity, replace the default `openid` scope. OIDC also needs an issuer and `jwks_uri`. |
| `.scopes([...])` | Replaces the scope list. The provider decides which scopes and refresh-token grants are available. |
| `.redirect(Redirect)` | Selects a callback registered with the provider and configured for the app. |
| `.param(name, value)` | Appends a provider parameter such as `prompt` or `login_hint`. Do not repeat protocol fields such as `state`, `nonce`, or `redirect_uri`. |
| `.session(SessionOptions)` | Configures the browser request and loopback response. |
| `.client_secret(secret)` | Sends HTTP Basic client authentication. A distributed native app cannot keep a client secret confidential. |

> [!WARNING]
> **Provider and callback validation.**
> URL checks currently use string prefixes, discovered endpoints are not all checked for HTTPS,
> and token requests can follow redirects that forward their request body to another origin.
> Callback matching also uses a prefix and does not check the authorization response's issuer.
> These gaps must be fixed before accepting untrusted provider configuration or production
> credentials. A configured issuer, PKCE, and matching `state` do not remove these limitations.

An authorization request contains a fresh `state`, a nonce when `openid` is requested, and a
PKCE verifier. `Client::complete` consumes the request. It must be completed with the client that
created it; the request does not currently enforce that association itself. The callback parser
reports missing or different state as `AuthError::StateMismatch` and provider refusal as
`AuthError::Provider`.

API excerpt; not a complete app. This helper retains the issuing client with its tokens. Call it
from a UI action under [`day::task`](https://daybrite.dev/api/day/fn.task.html); the app supplies the configured
client and decides where to retain the returned session.

```rust
use day_part_auth::{AuthError, Client, TokenSet};

pub struct AccountSession {
    pub client: Client,
    pub tokens: TokenSet,
}

pub async fn begin_sign_in(client: Client) -> Result<AccountSession, AuthError> {
    let tokens = client.sign_in().await?;
    Ok(AccountSession { client, tokens })
}
```

Retain the session's client when refreshing, reading account information, or revoking a token.
Rebuilding it from an editable provider picker can send an existing token to a different service.
Keep at most one sign-in or refresh active. On sign-out, invalidate pending completions so that a
late result cannot restore the session.

## Redirects and browser lifetime

| `Redirect` | Behavior |
| --- | --- |
| `Auto` | Uses the app scheme with `/auth/callback` on iOS, Android, and HarmonyOS; a loopback listener on desktops; the page URL without its fragment on Web. |
| `Scheme(String)` | Uses a custom scheme registered by the app. The provider must allow that exact callback. |
| `Loopback { port, path }` | Binds `127.0.0.1`; port `0` chooses a free port. `Redirect::loopback()` uses port `0` and `/callback`. Intended for desktops. |
| `Https(String)` | Requires a callback the app owns and can receive, such as a configured associated domain or Android App Link. Setting the enum alone does not configure that association. |

For scheme callbacks, merge this into the app's existing `Day.toml` `[app]` table. The identifier
is a protocol value; choose one specific to the app and register `exampleaccount://auth/callback`
with the provider.

```toml
[app]
scheme = "exampleaccount"
```

See the [deep-link reference](/docs/reference/deep-links) for routing and platform registration.

`SessionOptions` defaults to `ephemeral = false`, a 300-second loopback timeout, and a built-in
completion page. `ephemeral` asks Apple's authentication session for separate browser cookies;
it has no effect on the default desktop loopback flow. `timeout` applies to the loopback
listener, not every native sheet. `completion_page` replaces the listener's HTML response;
provide localized content from generated accessors if the app uses it.

> [!NOTE]
> **AppKit / UIKit: HTTPS callbacks.**
> The system session's HTTPS callback API requires macOS 14.4 or iOS 17.4 and the appropriate
> associated-domain configuration. `session::capabilities().https_callback` currently reports
> the Apple target family without checking that OS version.

`session::cancel()` requests cancellation of the native browser session. It does not cancel the
separate desktop loopback wait. Dropping a future does not reliably dismiss the browser or stop
the listener. Treat cancellation of UI state and cancellation of native work as separate steps.
The loopback page can say the flow is complete before the code exchange succeeds; use the
`sign_in()` result as the account state.

## Tokens and account state

`TokenSet` contains the access token, token type, optional expiry, optional refresh token,
optional `IdToken`, granted scope, and the raw token response. For an OIDC sign-in, the current
client requires an ID token. Its signature is checked against provider keys using RS256 or
ES256; claim checks cover audience, authorized party, expiry, issued-at, not-before, the request
nonce, and the access-token hash when present. Issuer checking applies when the provider has an
issuer configured. Clock leeway is 60 seconds. Key sets are cached for an hour, with one forced
fetch after signature verification fails.

`TokenSet::subject()` reads the ID token's subject. Identify an account by **issuer and subject**,
not by email or subject alone. `expires_in()` returns `None` when expiry is unknown, and
`expires_within(margin)` returns `false` in that case; neither result guarantees an access token
will still be accepted by its service.

| Operation | Contract |
| --- | --- |
| `client.refresh(refresh_token).await` | Requests replacement tokens and retains the old refresh token if the response omits one. See the refresh limitation below. |
| `client.userinfo(access_token).await` | Returns JSON. Before using it as identity, compare `sub` with the verified ID token's subject. |
| `client.revoke(token, hint).await` | Requests revocation; `hint` is `access_token` or `refresh_token`. Missing provider support returns `Unsupported`. |
| `client.end_session_url(id_token_hint, post_logout_redirect_uri)` | Returns an optional provider logout URL. It does not open the URL or clear local state. |

> [!WARNING]
> **Refresh compatibility.**
> The current OIDC client rejects refresh responses that omit `id_token`, although
> [OpenID Connect permits them](https://openid.net/specs/openid-connect-core-1_0.html#RefreshTokenResponse).
> It also lacks the original identity context needed to compare a refreshed ID token with the
> original sign-in. A rejected response can contain a rotated refresh token that the caller never
> receives. Do not rely on this refresh path for production sessions until these issues are fixed.

The crate does not persist tokens. Store retained refresh tokens through
[`day-part-keychain`](/docs/reference/keychain), accounting for that store's platform limits.
Do not put tokens in [preferences](/docs/reference/prefs), URLs, logs, or screenshots. Current
`Debug` implementations print credentials, and `TokenSet::raw` also contains token material.

Local sign-out means clearing memory, stored credentials, and pending account operations.
Revoking a token and signing out of the provider's browser session are separate operations.
Serialize refresh requests, and replace stored refresh tokens when the provider rotates them.

> [!WARNING]
> **Web / DOM: token clock.**
> ID-token validation, key caching, and token expiry use `std::time::SystemTime`, which traps on
> the current `wasm32-unknown-unknown` build. Browser compilation does not establish a working
> OAuth/OIDC token client. The separate browser WebAuthn API does not use this token clock.

## Native account credentials

Native helpers return credentials from the system, not an authenticated session with the app's
server. The server must verify the token signature, issuer, audience, expiry, and request nonce,
then establish its own session. Keep the expected nonce with the request; a nonce received only
alongside an untrusted token is not sufficient evidence of a pending sign-in.

### Sign in with Apple

`apple::sign_in(SignInWithApple::new())` requests the native sheet. The request can select name
and email scopes and supply a nonce and state. `AppleCredential` contains the user identifier,
identity token, authorization code, optional profile fields, and nonce information. Profile data
may be supplied only on the first authorization; retain needed values after server verification.
`apple::credential_state(user)` checks Apple's authorization state for a previously returned user.

> [!WARNING]
> **AppKit / UIKit: nonce and state.**
> The native request sends a hexadecimal SHA-256 nonce, but `AppleCredential::hashed_nonce`
> currently contains a base64url digest. Those values differ. The helper also substitutes the
> requested state when the returned state is absent instead of validating it. Correct these
> contracts before using the helper to establish accounts.

Enable Sign in with Apple for the app identifier and signing configuration. Follow Apple's
[Sign in with Apple guidance](https://developer.apple.com/sign-in-with-apple/) and
[`ASAuthorizationAppleIDProvider`](https://developer.apple.com/documentation/authenticationservices/asauthorizationappleidprovider).

Apple's web flow needs a registered Services ID, an HTTPS return URL, and server-held signing
credentials for token exchange. `Provider::apple()` supplies endpoints; entering a client ID
alone does not implement this flow. The current generic client does not supply Apple's complete
server exchange or a `form_post` callback handler. Keep the signing key on the server. See
[Apple's token exchange requirements](https://developer.apple.com/documentation/signinwithapplerestapi/generate-and-validate-tokens).

### Sign in with Google

On Android, `google::sign_in(GoogleSignIn::new(server_client_id))` requests a Google ID credential
through Credential Manager. Use the **web application/server** OAuth client ID for the token
audience, with the Android app registration and signing certificate configured in the same
project. The result includes `id_token`, account/profile fields, and the request nonce.

`filter_by_authorized_accounts(true)` limits selection to returning accounts; `auto_select(true)`
requests automatic selection when eligible. Both default to `false`. A `no_credential` provider
error means no matching credential was returned. Native Google sign-in is Android-only; other
targets use a separately configured browser provider flow.

Follow the [Credential Manager integration guide](https://developer.android.com/identity/sign-in/credential-manager-siwg)
and [Google ID-token verification](https://developers.google.com/identity/sign-in/android/backend-auth).

## Passkeys

A passkey lets a person prove possession of an account credential using their device or a
security key. The **relying party** is the service that registers the credential and verifies
later assertions. `passkeys::create` and `passkeys::get` open the platform request and return
JSON; they do not maintain an account database or verify a server session.

| Operation | Input | Output |
| --- | --- | --- |
| `passkeys::create(&options).await` | Server-issued creation options with `rp`, `user`, `challenge`, and credential parameters. | Registration credential JSON, including `clientDataJSON` and `attestationObject`. |
| `passkeys::get(&options).await` | Server-issued request options with a challenge and relying-party information. | Assertion JSON, including authenticator data, signature, and optional user handle. |
| `passkeys::support()` | No arguments. | Whether the target has an implementation. |
| `passkeys::platform_authenticator_available().await` | No arguments. | A capability hint, not proof of a configured account, entitlement, or available credential. Apple and Android currently use broad OS/API checks. |

Binary fields use base64url in the JSON. Obtain challenges from the server, bind each to a
pending operation, and expire it after use. Send the returned credential to that server, which
checks challenge, origin, relying-party ID, signature, and required user verification before
granting access. The [WebAuthn specification](https://www.w3.org/TR/webauthn-3/) defines these
registration and assertion checks.

> [!WARNING]
> **AppKit / UIKit: creation options.**
> The current implementation uses the platform authenticator and does not honor every JSON
> option, including algorithm selection, authenticator attachment, and attestation preference.
> Excluded credentials are passed through only on iOS 17.4 / macOS 14.4 and later. Do not assume
> an arbitrary server WebAuthn policy is enforced by this adapter.

> [!WARNING]
> **Windows / WinUI: credential metadata.**
> The current adapter reports `authenticatorAttachment = "platform"` and registration transport
> `internal` even when a security key was used. Treat those metadata fields as unreliable until
> corrected. The adapter constructs `clientDataJSON` with `https://<rp.id>` by default, or the
> supplied top-level `origin`; the server must apply its native-client origin policy.

The Showcase passkey buttons demonstrate credential requests. They generate local challenges
and display credential IDs, but do not register keys with a server or verify assertions. A
returned credential in that demo is not evidence of an authenticated account.

## Per-toolkit native realization

| Platform / toolkit | Native API and setup |
| --- | --- |
| AppKit / UIKit | [`ASWebAuthenticationSession`](https://developer.apple.com/documentation/authenticationservices/aswebauthenticationsession) for configured system browser sessions. Passkeys use [`ASAuthorizationPlatformPublicKeyCredentialProvider`](https://developer.apple.com/documentation/authenticationservices/asauthorizationplatformpublickeycredentialprovider); follow [Apple's passkey setup](https://developer.apple.com/documentation/authenticationservices/supporting-passkeys) and configure the relying party as a `webcredentials:` associated domain. |
| Android | [Custom Tabs](https://developer.chrome.com/docs/android/custom-tabs) receive redirects through [deep links](/docs/reference/deep-links). [Credential Manager](https://developer.android.com/identity/sign-in/credential-manager) handles passkeys and Google credentials. Publish the app/domain association described in [Digital Asset Links setup](https://developer.android.com/identity/credential-manager/prerequisites). |
| Windows / WinUI | [`WebAuthNAuthenticatorMakeCredential`](https://learn.microsoft.com/en-us/windows/win32/api/webauthn/nf-webauthn-webauthnauthenticatormakecredential) and [`WebAuthNAuthenticatorGetAssertion`](https://learn.microsoft.com/en-us/windows/win32/api/webauthn/nf-webauthn-webauthnauthenticatorgetassertion) provide the passkey dialogs. Browser sign-in opens the default browser. |
| Linux / GTK / Qt | Browser sign-in uses [Day's URL-opening API](/docs/reference/applications) and a loopback listener, following [OAuth for native apps](https://www.rfc-editor.org/rfc/rfc8252.html#section-7.3). This crate has no native passkey adapter for these toolkits. |
| HarmonyOS / ArkUI | Browser sign-in uses [Day's URL-opening API](/docs/reference/applications) and [app callbacks](/docs/reference/deep-links). It does not close the external browser tab. |
| Web / DOM | [`CredentialsContainer.create()` and `get()`](https://www.w3.org/TR/webauthn-3/#sctn-api) request passkeys in a secure context. The relying-party ID must satisfy the browser's domain rules. Start browser UI from a user gesture and handle popup blocking and credential-provider refusal. |

> [!NOTE]
> **AppKit: sandboxed loopback sign-in.**
> A sandboxed app needs outgoing network access for provider requests and incoming network access
> for the loopback listener. Check the app's signing entitlements before using the default
> desktop redirect; see [Apple's App Sandbox network permissions](https://developer.apple.com/documentation/security/app-sandbox).

> [!NOTE]
> **Android: local demo traffic.**
> A demo provider on `127.0.0.1` needs `cleartext-domains = ["127.0.0.1"]` in the existing
> `[android]` table of `Day.toml`. Keep this exception limited to the demo; provider traffic in a
> released app must use HTTPS. The crate contributes its Gradle dependencies and `INTERNET`
> permission through [part metadata](/docs/reference/parts).

## Errors, threading, and localized UI

| `AuthError` | App response |
| --- | --- |
| `Unsupported` | Hide or disable the unavailable operation and offer a supported flow. |
| `Cancelled` | Return to the signed-out UI without treating dismissal as a failed account. |
| `Timeout` | Offer a new attempt; do not reuse the expired authorization request. |
| `InvalidRequest(reason)` | Correct provider, client, callback, or passkey configuration. |
| `Provider { error, description }` | Handle protocol codes such as `access_denied` or `invalid_grant`; descriptions are provider-supplied text. |
| `StateMismatch` | Reject the callback and restart from a fresh request. |
| `InvalidToken(reason)` | Reject the result; do not grant access using partially decoded claims. |
| `Network(reason)` | Offer retry after connectivity recovers. |
| `Platform(reason)` | Explain that the native request failed and retain a supported alternative. |

Start browser and credential calls on the UI thread. Their futures are `Send`; await them under
[`day::task`](https://daybrite.dev/api/day/fn.task.html) to resume on the UI thread before updating signals or
formatting UI messages. The native APIs may start work when called, before their future is polled.

Use generated Fluent accessors for app-owned labels and error messages, and symbolic IDs for
controls. API excerpt; not a complete app: `start_account_sign_in` is the app's UI action, which
starts the task and owns its busy state and completion handling.

```rust
button(res::str::account_sign_in())
    .id(day::id!(account_sign_in))
    .action(start_account_sign_in)
```

Add the corresponding message to `resource/locales/en/app.ftl`:

```ftl
account_sign_in = Sign in
account_sign_in_failed = Sign-in could not be completed. Try again.
account_sign_in_cancelled = Sign-in cancelled.
```

Map error variants to messages such as [`res::str::account_sign_in_failed().format()`](https://daybrite.dev/api/day_fluent/struct.LocalizedText.html#method.format) on the UI
thread. `AuthError::to_string()` contains diagnostic English and provider details; it is not a
localized user-message API. See [localization](/docs/reference/localization) and
[symbolic element IDs](/docs/reference/accessibility). Keep tokens out of diagnostic reports.

## Local demonstration provider

The `demo-provider` feature adds an in-memory OpenID Connect server on loopback for native
demonstrations. Enable it only in a demo dependency:

```toml
[dependencies]
day-part-auth = { path = "../day-part-auth", features = ["demo-provider"] }
```

API excerpt; not a complete app. This function runs a synthetic account flow without a browser;
`ada` is a fixture identifier, not app-owned UI text. Keep `DemoProvider` alive for as long as
its endpoints are needed.

```rust
use day_part_auth::demo_provider::{DemoConfig, DemoProvider, sign_in_headless};
use day_part_auth::{AuthError, Client, TokenSet};

pub async fn demo_sign_in() -> Result<(DemoProvider, Client, TokenSet), AuthError> {
    let provider = DemoProvider::start(DemoConfig::default())?;
    let client = Client::new(provider.provider(), provider.client_id())
        .scopes(["openid", "profile", "email"]);
    let tokens = sign_in_headless(&client, Some("ada")).await?;
    Ok((provider, client, tokens))
}
```

The server uses ephemeral keys and synthetic accounts, permits demo redirects, and can approve
an account without authenticating a person. It is not an identity service for deployment. The
module is absent on Web even when the feature is enabled. The native demo and Showcase exercise
this provider; their success does not establish interoperability with an external provider.
