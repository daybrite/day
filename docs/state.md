---
title: "App state"
description: "Where state lives when an app has more than one window: Ambient values, the app scope, the window scope, and the focused-window rule for menu bars."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# App state

Choose a state’s owner by how long the value should live and who should share it. A text
field’s temporary input may belong to one piece. A document selection usually belongs to a
window. A login session may belong to the whole app.

| Scope | Lifetime | API | Typical use |
|---|---|---|---|
| Piece | Until its owning scope is disposed | `Signal::new` | Input or expanded state within a component |
| Window | Until the window closes | `T::scoped(…)`, then `T::ambient()` | A document’s selection and current page |
| App | Until the process exits | `T::app()` | A session or cache shared by windows |

For window state, use `Ambient` as shown below. This uses Day’s reactive scopes and window
registry, so the ownership rules are shared by all backends. The `Ambient` tests in
[`mock_e2e.rs`](../crates/day-pieces/tests/mock_e2e.rs) cover the behavior.

## The shape: a `Copy` struct of handles

```rust
use day::prelude::*;

/// State shared by the pieces in one window.
#[derive(Clone, Copy)]
struct Scene {
    items: Store<Keyed<Item>>,
    selected: Signal<Option<u32>>,
    section: Signal<Section>,
}

impl Ambient for Scene {
    fn create() -> Self {
        Scene {
            items: Store::new(Keyed::default()),
            selected: Signal::new(None),
            section: Signal::new(Section::Home),
        }
    }
}
```

`Signal`, `Memo`, `Trigger`, and `Store` are pointer-sized `Copy` handles. Copying `Scene`
copies those handles, so event handlers and page functions can access the same state without
cloning the underlying values. `Item` and `Section` in this example are app-defined types.

## Providing it

```rust
pub fn root() -> impl Piece {
    day::register_new_window(|| window_shell());   // use the same builder for each new window
    app_menu(menus());
    window_shell()
}

fn window_shell() -> impl Piece {
    Scene::scoped(move |scene| my_ui(scene))            // one Scene per window
}
```

Every piece built inside reads it back by type:

```rust
fn my_page() -> impl Piece {
    let scene = Scene::ambient();
    label(move || scene.selected.get().map(|i| i.to_string()).unwrap_or_default())
}
```

Pass `Scene` as an argument when the function allows it. Use `Scene::ambient()` when a
callback signature cannot accept state, such as a navigation page builder. It finds the
`Scene` provided by the enclosing scope.

For state that belongs to the whole app rather than to a window (a login session, a
sync engine, a document cache several windows share), use `T::app()` instead. It creates the value
on the reactive root scope the first time anything asks and returns that instance for the rest of the process,
from any window, any menu action, and any task.

### Two rules

**`ambient()` resolves while a piece builds.** Read it in the piece's body and capture the value;
calling it inside a reactive closure works on the first run and panics on the next, because a
re-running reaction is no longer inside the scope that provided the value.

```rust
let scene = Scene::ambient();                       // ✅ read once, at build
label(move || format!("{:?}", scene.selected.get()))

label(move || format!("{:?}", Scene::ambient().selected.get()))        // ❌ panics when the label re-runs
```

**Per-window state must be created at build time.** `T::scoped` does this for you (it defers
through `piece_fn`). Writing it by hand with `with_environment(Scene::create(), …)` creates the
value in the caller's scope instead, and since a piece's construction runs before its build, every
window would get the first one's.

## The focused-window rule

App-wide menu actions need to act on the window that is focused when the command runs.
Capturing one window’s `Scene` when installing the menu would keep targeting that window.
Use `Scene::focused()` inside the action instead:

```rust
fn front(f: impl Fn(Scene) + 'static) -> impl Fn() + 'static {
    move || if let Some(scene) = Scene::focused() { f(scene) }
}

menu_item("New Item").shortcut(Shortcut::new("n").shift()).action(front(|s| s.new_item()))
```

`T::focused()` (SwiftUI's `@FocusedValue`) resolves through the key window's own scope, falling
back to the primary window when no secondary one is key, which is the steady state on macOS,
where the primary window's delegate reports no focus events of its own. A window is marked key
the moment it is registered, so a command fired immediately after File ▸ New Window already
lands on the new window rather than on the one behind it. The reverse holds for a window on
its way out: a phone presents a secondary window as a cover and animates its dismissal, and
from the moment the close is requested the window behind it is the front one again, so a
command fired during that animation acts on it rather than on the departing sheet.

A toolbar already belongs to the piece that declares it. Put its declarations in the window
builder, and capture that window’s state directly. See [Toolbars](toolbars.md).

## Why not `thread_local!`

```rust
thread_local! { static SELECTED: Signal<Option<u32>> = Signal::global(None); }   // ❌
```

A thread-local selection is shared by every window on the UI thread. That may go unnoticed
until a second window opens and selecting a row in one window changes the other. Put the
selection in `Scene` when each window should have its own value.

The exception is state that really is process-wide and really has no owner: `Signal::global` is
still the right tool for, say, a network-reachability flag that the whole app observes. `T::app()`
is the typed version of the same idea, and is preferable when the state has more than one field.

## The primitives underneath

`Ambient` is a convenience layer over four functions you can use directly:

| | |
|---|---|
| `with_environment(value, \|\| content)` | provide `value` to a subtree |
| `environment::<T>()` | the nearest provided `T`, or `None` |
| `focused_environment::<T>()` | the `T` provided by the focused window |
| `app_environment::<T>(make)` | the app-wide `T`, created once |

All four sit on `day-reactive`'s scope context (`Scope::provide` / `Scope::use_context`), which
walks a scope's ancestors, so "ambient" means *provided by an ancestor of the scope this piece is
building in*.

## See also

- [docs/windows.md](windows.md) — opening windows, the New Window menu role, the cover fallback
- [docs/model.md](model.md) — `Store` and per-property observability
- [docs/navigation.md](navigation.md) — why a secondary window's nav is `.local()`
