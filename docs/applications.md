# System application handlers

Use `day::application_handlers(&HandlerQuery)` to read current OS associations, and
`day::default_browser()` for the OS-selected HTTPS browser. `HandlerQuery` accepts an
absolute URL (including `file:`), a scheme without `:`, a MIME type, or a file extension
without a leading dot. Discovery performs no network requests and opens nothing.

```rust,ignore
let handlers = day::application_handlers(&day::HandlerQuery::Extension("pdf".into()))?;
if let Some(viewer) = handlers.default {
    day::open_url_with_application("file:///Users/me/report.pdf", &viewer.id).await?;
}
```

Results contain an optional default and a deduplicated list including that default. Names
come from the OS. Application identifiers are opaque: persist them for an app preference,
then handle `NotFound`/`LaunchFailed` when the application is moved or removed. Query afresh
when presenting a chooser; do not cache the system default indefinitely. Choosing an app
for an open request does not change any system-wide associations.

Call discovery on the UI thread and await opening inside `day::task`. The native completion
may arrive from an OS queue; core uses a oneshot to resume the caller on its executor. No
native callback touches UI signals. Opening success reports OS acceptance, not page loading.
The target browser decides whether to use a tab or window; Day does not start duplicate
browser instances or require Apple Events automation permission.

AppKit implements every query kind with NSWorkspace and UniformTypeIdentifiers. UIKit and
other backends currently return `ApplicationError::Unsupported`; continue to use `open_url`
for their default handler. No fake browser name or fixed installed-app list is returned.
Malformed queries return `InvalidInput`, and a valid unregistered content type can return
an empty result. Native discovery, malformed queries, stale application IDs, and list focus
are covered by `cargo test -p day-appkit --test native_applications` (main-thread harness).
