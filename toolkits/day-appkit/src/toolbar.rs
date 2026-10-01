// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// AppKit: NSToolbar (docs/toolbars.md). The window's title-bar toolbar in the macOS 11
// unified style, not a strip of buttons drawn under the title bar. Items are
// NSToolbarItems, so they get the overflow menu, the ⌘-drag reorder, and the system's
// spacing and control sizes; search is an NSSearchToolbarItem, which is what collapses to a
// magnifier when the window narrows, and a menu item is an NSMenuToolbarItem, which draws the
// pull-down chevron.
// ---------------------------------------------------------------------------

use std::collections::HashMap;

use day_spec::ffi_guard;
use day_spec::sidetable::SideTable;
use day_spec::{
    Event, Icon, NodeId, Symbol, ToolbarItem, ToolbarItemKind, ToolbarPatch, ToolbarValue,
};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSBezelStyle, NSButton, NSControl, NSControlStateValueOff, NSControlStateValueOn, NSImage,
    NSMenuToolbarItem, NSSearchToolbarItem, NSTextField, NSToolbar, NSToolbarDelegate,
    NSToolbarDisplayMode, NSToolbarFlexibleSpaceItemIdentifier, NSToolbarItem,
    NSToolbarItemIdentifier, NSToolbarSpaceItemIdentifier, NSView, NSWindow, NSWindowToolbarStyle,
};
use objc2_foundation::{NSArray, NSCopying, NSObject, NSString};

use crate::{AppKit, Handle, emit};

fn refresh_cover_bar(key: usize) {
    if let Some(bar) = BARS.with(|b| b.with(key, |w| w.toolbar.clone())) {
        reconcile(&bar, key);
    }
}

pub(crate) fn cover_presented(cover: usize, window: &NSWindow) {
    if COVER_WINDOWS.with(|c| c.contains(cover)) {
        return;
    }
    let key = window as *const NSWindow as usize;
    COVER_COUNTS.with(|c| {
        let count = c.with(key, |n| *n).unwrap_or(0);
        c.insert(key, count + 1);
    });
    COVER_WINDOWS.with(|c| c.insert(cover, key));
    refresh_cover_bar(key);
}

pub(crate) fn cover_dismissed(cover: usize) {
    COVER_WINDOWS.with(|c| {
        c.remove(cover);
    });
}

/// The SF Symbol each standard symbol draws as: the shared Apple table (day-spec), so the
/// menu items in day-uikit and the toolbar items here never drift apart.
fn sf_symbol(s: Symbol) -> &'static str {
    day_spec::sf_symbol_name(s)
}

pub(crate) fn image_for(
    icon: &Icon,
    label: &str,
    mtm: MainThreadMarker,
) -> Option<Retained<NSImage>> {
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
        //
        // The glyph SVG comes first, exactly as the sidebar's `resolve_nav_icons` does it: on this
        // backend a `resource/vectors/` asset stages as an SVG and nothing else, so looking only
        // for a raster found nothing and the item silently fell back to drawing its label, a
        // toolbar button reading "Star" where a star belonged. NSImage renders the SVG at whatever
        // size the bar asks for, which is the better result anyway.
        Icon::Image(name) => {
            let _ = mtm;
            let path = day_spec::resource::resolve_vector_svg(name)
                .or_else(|| day_spec::resource::resolve_image_file(name))?;
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
const KIND_SEGMENTED: u8 = 3;

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
    impl ItemTarget {
        #[unsafe(method(fire:))]
        fn fire(&self, sender: &AnyObject) {
            ffi_guard::contain((), || {
                let ivars = self.ivars();
                match ivars.kind {
                    KIND_SEARCH => {
                        if let Some(field) = sender.downcast_ref::<NSTextField>() {
                            emit(day_spec::WINDOW_NODE, Event::ToolbarChanged {
                                action: ivars.action,
                                value: ToolbarValue::Text(field.stringValue().to_string()),
                            });
                        }
                    }
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
                    KIND_SEGMENTED => {
                        let index = sender
                            .downcast_ref::<objc2_app_kit::NSSegmentedControl>()
                            .map(|c| unsafe { c.selectedSegment() })
                            .unwrap_or(0);
                        if index >= 0 {
                            emit(
                                day_spec::WINDOW_NODE,
                                Event::ToolbarChanged {
                                    action: ivars.action,
                                    value: ToolbarValue::Selected(index as usize),
                                },
                            );
                        }
                    }
                    // A plain button rides the menu action rail, so one closure can back both a
                    // toolbar button and its menu-bar twin.
                    _ => emit(day_spec::WINDOW_NODE, Event::MenuAction(ivars.action)),
                }
            })
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
            ffi_guard::contain(None, || {
                let mtm = MainThreadMarker::from(self);
                make_item(mtm, self.ivars().key, &identifier.to_string())
            })
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
    /// The toolbar holds its delegate weakly, and each item holds its target weakly, so both
    /// must be owned here for the window's lifetime.
    _delegate: Retained<BarDelegate>,
    /// The window's whole bar, as Day's edits and patches left it (docs/toolbars.md).
    mirror: day_spec::ToolbarMirror,
    targets: HashMap<String, Retained<ItemTarget>>,
}

day_core::tls_group! {
    /// Window ptr → its live toolbar. A [`SideTable`]: the release path sweeps a closing
    /// secondary window's key, so the WinToolbar (items, targets, retained NSToolbar and
    /// delegate) goes with the window. Installing an empty bar used to be the only removal.
    static BARS: SideTable<WinToolbar> = SideTable::with_teardown(|w: WinToolbar| {
        // NSToolbar holds its delegate weakly; detach before the owned delegate drops.
        w.toolbar.setDelegate(None);
    });
    // Covers occlude the navigation surfaces whose commands this toolbar carries. Keep
    // the model alive, but remove its affordances until the last cover is dismissed.
    static COVER_COUNTS: SideTable<usize> = SideTable::new();
    static COVER_WINDOWS: SideTable<usize> = SideTable::with_teardown(|key| {
        COVER_COUNTS.with(|c| { c.with(key, |n| *n = n.saturating_sub(1)); });
        refresh_cover_bar(key);
    });
    /// Monotonic, so a replaced toolbar never reuses an autosave slot from the old one.
    static NEXT_BAR: std::cell::Cell<u64> = const { std::cell::Cell::new(1) };

}

/// The identifier each model item occupies, in bar order, with the system spacers synthesized
/// from the items' placements (docs/toolbars.md).
///
/// The app no longer writes spacers. `Navigation` items lead, a flexible space follows them, a
/// `Principal` item sits between two more, and everything trailing packs to the right, which is
/// the packing every desktop toolbar wants and the one apps used to spell out by hand, wrongly as
/// often as not. A window whose navigation host asked for one opens with AppKit's
/// `NSToolbarToggleSidebarItem`: the system glyph, the localized name, the position beside the
/// split's divider, and the `toggleSidebar:` action `NSSplitViewController` implements.
fn identifiers(key: usize) -> Retained<NSArray<NSToolbarItemIdentifier>> {
    use day_spec::ToolbarColumn as C;
    let names: Vec<Retained<NSString>> = BARS.with(|b| {
        b.with(key, |w| {
            if COVER_COUNTS.with(|c| c.with(key, |n| *n > 0).unwrap_or(false)) {
                return Vec::new();
            }
            let mut out: Vec<Retained<NSString>> = Vec::new();
            let items = w.mirror.items();
            let has = |c: C| items.iter().any(|i| i.column == c);
            // The SIDEBAR column, packed against the divider it acts on: a leading flexible
            // space pushes the show/hide button to the sidebar's trailing edge, which is where
            // Notes and Xcode put theirs.
            if has(C::Sidebar) {
                out.push(unsafe { NSToolbarFlexibleSpaceItemIdentifier.copy() });
                column_items(&mut out, items, C::Sidebar, false);
            }
            // AppKit tracks the sidebar's divider itself, so everything after this sits over
            // what is to the right of the sidebar (docs/toolbars.md).
            out.push(unsafe {
                objc2_app_kit::NSToolbarSidebarTrackingSeparatorItemIdentifier.copy()
            });
            // The content-list column, and a second separator pinned to its divider: the one
            // Day builds itself, because AppKit only vends the sidebar's.
            if has(C::List) {
                column_items(&mut out, items, C::List, true);
                out.push(NSString::from_str(LIST_SEPARATOR_ID));
            }
            // The detail column, and the window's items with it: side by side, a command
            // that acts on the whole window belongs over the content it is looking at.
            column_items(&mut out, items, C::Detail, true);
            column_items(&mut out, items, C::Window, true);
            out
        })
        .unwrap_or_default()
    });
    let refs: Vec<&NSToolbarItemIdentifier> = names.iter().map(|n| n.as_ref()).collect();
    NSArray::from_slice(&refs)
}

/// One column's items in bar order, with a flexible space where the packing turns around:
/// leading roles first, then the space, then the trailing ones, so the prominent action sits at
/// that column's right edge rather than adrift in the middle of it.
fn column_items(
    out: &mut Vec<Retained<NSString>>,
    items: &[ToolbarItem],
    col: day_spec::ToolbarColumn,
    spread: bool,
) {
    use day_spec::ToolbarPlacement as P;
    let mine: Vec<&ToolbarItem> = items.iter().filter(|i| i.column == col).collect();
    if mine.is_empty() {
        return;
    }
    let lead = [P::Navigation, P::Automatic];
    let trail = [P::Primary, P::Secondary, P::Bottom];
    let has_lead = mine.iter().any(|i| lead.contains(&i.placement));
    let has_trail = mine.iter().any(|i| trail.contains(&i.placement));
    for i in mine.iter().filter(|i| lead.contains(&i.placement)) {
        out.push(identifier_of(i));
    }
    if spread && has_lead && has_trail {
        out.push(unsafe { NSToolbarFlexibleSpaceItemIdentifier.copy() });
    }
    for i in mine.iter().filter(|i| i.placement == P::Principal) {
        out.push(identifier_of(i));
    }
    if spread && !has_lead && has_trail {
        out.push(unsafe { NSToolbarFlexibleSpaceItemIdentifier.copy() });
    }
    for i in mine.iter().filter(|i| trail.contains(&i.placement)) {
        out.push(identifier_of(i));
    }
}

/// The identifier of the tracking separator Day pins to the content-list divider. AppKit vends
/// one for the sidebar divider only, so a three-pane window builds its second here.
const LIST_SEPARATOR_ID: &str = "day.toolbar.list-separator";

fn identifier_of(item: &ToolbarItem) -> Retained<NSString> {
    // The sidebar affordance a `nav(Sidebar)` contributes for itself resolves to AppKit's
    // item (docs/toolbars.md): the system glyph, the localized name, the position beside the
    // split's divider, and the `toggleSidebar:` action `NSSplitViewController` implements. Day's
    // button is never built: one affordance, the platform's.
    if item.id == day_spec::SIDEBAR_TOGGLE_ID {
        return unsafe { objc2_app_kit::NSToolbarToggleSidebarItemIdentifier.copy() };
    }
    match item.kind {
        // macOS toolbars have no separator: a fixed gap is the stand-in, and the one
        // the system itself uses between groups.
        ToolbarItemKind::Separator => unsafe { NSToolbarSpaceItemIdentifier.copy() },
        _ => NSString::from_str(&item.id),
    }
}

/// Build the NSToolbarItem for `ident`. AppKit asks for a fresh item each time (including when
/// it builds the overflow menu), so nothing here is cached.
fn make_item(mtm: MainThreadMarker, key: usize, ident: &str) -> Option<Retained<NSToolbarItem>> {
    // The content-list divider's tracking separator. AppKit builds the sidebar's from its own
    // identifier but has none for a third pane, so Day binds this one to the split itself
    // (docs/toolbars.md); the items after it then sit over the detail, and the ones before it
    // over the list, at whatever width the user drags the dividers to.
    if ident == LIST_SEPARATOR_ID {
        let _ = mtm;
        let split = crate::list_split_view(key)?;
        return Some(Retained::into_super(unsafe {
            objc2_app_kit::NSTrackingSeparatorToolbarItem::
                trackingSeparatorToolbarItemWithIdentifier_splitView_dividerIndex(
                    &NSString::from_str(LIST_SEPARATOR_ID),
                    &split,
                    1,
                )
        }));
    }
    let (item, target) = BARS.with(|b| {
        b.with(key, |w| {
            let item = w.mirror.get(ident)?.clone();
            let target = w.targets.get(ident).cloned();
            Some((item, target))
        })
        .flatten()
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
                // NSSearchToolbarItem owns its search field's delegate. Use the field's
                // target/action contract so installing the item cannot replace our listener.
                field.setSendsSearchStringImmediately(true);
                field.setSendsWholeSearchString(false);
                unsafe {
                    field.setTarget(Some(&**t));
                    field.setAction(Some(sel!(fire:)));
                }
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
        ToolbarItemKind::Segmented { segments, selected } => {
            let bar_item = NSToolbarItem::initWithItemIdentifier(NSToolbarItem::alloc(mtm), &id);
            // The real thing: one NSSegmentedControl, `selectOne` tracking, which is what macOS
            // uses for a grouped either/or in a toolbar (Finder's view switcher, Mail's filters).
            let control = unsafe {
                objc2_app_kit::NSSegmentedControl::initWithFrame(
                    objc2_app_kit::NSSegmentedControl::alloc(mtm),
                    objc2_foundation::NSRect::new(
                        objc2_foundation::NSPoint::new(0.0, 0.0),
                        objc2_foundation::NSSize::new((segments.len() as f64) * 44.0, 24.0),
                    ),
                )
            };
            unsafe {
                control.setSegmentCount(segments.len() as isize);
                control.setSegmentStyle(objc2_app_kit::NSSegmentStyle::Automatic);
                control.setTrackingMode(objc2_app_kit::NSSegmentSwitchTracking::SelectOne);
                for (i, seg) in segments.iter().enumerate() {
                    let i = i as isize;
                    // An icon segment shows only the icon, like every other item in this bar;
                    // the title stays as the segment's accessible name and its tooltip.
                    match seg
                        .icon
                        .as_ref()
                        .and_then(|ic| image_for(ic, &seg.title, mtm))
                    {
                        Some(img) => {
                            control.setImage_forSegment(Some(&img), i);
                            control.setLabel_forSegment(&NSString::from_str(""), i);
                        }
                        None => control.setLabel_forSegment(&NSString::from_str(&seg.title), i),
                    }
                    let _: () = msg_send![
                        &*control,
                        setToolTip: &*NSString::from_str(&seg.title),
                        forSegment: i,
                    ];
                }
                if *selected < segments.len() {
                    control.setSelectedSegment(*selected as isize);
                }
                if let Some(t) = target.as_deref() {
                    control.setTarget(Some(t as &AnyObject));
                    control.setAction(Some(sel!(fire:)));
                }
            }
            bar_item.setView(Some(control.as_ref() as &NSView));
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
            if bar_item.image().is_none() {
                let button = unsafe {
                    NSButton::buttonWithTitle_target_action(
                        &label,
                        target.as_deref().map(|t| t as &AnyObject),
                        Some(sel!(fire:)),
                        mtm,
                    )
                };
                button.setBezelStyle(NSBezelStyle::Automatic);
                unsafe { button.setEnabled(item.enabled) };
                bar_item.setView(Some(button.as_ref() as &NSView));
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
    // day owns the enabled state; without this AppKit's automatic validation would gray out
    // every item whose target does not implement `validateToolbarItem:`.
    bar_item.setAutovalidates(false);
    bar_item.setEnabled(item.enabled);
    Some(bar_item)
}

/// The window a day root handle belongs to.
pub(crate) fn window_of(h: &Handle) -> Option<Retained<NSWindow>> {
    h.window()
}

impl AppKit {
    /// Edit this window's toolbar (docs/toolbars.md). Each op adds or takes away one item;
    /// every NSToolbarItem no op names stays exactly as it is, so a search field being typed
    /// into keeps its field editor while the page commands around it change.
    pub(crate) fn edit_toolbar(&mut self, h: &Handle, ops: &[day_spec::ToolbarOp]) -> bool {
        let Some(window) = window_of(h) else {
            return false;
        };
        let key = Retained::as_ptr(&window) as usize;
        let mtm = self.mtm();
        let fresh = !BARS.with(|b| b.contains(key));
        if fresh {
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
            // The model is the app's, and it is reactive: letting the user reorder items would
            // put an autosaved arrangement in permanent conflict with the next edit.
            toolbar.setAllowsUserCustomization(false);
            toolbar.setAutosavesConfiguration(false);
            // Icon-only in the unified style is the modern macOS toolbar; every item still
            // carries a label for the overflow menu and for VoiceOver.
            toolbar.setDisplayMode(NSToolbarDisplayMode::IconOnly);
            BARS.with(|b| {
                b.insert(
                    key,
                    WinToolbar {
                        toolbar,
                        _delegate: delegate,
                        mirror: day_spec::ToolbarMirror::default(),
                        targets: HashMap::new(),
                    },
                )
            });
        }

        // The model first, and a target for each new item that reports anything, so the
        // delegate's factory only ever reads. Removed items' targets outlive their NSToolbarItems
        // (which hold them weakly) until the native bar has let go of them below.
        let mut retired: Vec<Retained<ItemTarget>> = Vec::new();
        let mut gone: Vec<String> = Vec::new();
        let (toolbar, empty) = BARS
            .with(|b| {
                b.with(key, |w| {
                    for op in ops {
                        match op {
                            day_spec::ToolbarOp::Remove { id } => {
                                w.mirror.remove(id);
                                retired.extend(w.targets.remove(id));
                                gone.push(id.clone());
                            }
                            day_spec::ToolbarOp::Insert { index, item } => {
                                if item.action != 0 {
                                    let kind = match item.kind {
                                        ToolbarItemKind::Toggle { .. } => KIND_TOGGLE,
                                        ToolbarItemKind::Segmented { .. } => KIND_SEGMENTED,
                                        ToolbarItemKind::Search { .. } => KIND_SEARCH,
                                        _ => KIND_BUTTON,
                                    };
                                    let target = ItemTarget::new(mtm, item.action, kind);
                                    retired.extend(w.targets.insert(item.id.clone(), target));
                                }
                                w.mirror.insert(*index, item.clone());
                            }
                        }
                    }
                    (w.toolbar.clone(), w.mirror.is_empty())
                })
            })
            .expect("the bar was created above");

        if empty {
            // No commands at all now: the bar comes off the window.
            window.setToolbar(None);
            BARS.with(|b| {
                b.remove(key);
            });
        } else if fresh {
            // A new bar asks its delegate for its items as it is attached.
            let delegate = BARS.with(|b| b.with(key, |w| w._delegate.clone()));
            if let Some(delegate) = delegate {
                toolbar.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
            }
            window.setToolbarStyle(NSWindowToolbarStyle::Unified);
            window.setToolbar(Some(&toolbar));
        } else {
            // The removed items go by identifier. Then the bar is brought to the identifiers the
            // model now lays out (items, the spacers between groups, the column separators),
            // inserting new ones and dropping stale spacers. An item the edit replaced goes in
            // the first step, so the second builds it afresh against the new model.
            let at_of = |id: &str| {
                toolbar
                    .items()
                    .iter()
                    .position(|i| i.itemIdentifier().to_string() == id)
            };
            for id in &gone {
                if let Some(at) = at_of(id) {
                    toolbar.removeItemAtIndex(at as isize);
                }
            }
            reconcile(&toolbar, key);
        }
        drop(retired);
        report_content_size(&window);
        true
    }

    /// Apply a targeted change to one live item.
    pub(crate) fn patch_toolbar(&mut self, h: &Handle, patch: &ToolbarPatch) {
        let Some(window) = window_of(h) else { return };
        let key = Retained::as_ptr(&window) as usize;
        // Keep the model in step, so an item rebuilt later (the overflow menu asks for fresh
        // items) carries the current value rather than the one it was installed with.
        BARS.with(|b| {
            b.with(key, |w| w.mirror.patch(patch));
        });
        let Some(toolbar) = BARS.with(|b| b.with(key, |w| w.toolbar.clone())) else {
            return;
        };
        let target_id = patch.item().to_string();
        for bar_item in toolbar.items().iter() {
            if bar_item.itemIdentifier().to_string() != target_id {
                continue;
            }
            match patch {
                ToolbarPatch::Focus { .. } => {
                    if let Some(search) = bar_item.downcast_ref::<NSSearchToolbarItem>() {
                        search.beginSearchInteraction();
                        unsafe {
                            search.searchField().selectText(None);
                        }
                    }
                }
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
                ToolbarPatch::Selected { index, .. } => {
                    if let Some(view) = bar_item.view()
                        && let Some(seg) = view.downcast_ref::<objc2_app_kit::NSSegmentedControl>()
                    {
                        unsafe { seg.setSelectedSegment(*index as isize) };
                    }
                }
                ToolbarPatch::Enabled { on, .. } => {
                    bar_item.setEnabled(*on);
                    if let Some(view) = bar_item.view()
                        && let Some(control) = view.downcast_ref::<NSControl>()
                    {
                        unsafe {
                            control.setEnabled(*on);
                        }
                    }
                }
                // No completion affordance on NSSearchField (see the realize above).
                ToolbarPatch::Suggestions { .. } => {}
            }
        }
    }
}

/// Bring `toolbar`'s items to the identifiers its model lays out now, keeping every item that
/// is already where it belongs.
///
/// The kept items are in the right relative order (an edit never reorders them; a move arrives
/// as a remove and an insert), so one pass settles the bar: an identifier in place is kept; one
/// that is not wanted here is a stale spacer, or a model item still wanted further along, which
/// stays while the wanted one is inserted before it. System spacers are the only thing ever
/// removed here, and they hold no state.
fn reconcile(toolbar: &NSToolbar, key: usize) {
    let want: Vec<String> = identifiers(key).iter().map(|i| i.to_string()).collect();
    let system = |id: &str| id.starts_with("NSToolbar") || id == LIST_SEPARATOR_ID;
    let have_at = |at: usize| {
        let items = toolbar.items();
        (at < items.count()).then(|| items.objectAtIndex(at).itemIdentifier().to_string())
    };
    let mut at = 0usize;
    for (n, id) in want.iter().enumerate() {
        loop {
            match have_at(at) {
                Some(have) if have == *id => {
                    at += 1;
                    break;
                }
                Some(have) if !system(&have) && want[n + 1..].contains(&have) => {
                    insert_at(toolbar, id, &mut at);
                    break;
                }
                Some(_) => toolbar.removeItemAtIndex(at as isize),
                None => {
                    insert_at(toolbar, id, &mut at);
                    break;
                }
            }
        }
    }
    while toolbar.items().count() > at {
        toolbar.removeItemAtIndex(at as isize);
    }
}

/// Insert identifier `id` at `*at`, stepping past it when the delegate made one. It can decline
/// (the list separator on a window with no content list), and then there is nothing to step over.
fn insert_at(toolbar: &NSToolbar, id: &str, at: &mut usize) {
    // A synthesized tracking separator can move around a kept model item. AppKit permits
    // repeated spaces, but inserting any other identifier while it is still present throws.
    let repeatable = id == unsafe { NSToolbarFlexibleSpaceItemIdentifier.to_string() }
        || id == unsafe { NSToolbarSpaceItemIdentifier.to_string() };
    if !repeatable {
        if let Some(existing) = toolbar
            .items()
            .iter()
            .position(|item| item.itemIdentifier().to_string() == id)
        {
            toolbar.removeItemAtIndex(existing as isize);
            if existing < *at {
                *at -= 1;
            }
        }
    }
    let before = toolbar.items().count();
    toolbar.insertItemWithItemIdentifier_atIndex(&NSString::from_str(id), *at as isize);
    if toolbar.items().count() > before {
        *at += 1;
    }
}

/// Installing or removing a toolbar resizes the content view without a window resize, so day
/// has to be told the new size or the tree keeps laying out at the old height.
fn report_content_size(window: &NSWindow) {
    // A toolbar coming or going changes the title bar's height, so re-pin the content below it
    // and report the layout area that is left (§7.7).
    let Some(size) = crate::pin_below_title_bar(window) else {
        return;
    };
    // Secondary windows carry their root node on the window delegate; the primary reports at
    // WINDOW_NODE, the same as `windowDidResize:`.
    let node: NodeId = window
        .delegate()
        .and_then(|d| d.downcast::<crate::DayWinDelegate>().ok())
        .and_then(|d| d.ivars().node)
        .unwrap_or(day_spec::WINDOW_NODE);
    emit(node, Event::WindowResized(size));
}
