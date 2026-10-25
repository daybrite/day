<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-part-http

Make HTTP requests and WebSocket connections from Rust using Day’s networking API.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

It uses platform networking by default and supplies client policies for timeouts, redirects,
caching, and authentication. Requests that can block belong off the UI thread. Browser requests
follow browser security rules, including CORS.

## Use

Add this entry to your app’s `Cargo.toml`, at the same Day version as the rest of the app:

```toml
[dependencies]
day-part-http = "0.6"
```

## Documentation

- [Usage and platform limits](https://daybrite.dev/docs/reference/http/)
- [Fetch data in an app](https://daybrite.dev/docs/how-to/guide-http/)
- [Using platform services](https://daybrite.dev/docs/reference/parts/)
- [Rust API](https://daybrite.dev/api/day_part_http/index.html)

[Source](https://github.com/daybrite/day/tree/main/crates/day-part-http) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
