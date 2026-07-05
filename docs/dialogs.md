# Dialogs, modals & imperative presentation (`present`)

Alerts, confirmations, action sheets, text prompts (and later native pickers) are
*imperative request→response* interactions: an action opens one and needs the answer
back. SwiftUI is forced to model this as a detached binding
(`showAlert = true` … `.alert($showAlert)`) because `body` re-runs constantly. Day
doesn't have that constraint — a `button().action(|| …)` is a real closure on the
persistent main thread — so Day co-locates the request and its response with
**async/await**:

```rust
button(tr("delete")).action(|| day::task(async move {
    let choice = Alert::new(tr("delete-title"))
        .message(tr("delete-body"))
        .destructive(tr("delete"), Choice::Delete)   // a button carrying a payload
        .cancel(tr("cancel"))                          // dismissal → None
        .present().await;                              // -> Option<Choice>
    if choice == Some(Choice::Delete) {
        store.delete(id);
    }
}));
```

No boolean signal, no modifier attached elsewhere. `day::task` is the one explicit
opt-in ("this action starts an async flow").

## Layers

**Layer 0 — the primitive (plumbing).** Mirrors the `nav` controller pattern: an
imperative call routes a request through the tree to the backend, and the answer flows
back through the enqueue-only `Event` sink.

- `day_spec::present::PresentSpec` — `Dialog { title, message, buttons, sheet }` or
  `Prompt { title, message, placeholder, initial, ok, cancel }`. `PresentButton { label,
  role }`, `ButtonRole { Default, Cancel, Destructive }`.
- `PresentResult` — `Button(i64)` / `Text(String)` / `Dismissed`. Tagged so it crosses
  the C ABI (Qt/Android) as a **flat payload** (tag + index + string), the same style as
  canvas `encode_ops`.
- `Toolkit::present(req: u64, spec: &PresentSpec)` and `dismiss(req)` (default no-ops);
  `Event::PresentResult { req, result }`; `Cap::Dialogs`.
- `day-core` keeps a thread-local `PENDING: HashMap<u64, …>`. `present(spec)` mints a
  `req`, presents through `with_tree(|t| t.present(req, spec))`, and parks a waker. When
  the backend answers, `pump_events` routes `Event::PresentResult` to `resolve(req,
  result)`, which wakes the future. Native modals dismiss themselves; a *programmatic*
  resolve (dayscript) also calls `dismiss(req)`.

**Layer 1 — async (the surface).** A tiny single-threaded executor (`day::task`, ~60
lines, `std`-only): tasks are `Pin<Box<dyn Future>>` in a thread-local map, polled on the
main loop; the presentation future registers its `Waker` and is re-polled through the
existing `Platform::post`/`on_main`. Futures are correctly `!Send` (one UI thread); no
async dependency.

## API (`day-pieces::present`, re-exported in the prelude)

```rust
alert(tr("saved")).present();                    // fire-and-forget notice (1 button)
let ok: bool         = confirm(tr("quit?")).await;
let name: Option<..> = prompt(tr("name")).await; // -> Option<String>

// full builder — buttons carry a typed payload; `.cancel()` and dismissal → None
let picked: Option<Flavor> = Alert::new(tr("pick"))
    .button(tr("vanilla"), Flavor::Vanilla)
    .button(tr("pistachio"), Flavor::Pistachio)
    .sheet()                                       // bottom action sheet on mobile
    .cancel(tr("cancel"))
    .present().await;
```

Every text field is an `IntoText`, so titles/buttons localize through `tr()` (Fluent).

## Per-toolkit native mapping

| Toolkit | Dialog / sheet | Prompt |
|---|---|---|
| appkit | `NSAlert` + `beginSheetModalForWindow:` (async) | `NSAlert` with an `NSTextField` accessory |
| uikit | `UIAlertController` (`.alert` / `.actionSheet`) on the root VC | `UIAlertController` + `addTextField` |
| gtk | `AdwAlertDialog` (libadwaita 1.5; `response` signal) | `AdwAlertDialog` with a `GtkEntry` extra-child |
| qt | `QMessageBox.open()` + `finished` (shim) | `QInputDialog.getText` (shim) |
| android | `AlertDialog.Builder` (buttons / `setItems` for sheets) | `AlertDialog` + `EditText` |
| mock | records the spec; resolved programmatically | same |
| winui | `ContentDialog` (UNVERIFIED, no local Windows) | `ContentDialog` + `TextBox` |

All backends use the **non-blocking** async APIs (sheets / `open()` / callbacks), so the
main loop keeps running and dayscript stays live while a modal is up.

## The four pillars

- **dayscript** — presentations flow through the registry as a `req`-tagged spec, so a
  script can inspect the pending modal (`assert_presented`) and answer it
  (`- respond: { button: 1 }` / `{ text: "Ada" }` / `{ dismiss: true }`), which
  `dismiss`es the native control and resolves the future. This makes modal flows
  headless-testable and screenshot-able.
- **a11y** — native controllers are accessible for free.
- **Fluent** — spec fields are `IntoText`.
- **polyglot** — `PresentSpec`/results are an open per-kind set; a third-party crate can
  add a platform picker (contacts, photos, files, share) the same way
  `day-piece-combobox` adds a widget — a new spec variant + `Cap` + backend arms, zero
  day-core edits.

## Deferred

- **Native integration pickers** (contacts / photos / files / share): same present→result
  model with richer result payloads and `Cap`-gated fallbacks; designed here, implemented
  after the dialog family lands.
- **New windows**: need a multi-`Tree` refactor (thread-local tree keyed by window) and
  are desktop-only (`Cap::MultiWindow`); explicitly out of this pass.
- **Task/scope binding**: v1 tasks run at the root scope; cancelling an in-flight dialog
  when its owning subtree is disposed is a later refinement (signal writes to disposed
  scopes are already no-ops, so it's safe meanwhile).
