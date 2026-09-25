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

/// Named inputs for an application command. Call [`Self::build`] in the scope that owns it.
///
/// Field types are inferred, so labels accept localized text, strings, signals, or closures,
/// and actions can capture application state without an explicit `Rc` or `Box`.
///
/// ```
/// use day_pieces::prelude::*;
/// use day_reactive::Signal;
///
/// let count = Signal::new(0);
/// let add = Command {
///     id: "add",
///     label: "Add one",
///     action: move || count.update(|n| *n += 1),
/// }
/// .build()
/// .enabled(move || count.get() < 3);
/// assert!(add.invoke());
/// assert_eq!(count.get(), 1);
/// ```
pub struct Command<Id, Label, Action> {
    /// Default presentation id; this does not register a global shortcut or dispatch token.
    pub id: Id,
    /// Title, accepting the same text sources as a button.
    pub label: Label,
    /// Synchronous application operation. Availability is checked before it is called.
    pub action: Action,
}

impl<Id, Label, Action> Command<Id, Label, Action> {
    /// Initialize a shared command and capture the current reactive scope as its owner.
    /// Constructing the definition alone does not capture a scope or register any controls.
    pub fn build<M>(self) -> CommandHandle
    where
        Id: Into<String>,
        Label: IntoText<M>,
        Action: Fn() + 'static,
    {
        CommandHandle {
            id: Rc::from(self.id.into()),
            label: self.label.into_text(),
            enabled: Reactive::Const(true),
            checked: None,
            icon: None,
            shortcut: None,
            handler: Rc::new(self.action),
            owner: Scope::current(),
        }
    }
}

/// An initialized application operation shared by buttons, menus, toolbars, and direct callers.
///
/// Created by [`Command::build`]. Clones share the handler and reactive sources. Builders
/// configure a copy, not existing presentations. Invocation after the owning scope's disposal
/// is a no-op. This is a UI-thread value, not a global registry or a native edit role.
#[derive(Clone)]
pub struct CommandHandle {
    id: Rc<str>,
    label: TextSource,
    enabled: Reactive<bool>,
    checked: Option<Reactive<bool>>,
    icon: Option<Icon>,
    shortcut: Option<Shortcut>,
    handler: Rc<dyn Fn()>,
    owner: Scope,
}

impl CommandHandle {
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
        &self.id
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
        button.id(self.id.to_string())
    }

    /// Snapshot for `app_menu_reactive`, a reactive nav mapper, or `context_menu_fn`.
    /// A fixed menu keeps its snapshot until rebuilt, but invocation always rechecks availability.
    pub fn menu_item(&self) -> MenuEntry {
        let action = self.clone();
        let mut item = crate::menu_item(self.label())
            .id(self.id.to_string())
            .enabled(self.is_enabled())
            .action(move || {
                action.invoke();
            });
        if let Some(on) = self.is_checked() {
            item = item.checked(on);
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
        let mut item = crate::toolbar_button(self.id.to_string(), self.label())
            .enabled(self.is_enabled())
            .enabled_when(move || enabled.is_enabled())
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
