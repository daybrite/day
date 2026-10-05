---
title: "External URL declarations"
description: "Register additional protocol associations in Day.toml."
---

# External URL declarations

### Additional URL schemes

Root `url_schemes = ["feed", "web+feed"]` registers external protocol candidates alongside the
app's navigation scheme. Names use lowercase URI scheme syntax (`[a-z][a-z0-9+.-]*`). Install
`day::on_open_url` in the root to consume original URLs before routing; see
[external URL handling](deep-links.md#external-url-handlers). PWA protocol handlers support only
browser-permitted `web+` names. Scheme registration and MIME file associations do not force
browser handoff or change a user's default handler.
