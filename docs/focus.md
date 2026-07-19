# Focus

> Status: **implemented** on every backend — AppKit, UIKit, GTK, Qt, Android, WinUI, ArkUI, and
> mock. `Event::FocusChanged(bool)` (reserved by §8.3) is real, `Toolkit::focus` is the duty
> behind it, and DESIGN §4.4's controlled-input rule ("the native widget is the source of truth
> **while it has focus**") now rests on actual focus knowledge. The showcase's Focus page
> exercises every permutation below; the walkthrough asserts it on every scripted platform.

Day apps need two things from focus: to know when a control gains or loses it, and to move it.
Both are declarative — one reactive signal per form, no focus-node objects, no view references —
and both ride the machinery Day already has: the event sink, origin-tagged writes, and per-node
Toolkit duties.

## 1. The API

Focus binds to a signal with `.focused(…)`, exactly like every other two-way control binding.
One name covers both shapes (a marker-trait overload, the `IntoText` pattern):

```rust
// One field: a Bool signal. Native focus writes it; writing it moves focus.
let editing = Signal::new(false);
text_field(query).focused(editing)

// A form: one Option<K> signal for the whole group (the recommended shape).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Field { User, Pass }
let focus = Signal::new(Some(Field::User));
column((
    text_field(user)
        .on_submit(move || focus.set(Some(Field::Pass)))
        .focused((focus, Field::User)),
    text_field(pass).focused((focus, Field::Pass)),
    button("Log in").action(move || {
        if user.with(|s| s.is_empty()) { focus.set(Some(Field::User)); return; }
        …
    }),
))
```

- **Native → signal.** When the control gains focus the signal becomes `true` / `Some(K)`; when
  it loses focus, `false` / `None` — unless another bound control gained it in the same turn, in
  which case the group signal moves straight to that control's value (no `None` in between).
- **Signal → native.** Writing `Some(K)` / `true` requests native focus for the bound control.
  Writing `None` / `false` resigns it — on iOS and Android that dismisses the soft keyboard,
  matching the platform convention SwiftUI set.
- **Reading is free.** `focus.get() == Some(Field::User)` is a tracked read like any other —
  no separate "is focused" query, no event wiring.
- **Field chaining** is a write in `on_submit(f)`, which ships with the same change: the native
  end-editing / return hooks focus needs are the ones `Event::Submitted` needs.

Reserved with names but not implemented: `.focusable()` (opt a custom piece into focus),
`default_focus(…)` on containers, `focus_order(n)`, and focus scopes for dialogs. Tab/Shift-Tab
traversal stays native — Day wraps real widgets, so platform traversal is already correct
(§13: focus order follows layout order).

## 2. Semantics — the rules

1. **Writes are requests.** A write asks the toolkit on the next main-loop turn; the last write
   in a turn wins.
2. **The signal converges on reality.** Backends report the *resulting* state through
   `Event::FocusChanged`, and that report is what lands in the signal (through the echo guard).
   A request naming a target the platform will not focus produces no event — platform focus is
   unchanged, and the next real focus event corrects the signal.
3. **One-turn latency.** `focus.set(…)` then `focus.get()` in the same turn reads the old
   value — the same asynchrony every reference framework has, made explicit.
4. **Mount reconciliation.** When a piece bound to `K::V` mounts and the signal already reads
   `Some(K::V)`, it requests focus. Set the signal, then present the sheet: the field focuses
   when it appears. (The initial `false`/`None` is *not* applied — resigning focus a control
   never had would steal it from whoever has it.)
5. **Echo discipline.** A programmatic focus move fires native focus events; those must not
   re-request. The pieces layer keeps a per-binding echo cell of the native state and skips
   applies that already match it; backends additionally skip resigns when the control doesn't
   own focus (so a stale release can't blur a sibling).
6. **Loss and gain pair up.** When focus moves between two controls bound to the same group
   signal, the event pump dispatches the queued gain *before* the loss it arrived with, so the
   signal transitions `Some(A)` → `Some(B)` without an observable `None`. A loss with no queued
   successor (click on empty space, window resigned) writes `false` / `None`.

## 3. How it rides the existing machinery

| layer | what shipped |
|---|---|
| day-spec | `Event::FocusChanged(bool)` became real. New defaulted duty: `fn focus(&mut self, h: &Handle, node: NodeId, focused: bool) {}` — the `scroll_to`/`enable_gesture` shape. |
| day-core | `TreeOps::focus_node` (clones the handle, calls the duty), `NodeProbe.focused` (mirrored from `FocusChanged` for dayscript), gain-before-loss pairing in the pump, `enqueue_events` batch enqueue. |
| day-pieces | `Decorate::focused(…)` with `IntoFocusBinding` markers for `Signal<bool>` and `(Signal<Option<K>>, K)`; the echo cell; `on_submit(f)` on `text_field`. |
| day-mock | `focus #N true/false` op log + `MockWidget.focused` — the F1 test contract. The mock duty does **not** synthesize `FocusChanged` back; tests emit events explicitly. |
| day-script | `focus: { id, focused? }` drives the real duty (keyboards and end-editing flows engage) and `assert_focused: { id, focused? }` reads `NodeProbe.focused` (retryable — focus lands a turn after the request). |

## 4. Per-toolkit implementation

"Observe" is gain/loss detection, "drive" is the request/resign duty.

| backend | observe | drive |
|---|---|---|
| AppKit | text fields: `DayTextField` overrides `becomeFirstResponder` (gain); `controlTextDidEndEditing:` on the `DayTarget` delegate (loss — except `NSTextMovementReturn`, which is `Submitted`: AppKit keeps the field first responder on return). Non-text controls drive but don't observe in v1. | `makeFirstResponder(view)` / `(None)`. The resign guard unwraps the shared **field editor** back to its delegate field before deciding ownership. |
| UIKit | `EditingDidBegin` / `EditingDidEnd` targets on the text field; `EditingDidEndOnExit` is `Submitted` (registering it is also what makes Return dismiss the keyboard). | `becomeFirstResponder()` / `resignFirstResponder()` — focus **is** the keyboard on iOS. Buttons are not first-responder focusable. |
| GTK 4 | `EventControllerFocus` (`enter`/`leave`) on entry, button, switch, and scale — it tracks focus-within, which covers `GtkEntry`'s inner `GtkText`. `Entry::activate` is `Submitted`. | `grab_focus()`, retried once at `map` if the widget isn't mapped yet (rule 4); resign via `root.set_focus(None)`, only while the widget holds focus-within. |
| Qt 6 | `day_qt_enable_focus` — a `FocusIn`/`FocusOut` event filter (the `DayGestureFilter` pattern) on line edit, button, checkbox, and slider; popup-reason focus-outs are ignored (menus are transient). `returnPressed` is `Submitted`. | `setFocus(Qt::OtherFocusReason)` / `clearFocus()` (only while focused). Qt delivers focus events only in the *active* window, so the duty activates it first — via the OS when allowed, else app-locally (`QApplication::setActiveWindow`, kept by Qt for exactly this driving/embedding case). |
| Android | `View.OnFocusChangeListener` on the inner `TextInputEditText` → event kind 16; `OnEditorActionListener` (IME action or hardware enter key-down) → `Submitted` (kind 17). | `DayBridge.focusView`: `requestFocus()` + `InputMethodManager.showSoftInput` on gain; on resign, hide the IME and `clearFocus()` — which lands on `DayActivity`'s focusable-in-touch-mode root instead of snapping to the first focusable field. |
| WinUI | `GotFocus`/`LostFocus` per control (system XAML has no global focus event) on button, toggle, slider, and text box; `KeyDown` Enter in a `TextBox` is `Submitted`. | `Control.Focus(FocusState::Programmatic)` (draws no focus visual — by design); resign parks focus on an invisible 1×1 `ContentControl` sink (`IsTabStop` flipped around the call, so it never sits in the tab order). |
| ArkUI | `NODE_ON_FOCUS` / `NODE_ON_BLUR` registered on button, text input, toggle, and slider; `NODE_TEXT_INPUT_ON_SUBMIT` is `Submitted`. | `OH_ArkUI_FocusRequest(node)` (typed non-focusable errors ignored — rule 2); resign via `OH_ArkUI_FocusClear(OH_ArkUI_GetContextByNode(node))`, guarded by `NODE_FOCUS_STATUS`. |
| mock | logged op + `MockWidget.focused` | logged op |

**Focusability in practice:** text fields are focusable everywhere. On desktop, buttons,
toggles, and sliders are too — with the platform's own keyboard-access rules (macOS buttons
join the key loop only with Full Keyboard Access on, and AppKit v1 doesn't observe them; Qt
button focus policy is style-dependent). On touch mobile, non-text controls generally are not
focusable, and the bindings stay quiet there.

## 5. Testing it

- **Unit (mock):** two-way Bool binding, group moves without a `None` blip, mount-time
  `Some(K)` requests focus, `on_submit` fires — `day-pieces/tests/mock_e2e.rs`.
- **dayscript:** the showcase walkthrough's Focus block runs `focus` / `assert_focused` against
  the real duty on every scripted platform (macOS AppKit/GTK/Qt, iOS, Android — 208/208), with
  assertions kept to text fields, the one control focusable everywhere.
- **Showcase:** the Focus page (`apps/showcase/src/pages/focus.rs`) demonstrates the group
  signal steering a form (with Return chaining), the plain Bool binding, and non-text-control
  focus with a live readout.

## 6. Prior art — what Day adopted and rejected

- **SwiftUI** (`@FocusState`, `.focused(_:equals:)`): adopted — the Bool + Optional-of-Hashable
  binding shape, `nil` clears focus and dismisses the keyboard, moved focus writes back on loss.
  Rejected: the unconstructible binding (state can't live outside the view) — Day signals have
  no such wall.
- **Flutter** (`FocusNode`/`FocusScope`): rejected as an API (imperative node lifecycle inside a
  declarative tree); adopted as semantics — "focus changes apply after the build phase" is
  rule 3, and the scope-restore behavior informs the reserved focus-scope design.
- **floem** (nearest cousin): its `request_focus(when)` proved signal-driven focus writes work;
  its event-only *reads* are the asymmetry rule 2 closes.
- **iced / egui / Slint / GPUI**: converge on last-write-wins, next-frame application, and
  focus-nowhere being representable — all reflected in the rules.

## 7. Resolved design questions

1. **Group-signal loss semantics** — solved in the pump: a `FocusChanged(false)` scans the
   queue for a paired `FocusChanged(true)` from another node and dispatches the gain first, so
   group signals never blip through `None` (rule 6).
2. **Resign target on Android/WinUI** — both resign to a focusable root: Android to the
   activity's focusable-in-touch-mode content root, WinUI to a hidden focus-sink control.
3. **`focused_eq` naming** — one `.focused()` name via two marker-trait impls; the tuple form
   `(signal, key)` replaces a second method.
4. **Headless CI focus** — programmatic `grab_focus`/`setFocus` is window-local on every
   backend (Qt after the app-local activation above), so scripted runs don't depend on the
   window manager granting real OS focus.

## 8. Keyboard avoidance (the keyboard never covers the focused field)

Each mobile backend consumes the soft keyboard natively and resizes the Day root through the
`WindowResized` rail — the same relayout path a rotation takes — so the whole UI shrinks to the
visible area and the focused field is scrolled back into view:

- **Android**: the root's wrapper folds `WindowInsetsCompat.Type.ime()` into the bottom margin
  (alongside the system-bar insets), so a raised keyboard shrinks the root exactly like a taller
  navigation bar would. The resize flows to Day (`DayFixed.onSizeChanged` → `WindowResized`),
  Day relayouts, and the platform `ScrollView` then applies its stock resized-with-focus
  behavior — scrolling the focused descendant back into view.
- **iOS**: the app delegate observes `UIKeyboardWillChangeFrame`, clamps the root view's bottom
  to the keyboard's top, emits `WindowResized`, and after Day's relayout reveals the focused
  `UITextField` via `scrollRectToVisible` on its nearest enclosing `UIScrollView`. Show, hide,
  and height changes (emoji pane, hardware-keyboard toggles) all ride the one notification.
- **HarmonyOS**: the window's UIContext is set to `KeyboardAvoidMode.RESIZE`, so ArkUI shrinks
  the page instead of translating it; the host page's `onAreaChange` forwards the new area
  through the `resized()` NAPI export, and Day relayouts into it.

Desktop backends have no soft keyboard; nothing engages. There is no per-app opt-in — a Day app
gets avoidance by existing. The related programmatic primitive is `TreeOps::scroll_reveal`
(docs/scroll.md), which scrolls any element's nearest scroll ancestor into view.
