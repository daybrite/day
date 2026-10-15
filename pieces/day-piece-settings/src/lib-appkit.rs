// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// AppKit: the classic macOS preferences panel. The window carries an `NSToolbar` in the
// `.preference` style whose selectable items are the visible groups (icon + title); picking
// one swaps the content to that group's page and the window animates its frame to the page's
// height, the way every pre-Ventura System Preferences pane and every toolbar-tab panel
// (Safari, Xcode, Mail) behaves. Rows stay composed: System Settings rows ARE the composed
// shape. One visible group gets no toolbar at all, just the page.
//
// The content is a container piece (docs/extending.md §5): the native view hosts the Day
// pages, and its `Layout` measures the selected page at its natural height. That height is the
// one thing the native side needs, so the layout hands it over through a shared cell and the
// view resizes its window on the next run-loop turn, never from inside a layout pass.
// ---------------------------------------------------------------------------

use super::*;

use std::cell::Cell;

use day_appkit::AppKit;
use day_core::{Boundary, BuildCx, Flex, Layout, LayoutOps, RNode, with_tree};
use day_spec::{Event, Icon, NodeId, Proposal, Rect, Size, ffi_guard};
use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSImage, NSScreen, NSToolbar, NSToolbarDelegate, NSToolbarDisplayMode, NSToolbarItem,
    NSToolbarItemIdentifier, NSView, NSWindowToolbarStyle,
};
use objc2_foundation::{NSArray, NSObject, NSObjectNSDelayedPerforming, NSString};

/// The panel piece's kind under `day_appkit::RENDERERS`.
const KIND: &str = "day-piece-settings.panel";

/// The custom event a toolbar pick reports: `text` is the group id.
const SELECT_TAG: &str = "day-piece-settings:select";

/// The glyph a group without an icon shows in the bar: a blank item would read as a gap.
const FALLBACK_SYMBOL: &str = "gearshape";

/// Room left between the panel and the screen's visible edges when a page is taller than the
/// screen, and the floor the panel never shrinks under.
const SCREEN_MARGIN: f64 = 40.0;
const MIN_HEIGHT: f64 = 120.0;

/// One toolbar item: a visible group.
struct PanelItem {
    id: String,
    title: String,
    icon: Option<GroupIcon>,
}

/// What the layout and the native view share: the selected page's natural height (clamped to
/// the screen) and the hook that asks the view to fit its window to it.
#[derive(Default)]
struct PanelShared {
    wanted: Cell<f64>,
    /// The tallest page the screen has room for; `INFINITY` until the view is on a window.
    max_height: Cell<f64>,
    on_measured: RefCell<Option<Box<dyn Fn()>>>,
}

impl PanelShared {
    fn ceiling(&self) -> f64 {
        let m = self.max_height.get();
        if m > 0.0 { m } else { f64::INFINITY }
    }
    /// Record the page height the window should have; a change schedules a fit.
    fn want(&self, height: f64) {
        if (self.wanted.get() - height).abs() < 0.5 {
            return;
        }
        self.wanted.set(height);
        if let Some(f) = self.on_measured.borrow().as_ref() {
            f();
        }
    }
}

struct PanelProps {
    items: Vec<PanelItem>,
    selected: String,
    shared: Rc<PanelShared>,
}

enum PanelPatch {
    /// Highlight this group's item (a programmatic selection: `open_at`, a deep link).
    Select(String),
    /// New labels after a locale change, `(id, title)` per item.
    Relabel(Vec<(String, String)>),
}

/// The preferences window body: the toolbar tabs over the selected group's page. `None` with
/// one visible group, where the composed page alone is the panel.
pub(super) fn window_shell(inst: Rc<Installed>) -> Option<AnyPiece> {
    let groups = inst.visible_groups();
    if groups.len() <= 1 {
        return None;
    }
    let selected = inst.selected.get_untracked();
    if !groups.iter().any(|g| g.id == selected) {
        inst.selected.set(groups[0].id.to_string());
    }
    let items = groups
        .iter()
        .map(|g| PanelItem {
            id: g.id.to_string(),
            title: g.title.initial(),
            icon: g.icon.clone(),
        })
        .collect();
    // The title names the selected pane, as a toolbar-tab panel's does.
    let i = inst.clone();
    day_core::window_title(move || {
        let id = i.selected.get();
        match i.group(&id) {
            Some(g) => tracked(&g.title),
            None => tracked(&i.title),
        }
    });
    Some(AnyPiece::new(Panel { inst, items }))
}

/// A tracked read of a text source (subscribes to the locale behind a localized string).
fn tracked(t: &TextSource) -> String {
    match t {
        TextSource::Static(s) => s.clone(),
        TextSource::Dyn(f) => f(),
    }
}

struct Panel {
    inst: Rc<Installed>,
    items: Vec<PanelItem>,
}

impl Piece for Panel {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let Panel { inst, items } = self;
        let shared = Rc::new(PanelShared::default());
        let ids: Vec<&'static str> = inst.visible_groups().iter().map(|g| g.id).collect();
        let node = cx.native(
            KIND,
            &PanelProps {
                items,
                selected: inst.selected.get_untracked(),
                shared: shared.clone(),
            },
            Rc::new(PanelLayout { shared }),
            Flex {
                grow_w: true,
                grow_h: true,
                ..Default::default()
            },
            Boundary::Yes,
        );
        // One arm per group; exactly the selected one is mounted, in its own scope.
        cx.under(node, |cx| {
            let arms: Vec<AnyPiece> = ids
                .iter()
                .map(|&id| {
                    let i = inst.clone();
                    let b = inst.clone();
                    AnyPiece::new(when(
                        move || i.selected.get() == id,
                        move || match b.group(id) {
                            Some(g) => group_page(b.clone(), g, true),
                            None => column(()).any(),
                        },
                    ))
                })
                .collect();
            let _ = column(PieceVec(arms)).grow_w().build(cx);
        });
        let i = inst.clone();
        cx.on(node, move |ev| {
            if let Event::Custom { tag, text, .. } = ev
                && *tag == SELECT_TAG
            {
                i.selected.set_if_changed(text.clone());
            }
        });
        // A selection made elsewhere (`open_at`, a route) moves the highlight too.
        let i = inst.clone();
        watch(
            move || i.selected.get(),
            move |id, _| {
                let id = id.clone();
                with_tree(|t| t.patch(node, Box::new(PanelPatch::Select(id)), false));
            },
        );
        // The labels follow the locale.
        let i = inst.clone();
        watch(
            move || {
                i.groups
                    .iter()
                    .map(|g| (g.id.to_string(), tracked(&g.title)))
                    .collect::<Vec<_>>()
            },
            move |labels, prev| {
                if prev.is_some() {
                    let labels = labels.clone();
                    with_tree(|t| t.patch(node, Box::new(PanelPatch::Relabel(labels)), false));
                }
            },
        );
        node
    }
}

/// The page fills the panel; the panel's own height is the page's natural height, clamped to
/// what the screen has room for, so a window that fits its content reads it straight off
/// this node's frame (day-core's `size_to_fit`), and a tab switch re-measures here.
struct PanelLayout {
    shared: Rc<PanelShared>,
}

impl Layout for PanelLayout {
    fn measure(&self, cx: &mut dyn LayoutOps, children: &[RNode], p: Proposal) -> Size {
        let Some(&c) = children.first() else {
            return Size::ZERO;
        };
        let natural = cx.measure_child(c, Proposal::new(p.width, None));
        let height = natural
            .height
            .min(self.shared.ceiling())
            .ceil()
            .max(MIN_HEIGHT);
        self.shared.want(height);
        Size::new(p.width.unwrap_or(natural.width), height)
    }
    fn place(&self, cx: &mut dyn LayoutOps, children: &[RNode], bounds: Rect) {
        if let Some(&c) = children.first() {
            cx.place_child(c, Rect::from_size(bounds.size));
        }
    }
}

// --- the native view: toolbar delegate, item target, window fitter ------------------------

struct ViewIvars {
    node: NodeId,
    shared: Rc<PanelShared>,
    items: RefCell<Vec<PanelItem>>,
    selected: RefCell<String>,
    toolbar: RefCell<Option<Retained<NSToolbar>>>,
    /// Set while the window animates to a new height (`setFrame:display:animate:` spins the
    /// run loop, and the resizes it reports lay the tree out again).
    fitting: Cell<bool>,
}

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "DaySettingsPanel"]
    #[ivars = ViewIvars]
    struct PanelView;

    unsafe impl NSObjectProtocol for PanelView {}

    impl PanelView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        /// The view meets its window once the tree mounts it: that is when the bar goes on.
        #[unsafe(method(viewDidMoveToWindow))]
        fn view_did_move_to_window(&self) {
            ffi_guard::contain((), || self.install_toolbar());
        }

        #[unsafe(method(fitWindow))]
        fn fit_window(&self) {
            ffi_guard::contain((), || self.fit());
        }

        #[unsafe(method(pick:))]
        fn pick(&self, sender: &AnyObject) {
            ffi_guard::contain((), || {
                if let Some(item) = sender.downcast_ref::<NSToolbarItem>() {
                    let id = item.itemIdentifier().to_string();
                    *self.ivars().selected.borrow_mut() = id.clone();
                    day_appkit::emit(self.ivars().node, Event::custom(SELECT_TAG, id));
                }
            });
        }
    }

    unsafe impl NSToolbarDelegate for PanelView {
        #[unsafe(method_id(toolbar:itemForItemIdentifier:willBeInsertedIntoToolbar:))]
        fn item_for_identifier(
            &self,
            _toolbar: &NSToolbar,
            identifier: &NSToolbarItemIdentifier,
            _inserted: bool,
        ) -> Option<Retained<NSToolbarItem>> {
            ffi_guard::contain(None, || self.make_item(&identifier.to_string()))
        }

        #[unsafe(method_id(toolbarDefaultItemIdentifiers:))]
        fn default_identifiers(&self, _toolbar: &NSToolbar) -> Retained<NSArray<NSToolbarItemIdentifier>> {
            self.identifiers()
        }

        #[unsafe(method_id(toolbarAllowedItemIdentifiers:))]
        fn allowed_identifiers(&self, _toolbar: &NSToolbar) -> Retained<NSArray<NSToolbarItemIdentifier>> {
            self.identifiers()
        }

        #[unsafe(method_id(toolbarSelectableItemIdentifiers:))]
        fn selectable_identifiers(&self, _toolbar: &NSToolbar) -> Retained<NSArray<NSToolbarItemIdentifier>> {
            self.identifiers()
        }
    }
);

impl PanelView {
    fn new(mtm: MainThreadMarker, p: &PanelProps, node: NodeId) -> Retained<Self> {
        let items = p
            .items
            .iter()
            .map(|i| PanelItem {
                id: i.id.clone(),
                title: i.title.clone(),
                icon: i.icon.clone(),
            })
            .collect();
        let this = Self::alloc(mtm).set_ivars(ViewIvars {
            node,
            shared: p.shared.clone(),
            items: RefCell::new(items),
            selected: RefCell::new(p.selected.clone()),
            toolbar: RefCell::new(None),
            fitting: Cell::new(false),
        });
        let this: Retained<Self> = unsafe { msg_send![super(this), init] };
        // The layout's hook: a weak reference, so the view (which owns `shared`) is no cycle.
        let weak = Weak::from_retained(&this);
        *p.shared.on_measured.borrow_mut() = Some(Box::new(move || {
            if let Some(view) = weak.load() {
                view.schedule_fit();
            }
        }));
        this
    }

    fn identifiers(&self) -> Retained<NSArray<NSToolbarItemIdentifier>> {
        let names: Vec<Retained<NSString>> = self
            .ivars()
            .items
            .borrow()
            .iter()
            .map(|i| NSString::from_str(&i.id))
            .collect();
        let refs: Vec<&NSToolbarItemIdentifier> = names.iter().map(|n| n.as_ref()).collect();
        NSArray::from_slice(&refs)
    }

    /// One group's item: its glyph above its title, selectable, firing `pick:` at this view.
    fn make_item(&self, ident: &str) -> Option<Retained<NSToolbarItem>> {
        let mtm = self.mtm();
        let (title, icon) = {
            let items = self.ivars().items.borrow();
            let it = items.iter().find(|i| i.id == ident)?;
            (it.title.clone(), it.icon.clone())
        };
        let id = NSString::from_str(ident);
        let label = NSString::from_str(&title);
        let item = NSToolbarItem::initWithItemIdentifier(NSToolbarItem::alloc(mtm), &id);
        let image = match &icon {
            Some(GroupIcon::Symbol(s)) => day_appkit::toolbar_image(&Icon::Symbol(*s), &title, mtm),
            Some(GroupIcon::Image(name)) => {
                day_appkit::toolbar_image(&Icon::Image(name.as_str().to_owned()), &title, mtm)
            }
            None => None,
        };
        let image = image.or_else(|| {
            NSImage::imageWithSystemSymbolName_accessibilityDescription(
                &NSString::from_str(FALLBACK_SYMBOL),
                Some(&label),
            )
        });
        if let Some(img) = &image {
            item.setImage(Some(img));
        }
        item.setLabel(&label);
        item.setPaletteLabel(&label);
        item.setToolTip(Some(&label));
        // Day owns the enabled state; AppKit's validation would gray out an item whose target
        // has no `validateToolbarItem:`.
        item.setAutovalidates(false);
        item.setEnabled(true);
        unsafe {
            item.setTarget(Some(self));
            item.setAction(Some(sel!(pick:)));
        }
        Some(item)
    }

    /// Put the preference-style bar on the window the view just joined, once.
    fn install_toolbar(&self) {
        let Some(window) = self.window() else {
            return;
        };
        if self.ivars().toolbar.borrow().is_some() {
            return;
        }
        let mtm = self.mtm();
        let toolbar = NSToolbar::initWithIdentifier(
            NSToolbar::alloc(mtm),
            &NSString::from_str("day-piece-settings.panel"),
        );
        // The groups are the app's declaration: nothing to customize or remember.
        toolbar.setAllowsUserCustomization(false);
        toolbar.setAutosavesConfiguration(false);
        toolbar.setDisplayMode(NSToolbarDisplayMode::IconAndLabel);
        toolbar.setDelegate(Some(ProtocolObject::from_ref(self)));
        window.setToolbarStyle(NSWindowToolbarStyle::Preference);
        window.setToolbar(Some(&toolbar));
        let selected = self.ivars().selected.borrow().clone();
        toolbar.setSelectedItemIdentifier(Some(&NSString::from_str(&selected)));
        *self.ivars().toolbar.borrow_mut() = Some(toolbar);

        // The tallest page this screen has room for, below the title bar and bar.
        let screen = window.screen().or_else(|| NSScreen::mainScreen(mtm));
        if let (Some(screen), Some(content)) = (screen, window.contentView()) {
            let visible = screen.visibleFrame().size.height;
            let chrome = window.frame().size.height - content.frame().size.height;
            let room = visible - chrome - 2.0 * SCREEN_MARGIN;
            self.ivars().shared.max_height.set(room.max(MIN_HEIGHT));
        }
    }

    /// Fit on the next run-loop turn, coalescing the asks a layout pass makes.
    fn schedule_fit(&self) {
        unsafe {
            NSObject::cancelPreviousPerformRequestsWithTarget_selector_object(
                self,
                sel!(fitWindow),
                None,
            );
            self.performSelector_withObject_afterDelay(sel!(fitWindow), None, 0.0);
        }
    }

    /// Animate the window to the wanted page height, top edge pinned, the way a preferences
    /// panel changes panes. Runs outside any layout pass (a deferred perform), because the
    /// resizes it reports lay the tree out again.
    fn fit(&self) {
        let iv = self.ivars();
        if iv.fitting.get() {
            return;
        }
        let Some(window) = self.window() else {
            return;
        };
        let Some(content) = window.contentView() else {
            return;
        };
        let wanted = iv.shared.wanted.get();
        if wanted <= 0.0 {
            return;
        }
        let delta = wanted - content.frame().size.height;
        if delta.abs() < 0.5 {
            return;
        }
        let mut frame = window.frame();
        frame.size.height += delta;
        frame.origin.y -= delta;
        iv.fitting.set(true);
        window.setFrame_display_animate(frame, true, true);
        iv.fitting.set(false);
        // A page re-measured mid-animation wants another pass.
        if (iv.shared.wanted.get() - content.frame().size.height).abs() >= 0.5 {
            self.schedule_fit();
        }
    }
}

fn make(backend: &mut AppKit, p: &PanelProps, id: NodeId) -> Retained<NSView> {
    Retained::into_super(PanelView::new(backend.mtm(), p, id))
}

fn update(_backend: &mut AppKit, h: &Retained<NSView>, patch: &PanelPatch) {
    let Some(view) = h.downcast_ref::<PanelView>() else {
        return;
    };
    let iv = view.ivars();
    match patch {
        PanelPatch::Select(id) => {
            *iv.selected.borrow_mut() = id.clone();
            if let Some(toolbar) = iv.toolbar.borrow().as_ref() {
                let current = toolbar.selectedItemIdentifier().map(|s| s.to_string());
                if current.as_deref() != Some(id.as_str()) {
                    toolbar.setSelectedItemIdentifier(Some(&NSString::from_str(id)));
                }
            }
        }
        PanelPatch::Relabel(labels) => {
            {
                let mut items = iv.items.borrow_mut();
                for (id, title) in labels {
                    if let Some(it) = items.iter_mut().find(|i| i.id == *id) {
                        it.title = title.clone();
                    }
                }
            }
            if let Some(toolbar) = iv.toolbar.borrow().as_ref() {
                for item in toolbar.items().iter() {
                    let id = item.itemIdentifier().to_string();
                    if let Some((_, title)) = labels.iter().find(|(i, _)| *i == id) {
                        let label = NSString::from_str(title);
                        item.setLabel(&label);
                        item.setPaletteLabel(&label);
                        item.setToolTip(Some(&label));
                    }
                }
            }
        }
    }
}

/// The bar holds its delegate weakly and the layout's hook holds the view weakly, so teardown
/// is detaching both before the view goes.
fn release(_backend: &mut AppKit, h: &Retained<NSView>) {
    let Some(view) = h.downcast_ref::<PanelView>() else {
        return;
    };
    let iv = view.ivars();
    unsafe {
        NSObject::cancelPreviousPerformRequestsWithTarget_selector_object(
            view,
            sel!(fitWindow),
            None,
        );
    }
    *iv.shared.on_measured.borrow_mut() = None;
    if let Some(toolbar) = iv.toolbar.borrow_mut().take() {
        toolbar.setDelegate(None);
        if let Some(window) = view.window() {
            window.setToolbar(None);
        }
    }
}

day_pieces::renderer!(day_appkit::RENDERERS, AppKit,
    kind: KIND, props: PanelProps, patch: PanelPatch,
    make: make, update: update, measure: day_pieces::fill_measure, release: release);
