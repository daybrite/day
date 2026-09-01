---
title: "App state"
description: "Where state lives when an app has more than one window: Ambient values, the app scope, the window scope, and the focused-window rule for menu bars."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# App state (§4.3)

> **Status: implemented** on every backend; the whole mechanism is `day-reactive` scope context
> plus the window registry, so there is no toolkit duty behind it and nothing to degrade.
> Verified by the `Ambient` suite in `crates/day-pieces/tests/mock_e2e.rs` and by the scaffold,
> whose entire state is one per-window struct (`day new` output; `Day-Rise`).

A `Signal` has to live somewhere. Day gives you three places, and picking the right one is
most of the answer to "how do I structure an app's state":

| Scope | Lives as long as | Reach it with | SwiftUI |
|---|---|---|---|
| **Piece** | the piece that made it | a local `Signal::new` | `@State` in a view |
| **Window** | that window | `T::scoped(…)` → `T::ambient()` | `@State` on a `WindowGroup` root |
| **App** | the process | `T::app()` | `@StateObject` on the `App` |

The trap is that on a one-window app all three behave identically, so the wrong choice is
invisible until [File ▸ New Window](windows.md) opens a second one and both windows turn out
to share a selection.

## The shape: a `Copy` struct of handles

```rust
use day::prelude::*;

/// Everything ONE WINDOW owns.
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

`Signal`, `Memo`, `Trigger` and `Store` are all `Copy` and all pointer-sized, so the struct is a
bundle of handles: it moves into an event handler or a page function without an `Rc`, a `clone()`,
or a lifetime, so passing state around costs as little as reaching for a global.

## Providing it

```rust
pub fn root() -> impl Piece {
    day::register_new_window(|| window_shell(false));   // File ▸ New Window: the SAME shell
    app_menu(menus());
    window_shell(true)
}

fn window_shell(primary: bool) -> impl Piece {
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

`ambient()` is the read to reach for when a piece cannot take the value as an argument:
`selector(…).item_icon(key, title, icon, my_page)` takes a bare `fn() -> impl Piece`, and that is
the case SwiftUI's `@EnvironmentObject` exists for. When you *can* pass it, pass it: an
argument is clearer than a lookup, and `Scene` is `Copy`.

For state that belongs to the whole app rather than to a window (a login session, a
sync engine, a document cache several windows share), use `T::app()` instead. It creates the value
on the reactive root scope the first time anything asks and hands out the same instance forever,
from any window, any menu action, and any task.

### Two rules

**`ambient()` resolves while a piece builds.** Read it in the piece's body and capture the value;
calling it inside a reactive closure works on the first run and panics on the next, because a
re-running reaction is no longer inside the scope that provided the value.

```rust
let scene = Scene::ambient();                       // ✅ read once, at build
label(move || scene.title.read())

label(move || Scene::ambient().title.read())        // ❌ panics when the label re-runs
```

**Per-window state must be created at build time.** `T::scoped` does this for you (it defers
through `piece_fn`). Writing it by hand with `with_environment(Scene::create(), …)` creates the
value in the caller's scope instead, and since a piece's construction runs before its build, every
window would get the first one's.

## The focused-window rule

A desktop menu bar is one bar for the whole app. Its items belong to no window, so they cannot
capture a `Scene`; they have to resolve the front one when they run:

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
lands on the new window rather than on the one behind it.

Toolbars need none of this: `toolbar(…)` / `toolbar_reactive(…)` install on the window being
built, so a toolbar declared inside the window shell already belongs to its own window, but for
the same reason it must be declared *there* and not in `root()`, or every window gets the first
one's bar ([docs/toolbars.md](toolbars.md)).

## Why not `thread_local!`

```rust
thread_local! { static SELECTED: Signal<Option<u32>> = Signal::global(None); }   // ❌
```

One instance per process. It compiles, it is easy to reach, and it is correct right up until the
app grows a second window, at which point both windows share the selection, both scroll each
other's lists, and the fix is a refactor of every call site rather than a change of one line. A
`Scene` field costs the same to write and never has to be undone.

The exception is state that really is process-wide and really has no owner: `Signal::global` is
still the right tool for, say, a network-reachability flag that the whole app observes. `T::app()`
is the typed version of the same idea, and is preferable when the state has more than one field.

## The primitives underneath

`Ambient` is a thin layer over three functions you can use directly:

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
