---
navTitle: "Focus"
title: "Focus backend contract"
description: "Focus backend contract: retained contributor discussion and its implementation boundaries."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Focus backend contract

> [!NOTE]
> **Status: implementation notes; runtime claims not reverified.** This contributor record preserves earlier design and implementation
> discussion. Examples here are not stable API guidance. See the [app documentation](../focus.md)
> for the current entry point. Platform execution claims are historical and were not rerun in Phase 2.

## 3. How it rides the existing machinery

| layer | what shipped |
|---|---|
| day-spec | `Event::FocusChanged(bool)` became real. New defaulted duty: [`fn focus(&mut self, h: &Handle, node: NodeId, focused: bool) {}`](https://daybrite.dev/api/day_spec/trait.Toolkit.html#method.focus) — the [`scroll_to`](https://daybrite.dev/api/day_core/fn.scroll_to.html)/`enable_gesture` shape. |
| day-core | [`TreeOps::focus_node`](https://daybrite.dev/api/day_core/trait.TreeOps.html#tymethod.focus_node) (clones the handle, calls the duty), `NodeProbe.focused` (mirrored from `FocusChanged` for dayscript), gain-before-loss pairing in the pump, [`enqueue_events`](https://daybrite.dev/api/day_core/fn.enqueue_events.html) batch enqueue. |
| day-pieces | [`Decorate::focused(…)`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.focused) with `IntoFocusBinding` markers for `Signal<bool>` and `(Signal<Option<K>>, K)`; the echo cell; [`on_submit(f)`](https://daybrite.dev/api/day_pieces/trait.TextFieldBuilder.html#tymethod.on_submit) on [`text_field`](https://daybrite.dev/api/day_pieces/fn.text_field.html). |
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
| Android | `View.OnFocusChangeListener` on the inner `TextInputEditText` → event kind 16; `OnEditorActionListener` (IME action or hardware enter key-down) → `Submitted` (kind 17). | `DayBridge.focusView`: `requestFocus()` + `InputMethodManager.showSoftInput` on gain; on resign, `WindowInsetsControllerCompat.hide(Type.ime())` and `clearFocus()` — which lands on `DayActivity`'s focusable-in-touch-mode root instead of snapping to the first focusable field. |
| XAML | `GotFocus`/`LostFocus` per control (the shim was written for system XAML, which has no global focus event; the WinUI build keeps the same wiring) on button, toggle, slider, and text box; `KeyDown` Enter in a `TextBox` is `Submitted`. | `Control.Focus(FocusState::Programmatic)` (draws no focus visual); resign parks focus on an invisible 1×1 `ContentControl` sink (`IsTabStop` flipped around the call, so it never sits in the tab order). |
| ArkUI | `NODE_ON_FOCUS` / `NODE_ON_BLUR` registered on button, text input, toggle, slider, and the canvas host; `NODE_TEXT_INPUT_ON_SUBMIT` is `Submitted`. | `OH_ArkUI_FocusRequest(node)` (typed non-focusable errors ignored — rule 2); resign via `OH_ArkUI_FocusClear(OH_ArkUI_GetContextByNode(node))`, guarded by `NODE_FOCUS_STATUS`. |
| mock | logged op + `MockWidget.focused` | logged op |

**Which controls are focusable.** Text fields are focusable everywhere. On desktop, buttons,
toggles, and sliders are too, with the platform's keyboard-access rules (macOS buttons
join the key loop only with Full Keyboard Access on, and AppKit v1 doesn't observe them; Qt
button focus policy is style-dependent). On touch mobile, non-text controls generally are not
focusable, and the bindings stay quiet there.

**Removing focused controls on Windows GTK.** Day resigns focus before detaching a focused
control or its containing subtree. GTK 4.24 can defer focus cleanup until after the widget
has lost its native surface, leaving the Windows IME message filter registered against a
disposed input. Clearing focus while the subtree is attached lets GTK unregister that filter;
removing an unrelated subtree preserves the current focus.

**An element is its wrappers too.** An id, [`.focusable()`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.focusable) or [`.on_key`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.on_key) often sits on a layout
wrapper ([`.frame(..)`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.frame), [`.padding(..)`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.padding)) above the native view. Focusing it descends to the view,
a focus change or key the toolkit reports on the view reaches the wrapper's handlers and probe,
and [`.on_key`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.on_key) marks the view as well, so a backend claims the key for it.

**A [`canvas`](https://daybrite.dev/api/day_pieces/fn.canvas.html) is focusable on every toolkit**, and observes both ways. It is the
one built-in piece with no native control underneath, so nothing would otherwise make it the
focused element, and without that, keys could never reach what an app draws.
[`Decorate::on_key`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.on_key) is built on this ([docs/menus](../menus.md)): keys follow focus, so a canvas
that has it hears the arrows and a text field that takes it gets them back.

| backend | focusable | takes focus on press |
|---|---|---|
| AppKit | `acceptsFirstResponder` | `mouseDown:`, before forwarding to the gesture recognizers |
| UIKit | `canBecomeFirstResponder` | `touchesBegan:`, likewise |
| GTK 4 | `set_focusable` | a `GestureClick` that calls `grab_focus` |
| Qt 6 | `Qt::StrongFocus` | the focus policy itself (click or tab) |
| Web | a `tabindex` | a `pointerdown` listener |
| XAML | `Control::IsTabStop` on a `ContentControl` host wrapped around the `Canvas` — in system XAML `IsTabStop` and `Focus` are Control members and a Panel is not a Control; WinUI 3 moved both up to `UIElement`, but the WinUI build keeps the same host | `PointerPressed` on the host, which the press reaches by bubbling out of the Canvas; the Panel also takes a transparent `Background`, because an unpainted one is not hit-testable |
| ArkUI | a focusable Stack containing a custom drawing surface and a transparent focus button; the scope's focus request reaches that button ([docs/harmonyos](../harmonyos.md)) | ArkUI's own focus handling |
| Android | `setFocusableInTouchMode`, without which `requestFocus` refuses it during touch input | `dispatchTouchEvent`, before the gesture listener, and only for a canvas the app hung [`.on_key`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.on_key) on: in touch mode a focus move can dismiss a raised soft keyboard |

Each reports `FocusChanged` both ways (`becomeFirstResponder`/`resignFirstResponder`,
`focus`/`blur`, `GotFocus`/`LostFocus`, `NODE_ON_FOCUS`/`NODE_ON_BLUR`, `onFocusChanged`), so [`.focused(signal)`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.focused)
binds two-way and `assert_focused` can see it.

ArkUI's drawing surface and focus button are siblings. A button with custom content becomes
a focus scope during native layout, and the custom drawing node cannot hold focus. Giving
that outer button a label preserves focus but switches it to text measurement, which skips
the drawing surface. A Stack lays out both siblings; its transparent button has an empty
label and reports focus and keys for the canvas. The private button is hidden from
accessibility, which remains on the host. The portable `canvas-focus-after-layout` case
checks drawing and focus on painted and empty canvases after a rendered frame.

**A view that is not in a window cannot hold the keyboard**, and Day does not work around
that. On iOS a full-screen modal takes the presenting view out of the window, so a canvas
behind a compact inspector sheet ([docs/inspector](../inspector.md)) refuses focus until the
sheet closes; the request lapses and the signal snaps back, which is rule 2.

### Automatic keyboard dismissal on mobile

Android observes editor focus per activity window, including text areas and native search
fields. A deferred check keeps the IME open during editor-to-editor focus transfers, but hides
it through `WindowInsetsControllerCompat.hide(Type.ime())` when no visible enabled editor
remains; before API 30 that compat call can leave the keyboard up, so those systems also get
`InputMethodManager.hideSoftInputFromWindow`. The dismissal intent survives late IME-show completion and is checked again after
keyboard-inset layout; a newly focused editor clears it. A layout check covers hidden/disabled
editors; subtree removal ends editing while the
window token is still attached. Navigation fragments, departing tabs and dismissed covers end
editing on the outgoing surface. The observer is detached with its window and does not hide
the IME in a different window or transient popup. WebViews are conservatively treated as editor
owners because HTML editing need not expose a native EditText.

UIKit uses `endEditing(false)` on departing page/cover controllers, removed subtrees and hidden
document pages. The call stays scoped to the outgoing surface and respects refusal to resign.
Native focus events continue to update [`.focused(...)`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.focused) bindings. No app opt-in is required.

Do not dismiss on every button tap, every reactive rebuild, or every Return key: formatting
buttons, validation errors, multiline editors and Next-field chaining need to keep editing.
When a submit action leaves its form visible, the app can clear its focus binding explicitly.
When it replaces the form or navigates away, the framework dismisses automatically.

## 5. Testing it

`focused-inputs-survive-page-removal` repeatedly removes and remounts a focused field's page,
then checks that an unrelated surviving field keeps focus and accepts input. GTK's
`native_focus_removal` test also covers entries, spin buttons and text views through the
backend's remove/release paths. Run it on a Windows desktop with `G_DEBUG=fatal-criticals`
to catch stale IME callbacks as well as incorrect focus state.

- **Native IME regression:** Day-News `scripts/test-android-keyboard.py` runs on a chosen
  emulator with an isolated synthetic account, checking Android's actual `mInputShown`
  state for field transfer, explicit release, navigation pop, login-form replacement and
  cover dismissal. Its screenshots show the keyboard and the recovered settings surface.
  UIKit uses the equivalent simulator flow and checks focus plus rendered keyboard visibility.


- **Unit (mock):** two-way Bool binding, group moves without a `None` blip, mount-time
  `Some(K)` requests focus, `on_submit` fires (`day-pieces/tests/mock_e2e.rs`).
- **dayscript:** the showcase walkthrough's Focus block runs `focus` / `assert_focused` against
  the real duty on every scripted platform (macOS AppKit/GTK/Qt, iOS, Android: 208/208), with
  assertions kept to text fields, the one control focusable everywhere.
- **Showcase:** the Focus page (`Day-Showcase/src/pages/focus.rs`) demonstrates the group
  signal steering a form (with Return chaining), the plain Bool binding, and non-text-control
  focus with a live readout.

## 8. Keyboard avoidance (the keyboard never covers the focused field)

Each mobile backend consumes the soft keyboard natively and resizes the Day root through the
`WindowResized` event (the same relayout path a rotation takes), so the whole UI shrinks to the
visible area and the focused field is scrolled back into view:

- **Android**: the root's wrapper folds `WindowInsetsCompat.Type.ime()` into the bottom margin
  (alongside the system-bar insets), so a raised keyboard shrinks the root exactly like a taller
  navigation bar would. The resize flows to Day (`DayFixed.onSizeChanged` → `WindowResized`),
  Day relayouts, and the platform `ScrollView` then applies its stock resized-with-focus
  behavior, scrolling the focused descendant back into view.
- **iOS**: the app delegate observes `UIKeyboardWillChangeFrame`, clamps the root view's bottom
  to the keyboard's top, emits `WindowResized`, and after Day's relayout reveals the focused
  `UITextField` via `scrollRectToVisible` on its nearest enclosing `UIScrollView`. Show, hide,
  and height changes (emoji pane, hardware-keyboard toggles) all ride the one notification.
- **HarmonyOS**: the window's UIContext is set to `KeyboardAvoidMode.RESIZE`, so ArkUI shrinks
  the page instead of translating it; the host page's `onAreaChange` forwards the new area
  through the `resized()` NAPI export, and Day relayouts into it.

Desktop backends have no soft keyboard, so nothing engages. Avoidance needs no per-app opt-in;
every Day app gets it. The related programmatic primitive is [`TreeOps::scroll_reveal`](https://daybrite.dev/api/day_core/trait.TreeOps.html#tymethod.scroll_reveal)
([docs/scroll](../scroll.md)), which scrolls any element's nearest scroll ancestor into view.

## Verification scope

This retained technical material has not received a complete signature, default, lifecycle,
and native-runtime review against the 2026-10-22 working tree. Treat those details and unmarked
code examples as unverified for this revision. The
[technical-review ledger](https://github.com/daybrite/day/blob/main/docs/history/documentation/coverage.csv)
records its status. Code blocks are API excerpts or integration fragments unless explicitly
designated in the [example catalog](/docs/reference/examples/catalog#technical-recipes).
No current all-platform pass or screenshot attribution is established by this page.
