---
title: Accessibility
description: "What native widgets give you for free, the annotations Day adds, and how CI verifies the native accessibility tree."
order: 22
section: Guides
---

Accessibility is where Day's native-widget premise pays off most directly. A Day button is a real
`NSButton`, a real Material button, a real `GtkButton` — so VoiceOver, TalkBack, Narrator, and
Orca already know how to focus it, name its role, and activate it. You start from the platform's
baseline instead of from zero, which is the opposite of the situation in renderer-based
frameworks, where every accessible behavior must be reimplemented.

That baseline still needs your input in three places: labels for things whose purpose isn't their
text, roles for things you drew yourself, and stable identifiers for automation. Day gives all
three one API.

## Annotating pieces

```rust
// An icon-only button: its accessible name can't be derived, so provide one.
button("✕")
    .a11y(|a| a.label(tr("close")))
    .id("close-button")

// A custom-drawn gauge: no native control underneath, so declare role and value.
canvas(move |d, size| draw_gauge(d, size, value.get()))
    .frame(120.0, 120.0)
    .a11y(|a| a.role(Role::Meter).label(tr("cpu-usage")))

// Decorative artwork: remove it from the accessibility tree entirely.
image("hero-banner").a11y(|a| a.decorative())
```

The builder covers `label`, `hint`, `value`, `role`, `hidden`, and `decorative`. Labels are
ordinary text values, so `tr(...)` works — accessible names are localized like everything else.
Built-in Pieces set sensible defaults (a `toggle` is a switch with its title as its name); your
annotations merge over those defaults.

Two rules Day enforces rather than suggests, via `day lint`:

- an interactive Piece with no derivable label is a warning (an error with `--strict`);
- an element id leaking into an accessible *label* is an error — ids are for machines, labels are
  for people, and screen readers reading `"save-button"` aloud is the bug this catches.

## Identifiers

`.id("save-button")` sets a stable identifier used by three consumers: [dayscript](/docs/dayscript)
element targeting, external automation tools, and Day's own diagnostics. For external tools the
platform mapping is uneven, and it's worth knowing the truth rather than assuming:

| Platform | Identifier surface |
|---|---|
| iOS / macOS | `accessibilityIdentifier` — full support (XCUITest etc.) |
| Windows (WinUI, Qt) | UIA `AutomationId` — full support |
| Android | `uniqueId`, API 33+ only; older versions expose no automation id to Appium/UiAutomator |
| GTK | no public settable AT-SPI id today — inspector-visible only |
| Web | DOM `id` |

dayscript is unaffected by this table — it resolves ids inside the app, uniformly everywhere.
The table matters only when pointing external tooling (Appium, UIA scrapers) at a Day app.

## Verifying, not trusting

Setting an attribute and the platform *exposing* it are different things, so Day includes an
audit step that reads the native accessibility tree back and diffs it against what your code
declared:

```yaml
# in a dayscript flow
- a11y_audit:
```

The audit walks id'd nodes, asks the toolkit for the realized role, label, and value (via each
backend's read-back hooks), and fails the script on mismatch. Run in CI, this turns "we set the
labels" into a regression-tested claim. Read-back is implemented on the Apple backends and Qt;
backends that can't yet read their native tree (Android, GTK) skip rather than fake it.

## Current limits, plainly

- **GTK off Linux has no accessibility tree.** GTK's AT-SPI bridge is Linux-only, so the
  `macos-gtk` and `windows-gtk` development combos are invisible to screen readers. Ship the
  platform-native target for real users.
- **Android annotations are partial** — labels and values map today (`contentDescription`, state
  description); role and hint refinement is still open, and audit read-back isn't implemented.
- **Qt has the strongest cross-OS story** (its `QAccessible` layer bridges to the native
  accessibility API on every OS), which makes `linux-qt` a reasonable choice when Linux
  accessibility is a hard requirement.
- **Reactive values**: an a11y `value` set at build time is a snapshot; live values (a slider
  announcing as it moves) work through the control's native behavior, but custom reactive a11y
  values on canvas pieces are still a designed-not-built refinement.
- **Focus order** follows layout order; explicit focus groups and custom sort priority aren't
  exposed yet.

None of these limits are hidden in the tooling: `day doctor` and the audit step report what each
target actually supports. The [accessibility reference](/docs/internal/accessibility) has the
full per-backend mapping tables.
