// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
//! Window chrome (docs/window-chrome.md): frameless and overlay title bars, transparent and
//! material backgrounds, placement, and the drag regions a custom title bar is made of.
use super::*;
use day_spec::{WindowBackground, WindowChrome, WindowMaterial, WindowOptions, WindowPlacement};
use objc2_app_kit::{
    NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView,
    NSWindowStyleMask, NSWindowTitleVisibility,
};

define_class!(
    /// An `NSWindow` that can become key and main without a title bar. AppKit refuses both to a
    /// borderless window by default, which would leave a frameless window unable to take a
    /// single keystroke.
    #[unsafe(super(NSWindow, objc2_app_kit::NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DayFramelessWindow"]
    pub(super) struct DayFramelessWindow;

    impl DayFramelessWindow {
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key_window(&self) -> bool {
            true
        }

        #[unsafe(method(canBecomeMainWindow))]
        fn can_become_main_window(&self) -> bool {
            true
        }
    }
);

/// The style mask a window opens with: the kind's (preferences drop resize and minimize), then
/// the chrome's.
pub(super) fn style_mask(options: &WindowOptions, prefs_style: bool) -> NSWindowStyleMask {
    let resizable = options.resizable && !prefs_style;
    if options.chrome == WindowChrome::Frameless {
        let mut style = NSWindowStyleMask::Borderless | NSWindowStyleMask::Miniaturizable;
        if resizable {
            style |= NSWindowStyleMask::Resizable;
        }
        return style;
    }
    let mut style = NSWindowStyleMask::Titled | NSWindowStyleMask::Closable;
    if !prefs_style {
        style |= NSWindowStyleMask::Miniaturizable;
        if resizable {
            style |= NSWindowStyleMask::Resizable;
        }
        // The unified title bar runs the full width, which is what lets a toolbar's
        // SIDEBAR TRACKING SEPARATOR find the split's divider and pin the sidebar's own
        // items over the sidebar (docs/toolbars.md). AppKit only configures that item on a
        // window with this mask, and Mail, Finder and Notes all carry it.
        style |= NSWindowStyleMask::FullSizeContentView;
    }
    if options.chrome == WindowChrome::Overlay {
        style |= NSWindowStyleMask::FullSizeContentView;
    }
    style
}

/// Allocate the window: the key-capable subclass for a frameless one, a plain `NSWindow`
/// otherwise.
pub(super) fn alloc_window(
    mtm: MainThreadMarker,
    rect: NSRect,
    style: NSWindowStyleMask,
    frameless: bool,
) -> Retained<NSWindow> {
    if frameless {
        let this = DayFramelessWindow::alloc(mtm).set_ivars(());
        let window: Retained<DayFramelessWindow> = unsafe {
            msg_send![
                super(this),
                initWithContentRect: rect,
                styleMask: style,
                backing: NSBackingStoreType::Buffered,
                defer: false
            ]
        };
        return Retained::into_super(window);
    }
    unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            rect,
            style,
            NSBackingStoreType::Buffered,
            false,
        )
    }
}

/// Everything about the chrome that is set after the window exists: the overlay title bar, the
/// background, the shadow.
pub(super) fn apply(window: &NSWindow, content: &NSView, options: &WindowOptions) {
    let mtm = window.mtm();
    if options.chrome == WindowChrome::Overlay {
        // The title bar stays, drawn over the content: the traffic lights keep their place
        // and the content runs to the top edge. `pin_below_title_bar` reads this flag to stop
        // pushing the content down and report the bar's height as a safe-area inset instead.
        window.setTitlebarAppearsTransparent(true);
        window.setTitleVisibility(NSWindowTitleVisibility::Hidden);
    }
    unsafe { window.setHasShadow(options.shadow) };
    match options.background {
        WindowBackground::Opaque => {}
        WindowBackground::Transparent => {
            window.setOpaque(false);
            window.setBackgroundColor(Some(&NSColor::clearColor()));
        }
        WindowBackground::Material(material) => {
            let effect = NSVisualEffectView::new(mtm);
            effect.setMaterial(match material {
                WindowMaterial::Window => NSVisualEffectMaterial::WindowBackground,
                WindowMaterial::Sidebar => NSVisualEffectMaterial::Sidebar,
                WindowMaterial::Transient => NSVisualEffectMaterial::Popover,
            });
            effect.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
            // Window and sidebar materials follow the window's active state, as the system's
            // own do: vibrant while the window is key, flat while it is behind. A transient
            // panel (a popover, a heads-up display) stays vibrant.
            effect.setState(if material == WindowMaterial::Transient {
                NSVisualEffectState::Active
            } else {
                NSVisualEffectState::FollowsWindowActiveState
            });
            effect.setFrame(content.bounds());
            effect.setAutoresizingMask(
                objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable
                    | objc2_app_kit::NSAutoresizingMaskOptions::ViewHeightSizable,
            );
            // Behind everything Day builds into the content view.
            content.addSubview_positioned_relativeTo(
                &effect,
                objc2_app_kit::NSWindowOrderingMode::Below,
                None,
            );
            window.setOpaque(false);
            window.setBackgroundColor(Some(&NSColor::clearColor()));
        }
    }
}

/// Put the window where `placement` says. `Automatic` keeps whatever the caller already did
/// (the cascade for ordinary windows, centering for preferences).
pub(super) fn place(window: &NSWindow, placement: WindowPlacement) {
    match placement {
        WindowPlacement::Automatic => {}
        WindowPlacement::Centered => window.center(),
        WindowPlacement::At(p) => {
            // Day's desktop coordinates run down from the top of the main screen; AppKit's run
            // up from its bottom.
            let mtm = window.mtm();
            let height = objc2_app_kit::NSScreen::mainScreen(mtm)
                .map(|s| s.frame().size.height)
                .unwrap_or(0.0);
            window.setFrameTopLeftPoint(NSPoint::new(p.x, height - p.y));
        }
    }
}

/// Whether the window runs its content under the title bar (`WindowChrome::Overlay`): the one
/// window Day asks for a transparent title bar.
pub(super) fn is_overlay(window: &NSWindow) -> bool {
    window.titlebarAppearsTransparent()
        && window
            .styleMask()
            .contains(NSWindowStyleMask::FullSizeContentView)
}

/// Tell day-core how tall the title bar over an overlay window's content is, as the window's
/// safe-area top inset (docs/window-chrome.md). A window's content reads it while it builds, so
/// the report has to be there first: a secondary window's lands synchronously (day-core builds
/// the content right after `open_window` returns), and the first window's is seeded before the
/// tree exists. Later changes (a toolbar added under the bar) arrive as ordinary reports.
pub(super) fn report_title_bar_inset(window: &NSWindow, top: f64) {
    let node = window
        .delegate()
        .and_then(|d| d.downcast::<DayWinDelegate>().ok())
        .and_then(|d| d.ivars().node);
    let insets = day_spec::Insets {
        top,
        ..Default::default()
    };
    match node {
        // Only a signal write: safe inside the tree's borrow, and its readers run at turn end.
        Some(node) => day_core::set_window_safe_area(day_core::id_to_rnode(node), insets),
        None if !day_core::has_tree() => day_core::seed_safe_area(insets),
        // The first window, after boot: `set_safe_area` looks the root up in the tree, which the
        // caller may hold, so it goes on the next turn.
        None => {
            <AppKit as day_spec::Platform>::post(Box::new(move || day_core::set_safe_area(insets)))
        }
    }
}

// ---------------------------------------------------------------------------
// Drag regions: `.window_drag_region()`
// ---------------------------------------------------------------------------

thread_local! {
    /// The views marked as drag regions. Strong references, so an entry can never dangle;
    /// views that have left their window are dropped on the next press.
    static REGIONS: RefCell<Vec<Retained<NSView>>> = const { RefCell::new(Vec::new()) };
    static DRAG_MONITOR: RefCell<Option<DragMonitor>> = const { RefCell::new(None) };
}

struct DragMonitor(Retained<objc2::runtime::AnyObject>);
impl Drop for DragMonitor {
    fn drop(&mut self) {
        unsafe { NSEvent::removeMonitor(&self.0) };
    }
}

/// Mark or unmark `view` as a drag region.
pub(super) fn set_drag_region(view: &NSView, drag: bool) {
    REGIONS.with(|r| {
        let mut regions = r.borrow_mut();
        regions.retain(|v| ptr_of(v) != ptr_of(view));
        if drag {
            regions.push(Retained::from(view));
        }
    });
    if drag {
        install_monitor();
    }
}

/// Whether a press on `view` belongs to the view itself rather than to the region around it:
/// a button, an editable or selectable field, a text view, any other control.
fn is_interactive(view: &NSView) -> bool {
    if let Some(field) = view.downcast_ref::<NSTextField>() {
        return field.isEditable() || field.isSelectable();
    }
    view.downcast_ref::<objc2_app_kit::NSControl>().is_some()
        || view.downcast_ref::<NSTextView>().is_some()
}

/// What a double click on a title bar does, per the user's choice in System Settings ▸ Desktop &
/// Dock: zoom (the default), minimize, or nothing.
fn double_click(window: &NSWindow) {
    let defaults = unsafe { objc2_foundation::NSUserDefaults::standardUserDefaults() };
    let action = unsafe { defaults.stringForKey(&NSString::from_str("AppleActionOnDoubleClick")) }
        .map(|s| s.to_string());
    match action.as_deref() {
        Some("None") => {}
        Some("Minimize") => window.performMiniaturize(None),
        // Older systems spelled the minimize choice as its own boolean.
        Some(_) => window.performZoom(None),
        None => {
            if unsafe { defaults.boolForKey(&NSString::from_str("AppleMiniaturizeOnDoubleClick")) }
            {
                window.performMiniaturize(None)
            } else {
                window.performZoom(None)
            }
        }
    }
}

fn install_monitor() {
    DRAG_MONITOR.with(|slot| {
        if slot.borrow().is_some() {
            return;
        }
        let block = block2::RcBlock::new(|raw: std::ptr::NonNull<NSEvent>| -> *mut NSEvent {
            let event = unsafe { raw.as_ref() };
            ffi_guard::contain(raw.as_ptr(), || {
                let Some(mtm) = MainThreadMarker::new() else {
                    return raw.as_ptr();
                };
                let Some(window) = event.window(mtm) else {
                    return raw.as_ptr();
                };
                let Some(content) = window.contentView() else {
                    return raw.as_ptr();
                };
                REGIONS.with(|r| r.borrow_mut().retain(|v| v.window().is_some()));
                // `hitTest:` takes a point in the receiver's superview's coordinates, and a content
                // view's superview spans the window: the event's window location as it is.
                let mut view = content.hitTest(unsafe { event.locationInWindow() });
                while let Some(v) = view {
                    // A control inside the region keeps its own click.
                    if is_interactive(&v) {
                        return raw.as_ptr();
                    }
                    if REGIONS.with(|r| r.borrow().iter().any(|x| ptr_of(x) == ptr_of(&v))) {
                        if unsafe { event.clickCount() } == 2 {
                            double_click(&window);
                        } else {
                            log::debug!("appkit: window drag from a drag region");
                            window.performWindowDragWithEvent(event);
                        }
                        return std::ptr::null_mut();
                    }
                    view = unsafe { v.superview() };
                }
                raw.as_ptr()
            })
        });
        *slot.borrow_mut() = unsafe {
            NSEvent::addLocalMonitorForEventsMatchingMask_handler(
                objc2_app_kit::NSEventMask::LeftMouseDown,
                &block,
            )
        }
        .map(DragMonitor);
    });
}

// ---------------------------------------------------------------------------
// Window properties (docs/windows.md "Window properties") and displays
// ---------------------------------------------------------------------------

/// The height of the display AppKit measures from: screen coordinates run up from the bottom of
/// the first screen, Day's run down from its top.
fn desktop_height(mtm: MainThreadMarker) -> f64 {
    objc2_app_kit::NSScreen::screens(mtm)
        .iter()
        .next()
        .map(|s| s.frame().size.height)
        .unwrap_or(0.0)
}

/// An AppKit screen rectangle in Day's desktop coordinates (top-left origin, points).
fn to_day(r: NSRect, height: f64) -> day_spec::Rect {
    day_spec::Rect::new(
        r.origin.x,
        height - (r.origin.y + r.size.height),
        r.size.width,
        r.size.height,
    )
}

/// The window's outer frame on the desktop.
pub(super) fn frame_of(window: &NSWindow) -> day_spec::Rect {
    to_day(window.frame(), desktop_height(window.mtm()))
}

/// Every attached display.
pub(super) fn monitors(mtm: MainThreadMarker) -> Vec<day_spec::Monitor> {
    let height = desktop_height(mtm);
    objc2_app_kit::NSScreen::screens(mtm)
        .iter()
        .enumerate()
        .map(|(i, s)| {
            // The CGDirectDisplayID: stable while the display stays attached.
            let number: Option<Retained<objc2::runtime::AnyObject>> = unsafe {
                let desc = s.deviceDescription();
                msg_send![&desc, objectForKey: &*NSString::from_str("NSScreenNumber")]
            };
            let id = number
                .map(|n| {
                    let v: u32 = unsafe { msg_send![&n, unsignedIntValue] };
                    v.to_string()
                })
                .unwrap_or_else(|| i.to_string());
            day_spec::Monitor {
                id,
                name: s.localizedName().to_string(),
                frame: to_day(s.frame(), height),
                work_area: to_day(s.visibleFrame(), height),
                scale: s.backingScaleFactor(),
                primary: i == 0,
            }
        })
        .collect()
}

/// Every [`day_spec::WindowChange`] except the display state and content protection, which
/// `apply_window` handles itself.
pub(super) fn apply_property(window: &NSWindow, change: &day_spec::WindowChange) {
    use day_spec::WindowChange as C;
    let mtm = window.mtm();
    let set_mask = |bit: NSWindowStyleMask, on: bool| {
        let mask = window.styleMask();
        window.setStyleMask(if on { mask | bit } else { mask & !bit });
    };
    match change {
        C::Frame { origin, size } => {
            if let Some(size) = size {
                // `size` is what Day lays out. A standard window's content view runs under its
                // title bar (full-size content), so the bar's height goes back on top; an overlay
                // window lays out under the bar already.
                let top = match (window.contentView(), is_overlay(window)) {
                    (Some(content), false) => (content.frame().size.height
                        - unsafe { window.contentLayoutRect() }.size.height)
                        .max(0.0),
                    _ => 0.0,
                };
                window.setContentSize(NSSize::new(size.width, size.height + top));
            }
            if let Some(p) = origin {
                window.setFrameTopLeftPoint(NSPoint::new(p.x, desktop_height(mtm) - p.y));
            }
        }
        C::Limits { min, max } => {
            let min = min.unwrap_or(Size::new(0.0, 0.0));
            let max = max.unwrap_or(Size::new(f64::MAX, f64::MAX));
            unsafe {
                window.setContentMinSize(NSSize::new(min.width, min.height));
                window.setContentMaxSize(NSSize::new(max.width, max.height));
            }
        }
        C::Level(level) => window.setLevel(match level {
            // NSFloatingWindowLevel, the level of palettes and utility panels.
            day_spec::WindowLevel::Floating => 3,
            day_spec::WindowLevel::Normal => 0,
        }),
        C::OnAllWorkspaces(on) => {
            let b = window.collectionBehavior();
            let all = objc2_app_kit::NSWindowCollectionBehavior::CanJoinAllSpaces;
            window.setCollectionBehavior(if *on { b | all } else { b & !all });
        }
        // macOS has no taskbar; the window list it does have is the Window menu.
        C::SkipTaskbar(skip) => unsafe { window.setExcludedFromWindowsMenu(*skip) },
        C::Resizable(on) => set_mask(NSWindowStyleMask::Resizable, *on),
        C::Minimizable(on) => set_mask(NSWindowStyleMask::Miniaturizable, *on),
        C::Closable(on) => set_mask(NSWindowStyleMask::Closable, *on),
        // The zoom button is the only thing a maximizable window has that another lacks.
        C::Maximizable(on) => {
            if let Some(zoom) =
                window.standardWindowButton(objc2_app_kit::NSWindowButton::ZoomButton)
            {
                zoom.setEnabled(*on);
            }
        }
        C::Visible(true) => window.makeKeyAndOrderFront(None),
        C::Visible(false) => window.orderOut(None),
        C::Appearance(dark) => {
            let name = dark.map(|d| unsafe {
                if d {
                    objc2_app_kit::NSAppearanceNameDarkAqua
                } else {
                    objc2_app_kit::NSAppearanceNameAqua
                }
            });
            let appearance =
                name.and_then(|n| unsafe { objc2_app_kit::NSAppearance::appearanceNamed(n) });
            window.setAppearance(appearance.as_deref());
        }
        C::RequestAttention(a) => {
            let kind = match a {
                day_spec::Attention::Critical => {
                    objc2_app_kit::NSRequestUserAttentionType::CriticalRequest
                }
                day_spec::Attention::Informational => {
                    objc2_app_kit::NSRequestUserAttentionType::InformationalRequest
                }
            };
            let _ = NSApplication::sharedApplication(mtm).requestUserAttention(kind);
        }
        C::State(_) | C::ContentProtected(_) => {}
    }
}
