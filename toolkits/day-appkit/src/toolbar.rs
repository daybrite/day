// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// AppKit: NSToolbar (docs/toolbars.md). The window's real title-bar toolbar in the macOS 11
// unified style — not a strip of buttons drawn under the title bar. Items are real
// NSToolbarItems, so they get the overflow menu, the ⌘-drag reorder, and the system's own
// spacing and control sizes; search is an NSSearchToolbarItem, which is what collapses to a
// magnifier when the window narrows, and a menu item is an NSMenuToolbarItem, which draws the
// pull-down chevron.
// ---------------------------------------------------------------------------

use std::cell::RefCell;
use std::collections::HashMap;

use day_spec::{
    Event, Icon, NodeId, Symbol, ToolbarItem, ToolbarItemKind, ToolbarPatch, ToolbarValue,
};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSBezelStyle, NSButton, NSControlStateValueOff, NSControlStateValueOn,
    NSControlTextEditingDelegate, NSImage, NSMenuToolbarItem, NSSearchToolbarItem, NSTextField,
    NSTextFieldDelegate, NSToolbar, NSToolbarDelegate, NSToolbarDisplayMode,
    NSToolbarFlexibleSpaceItemIdentifier, NSToolbarItem, NSToolbarItemIdentifier,
    NSToolbarSpaceItemIdentifier, NSView, NSWindow, NSWindowToolbarStyle,
};
use objc2_foundation::{NSArray, NSCopying, NSNotification, NSObject, NSString};

use crate::{AppKit, Handle, emit};

/// The SF Symbol each standard symbol draws as. These are the system's own glyphs, so they
/// match the user's Mac — weight, optical size, accent colour and all.
fn sf_symbol(s: Symbol) -> &'static str {
    match s {
        Symbol::Add => "plus",
        Symbol::Remove => "minus",
        Symbol::Delete => "trash",
        Symbol::Edit => "pencil",
        Symbol::New => "square.and.pencil",
        Symbol::Open => "folder",
        Symbol::Save => "square.and.arrow.down",
        Symbol::Print => "printer",
        Symbol::Refresh => "arrow.clockwise",
        Symbol::Search => "magnifyingglass",
        Symbol::Share => "square.and.arrow.up",
        Symbol::Settings => "gearshape",
        Symbol::Info => "info.circle",
        Symbol::Star => "star",
        Symbol::Bookmark => "bookmark",
        Symbol::Back => "chevron.backward",
        Symbol::Forward => "chevron.forward",
        Symbol::Up => "chevron.up",
        Symbol::Down => "chevron.down",
        Symbol::Home => "house",
        Symbol::Sidebar => "sidebar.leading",
        Symbol::Filter => "line.3.horizontal.decrease",
        Symbol::Sort => "arrow.up.arrow.down",
        Symbol::More => "ellipsis",
        Symbol::Play => "play.fill",
        Symbol::Pause => "pause.fill",
        Symbol::Stop => "stop.fill",
        Symbol::ZoomIn => "plus.magnifyingglass",
        Symbol::ZoomOut => "minus.magnifyingglass",
        Symbol::Undo => "arrow.uturn.backward",
        Symbol::Redo => "arrow.uturn.forward",
        Symbol::Copy => "doc.on.doc",
        Symbol::Cut => "scissors",
        Symbol::Paste => "doc.on.clipboard",
        Symbol::Mail => "envelope",
        Symbol::Folder => "folder",
        Symbol::Document => "doc",
        Symbol::Check => "checkmark",
        Symbol::Close => "xmark",
        Symbol::Warning => "exclamationmark.triangle",
        // The vocabulary is `#[non_exhaustive]`: an unmapped symbol draws no image rather than
        // an arbitrary wrong one — the item still shows its label.
        _ => "",
    }
}

fn image_for(icon: &Icon, label: &str, mtm: MainThreadMarker) -> Option<Retained<NSImage>> {
    match icon {
        Icon::Symbol(s) => {
            let name = sf_symbol(*s);
            if name.is_empty() {
                return None;
            }
            NSImage::imageWithSystemSymbolName_accessibilityDescription(
                &NSString::from_str(name),
                Some(&NSString::from_str(label)),
            )
        }
        // A bundled image, as a template so the system tints it for the title bar the way it
        // tints its own symbols.
        Icon::Image(name) => {
            let _ = mtm;
            let path = day_spec::resource::resolve_image_file(name)?;
            use objc2::AllocAnyThread as _;
            let img = unsafe {
                NSImage::initWithContentsOfFile(
                    NSImage::alloc(),
                    &NSString::from_str(&path.to_string_lossy()),
                )
            }?;
            unsafe { img.setTemplate(true) };
            Some(img)
        }
    }
}

// --- the per-item target -----------------------------------------------------------------

/// What a target reports when it fires.
const KIND_BUTTON: u8 = 0;
const KIND_TOGGLE: u8 = 1;
const KIND_SEARCH: u8 = 2;

struct ItemIvars {
    action: u64,
    kind: u8,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DayToolbarTarget"]
    #[ivars = ItemIvars]
    struct ItemTarget;

    unsafe impl NSObjectProtocol for ItemTarget {}
    unsafe impl NSTextFieldDelegate for ItemTarget {}

    /// The search field reports every keystroke here; a programmatic `setStringValue` does not
    /// fire this delegate, so the sync in `update_toolbar` needs no suppression.
    unsafe impl NSControlTextEditingDelegate for ItemTarget {
        #[unsafe(method(controlTextDidChange:))]
        fn control_text_did_change(&self, notification: &NSNotification) {
            let ivars = self.ivars();
            if ivars.kind != KIND_SEARCH {
                return;
            }
            if let Some(obj) = unsafe { notification.object() }
                && let Ok(tf) = obj.downcast::<NSTextField>()
            {
                emit(
                    day_spec::WINDOW_NODE,
                    Event::ToolbarChanged {
                        action: ivars.action,
                        value: ToolbarValue::Text(tf.stringValue().to_string()),
                    },
                );
            }
        }
    }

    impl ItemTarget {
        #[unsafe(method(fire:))]
        fn fire(&self, sender: &AnyObject) {
            let ivars = self.ivars();
            match ivars.kind {
                KIND_TOGGLE => {
                    let on = sender
                        .downcast_ref::<NSButton>()
                        .map(|b| b.state() == NSControlStateValueOn)
                        .unwrap_or(false);
                    emit(
                        day_spec::WINDOW_NODE,
                        Event::ToolbarChanged {
                            action: ivars.action,
                            value: ToolbarValue::On(on),
                        },
                    );
                }
                // A plain button rides the menu action rail, so one closure can back both a
                // toolbar button and its menu-bar twin.
                _ => emit(day_spec::WINDOW_NODE, Event::MenuAction(ivars.action)),
            }
        }
    }
);

impl ItemTarget {
    fn new(mtm: MainThreadMarker, action: u64, kind: u8) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ItemIvars { action, kind });
        unsafe { msg_send![super(this), init] }
    }
}

// --- the toolbar delegate ----------------------------------------------------------------

struct BarIvars {
    /// The window this bar belongs to, as the key into [`BARS`].
    key: usize,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DayToolbarDelegate"]
    #[ivars = BarIvars]
    struct BarDelegate;

    unsafe impl NSObjectProtocol for BarDelegate {}

    unsafe impl NSToolbarDelegate for BarDelegate {
        #[unsafe(method_id(toolbar:itemForItemIdentifier:willBeInsertedIntoToolbar:))]
        fn item_for_identifier(
            &self,
            _toolbar: &NSToolbar,
            identifier: &NSToolbarItemIdentifier,
            _inserted: bool,
        ) -> Option<Retained<NSToolbarItem>> {
            let mtm = MainThreadMarker::from(self);
            make_item(mtm, self.ivars().key, &identifier.to_string())
        }

        #[unsafe(method_id(toolbarDefaultItemIdentifiers:))]
        fn default_identifiers(
            &self,
            _toolbar: &NSToolbar,
        ) -> Retained<NSArray<NSToolbarItemIdentifier>> {
            identifiers(self.ivars().key)
        }

        #[unsafe(method_id(toolbarAllowedItemIdentifiers:))]
        fn allowed_identifiers(
            &self,
            _toolbar: &NSToolbar,
        ) -> Retained<NSArray<NSToolbarItemIdentifier>> {
            identifiers(self.ivars().key)
        }
    }
);

impl BarDelegate {
    fn new(mtm: MainThreadMarker, key: usize) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(BarIvars { key });
        unsafe { msg_send![super(this), init] }
    }
}

/// One window's live toolbar.
struct WinToolbar {
    toolbar: Retained<NSToolbar>,
    /// The toolbar holds its delegate weakly, and each item holds its target weakly — both
    /// must be owned here for the window's lifetime.
    _delegate: Retained<BarDelegate>,
    items: Vec<ToolbarItem>,
    targets: HashMap<String, Retained<ItemTarget>>,
}

thread_local! {
    static BARS: RefCell<HashMap<usize, WinToolbar>> = RefCell::new(HashMap::new());
    /// Monotonic, so a replaced toolbar never reuses an autosave slot from the old one.
    static NEXT_BAR: std::cell::Cell<u64> = const { std::cell::Cell::new(1) };
}

/// The identifier each model item occupies, in bar order. Spacers use the system identifiers —
/// they are what AppKit recognizes as spacers, and they may legitimately repeat.
fn identifiers(key: usize) -> Retained<NSArray<NSToolbarItemIdentifier>> {
    let names: Vec<Retained<NSString>> = BARS.with(|b| {
        b.borrow()
            .get(&key)
            .map(|w| w.items.iter().map(identifier_of).collect())
            .unwrap_or_default()
    });
    let refs: Vec<&NSToolbarItemIdentifier> = names.iter().map(|n| n.as_ref()).collect();
    NSArray::from_slice(&refs)
}

fn identifier_of(item: &ToolbarItem) -> Retained<NSString> {
    match item.kind {
        ToolbarItemKind::FlexibleSpace => unsafe { NSToolbarFlexibleSpaceItemIdentifier.copy() },
        // macOS toolbars have no separator: a fixed gap is the honest stand-in, and the one
        // the system itself uses between groups.
        ToolbarItemKind::Space | ToolbarItemKind::Separator => unsafe {
            NSToolbarSpaceItemIdentifier.copy()
        },
        // The system item, not one of ours: AppKit gives it the right glyph, the localized
        // name, the leading position next to the split's divider, and the `toggleSidebar:`
        // action that NSSplitViewController implements (docs/toolbars.md, docs/navigation.md).
        ToolbarItemKind::SidebarToggle => unsafe {
            objc2_app_kit::NSToolbarToggleSidebarItemIdentifier.copy()
        },
        _ => NSString::from_str(&item.id),
    }
}

/// Build the NSToolbarItem for `ident`. AppKit asks for a fresh item each time (including when
/// it builds the overflow menu), so nothing here is cached.
fn make_item(mtm: MainThreadMarker, key: usize, ident: &str) -> Option<Retained<NSToolbarItem>> {
    let (item, target) = BARS.with(|b| {
        let b = b.borrow();
        let w = b.get(&key)?;
        let item = w.items.iter().find(|i| i.id == ident)?.clone();
        let target = w.targets.get(ident).cloned();
        Some((item, target))
    })?;

    let id = NSString::from_str(&item.id);
    let label = NSString::from_str(&item.label);
    let tip = NSString::from_str(item.tooltip.as_deref().unwrap_or(&item.label));

    let bar_item: Retained<NSToolbarItem> = match &item.kind {
        // `suggestions` unused: NSSearchField's menu is a RECENTS list, not completions for the
        // current text, so offering it as one would misrepresent what the control does.
        ToolbarItemKind::Search {
            text, placeholder, ..
        } => {
            let search =
                NSSearchToolbarItem::initWithItemIdentifier(NSSearchToolbarItem::alloc(mtm), &id);
            let field = search.searchField();
            field.setStringValue(&NSString::from_str(text));
            if !placeholder.is_empty() {
                field.setPlaceholderString(Some(&NSString::from_str(placeholder)));
            }
            if let Some(t) = &target {
                let tf: &NSTextField = field.as_ref();
                unsafe { tf.setDelegate(Some(ProtocolObject::from_ref(&**t))) };
            }
            Retained::into_super(search)
        }
        ToolbarItemKind::Menu { items } => {
            let menu_item =
                NSMenuToolbarItem::initWithItemIdentifier(NSMenuToolbarItem::alloc(mtm), &id);
            let menu = crate::build_ns_menu(mtm, &item.label, items);
            menu_item.setMenu(&menu);
            if let Some(icon) = &item.icon
                && let Some(img) = image_for(icon, &item.label, mtm)
            {
                menu_item.setImage(Some(&img));
            }
            Retained::into_super(menu_item)
        }
        ToolbarItemKind::Toggle { on } => {
            let bar_item = NSToolbarItem::initWithItemIdentifier(NSToolbarItem::alloc(mtm), &id);
            // A push-on/push-off button is how a toolbar shows a sticky state on macOS; the
            // system draws the "on" bezel for us.
            let button = unsafe {
                NSButton::buttonWithTitle_target_action(
                    &label,
                    target.as_deref().map(|t| t as &AnyObject),
                    Some(sel!(fire:)),
                    mtm,
                )
            };
            button.setBezelStyle(NSBezelStyle::Toolbar);
            unsafe { button.setButtonType(objc2_app_kit::NSButtonType::PushOnPushOff) };
            if let Some(icon) = &item.icon
                && let Some(img) = image_for(icon, &item.label, mtm)
            {
                button.setImage(Some(&img));
                // An icon item shows the icon alone; the label still names it everywhere the
                // system needs a name (overflow menu, VoiceOver).
                button.setTitle(&NSString::from_str(""));
            }
            button.setState(if *on {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
            bar_item.setView(Some(button.as_ref() as &NSView));
            bar_item
        }
        ToolbarItemKind::Label => {
            let bar_item = NSToolbarItem::initWithItemIdentifier(NSToolbarItem::alloc(mtm), &id);
            let field = NSTextField::labelWithString(&label, mtm);
            bar_item.setView(Some(field.as_ref() as &NSView));
            bar_item
        }
        // Button, and anything a future model adds: a plain image+label command.
        _ => {
            let bar_item = NSToolbarItem::initWithItemIdentifier(NSToolbarItem::alloc(mtm), &id);
            if let Some(icon) = &item.icon
                && let Some(img) = image_for(icon, &item.label, mtm)
            {
                bar_item.setImage(Some(&img));
            }
            // macOS 11's bordered items are the modern toolbar button look.
            bar_item.setBordered(true);
            if let Some(t) = &target {
                unsafe {
                    bar_item.setTarget(Some(&**t as &AnyObject));
                    bar_item.setAction(Some(sel!(fire:)));
                }
            }
            bar_item
        }
    };

    bar_item.setLabel(&label);
    bar_item.setPaletteLabel(&label);
    bar_item.setToolTip(Some(&tip));
    // day owns the enabled state; without this AppKit's automatic validation would grey out
    // every item whose target does not implement `validateToolbarItem:`.
    bar_item.setAutovalidates(false);
    bar_item.setEnabled(item.enabled);
    Some(bar_item)
}

/// The window a day root handle belongs to.
fn window_of(h: &Handle) -> Option<Retained<NSWindow>> {
    h.window()
}

impl AppKit {
    /// Install `items` as this window's toolbar (docs/toolbars.md). An empty slice removes it.
    pub(crate) fn install_toolbar(&mut self, h: &Handle, items: &[ToolbarItem]) {
        let Some(window) = window_of(h) else { return };
        let mtm = self.mtm();
        let key = Retained::as_ptr(&window) as usize;

        if items.is_empty() {
            window.setToolbar(None);
            BARS.with(|b| b.borrow_mut().remove(&key));
            report_content_size(&window);
            return;
        }

        // One target per item that has something to report, created up front so the delegate's
        // item factory only ever reads.
        let mut targets = HashMap::new();
        for item in items {
            let kind = match item.kind {
                ToolbarItemKind::Toggle { .. } => KIND_TOGGLE,
                ToolbarItemKind::Search { .. } => KIND_SEARCH,
                _ => KIND_BUTTON,
            };
            if item.action != 0 {
                targets.insert(item.id.clone(), ItemTarget::new(mtm, item.action, kind));
            }
        }

        let existing = BARS.with(|b| b.borrow().contains_key(&key));
        if existing {
            // Reuse the live NSToolbar — replacing it flashes the title bar — but rebuild its
            // items (see below). A full replace is rare: the builder re-runs on a locale change or
            // a change in the bar's shape, never on a keystroke — typing patches the item in place
            // through `day_core::patch_toolbar` — so the focus this costs is not focus in use.
            BARS.with(|b| {
                if let Some(w) = b.borrow_mut().get_mut(&key) {
                    w.items = items.to_vec();
                    w.targets = targets;
                }
            });
            let toolbar = BARS.with(|b| b.borrow().get(&key).map(|w| w.toolbar.clone()));
            if let Some(toolbar) = toolbar {
                let ids = identifiers(key);
                // Clear first, then set. `setItemIdentifiers` diffs BY IDENTIFIER: it inserts the
                // new ones, removes the departed, and leaves every other item exactly as it was —
                // still carrying the previous model's label and, worse, the previous `ItemTarget`,
                // whose action id day-core had already swept. That is why a locale switch left the
                // search field dead (its input dispatched into nothing) and the labels in the old
                // language: same ids, new model, untouched items. Clearing drops them all so each
                // is rebuilt through the delegate against the model swapped in above.
                toolbar.setItemIdentifiers(&NSArray::new());
                toolbar.setItemIdentifiers(&ids);
            }
            report_content_size(&window);
            return;
        }

        let ident = NEXT_BAR.with(|c| {
            let n = c.get();
            c.set(n + 1);
            n
        });
        let toolbar = NSToolbar::initWithIdentifier(
            NSToolbar::alloc(mtm),
            &NSString::from_str(&format!("day.toolbar.{ident}")),
        );
        let delegate = BarDelegate::new(mtm, key);
        // The model is the app's, and it is reactive: letting the user reorder items would put
        // an autosaved arrangement in permanent conflict with the next install.
        toolbar.setAllowsUserCustomization(false);
        toolbar.setAutosavesConfiguration(false);
        // Icon-only in the unified style is the modern macOS toolbar; every item still carries
        // a label for the overflow menu and for VoiceOver.
        toolbar.setDisplayMode(NSToolbarDisplayMode::IconOnly);

        BARS.with(|b| {
            b.borrow_mut().insert(
                key,
                WinToolbar {
                    toolbar: toolbar.clone(),
                    _delegate: delegate.clone(),
                    items: items.to_vec(),
                    targets,
                },
            )
        });

        toolbar.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        window.setToolbarStyle(NSWindowToolbarStyle::Unified);
        window.setToolbar(Some(&toolbar));
        report_content_size(&window);
    }

    /// Apply a targeted change to one live item.
    pub(crate) fn patch_toolbar(&mut self, h: &Handle, patch: &ToolbarPatch) {
        let Some(window) = window_of(h) else { return };
        let key = Retained::as_ptr(&window) as usize;
        // Keep the model in step, so an item rebuilt later (the overflow menu asks for fresh
        // items) carries the current value rather than the one it was installed with.
        BARS.with(|b| {
            if let Some(w) = b.borrow_mut().get_mut(&key) {
                apply_to_model(&mut w.items, patch);
            }
        });
        let Some(toolbar) = BARS.with(|b| b.borrow().get(&key).map(|w| w.toolbar.clone())) else {
            return;
        };
        let target_id = match patch {
            ToolbarPatch::Text { item, .. }
            | ToolbarPatch::On { item, .. }
            | ToolbarPatch::Enabled { item, .. }
            | ToolbarPatch::Suggestions { item, .. } => item.clone(),
        };
        for bar_item in toolbar.items().iter() {
            if bar_item.itemIdentifier().to_string() != target_id {
                continue;
            }
            match patch {
                ToolbarPatch::Text { text, .. } => {
                    if let Some(search) = bar_item.downcast_ref::<NSSearchToolbarItem>() {
                        let field = search.searchField();
                        if field.stringValue().to_string() != *text {
                            field.setStringValue(&NSString::from_str(text));
                        }
                    }
                }
                ToolbarPatch::On { on, .. } => {
                    if let Some(view) = bar_item.view()
                        && let Some(button) = view.downcast_ref::<NSButton>()
                    {
                        button.setState(if *on {
                            NSControlStateValueOn
                        } else {
                            NSControlStateValueOff
                        });
                    }
                }
                ToolbarPatch::Enabled { on, .. } => bar_item.setEnabled(*on),
                // No completion affordance on NSSearchField (see the realize above).
                ToolbarPatch::Suggestions { .. } => {}
            }
        }
    }
}

fn apply_to_model(items: &mut [ToolbarItem], patch: &ToolbarPatch) {
    match patch {
        ToolbarPatch::Text { item, text } => {
            if let Some(it) = items.iter_mut().find(|i| i.id == *item)
                && let ToolbarItemKind::Search { text: t, .. } = &mut it.kind
            {
                *t = text.clone();
            }
        }
        ToolbarPatch::On { item, on } => {
            if let Some(it) = items.iter_mut().find(|i| i.id == *item)
                && let ToolbarItemKind::Toggle { on: o } = &mut it.kind
            {
                *o = *on;
            }
        }
        // No native completion list on this toolkit's search widget (docs/search.md).
        ToolbarPatch::Suggestions { .. } => {}
        ToolbarPatch::Enabled { item, on } => {
            if let Some(it) = items.iter_mut().find(|i| i.id == *item) {
                it.enabled = *on;
            }
        }
    }
}

/// Installing or removing a toolbar resizes the content view without a window resize, so day
/// has to be told the new size or the tree keeps laying out at the old height.
fn report_content_size(window: &NSWindow) {
    let Some(content) = window.contentView() else {
        return;
    };
    let b = content.bounds();
    // Secondary windows carry their root node on the window delegate; the primary reports at
    // WINDOW_NODE, the same as `windowDidResize:`.
    let node: NodeId = window
        .delegate()
        .and_then(|d| d.downcast::<crate::DayWinDelegate>().ok())
        .and_then(|d| d.ivars().node)
        .unwrap_or(day_spec::WINDOW_NODE);
    emit(
        node,
        Event::WindowResized(day_spec::Size::new(b.size.width, b.size.height)),
    );
}
