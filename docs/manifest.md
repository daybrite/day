---
navTitle: "URL declarations"
title: "External URL declarations"
description: "Register additional protocol associations in Day.toml."
---

# External URL declarations

Register the URL schemes your app can open and the URLs that launchers may offer as shortcuts.
These declarations live in `Day.toml`; app code handles incoming URLs.

## Additional URL schemes

Root `url_schemes = ["feed", "web+feed"]` registers external protocol candidates alongside the
app's navigation scheme. Names use lowercase URI scheme syntax (`[a-z][a-z0-9+.-]*`). Install
[`day::on_open_url`](https://daybrite.dev/api/day/fn.on_open_url.html) in the root to consume original URLs before routing; see
[external URL handling](deep-links.md#external-url-handlers). PWA protocol handlers support only
browser-permitted `web+` names. Scheme registration and MIME file associations do not force
browser handoff or change a user's default handler.

## Platform guidance

- [Apple: custom URL schemes](https://developer.apple.com/documentation/xcode/defining-a-custom-url-scheme-for-your-app)
- [Android: deep links](https://developer.android.com/training/app-links/deep-linking)
