// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Reusable application commands. See `docs/commands.md` for ownership and presentation rules.

use std::rc::Rc;

use day_reactive::{Scope, untrack};
use day_spec::{Icon, Shortcut, Symbol};

use crate::{
    Button, Decorate, Decorated, IntoReactive, IntoText, MenuEntry, Reactive, TextSource,
    ToolbarEntry,
};

/// An application operation shared by buttons, menus, toolbars, and direct callers
/// (docs/commands.md): one id, one title, one availability, one handler, presented through
/// [`button`](Command::button), [`menu_item`](Command::menu_item) and
/// [`toolbar_item`](Command::toolbar_item).
///
/// Built with [`Command::new`] in the scope that owns it. Clones share the handler and reactive
/// sources. Builders configure a copy, not existing presentations. Invocation after the owning
/// scope's disposal is a no-op. This is a UI-thread value, not a global registry or a native
/// edit role.
///
/// ```
/// use day_pieces::prelude::*;
/// use day_reactive::Signal;
///
/// let count = Signal::new(0);
/// let add = Command::new(day_core::id!(add), "Add one", move || count.update(|n| *n += 1))
///     .enabled(move || count.get() < 3);
/// assert!(add.invoke());
/// assert_eq!(count.get(), 1);
/// ```
#[derive(Clone)]
pub struct Command {
    id: day_core::ElementId,
    label: TextSource,
    enabled: Reactive<bool>,
    checked: Option<Reactive<bool>>,
    icon: Option<Icon>,
    shortcut: Option<Shortcut>,
    handler: Rc<dyn Fn()>,
    owner: Scope,
}

impl Command {
    /// A command with its default presentation id, its title (the same sources as a button:
    /// a literal, a localized text, a signal, a closure) and its synchronous handler.
    /// Availability is checked before the handler is called. Captures the current reactive
    /// scope as the command's owner.
    pub fn new<M>(
        id: day_core::ElementId,
        label: impl IntoText<M>,
        action: impl Fn() + 'static,
    ) -> Command {
        Command {
            id,
            label: label.into_text(),
            enabled: Reactive::Const(true),
            checked: None,
            icon: None,
            shortcut: None,
            handler: Rc::new(action),
            owner: Scope::current(),
        }
    }

    /// Availability shared by every presentation and checked again on every invocation.
    pub fn enabled<M>(mut self, enabled: impl IntoReactive<bool, M>) -> Self {
        self.enabled = enabled.into_reactive();
        self
    }

    /// Optional check state. The handler owns mutations; invoking never flips this itself.
    pub fn checked<M>(mut self, checked: impl IntoReactive<bool, M>) -> Self {
        self.checked = Some(checked.into_reactive());
        self
    }

    pub fn icon(mut self, symbol: Symbol) -> Self {
        self.icon = Some(Icon::Symbol(symbol));
        self
    }

    pub fn image(mut self, name: impl Into<day_spec::ImageName>) -> Self {
        self.icon = Some(Icon::Image(name.into().as_str().to_owned()));
        self
    }

    /// Menu accelerator metadata. Only installing a menu entry installs the shortcut.
    pub fn shortcut(mut self, shortcut: Shortcut) -> Self {
        self.shortcut = Some(shortcut);
        self
    }

    pub fn id(&self) -> &str {
        self.id.as_str()
    }

    /// Current title, with tracked reads for reactive presentation builders.
    pub fn label(&self) -> String {
        if self.owner.is_alive() {
            self.label.resolve()
        } else {
            String::new()
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.owner.is_alive() && self.enabled.get()
    }

    pub fn is_checked(&self) -> Option<bool> {
        self.checked
            .as_ref()
            .map(|v| self.owner.is_alive() && v.get())
    }

    /// Run if still available. Returns whether the handler ran, not whether its work succeeded.
    /// Reads are untracked; invoking inside an effect does not subscribe it to the handler.
    pub fn invoke(&self) -> bool {
        untrack(|| {
            if !self.is_enabled() {
                return false;
            }
            (self.handler)();
            true
        })
    }

    /// A live button with this command's default id, title, icon, availability, and handler.
    /// A push button does not draw a check mark. Add presentation styling as usual.
    pub fn button(&self) -> Decorated<Button> {
        let (title, enabled, action) = (self.clone(), self.clone(), self.clone());
        let mut button = crate::button(move || title.label())
            .enabled(move || enabled.is_enabled())
            .action(move || {
                action.invoke();
            });
        button = match &self.icon {
            Some(Icon::Symbol(s)) => button.icon(*s),
            Some(Icon::Image(name)) => button.image(name.clone()),
            None => button,
        };
        button.id(self.id.clone())
    }

    /// A live menu entry: its title, availability and check mark follow the command, and the
    /// menu it sits in re-lowers when they change. Invocation always rechecks availability.
    pub fn menu_item(&self) -> MenuEntry {
        let (title, enabled, action) = (self.clone(), self.clone(), self.clone());
        let mut item = crate::menu_item(move || title.label())
            .id(self.id.clone())
            .enabled(move || enabled.is_enabled())
            .action(move || {
                action.invoke();
            });
        if self.is_checked().is_some() {
            let checked = self.clone();
            item = item.checked(move || checked.is_checked().unwrap_or(false));
        }
        if let Some(shortcut) = &self.shortcut {
            item = item.shortcut(shortcut.clone());
        }
        match &self.icon {
            Some(Icon::Symbol(s)) => item.icon(*s),
            Some(Icon::Image(name)) => item.image(name.clone()),
            None => item,
        }
    }

    /// Snapshot for a derived toolbar: `.toolbar(move || vec![command.toolbar_item()])`.
    /// Checked commands draw a toggle; the handler remains the sole owner of its state.
    /// Reactive builders refresh titles/checks; availability also gets a targeted live binding.
    pub fn toolbar_item(&self) -> ToolbarEntry {
        let (enabled, action) = (self.clone(), self.clone());
        let mut item = crate::toolbar_button(self.id.clone(), self.label())
            .enabled(move || enabled.is_enabled())
            .action(move || {
                action.invoke();
            });
        if self.is_checked().is_some() {
            let checked = self.clone();
            item = item.command_checked(Rc::new(move || checked.is_checked().unwrap_or(false)));
        }
        match &self.icon {
            Some(Icon::Symbol(s)) => item.icon(*s),
            Some(Icon::Image(name)) => item.image(name.clone()),
            None => item,
        }
    }
}
