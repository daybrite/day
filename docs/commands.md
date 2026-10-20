---
title: "Reusable commands"
description: "Define an application operation once and present it in buttons, menus, and toolbars."
---

<!-- Copyright © The Daybrite Project. SPDX-License-Identifier: CC-BY-SA-4.0 -->

# Reusable commands

`day::Command` is an application operation: one id, one title, one availability, one handler.
`Command::new(id, label, action)` builds it, and clones share its behavior and presentation
metadata. The type is in the prelude and lives in `day-pieces`, above the toolkit-neutral menu,
toolbar, and button models. There is no
new backend protocol, global action registry, or toolkit dependency. Each presentation keeps its
normal native widget and dispatch registration; the command shares Rust behavior and state.

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](snippet-catalog.md) and [catalog setup](https://daybrite.dev/docs/localization).

## Define once, present in several places

```rust
use day::prelude::*;

fn counter() -> impl Piece {
    let count = Signal::new(0);
    let add = Command::new(id!(add), res::str::add_one(), move || {
        count.update(|n| *n += 1)
    })
    .enabled(move || count.get() < 3)
    .icon(Symbol::Add)
    .shortcut(Shortcut::primary("+"));
    let menu = add.clone();
    let toolbar = add.clone();
    column((
        label(res::str::counter_value(count)).id(id!(counter_label)),
        add.button(),
        label(res::str::more_actions()).context_menu(move || vec![menu.menu_item()]),
    ))
    .toolbar(move || vec![toolbar.toolbar_item()])
}
```

The generated title accepts the same `IntoText`
inputs as a button: localized text, strings, signals, and `Fn() -> String`. Availability and
optional check state accept booleans, signals, or closures. Reactive reads are tracked when the
presentation evaluates them. A changing label can therefore say “Star” or “Unstar” from the same
state that supplies `.checked(move || starred.get())`.

| API | Contract |
|---|---|
| `Command::new(id, label, action)` | The three required inputs, type-distinct so they cannot be swapped; captures the current reactive scope as the command's owner. |
| `.enabled(value)` | Availability, default true; checked again before **every** invocation. |
| `.checked(value)` | Optional check state; the handler decides whether and how to change it. |
| `.icon(Symbol)` / `.image(name)` | Default icon for all presentations that can draw it. |
| `.shortcut(Shortcut)` | Menu accelerator metadata; creating a command alone installs no shortcut. |
| `.id()`, `.label()`, `.is_enabled()`, `.is_checked()` | Read metadata/current state for custom presentations; check state is `Option<bool>`. |
| `.invoke()` | Return true if the handler ran, false if unavailable or its owner scope was disposed. This is not an operation-success result. |
| `.button()` | Live title and availability; native push button with the command's default id and icon. |
| `.menu_item()` | Snapshot of title, availability, check, shortcut and icon; guarded handler. |
| `.toolbar_item()` | Toolbar button, or native toggle when checkable; live availability/check bindings and guarded handler. |

Optional modifiers and presentation methods chain onto the `Command`. The constructor's
parameters are generic: ids accept `Into<String>`, labels accept `IntoText`, and actions accept
capturing `Fn()` closures without manual `Rc` wrapping or a `Clone` requirement. Command
factories return `Command`, so callers need not name closure types:

```rust
fn refresh_command() -> Command {
    Command::new(id!(refresh), res::str::refresh(), refresh_document).icon(Symbol::Refresh)
}
```

The handle's builders configure values. Handle clones share their handler and reactive sources, but calling a
builder on a clone does **not** mutate previously created commands or controls. Change a signal
read by the definition to update existing presentations.

## Reactive presentation and ids

Content buttons bind automatically. Use a **derived** toolbar for changing labels or locale:

```rust
let bar_command = save.clone();
content.toolbar(move || vec![bar_command.toolbar_item().id(id!(save_toolbar))]);
```

Toolbar availability and check state also bind in a fixed list, but its title is a snapshot.
A derived toolbar uses the existing contribution replacement/rebinding machinery. It is not a
new per-frame binding or a timer. The app menu is live (a `Command::menu_item` re-lowers the
menu when its title or availability changes); a context menu is built per summon from a
closure:

```rust
// Construct application-menu commands in application scope, not a disposable page.
let menu_save = save.clone();
app_menu(move || vec![sub_menu(res::str::file(), vec![menu_save.menu_item()])]);
```

A `context_menu(vec![...])` keeps its installed snapshot until replaced.
Even a stale enabled snapshot cannot execute an unavailable command: the registered callback
always calls `invoke()`. Do not override the adapter's `.action(...)` if you want that guarantee.
Presentation-specific `.enabled(...)` overrides likewise do not replace the command's guard.

The command id is a default UI id, not a globally unique dispatch token. Buttons, menus, and
toolbars retain their existing separate dispatch rules. Give two buttons in the same tree
separate ids (`command.button().id(id!(save_footer))`); keep toolbar ids unique within their chrome
and menu ids unambiguous in the installed menu. Toolkit ids remain internal. Sharing a command
does not register a second application-wide keyboard handler or resolve accelerator conflicts.
Install a shortcut in one appropriate menu; do not assign the same accelerator to unrelated
operations. Showcase's screenshot command uses primary+Alt+S, clear of File > Save.

## Ownership, windows, and checked commands

A command records `Scope::current()` in `Command::new`. Build it in the scope that owns the
operation. Constructing it does not extend the lifetime of the signals its handler captures;
those must still outlive its use. After disposal it is unavailable, `invoke()`
is a no-op, its title reads empty, and an optional check reads false. This prevents an escaped
command from touching its captured disposed signals. A retained clone still retains its captured
Rust resources until dropped; it does not keep the reactive scope alive. Native registrations and
bindings remain owned by their normal menu, toolbar contribution, or piece scopes.

Capture a particular window's scene or row key for window/row commands. For an application-menu
operation that should target the front window, explicitly resolve `Scene::focused()` (or the
app's equivalent) **inside the state predicates and handler**. Commands do not implicitly change
focus, select a document, or retarget captured state. Two commands may have the same semantic id
and act on independent window scenes. Build reusable command handles in their owning scope,
not a short-lived rendering pass that should not own the operation.

A check is a read-only projection of app state. The command's handler owns changes. A toolbar
toggle request invokes the operation once when its requested state differs from the current
check, then reads the resulting check back into the native widget, including when the handler
declines to change state. Setting the already-current state is a no-op. This avoids an extra
mirrored `Signal<bool>` and corrects toolkits that optimistically flip a toggle. The boolean
payload expresses intent; the command never assigns its state directly. Use `toolbar_toggle` for
a two-way boolean setter. A push button does not display checked state; use a changing title/icon,
a separate toggle, or the menu/toolbar presentation.

## Limits and platform behavior

This is an app-operation abstraction, not an implementation of native responder-chain editing.
Keep `menu_role(MenuRole::Copy/Undo/...)` for focused text editing and OS commands. It is also not
a command palette, undo stack, global hotkey manager, async executor, or automatic busy/error
state machine. Start async work with the existing task APIs and include your own busy signal in
`.enabled(...)`. Direct custom gestures can call `.invoke()`; they must separately present the
command's title/availability/accessibility state. No automatic visibility or gesture styling is
implied. Arguments can be captured by typed Rust command factories, without an erased payload API.

| Toolkit | Presentation details and limitations |
|---|---|
| AppKit | Native `NSButton`, `NSMenuItem`, and toolbar items/toggles. Primary shortcuts use Command. Standard edit roles still follow native focus. |
| UIKit | Native buttons, context `UIMenu`/`UIAction`, and navigation-bar toolbar contributions. Main menus depend on the iPad/Catalyst/hardware-keyboard context; never make an operation reachable only from an app menu on iPhone. |
| GTK | `GtkButton`, stateful menu `GAction`, and header-bar controls. Header bars are window chrome; they do not reproduce AppKit's separate per-column toolbar placement. |
| Qt | Buttons, menu `QAction`, and `QToolBar` actions. Day creates separate native presentations; this does not expose or share a native `QAction` object. Icon availability follows the platform theme and Day's fallback mapping. |
| Android MDC | Material buttons and app-bar items/overflow. Global menu shortcuts are not a portable hardware-keyboard binding; overflow icons may be omitted and deeper menu nesting flattens. |
| Windows XAML | Native buttons, `MenuBar`/`MenuFlyout`, and `CommandBar` items. Primary means Ctrl. Command availability is Day's predicate, not a public WinUI `ICommand`; existing placement and icon limitations apply. |
| Web DOM | DOM buttons, composed toolbars and per-piece context menus. No native application menu/OS-wide accelerator installation; browser-reserved shortcuts remain the browser's. Provide visible buttons. |
| ArkUI | Buttons can invoke commands and follow availability. Unsupported application-menu/toolbar surfaces remain unsupported: commands do not fabricate an OS menu or bar. Keep content buttons for those operations. |

The detailed, evolving matrices live in [menus](menus.md), [toolbars](toolbars.md), and
[buttons](buttons.md). This API uses their existing spec models and event routing, on all targets.
Windows behavior requires Windows runtime validation; an OpenHarmony compile-check does not
claim emulator validation. `dayscript toolbar:` drives the model and is not proof that native
chrome rendered; inspect screenshots too.

## Sample-app audit and migrations

All eight `Day-*` sample applications and Games-Fair were reviewed. This is an operation-level
refactor, not a mechanical replacement of every one-off `.action(...)`.

| Application | Findings and changes |
|---|---|
| Day-Showcase | Replaced the private command struct with Day commands for star, appearance, pseudo-locale, screenshots and recording/playback/clear. Menus use adapters; sidebar star and page toolbar use a bound page command; toolbar star no longer needs a mirror signal. Scripting playback uses the same guard/handler as the menu. Added the Menus & dialogs counter example across buttons, toolbar, and dynamic context menu, with localized explanation and Dayscript coverage. |
| Day-News | Shared Refresh and Mark All Read definitions now supply menu/toolbar/content fallback controls. The mobile Mark All Read button now uses the same unread-availability guard as chrome. Subscription setup's follow-up refresh remains ordinary domain logic. |
| Day-Sketch | Three zoom definitions now own labels, icons, localized accelerators, and handlers for both View menu and toolbar. Standard undo/copy/paste roles and canvas gestures remain native/domain-specific. Grouping/arrangement are further candidates, but their context-selection rules should stay explicit. |
| Day-Trader | Add Symbol is one definition used by the Symbols menu and toolbar, preserving the toolbar's script id and primary placement. Reloads following a settings mutation remain domain logic rather than a second user command. |
| Day-Rise | Add/Done/Delete/Show Done are candidates. Existing app-menu operations resolve the focused scene, while page toolbars capture their own scene; that distinction is intentional. Kept this code and the list's row-specific/swipe setters rather than replacing them with an app-global command. |
| Day-Tunes | Playback/stop/skip/favorite and catalog refresh are strong candidates; the player is app-wide, unlike each window's selected station. Kept its current transport/two-way controls in this pass. A later migration can bind commands to the app player while retaining those eligibility distinctions. |
| Day-Skies | Most handlers are parameterized per-city navigation/editing or settings side effects. Kept these direct handlers; a future user-facing refresh shared with chrome would warrant a command. |
| Day-Bench | Benchmark controls mostly start distinct workloads or write parameters. Kept one-off actions/two-way bindings so the benchmark workload and measurement remain unchanged. |
| Games-Fair | A gamekit Close command now powers the canvas header and 18 pause/results Quit buttons across nine games. Quit routes through the same shell close hook as the header; game-specific ids/colors remain. Custom Sudoku/Match Three controls and gameplay moves remain direct domain/gesture operations. |

The new mock regressions in `crates/day-pieces/tests/mock_e2e.rs` cover shared reactive labels,
checked state, all invocation paths, stale enabled snapshots, scope disposal, independent captured
state, and native toggle rejection. Showcase's `dayscript/commands.yaml` covers button/toolbar
interoperation, revisiting the page, shared menu/toolbar check state, and repeated desired-state
requests. Existing app walkthroughs cover migrated operations.

## Assessment: named definitions for other components

This section recorded an assessment (2026-09) of named-field definitions for other
constructors, with the then-current `Command { id, label, action }.build()` as the model. The
outcome is the house style in [docs/api-style.md](api-style.md): a constructor keeps up to
three positional, type-distinct arguments (`Command::new(id, label, action)` is one), coupled
configuration is one value with its own builder (`Search`, `ContentList`, `Reorder`, `Delete`,
`Move`), items are values (`item(..)`, `toolbar_button(..)`, `menu_item(..)`, `(value, title)`
picker options), and named fields are reserved for same-typed arguments, where the generated
`res::str` argument structs use them ([docs/localization.md](localization.md)). No
`.into_piece()`/`.into_entry()` wrappers were added.
