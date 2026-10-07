// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
//! The desktop shell around Day's windows (docs/window-chrome.md, docs/windows.md,
//! docs/status-item.md): window chrome and drag regions, window properties and displays, status
//! items, launcher progress and badges, and staying alive with no window open.
//!
//! GTK 4 decides what is possible here. It removed window positioning, keep-above, stick,
//! skip-taskbar and the urgency hint, so those answer Unsupported; it has no maximum window
//! size either. What it does have is used as GNOME apps use it: client-side decorations,
//! `GtkWindowControls` for an app-drawn title bar, and `gdk::Toplevel`'s move and resize
//! requests, which every compositor and window manager honors with its own snapping.
use std::cell::{Cell, RefCell};

use adw::prelude::*;
use gtk4::glib::WeakRef;
use libadwaita as adw;

use day_spec::{Event, NodeId, WindowBackground, WindowChrome, WindowOptions};

use crate::{emit, ffi_guard};

// ---------------------------------------------------------------------------
// Per-window chrome bookkeeping
// ---------------------------------------------------------------------------

/// One Day window's chrome: its header bar (always made, shown only for `Standard`), the window
/// controls an `Overlay` window draws over its content, and which title-bar buttons the app
/// turned off.
struct Shell {
    window: WeakRef<gtk4::Window>,
    header: adw::HeaderBar,
    shows_header: bool,
    controls: Vec<gtk4::WindowControls>,
    /// The overlay band's height (the safe-area top inset), `None` for other chrome.
    overlay_band: Option<f64>,
    minimizable: Cell<bool>,
    maximizable: Cell<bool>,
}

thread_local! {
    static SHELLS: RefCell<Vec<Shell>> = const { RefCell::new(Vec::new()) };
    static KEEP_RUNNING: RefCell<Option<gtk4::gio::ApplicationHoldGuard>> =
        const { RefCell::new(None) };
}

fn with_shell<R>(window: &gtk4::Window, f: impl FnOnce(&Shell) -> R) -> Option<R> {
    SHELLS.with(|s| {
        s.borrow()
            .iter()
            .find(|x| x.window.upgrade().as_ref() == Some(window))
            .map(f)
    })
}

/// The height of the header bar over `window`'s content, 0 when it shows none (an `Overlay` or
/// `Frameless` window lays out to its top edge).
pub(crate) fn bar_height(window: &gtk4::Window) -> f64 {
    with_shell(window, |s| {
        if !s.shows_header {
            return 0.0;
        }
        let h = s.header.height();
        if h > 0 {
            h as f64
        } else {
            f64::from(s.header.measure(gtk4::Orientation::Vertical, -1).1.max(0))
        }
    })
    .unwrap_or(0.0)
}

/// The safe-area top inset an `Overlay` window reports, `None` for any other chrome.
pub(crate) fn overlay_inset(window: &gtk4::Window) -> Option<f64> {
    with_shell(window, |s| s.overlay_band).flatten()
}

/// Put `content` (the `AdwToolbarView` holding Day's tree) into `window` with the chrome
/// `options` asks for, and apply the options that live on the window itself: size limits,
/// resizability, background, shadow. Returns whether the header bar is shown.
pub(crate) fn dress(
    window: &adw::ApplicationWindow,
    toolbar: &adw::ToolbarView,
    header: &adw::HeaderBar,
    options: &WindowOptions,
) -> bool {
    let win: &gtk4::Window = window.upcast_ref();
    let mut controls = Vec::new();
    let mut overlay_band = None;
    let shows_header = match options.chrome {
        WindowChrome::Standard => {
            // A day toolbar packs into this bar (docs/toolbars.md); only a shown bar can hold
            // one, so the other chromes leave it unregistered.
            crate::toolbar::register_header(window, header);
            toolbar.add_top_bar(header);
            window.set_content(Some(toolbar));
            true
        }
        WindowChrome::Overlay => {
            // The content runs to the top edge; the platform's own window controls sit over
            // it at the corners, in a band as tall as the header bar the window would have
            // had, and that band is what the app's title bar pads by (docs/window-chrome.md).
            // The band's middle stays the app's: only the controls take presses.
            let band = header.measure(gtk4::Orientation::Vertical, -1).1;
            let band = if band > 0 { band } else { 47 };
            let overlay = gtk4::Overlay::new();
            overlay.set_child(Some(toolbar));
            for (side, align) in [
                (gtk4::PackType::Start, gtk4::Align::Start),
                (gtk4::PackType::End, gtk4::Align::End),
            ] {
                let holder = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
                holder.set_height_request(band);
                holder.set_halign(align);
                holder.set_valign(gtk4::Align::Start);
                let c = gtk4::WindowControls::new(side);
                c.set_valign(gtk4::Align::Center);
                c.set_margin_start(6);
                c.set_margin_end(6);
                holder.append(&c);
                overlay.add_overlay(&holder);
                controls.push(c);
            }
            window.set_content(Some(&overlay));
            overlay_band = Some(f64::from(band));
            false
        }
        WindowChrome::Frameless => {
            window.set_content(Some(toolbar));
            false
        }
    };
    window.set_resizable(options.resizable);
    // A window without the shadow drops the client-side decoration with it: GTK's shadow is
    // the decoration's margin, which is also where its resize edges live, so those come back
    // through `edge_resize` below.
    if !options.shadow {
        window.set_decorated(false);
    }
    if options.background == WindowBackground::Transparent {
        transparent_css();
        window.add_css_class("day-transparent");
    }
    if !window.is_decorated() {
        edge_resize(win);
    }
    let bar = if shows_header {
        f64::from(header.measure(gtk4::Orientation::Vertical, -1).1.max(0))
    } else {
        0.0
    };
    // The smallest content size, plus the bar above it. GTK 4 has no maximum window size, so
    // `max_size` is not enforced (`Cap::WindowGeometry` covers size and minimum only).
    if let Some(min) = options.min_size {
        win.set_size_request(min.width as i32, (min.height + bar) as i32);
    }
    // `placement`: GTK 4 removed window positioning (`gtk_window_move`, the position property),
    // and Wayland forbids it, so every placement is the window manager's choice. Mutter and
    // most X11 managers center a new window over its parent or the screen anyway.
    SHELLS.with(|s| {
        let mut s = s.borrow_mut();
        s.retain(|x| x.window.upgrade().is_some());
        s.push(Shell {
            window: win.downgrade(),
            header: header.clone(),
            shows_header,
            controls,
            overlay_band,
            minimizable: Cell::new(true),
            maximizable: Cell::new(true),
        });
    });
    shows_header
}

/// The display-wide rule a transparent window wears: nothing painted behind its content. Needs
/// a compositor; without one (bare X11) the window's unpainted pixels show black instead.
fn transparent_css() {
    thread_local! {
        static DONE: Cell<bool> = const { Cell::new(false) };
    }
    if DONE.with(|d| d.replace(true)) {
        return;
    }
    let p = gtk4::CssProvider::new();
    p.load_from_data(
        "window.day-transparent, window.day-transparent > * { background: none; box-shadow: none; }",
    );
    if let Some(display) = gtk4::gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(
            &display,
            &p,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

/// Whether this display can show a transparent window: a compositing manager runs (always on
/// Wayland; on X11 only with one) and the visual has alpha. `Cap::TransparentWindow`.
pub(crate) fn can_be_transparent() -> bool {
    gtk4::gdk::Display::default().is_some_and(|d| d.is_composited() && d.is_rgba())
}

// ---------------------------------------------------------------------------
// Edge resizing for an undecorated window
// ---------------------------------------------------------------------------

/// How close to a window edge a press resizes, in points: the width of GTK's own CSD resize
/// border.
const RESIZE_BORDER: f64 = 6.0;

fn edge_at(w: f64, h: f64, x: f64, y: f64) -> Option<gtk4::gdk::SurfaceEdge> {
    use gtk4::gdk::SurfaceEdge as E;
    let left = x < RESIZE_BORDER;
    let right = x > w - RESIZE_BORDER;
    let top = y < RESIZE_BORDER;
    let bottom = y > h - RESIZE_BORDER;
    Some(match (top, bottom, left, right) {
        (true, _, true, _) => E::NorthWest,
        (true, _, _, true) => E::NorthEast,
        (_, true, true, _) => E::SouthWest,
        (_, true, _, true) => E::SouthEast,
        (true, _, _, _) => E::North,
        (_, true, _, _) => E::South,
        (_, _, true, _) => E::West,
        (_, _, _, true) => E::East,
        _ => return None,
    })
}

fn edge_cursor(edge: gtk4::gdk::SurfaceEdge) -> &'static str {
    use gtk4::gdk::SurfaceEdge as E;
    match edge {
        E::NorthWest => "nw-resize",
        E::NorthEast => "ne-resize",
        E::SouthWest => "sw-resize",
        E::SouthEast => "se-resize",
        E::North => "n-resize",
        E::South => "s-resize",
        E::West => "w-resize",
        _ => "e-resize",
    }
}

/// An undecorated window has no resize border of its own (GTK draws it as part of the
/// decoration), so a press within [`RESIZE_BORDER`] of an edge asks the window manager for an
/// interactive resize from that edge, as GTK's own border does.
fn edge_resize(window: &gtk4::Window) {
    let click = gtk4::GestureClick::new();
    click.set_button(gtk4::gdk::BUTTON_PRIMARY);
    click.set_propagation_phase(gtk4::PropagationPhase::Capture);
    click.connect_pressed(|g, _n, x, y| {
        ffi_guard::contain((), || {
            let Some(w) = g.widget().and_downcast::<gtk4::Window>() else {
                return;
            };
            if !w.is_resizable() || w.is_maximized() || w.is_fullscreen() {
                return;
            }
            let Some(edge) = edge_at(f64::from(w.width()), f64::from(w.height()), x, y) else {
                return;
            };
            let Some(toplevel) = w
                .surface()
                .and_then(|s| s.downcast::<gtk4::gdk::Toplevel>().ok())
            else {
                return;
            };
            let (tx, ty) = w.surface_transform();
            toplevel.begin_resize(
                edge,
                g.device().as_ref(),
                g.current_button() as i32,
                x + tx,
                y + ty,
                g.current_event_time(),
            );
            g.set_state(gtk4::EventSequenceState::Claimed);
        })
    });
    window.add_controller(click);
    let motion = gtk4::EventControllerMotion::new();
    let shaped = std::rc::Rc::new(Cell::new(false));
    motion.connect_motion(move |m, x, y| {
        ffi_guard::contain((), || {
            let Some(w) = m.widget().and_downcast::<gtk4::Window>() else {
                return;
            };
            let edge = (w.is_resizable() && !w.is_maximized())
                .then(|| edge_at(f64::from(w.width()), f64::from(w.height()), x, y))
                .flatten();
            match edge {
                Some(e) => {
                    w.set_cursor_from_name(Some(edge_cursor(e)));
                    shaped.set(true);
                }
                // Only undo what this controller set: the window's own cursor is otherwise none.
                None if shaped.replace(false) => w.set_cursor_from_name(None),
                None => {}
            }
        })
    });
    window.add_controller(motion);
}

// ---------------------------------------------------------------------------
// Drag regions: `.window_drag_region()`
// ---------------------------------------------------------------------------

thread_local! {
    /// The widgets marked as drag regions, with the controllers that make them one.
    static REGIONS: RefCell<Vec<(WeakRef<gtk4::Widget>, Vec<gtk4::EventController>)>> =
        const { RefCell::new(Vec::new()) };
}

/// Mark or unmark `widget` as a drag region (the `set_drag_region` duty), with GtkWindowHandle's
/// behavior: a drag past the threshold on the region's own background asks the window manager
/// for an interactive move (snapping and edge tiling are its own), a double click does what
/// `gtk-titlebar-double-click` says, and a secondary click opens the window menu. A control
/// inside the region claims its own press first, so it keeps its click.
pub(crate) fn set_drag_region(widget: &gtk4::Widget, drag: bool) {
    REGIONS.with(|r| {
        let mut regions = r.borrow_mut();
        regions.retain(|(w, ctrls)| match w.upgrade() {
            Some(w) if &w == widget => {
                for c in ctrls {
                    w.remove_controller(c);
                }
                false
            }
            Some(_) => true,
            None => false,
        });
        if !drag {
            return;
        }
        let drag = gtk4::GestureDrag::new();
        drag.set_button(gtk4::gdk::BUTTON_PRIMARY);
        drag.connect_drag_update(|g, dx, dy| {
            ffi_guard::contain((), || {
                let Some(w) = g.widget() else {
                    return;
                };
                if !w.drag_check_threshold(0, 0, dx as i32, dy as i32) {
                    return;
                }
                let Some((x, y)) = g.start_point() else {
                    return;
                };
                begin_move(&w, g.upcast_ref(), g.current_button(), x, y);
                g.reset();
            })
        });
        let click = gtk4::GestureClick::new();
        click.set_button(0);
        click.connect_pressed(|g, n, _x, _y| {
            ffi_guard::contain((), || {
                let Some(window) = g
                    .widget()
                    .and_then(|w| w.root())
                    .and_downcast::<gtk4::Window>()
                else {
                    return;
                };
                let button = g.current_button();
                if button == gtk4::gdk::BUTTON_SECONDARY {
                    show_window_menu(&window, g.upcast_ref());
                    g.set_state(gtk4::EventSequenceState::Claimed);
                } else if button == gtk4::gdk::BUTTON_PRIMARY && n == 2 {
                    title_bar_double_click(&window, g.upcast_ref());
                    g.set_state(gtk4::EventSequenceState::Claimed);
                }
            })
        });
        widget.add_controller(drag.clone());
        widget.add_controller(click.clone());
        regions.push((widget.downgrade(), vec![drag.upcast(), click.upcast()]));
    });
}

/// Hand the press at `(x, y)` in `widget` to the window manager as an interactive move.
fn begin_move(widget: &gtk4::Widget, g: &gtk4::Gesture, button: u32, x: f64, y: f64) {
    let Some(native) = widget.native() else {
        return;
    };
    let Some((nx, ny)) = widget.translate_coordinates(&native, x, y) else {
        return;
    };
    let (tx, ty) = native.surface_transform();
    let Some(toplevel) = native
        .surface()
        .and_then(|s| s.downcast::<gtk4::gdk::Toplevel>().ok())
    else {
        return;
    };
    let Some(device) = g.device() else {
        return;
    };
    log::debug!("gtk: drag region starts a window move");
    toplevel.begin_move(
        &device,
        button as i32,
        nx + tx,
        ny + ty,
        g.current_event_time(),
    );
}

fn show_window_menu(window: &gtk4::Window, g: &gtk4::Gesture) {
    if let (Some(toplevel), Some(event)) = (
        window
            .surface()
            .and_then(|s| s.downcast::<gtk4::gdk::Toplevel>().ok()),
        g.current_event(),
    ) {
        toplevel.show_window_menu(event);
    }
}

/// What a double click on a title bar does, per the desktop's `gtk-titlebar-double-click`
/// setting: toggle maximize (the default), minimize, the window menu, or nothing.
fn title_bar_double_click(window: &gtk4::Window, g: &gtk4::Gesture) {
    let action = gtk4::Settings::default()
        .and_then(|s| s.gtk_titlebar_double_click())
        .map(|s| s.to_string());
    match action.as_deref().unwrap_or("toggle-maximize") {
        "toggle-maximize" => {
            if window.is_maximized() {
                window.unmaximize()
            } else {
                window.maximize()
            }
        }
        "minimize" => window.minimize(),
        "menu" => show_window_menu(window, g),
        // "none", and "lower", which GTK 4 itself no longer offers.
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Window properties (docs/windows.md "Window properties") and displays
// ---------------------------------------------------------------------------

/// Every [`day_spec::WindowChange`] but the display state and content protection. What GTK 4
/// cannot do is left alone (see the module comment): `Frame`'s origin, a maximum size, the
/// stacking level, every workspace, the taskbar, per-window appearance, attention.
pub(crate) fn apply_property(window: &gtk4::Window, change: &day_spec::WindowChange) {
    use day_spec::WindowChange as C;
    match change {
        // `size` is what Day lays out: the header bar goes back on top.
        C::Frame {
            size: Some(size), ..
        } => {
            let bar = bar_height(window);
            window.set_default_size(size.width as i32, (size.height + bar) as i32);
        }
        C::Limits { min, .. } => match min {
            Some(min) => {
                let bar = bar_height(window);
                window.set_size_request(min.width as i32, (min.height + bar) as i32);
            }
            None => window.set_size_request(-1, -1),
        },
        C::Resizable(on) => window.set_resizable(*on),
        C::Closable(on) => {
            window.set_deletable(*on);
            relayout_buttons(window);
        }
        C::Minimizable(on) => {
            with_shell(window, |s| s.minimizable.set(*on));
            relayout_buttons(window);
        }
        C::Maximizable(on) => {
            with_shell(window, |s| s.maximizable.set(*on));
            relayout_buttons(window);
        }
        C::Visible(true) => window.present(),
        C::Visible(false) => window.set_visible(false),
        _ => {}
    }
}

/// Take the buttons the app turned off out of the window's decoration layout: GTK has no
/// per-window minimize/maximize switch, but a header bar and window controls each take their
/// own layout string, starting from the desktop's (`gtk-decoration-layout`).
fn relayout_buttons(window: &gtk4::Window) {
    let layout = gtk4::Settings::default()
        .and_then(|s| s.gtk_decoration_layout())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "menu:minimize,maximize,close".to_owned());
    with_shell(window, |s| {
        let keep = |b: &str| match b {
            "minimize" => s.minimizable.get(),
            "maximize" => s.maximizable.get(),
            "close" => window.is_deletable(),
            _ => true,
        };
        let filtered = layout
            .split(':')
            .map(|side| {
                side.split(',')
                    .filter(|b| keep(b))
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .collect::<Vec<_>>()
            .join(":");
        s.header.set_decoration_layout(Some(&filtered));
        for c in &s.controls {
            c.set_decoration_layout(Some(&filtered));
        }
    });
}

/// Every attached display. GTK 4 reports each monitor's geometry in application pixels (Day's
/// points) and its integer scale; it has no work area and no primary monitor, so the work area
/// is the whole display and the first listed counts as primary.
pub(crate) fn monitors() -> Vec<day_spec::Monitor> {
    let Some(display) = gtk4::gdk::Display::default() else {
        return Vec::new();
    };
    let list = display.monitors();
    (0..list.n_items())
        .filter_map(|i| list.item(i).and_downcast::<gtk4::gdk::Monitor>())
        .enumerate()
        .map(|(i, m)| {
            let g = m.geometry();
            let frame = day_spec::Rect::new(
                f64::from(g.x()),
                f64::from(g.y()),
                f64::from(g.width()),
                f64::from(g.height()),
            );
            let name = m
                .description()
                .or_else(|| m.model())
                .map(|s| s.to_string())
                .unwrap_or_default();
            day_spec::Monitor {
                id: m
                    .connector()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| i.to_string()),
                name,
                frame,
                work_area: frame,
                scale: f64::from(m.scale_factor()),
                primary: i == 0,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Keeping the app running (docs/windows.md "Keeping the app running")
// ---------------------------------------------------------------------------

/// Hold the application while day-core wants it alive with no window open: GtkApplication
/// otherwise ends `run` as its last window goes.
pub(crate) fn set_keep_running(app: Option<&adw::Application>, keep: bool) {
    KEEP_RUNNING.with(|k| {
        let mut k = k.borrow_mut();
        match (keep, app) {
            (true, Some(app)) if k.is_none() => *k = Some(app.hold()),
            (false, _) => *k = None,
            _ => {}
        }
    });
}

/// Whether closing the primary window leaves the process up.
pub(crate) fn keeps_running() -> bool {
    KEEP_RUNNING.with(|k| k.borrow().is_some())
}

/// The primary window's close: today's rule (the app quits, taking every window with it) unless
/// day-core asked to keep running, in which case the close is reported like any other window's
/// and day-core decides.
pub(crate) fn primary_close(app: &adw::Application) {
    if keeps_running() {
        emit(day_spec::WINDOW_NODE, Event::WindowClosed);
    } else {
        app.quit();
    }
}

/// The safe-area inset an overlay window reports, as Day's insets.
pub(crate) fn top_inset(top: f64) -> day_spec::Insets {
    day_spec::Insets {
        top,
        ..Default::default()
    }
}

/// Report an overlay window's band as its safe-area top inset (docs/window-chrome.md), before
/// its content builds: a secondary window's synchronously (day-core builds the content right
/// after `open_window` returns), the primary's as a seed, since its tree does not exist yet.
pub(crate) fn report_overlay_inset(window: &gtk4::Window, node: Option<NodeId>) {
    let Some(top) = overlay_inset(window) else {
        return;
    };
    log::debug!("gtk: overlay title bar inset {top}");
    match node {
        Some(node) => day_core::set_window_safe_area(day_core::id_to_rnode(node), top_inset(top)),
        None => day_core::seed_safe_area(top_inset(top)),
    }
}

// ---------------------------------------------------------------------------
// Status items, launcher progress and badges (Linux: StatusNotifierItem, Unity LauncherEntry)
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
pub(crate) use tray::{set_status_items, status_item_support};
#[cfg(target_os = "linux")]
pub(crate) use unity::{set_badge, set_progress};

/// The Unity LauncherEntry needs a Linux session bus and a dock that listens.
#[cfg(not(target_os = "linux"))]
pub(crate) fn set_progress(_progress: day_spec::AppProgress) {}

#[cfg(not(target_os = "linux"))]
pub(crate) fn set_badge(_badge: &day_spec::AppBadge) {}

/// Badge count and progress on the app's dock icon through the Unity LauncherEntry signal
/// (docs/status-item.md, docs/badge.md), which Ubuntu Dock, Dash to Dock, Plasma's task manager,
/// Latte and Plank read. A signal, not a call: nothing answers, so whether a dock shows it is
/// the desktop's business, which is why the Caps say Emulated. The dock matches it to the app by
/// desktop id, which is the app id (`DAY_APP_ID`).
#[cfg(target_os = "linux")]
mod unity {
    use super::*;
    use day_dbus::Connection;
    use day_dbus::launcher::{self, LauncherProps};

    thread_local! {
        static CONN: RefCell<Option<Connection>> = const { RefCell::new(None) };
        static PROPS: Cell<LauncherProps> = Cell::new(LauncherProps::default());
    }

    fn desktop_id() -> Option<String> {
        std::env::var("DAY_APP_ID")
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(|| option_env!("DAY_APP_ID").map(str::to_owned))
    }

    /// Send the whole state: every `Update` restates count, progress and urgency.
    fn send(props: LauncherProps) {
        PROPS.with(|p| p.set(props));
        let Some(id) = desktop_id() else {
            return;
        };
        CONN.with(|c| {
            let mut c = c.borrow_mut();
            if c.as_ref().is_none_or(|conn| conn.is_closed()) {
                *c = Connection::session().ok();
            }
            if let Some(conn) = c.as_ref() {
                match launcher::update(conn, &id, props) {
                    Ok(()) => log::debug!("gtk: launcher entry {id} {props:?}"),
                    Err(e) => log::debug!("gtk: launcher entry {id} not sent: {e}"),
                }
            }
        });
    }

    /// `AppProgress`: a fraction shows the bar; an error also marks the icon urgent. The protocol
    /// has no indeterminate or paused bar, so indeterminate shows none and paused shows its value.
    pub(crate) fn set_progress(progress: day_spec::AppProgress) {
        use day_spec::AppProgress as P;
        let mut props = PROPS.with(|p| p.get());
        (props.progress, props.urgent) = match progress {
            P::None | P::Indeterminate => (None, None),
            P::Value(v) | P::Paused(v) => (Some(v), None),
            P::Error(v) => (Some(v), Some(true)),
        };
        send(props);
    }

    /// `AppBadge`: only a count exists in the protocol. Text and a dot are not substituted
    /// (`Cap::AppBadgeText`/`AppBadgeDot` stay Unsupported), so they clear nothing either.
    pub(crate) fn set_badge(badge: &day_spec::AppBadge) {
        let mut props = PROPS.with(|p| p.get());
        props.count = match badge {
            day_spec::AppBadge::None | day_spec::AppBadge::Count(0) => None,
            day_spec::AppBadge::Count(n) => Some(i64::from(*n)),
            _ => return,
        };
        send(props);
    }
}

/// Status items need a StatusNotifierItem host, which only Linux desktops run.
#[cfg(not(target_os = "linux"))]
pub(crate) fn set_status_items(_items: &[day_spec::StatusItemSpec]) {}

#[cfg(not(target_os = "linux"))]
pub(crate) fn status_item_support() -> day_spec::Support {
    day_spec::Support::Unsupported
}

#[cfg(target_os = "linux")]
mod tray {
    use super::*;
    use day_dbus::Connection;
    use day_dbus::sni::{self, MenuEntry, Pixmap, StatusItem};
    use day_spec::{MenuItem, MenuRole, Platform as _, StatusItemSpec, Support};

    /// What a menu entry id stands for.
    #[derive(Clone, Copy)]
    enum Pick {
        Action(u64),
        Quit,
    }

    /// One shown item: its own bus connection (the item's object paths are fixed by the
    /// protocol, so a connection holds one item), the server, and what each entry id runs.
    struct Shown {
        spec: StatusItemSpec,
        item: StatusItem,
        picks: Vec<(i32, Pick)>,
        _conn: Connection,
    }

    thread_local! {
        static SHOWN: RefCell<Vec<Shown>> = const { RefCell::new(Vec::new()) };
    }

    /// `Cap::StatusItem`, decided by the desktop: Native while a StatusNotifierWatcher is on
    /// the session bus (KDE Plasma, most panels, GNOME with the AppIndicator extension),
    /// Unsupported without one (stock GNOME) or without a session bus. Asked afresh each time,
    /// since a panel can start after the app.
    pub(crate) fn status_item_support() -> Support {
        match Connection::session() {
            Ok(conn) if sni::watcher_available(&conn) => Support::Native,
            _ => Support::Unsupported,
        }
    }

    /// Lower Day's menu into dbusmenu entries, numbering them from `next` and recording what
    /// each id runs. A role item does its role where a tray menu can (Quit); the others are
    /// window commands with nothing to act on here, so they are left out.
    fn lower(items: &[MenuItem], next: &mut i32, picks: &mut Vec<(i32, Pick)>) -> Vec<MenuEntry> {
        let mut out = Vec::new();
        for item in items {
            let id = *next;
            *next += 1;
            match item {
                MenuItem::Action {
                    action,
                    label,
                    enabled,
                    checked,
                    role,
                    ..
                } => {
                    let pick = match (*action, role) {
                        (0, Some(MenuRole::Quit)) => Pick::Quit,
                        (0, _) => continue,
                        (a, _) => Pick::Action(a),
                    };
                    let label = if label.is_empty() && matches!(pick, Pick::Quit) {
                        "Quit".to_owned()
                    } else {
                        label.clone()
                    };
                    let check = match checked {
                        None => sni::Check::None,
                        Some(true) => sni::Check::On,
                        Some(false) => sni::Check::Off,
                    };
                    picks.push((id, pick));
                    out.push(
                        MenuEntry::item(id, label)
                            .with_enabled(*enabled)
                            .with_check(check),
                    );
                }
                MenuItem::Submenu { label, items, .. } => {
                    let children = lower(items, next, picks);
                    out.push(MenuEntry::submenu(id, label.clone(), children));
                }
                MenuItem::Separator => out.push(MenuEntry::separator(id)),
                #[allow(unreachable_patterns)]
                _ => {}
            }
        }
        out
    }

    /// The pixmap sizes panels ask for most: 16 and 22 (KDE, Xfce), 32 and 48 for HiDPI.
    const SIZES: [i32; 4] = [16, 22, 32, 48];

    /// The item's icon as a theme name (hosts prefer it) and ARGB pixmaps. A symbol is the
    /// theme's symbolic icon plus Day's own outline drawn at each size; a bundled image or vector
    /// is drawn through gdk-pixbuf (librsvg for vectors). A template is recolored to the theme
    /// foreground. No icon: the app's own icon name (`DAY_ICON_NAME`, staged by `day launch`).
    fn icon_of(spec: &StatusItemSpec) -> (String, Vec<Pixmap>) {
        let (name, svg_or_file): (String, Option<Source>) = match &spec.icon {
            Some(day_spec::Icon::Symbol(sym)) => (
                crate::toolbar::icon_name_for(*sym)
                    .unwrap_or_default()
                    .to_owned(),
                sym.outline_svg().map(|s| Source::Svg(s.to_string())),
            ),
            Some(day_spec::Icon::Image(name)) => (
                String::new(),
                day_spec::resource::resolve_vector_svg(name)
                    .or_else(|| day_spec::resource::resolve_image_file(name))
                    .map(Source::File),
            ),
            None => (std::env::var("DAY_ICON_NAME").unwrap_or_default(), None),
        };
        let template = spec.template || matches!(spec.icon, Some(day_spec::Icon::Symbol(_)));
        let pixmaps: Vec<Pixmap> = svg_or_file
            .map(|src| {
                SIZES
                    .iter()
                    .filter_map(|px| pixmap(&src, *px, template))
                    .collect()
            })
            .unwrap_or_default();
        // Nothing drawn (a host without gdk-pixbuf's SVG loader) and no theme name: the app's
        // own icon, rather than an invisible item.
        let name = if name.is_empty() && pixmaps.is_empty() {
            std::env::var("DAY_ICON_NAME").unwrap_or_default()
        } else {
            name
        };
        (name, pixmaps)
    }

    enum Source {
        Svg(String),
        File(std::path::PathBuf),
    }

    fn pixmap(src: &Source, px: i32, template: bool) -> Option<Pixmap> {
        let pixbuf = match src {
            Source::Svg(svg) => {
                let stream = gtk4::gio::MemoryInputStream::from_bytes(&gtk4::glib::Bytes::from(
                    svg.as_bytes(),
                ));
                gtk4::gdk_pixbuf::Pixbuf::from_stream_at_scale(
                    &stream,
                    px,
                    px,
                    true,
                    gtk4::gio::Cancellable::NONE,
                )
                .ok()?
            }
            Source::File(path) => {
                gtk4::gdk_pixbuf::Pixbuf::from_file_at_scale(path, px, px, true).ok()?
            }
        };
        let pixbuf = if pixbuf.has_alpha() {
            pixbuf
        } else {
            pixbuf.add_alpha(false, 0, 0, 0).ok()?
        };
        if template {
            let (r, g, b) = if adw::StyleManager::default().is_dark() {
                (0xff, 0xff, 0xff)
            } else {
                (0x1a, 0x1a, 0x1a)
            };
            crate::recolor_pixbuf(&pixbuf, r, g, b);
        }
        let (w, h) = (pixbuf.width(), pixbuf.height());
        let stride = usize::try_from(pixbuf.rowstride()).ok()?;
        let bytes = pixbuf.read_pixel_bytes();
        let row = usize::try_from(w).ok()? * 4;
        let mut rgba = Vec::with_capacity(row * usize::try_from(h).ok()?);
        for y in 0..usize::try_from(h).ok()? {
            rgba.extend_from_slice(bytes.get(y * stride..y * stride + row)?);
        }
        Pixmap::from_rgba(u32::try_from(w).ok()?, u32::try_from(h).ok()?, &rgba)
    }

    fn item_of(spec: &StatusItemSpec) -> sni::Item {
        let (icon_name, icon_pixmaps) = icon_of(spec);
        sni::Item {
            id: spec.id.clone(),
            title: if spec.title.is_empty() {
                spec.tooltip.clone()
            } else {
                spec.title.clone()
            },
            tooltip: spec.tooltip.clone(),
            icon_name,
            icon_pixmaps,
            item_is_menu: spec.activate == 0,
            ..Default::default()
        }
    }

    /// A click or menu choice, on the GTK main loop.
    fn deliver(id: &str, event: sni::Event) {
        let found = SHOWN.with(|s| {
            s.borrow().iter().find(|x| x.spec.id == id).map(|x| {
                let pick = match &event {
                    sni::Event::MenuItem { id } => {
                        x.picks.iter().find(|(e, _)| e == id).map(|(_, p)| *p)
                    }
                    sni::Event::Activate { .. } if x.spec.activate != 0 => {
                        Some(Pick::Action(x.spec.activate))
                    }
                    _ => None,
                };
                (pick, matches!(event, sni::Event::Activate { .. }))
            })
        });
        let Some((Some(pick), activate)) = found else {
            return;
        };
        log::debug!("gtk: status item {id} event {event:?}");
        match pick {
            // The item's own click runs its action straight; a menu choice goes the app menu's
            // way, through `Event::MenuAction`.
            Pick::Action(a) if activate => day_core::dispatch_menu_action(a),
            Pick::Action(a) => emit(day_spec::WINDOW_NODE, Event::MenuAction(a)),
            Pick::Quit => day_core::quit(),
        }
    }

    /// Show `items`, replacing the previous set (the `set_status_items` duty): diffed by id.
    pub(crate) fn set_status_items(items: &[StatusItemSpec]) {
        SHOWN.with(|s| {
            let mut shown = s.borrow_mut();
            shown.retain(|x| items.iter().any(|i| i.id == x.spec.id));
            for spec in items {
                let mut picks = Vec::new();
                let menu = lower(&spec.menu, &mut 1, &mut picks);
                if let Some(x) = shown.iter_mut().find(|x| x.spec.id == spec.id) {
                    if x.spec == *spec {
                        continue;
                    }
                    if x.spec.menu != spec.menu {
                        let _ = x.item.set_menu(menu);
                        x.picks = picks;
                    }
                    if x.spec.icon != spec.icon || x.spec.template != spec.template {
                        let (name, pixmaps) = icon_of(spec);
                        let _ = x.item.set_icon(&name, pixmaps);
                    }
                    if x.spec.title != spec.title {
                        let _ = x.item.set_title(&item_of(spec).title);
                    }
                    if x.spec.tooltip != spec.tooltip {
                        let _ = x.item.set_tooltip(&spec.tooltip);
                    }
                    // `ItemIsMenu` is read once at registration; a changed `activate` is
                    // honored by `deliver`, which reads the stored spec.
                    x.spec = spec.clone();
                    continue;
                }
                let conn = match Connection::session() {
                    Ok(c) => c,
                    Err(e) => {
                        log::warn!("gtk: no session bus for status item {}: {e}", spec.id);
                        continue;
                    }
                };
                let id = spec.id.clone();
                let item = StatusItem::new(&conn, item_of(spec), menu, move |event| {
                    let id = id.clone();
                    // The connection's reader thread: hop to the GTK main loop.
                    crate::Gtk::post(Box::new(move || deliver(&id, event)));
                });
                match item {
                    Ok(item) => {
                        log::debug!("gtk: status item {} on {}", spec.id, item.service());
                        shown.push(Shown {
                            spec: spec.clone(),
                            item,
                            picks,
                            _conn: conn,
                        });
                    }
                    Err(e) => log::warn!("gtk: status item {} not shown: {e}", spec.id),
                }
            }
        });
    }
}
