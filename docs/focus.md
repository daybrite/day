# Focus — design plan

> Status: **plan for review** — nothing here is implemented yet. `Event::FocusChanged(bool)`
> exists in day-spec (reserved by §8.3) with no producers or consumers; everything else is new.
> DESIGN.md already charters focus to day-core (§3.2) and depends on focus knowledge for the
> controlled-input rule (§4.4: "the native widget is the source of truth **while it has focus**"),
> so this is a correctness workstream as much as a feature.

Day apps need two things from focus: to know when a control gains or loses it, and to move it.
Both should be declarative — one reactive signal per form, no focus-node objects, no view
references — and both should ride the machinery Day already has: the event sink, origin-tagged
writes, and per-node Toolkit duties.

## 1. The API

Focus binds to a signal, exactly like every other two-way control binding:

```rust
// One field: a Bool signal. Native focus writes it; writing it moves focus.
let editing = Signal::new(false);
text_field(query).focused(editing)

// A form: one Option<K> signal for the whole group (the recommended shape).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Field { User, Pass }
let focus = Signal::new(Some(Field::User));
column((
    text_field(user).focused_eq(focus, Field::User),
    text_field(pass).focused_eq(focus, Field::Pass),
    button("Log in").action(move || {
        if user.with(|s| s.is_empty()) { focus.set(Some(Field::User)); return; }
        …
    }),
))
```

- **Native → signal.** When the control gains focus the signal becomes `true` / `Some(K)`; when
  it loses focus, `false` / `None` (unless another bound control gained it in the same turn, in
  which case the group signal moves straight to that control's value).
- **Signal → native.** Writing `Some(K)` / `true` requests native focus for the bound control.
  Writing `None` / `false` resigns it — on iOS and Android that dismisses the soft keyboard,
  matching the platform convention SwiftUI set.
- **Reading is free.** `focus.get() == Some(Field::User)` is a tracked read like any other —
  no separate "is focused" query, no event wiring (the gap floem's users hit).
- **Field chaining** is a write in `on_submit`: `focus.set(Some(Field::Pass))`.

`on_submit(f)` on text fields ships in the same change: the native end-editing / return hooks
focus needs are the same ones `Event::Submitted` needs, and the pieces layer already has its
(empty) `Submitted` arm.

Reserved for later, with names but no v1 implementation: `.focusable()` (opt a custom piece into
focus), `default_focus(…)` on containers, `focus_order(n)`, and focus scopes for dialogs.
Tab/Shift-Tab traversal stays native — Day wraps real widgets, so platform traversal is already
correct (§13: focus order follows layout order).

## 2. Semantics — the rules

Prior art agrees on the traps (SwiftUI's state/reality divergence, Flutter's node lifecycle,
floem's write/read asymmetry). Day's rules:

1. **Writes are requests.** Focus requests coalesce per turn; the last write in a turn wins, and
   conflicting writes in one turn log a debug warning (SwiftUI's duplicate-binding warning,
   moved to where the mistake happens).
2. **The signal is eventually truthful.** day-core resolves requests at the end of the turn and
   writes the *actual* outcome back into the signal through the echo guard. A request naming a
   disabled, hidden, or unmounted piece leaves platform focus unchanged and the signal snaps
   back to reality. No silent lying state.
3. **One-turn latency is documented.** `focus.set(…)` then `focus.get()` in the same turn reads
   the old value — the same asynchrony every reference framework has, made explicit.
4. **Mount reconciliation.** When a piece bound to `K::V` mounts and the signal already reads
   `Some(K::V)`, it requests focus. Set the signal, then present the sheet: the field focuses
   when it appears. This covers most of what SwiftUI needs `defaultFocus` for.
5. **Echo discipline.** A programmatic focus move fires native focus events; those must not
   re-request. The pieces layer uses the selector's echo-cell pattern; backends additionally
   skip the apply when the control already has focus (the `set_if_changed` layer).
6. **Focus loss without a successor** (user clicked empty space, window resigned) writes
   `false` / `None`. Window-level activation is a separate concern and stays with lifecycle.

## 3. How it rides the existing machinery

| layer | change |
|---|---|
| day-spec | `Event::FocusChanged(bool)` — **already declared**, becomes real. New defaulted Toolkit duty: `fn focus(&mut self, h: &Handle, node: NodeId, focused: bool) {}` (the `scroll_to` / `enable_gesture` shape; default no-op keeps all backends compiling). |
| day-core | Routes `FocusChanged` through the existing per-node handler path (no pump changes). Tracks the resolved focus per window for rule 2/4, and mirrors it into `NodeProbe.focused` for dayscript. |
| day-pieces | `.focused(sig)` / `.focused_eq(sig, K)` on `Decorate` (the `on_tap` template: enable + `cx.on`), echo cell per binding, `on_submit(f)` on `text_field`. |
| day-mock | Logs `focus #N true/false` ops; `MockWidget.focused`; tests drive both directions (`probe.emit(node, FocusChanged(true))`, and signal writes asserting the logged duty). |
| day-script | `focus: { id }` step (drives the real duty, not a synthetic event, so keyboards and Submitted flows engage) and `assert_focused: { id }` reading `NodeProbe.focused`. The script engine still cannot see the native IME (§14.2) — manual keyboard smokes stay. |

## 4. Per-toolkit implementation map

Every backend already delivers per-node events through one sink; focus rides the same channel.
"Observe" is gain/loss detection, "drive" is the request/resign duty.

| backend | observe | drive | notes |
|---|---|---|---|
| AppKit | `controlTextDidBeginEditing`/`DidEndEditing` on the existing `DayTarget` delegate for text fields; one KVO on `NSWindow.firstResponder` (unwrapping the **field editor** back to its delegate field) for everything else | `window.makeFirstResponder(view)` / `(None)` | The first responder for a focused `NSTextField` is the shared field-editor `NSTextView`, never the field itself — map it back via the existing view→target table. Buttons/sliders join the key loop only with Full Keyboard Access on; programmatic focus works regardless. |
| UIKit | `EditingDidBegin`/`DidEnd` (+ `EditingDidEndOnExit` for Submitted) added to the existing `addTarget` wiring | `becomeFirstResponder()` / `resignFirstResponder()` | Focus **is** the keyboard on iOS: request raises it, resign dismisses it. Buttons are not first-responder focusable; the iPadOS `UIFocusSystem` is out of v1 scope. |
| GTK 4 | `EventControllerFocus` (`enter`/`leave`) attached at realize — tracks focus-within, which handles `GtkEntry`'s internal `GtkText` child | `widget.grab_focus()`; resign via `window.set_focus(None)` | Only works once mapped (rule 4's mount reconciliation handles this). `:focus-visible` means keyboard-initiated focus draws a ring, click focus may not — events still fire. |
| Qt 6 | `day_qt_enable_focus(w, id, cb)` — an event filter for `FocusIn`/`FocusOut`, one-for-one with the existing `DayGestureFilter` pattern (no subclassing) | `setFocus(Qt::OtherFocusReason)` / `clearFocus()` | `QFocusEvent::reason()` lets us ignore popup/menu transient focus-out. Button focus policy is style-dependent on macOS (matches the OS convention). |
| Android | `View.OnFocusChangeListener` → `nativeOnEvent(id, 16, hasFocus, null)` (new event kind), attached to the inner `TextInputEditText` via the existing `editTextOf` helper | `requestFocus()` / `clearFocus()` **plus** `InputMethodManager` show/hide for text fields | `requestFocus` alone does not raise the soft keyboard — the duty must pair with IMM. Touch mode: buttons/toggles are focusable only via d-pad/keyboard; don't force `focusableInTouchMode`. Resign needs a focusable root or focus snaps to the first focusable view. |
| WinUI | `GotFocus`/`LostFocus` on each control (system XAML has no global FocusManager event) | `control.Focus(FocusState::Programmatic)`; no clear — resign moves focus to a focusable root | Only `Control` subclasses have `Focus()` in system XAML. `Programmatic` focus draws no focus visual (by design). Island↔Win32 focus handoff (`NavigateFocus`) is out of v1 scope. |
| ArkUI | `NODE_ON_FOCUS` / `NODE_ON_BLUR` added to the shim's event registration + receiver switch | `OH_ArkUI_FocusRequest(node)`; `OH_ArkUI_FocusClear(ctx)` via `OH_ArkUI_GetContextByNode` | The focus NDK header is since API 15; the scaffold targets API 18. `FocusRequest` returns typed errors (non-focusable / non-existent) — exactly what rule 2 wants. `NODE_FOCUS_ON_TOUCH` controls tap-to-focus per node. |
| mock | logged op + `MockWidget.focused` | logged op | The op log is the M1 test contract. |

**Focusability truth table (v1):** text fields and search fields are focusable everywhere; on
desktop, buttons/toggles/sliders/lists too (with the macOS Full-Keyboard-Access and Qt-style
caveats); on touch mobile, non-text controls generally are not. The docs state this per
platform; a runtime `Cap`-style probe is deferred until a real app needs to branch on it.

## 5. Prior art — what Day adopts and rejects

- **SwiftUI** (`@FocusState`, `.focused(_:equals:)`): adopted — the Bool + Optional-of-Hashable
  binding shape, `nil` clears focus and dismisses the keyboard, moved focus writes back on loss.
  Rejected: the unconstructible binding (state can't live outside the view) — Day signals have
  no such wall — and the possibility of state that disagrees with reality (rule 2).
- **Flutter** (`FocusNode`/`FocusScope`): rejected as an API (imperative node lifecycle inside a
  declarative tree is the exact residue Day avoids); adopted as semantics — the scope-restore
  behavior for dialogs informs the reserved focus-scope design, and "focus changes apply after
  the build phase" confirms rule 3.
- **floem** (nearest cousin): its `request_focus(when)` proves signal-driven focus writes work;
  its event-only *reads* are the asymmetry rule 2 closes. Its `FocusNavMeta` (order, group,
  scope) previews the reserved traversal names.
- **iced / egui / Slint / GPUI**: converge on last-write-wins, next-frame application, and
  focus-nowhere being representable — all reflected in the rules. GPUI's open "initial focus"
  discussion is why rule 4 is in v1 rather than deferred.

## 6. Phasing

- **F1 — spec + core + pieces + mock.** The duty, the two modifiers, the rules engine
  (coalesce/resolve/write-back), `NodeProbe.focused`, mock ops, unit tests for the echo loop and
  the unmounted-target snap-back. `Event::Submitted` plumbing in pieces (`on_submit`).
- **F2 — desktop backends.** AppKit (field-editor mapping is the risk item), GTK, Qt. dayscript
  `focus` / `assert_focused` steps; a showcase Controls-page demo (focus follows an enum signal
  across the form) exercised by the walkthrough on all three.
- **F3 — mobile + OHOS.** UIKit and Android (keyboard raise/dismiss paired with the §7.7
  keyboard-insets work — same workstream from the app's view), ArkUI, WinUI in CI. Emulator
  walkthrough steps where the platform allows; manual IME smokes per §14.2.
- **Reserved.** `.focusable()`, `default_focus`, `focus_order`, focus scopes with restore,
  focused-value publication (menus reading the focused field) — a derived signal once focus is
  a signal, so nothing in F1–F3 needs to anticipate it.

## 7. Open questions

1. **Group-signal loss semantics.** When focus moves between two controls bound to the same
   group signal, the signal should transition `Some(A)` → `Some(B)` without an observable `None`
   in between — this needs the end-of-turn resolution to pair the loss+gain events. Confirm the
   event ordering per backend allows it (Qt's `focusChanged(old, now)` does; per-element event
   pairs need turn-level pairing).
2. **Resign target on Android/WinUI.** Both platforms have no true "focus nothing" — decide
   whether Day resigns to the window root (needs a focusable root container) or documents that
   `None` only dismisses the keyboard there.
3. **`focused_eq` naming.** `focused(sig)` + `focused_eq(sig, value)` vs a single overloaded
   name via a marker trait (the `IntoText` two-marker pattern would allow one name). Cosmetic;
   decide at implementation.
4. **Does the walkthrough assert focus on GTK CI?** Headless xvfb windows may never be "active",
   which can suppress focus events — verify early in F2, fall back to assert-on-probe if so.
