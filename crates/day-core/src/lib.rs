//! day-core — the Piece model, realized tree, mounter, layout engine, and event routing
//! (DESIGN.md §5, §7). Build-once: pieces are constructed exactly once; all dynamism flows
//! through reactive bindings (day-reactive) writing to the thread-local tree.

mod anim;
mod build;
pub mod frame;
mod layout;
pub mod lifecycle;
pub mod list;
pub mod menu;
mod nav;
mod present;
pub mod shield;
mod tree;

pub use anim::{current_anim, with_animation};
pub use build::*;
pub use frame::{
    FrameConsumer, add_frame_consumer, frame_consumer_count, install_frame_requester,
    remove_frame_consumer,
};
pub use layout::*;
pub use lifecycle::{dispatch_lifecycle, lifecycle_supported, on_lifecycle};
pub use list::{BuiltRow, ListDriver, install_list, list_reload, list_scroll_to_end};
pub use menu::{dispatch_menu_action, register_menu_action, set_app_menu};
pub use nav::*;
pub use present::*;
// The resource seam lives in day-spec (backends depend only on day-spec); re-export for the facade.
pub use day_spec::resource::{
    AssetName, FontFamily, ImageName, Resource, ResourceOpener, resource, set_resource_opener,
};
pub use tree::*;

/// The app-wide layout direction (docs/localization): mirrors every horizontal placement in
/// the place pass when [`day_geometry::LayoutDirection::Rtl`]. Resolved lazily from the
/// `DAY_LOCALE` launch environment (so toolkits can read it before any UI exists);
/// `set_layout_direction` (called by `install_locales` for the resolved locale) overrides.
/// Fixed for the life of the process — switching locale at runtime does not re-mirror.
pub fn layout_direction() -> day_geometry::LayoutDirection {
    DIRECTION.with(|d| {
        if let Some(dir) = d.get() {
            return dir;
        }
        let dir = std::env::var("DAY_LOCALE")
            .map(|l| direction_of_locale(&l))
            .unwrap_or_default();
        d.set(Some(dir));
        dir
    })
}

/// Override the layout direction (normally from `install_locales`). Must be called before the
/// first layout pass to take effect everywhere.
pub fn set_layout_direction(dir: day_geometry::LayoutDirection) {
    DIRECTION.with(|d| d.set(Some(dir)));
}

/// Whether the app is being rendered right-to-left (docs/localization) — a convenience over
/// [`layout_direction`]. The layout engine already mirrors widget *placement* under an RTL locale,
/// but a `canvas` draws in its own coordinate space, so a custom drawing that has a reading
/// direction (a battery that drains one way, an arrow, a progress sweep) can call this to mirror
/// itself. Fixed for the life of the process, like [`layout_direction`].
pub fn is_rtl() -> bool {
    layout_direction() == day_geometry::LayoutDirection::Rtl
}

/// The writing direction a locale implies (language subtag match).
pub fn direction_of_locale(locale: &str) -> day_geometry::LayoutDirection {
    let lang = locale
        .split(['-', '_'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    match lang.as_str() {
        "ar" | "he" | "iw" | "fa" | "ur" | "ps" | "sd" | "ug" | "yi" | "dv" | "ku" => {
            day_geometry::LayoutDirection::Rtl
        }
        _ => day_geometry::LayoutDirection::Ltr,
    }
}

thread_local! {
    static DIRECTION: std::cell::Cell<Option<day_geometry::LayoutDirection>> =
        const { std::cell::Cell::new(None) };
}

use day_spec::{Platform, WindowOptions};

/// Run a posted main-thread task, CONTAINING any panic (the `pump_events` twin for the poster /
/// scheduler doors): log the cause and reset the reactive runtime so the app keeps running
/// (degraded) instead of aborting across the native trampoline's non-unwind boundary.
fn contain_posted_panic(f: Box<dyn FnOnce() + Send>) {
    if let Err(payload) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        let msg = payload
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown panic".to_string());
        eprintln!(
            "day: a posted main-thread task panicked and was contained — the app continues, but \
             reactive/UI state may be inconsistent until the next interaction. Cause: {msg}"
        );
        day_reactive::recover_from_panic();
    }
}

/// Launch a Day app on the given platform backend: sets up the reactive scheduler and the
/// cross-thread poster, mounts the root piece into the window's content container, runs the
/// initial layout, and installs the turn-end layout callback (§3.3). The backend then owns
/// the native main loop.
pub fn launch_with<P: Platform>(
    backend: P,
    options: WindowOptions,
    root_piece: impl FnOnce() -> AnyPiece + 'static,
) {
    // Reactive plumbing rides the platform's main-loop poster. Both doors CONTAIN panics (the
    // `pump_events` rationale, tree.rs): posted closures run inside native main-loop trampolines
    // (a glib idle, a GCD block) that ABORT the process on unwind (`panic_cannot_unwind`) — so a
    // panic in a `Setter` write's drain or a scheduled `flush_sync` would SIGABRT the app instead
    // of surfacing. Contain at this single backend-agnostic boundary and reset the runtime.
    day_reactive::install_main_poster(|f| {
        P::post(Box::new(move || contain_posted_panic(f)));
    });
    day_reactive::install_scheduler(|| {
        P::post(Box::new(|| {
            contain_posted_panic(Box::new(day_reactive::flush_sync));
        }))
    });
    // The async-spawn door (docs/async.md): day-reactive's `Resource` runs its fetch futures on
    // this executor; the returned closure aborts (a no-op once the task completed — the contract
    // Resource's eager-poll ordering relies on).
    day_reactive::install_spawner(|fut| {
        let handle = present::task(fut);
        Box::new(move || handle.abort())
    });
    // The frame clock (§8.4): the animation driver re-arms the platform's vsync callback while any
    // frame consumer (game loop / self-driven animation) is live.
    frame::install_frame_requester(|cb| P::request_frame(cb));

    // WillLaunch: before the window/UI exists (docs/lifecycle.md). Fired uniformly by day-core so
    // it is reliable on every backend; handlers must not touch the tree (there isn't one yet).
    lifecycle::dispatch_lifecycle(day_spec::Lifecycle::WillLaunch);

    P::run(
        backend,
        options,
        Box::new(move |mut toolkit, root_handle, size| {
            day_spec::Toolkit::set_event_sink(&mut toolkit, Box::new(tree::enqueue_event));
            let tree = Tree::new(toolkit, root_handle, size);
            let root = tree.root();
            tree::install_tree(Box::new(tree));

            // The backend is now known: warn about any lifecycle handlers already registered for
            // phases this platform doesn't deliver (docs/lifecycle.md).
            lifecycle::warn_unsupported_registrations();

            // Window resize → relayout.
            with_tree(|t| {
                let rn = root;
                t.on_event(
                    rn,
                    std::rc::Rc::new(move |ev| {
                        if let day_spec::Event::WindowResized(size) = ev {
                            let s = *size;
                            with_tree(|t| t.set_window_size(s));
                        }
                    }),
                );
            });

            // Build the root piece under the window container.
            let piece = root_piece();
            let mut cx = BuildCx::new(root);
            let _ = piece.build(&mut cx);

            // Initial layout, then keep laying out at every turn boundary.
            with_tree(|t| {
                t.mark_layout_dirty();
                t.layout_if_needed();
            });
            day_reactive::on_turn_end(|| with_tree(|t| t.layout_if_needed()));

            // DidLaunch: the UI is mounted and laid out, the app is about to run (docs/lifecycle.md).
            lifecycle::dispatch_lifecycle(day_spec::Lifecycle::DidLaunch);

            // Startup deep link (docs/navigation.md): uniform across platforms — desktop
            // sets the env directly, mobile shells forward the launch URL/intent into it.
            // Deferred one turn so the first frame mounts before the destination pushes.
            if let Ok(route) = std::env::var("DAY_DEEPLINK")
                && !route.is_empty()
            {
                day_reactive::on_main(move || {
                    if !nav::navigate(&route) {
                        eprintln!("day: DAY_DEEPLINK {route:?} did not match a route");
                    }
                });
            }

            // Verification hook (headless CI / no-input environments): drive the app through
            // Day's own event path once the native loop starts (delayed past first allocation
            // so snapshots see a laid-out window). Precursor of dayscript (§14).
            if let Ok(spec) = std::env::var("DAY_AUTODRIVE") {
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(800));
                    day_reactive::on_main(move || autodrive(&spec));
                });
            }
        }),
    );
}

/// `DAY_AUTODRIVE="<id>:press;<id>:text:Ada;<id>:value:80;<id>:toggle:true;<id>:tap;
/// <id>:drag:40:60;shot:/tmp/x.png"` — synthesized Day events by element id, plus snapshots.
fn autodrive(spec: &str) {
    use day_spec::{DragPhase, Event, Point};
    for step in spec.split(';').filter(|s| !s.is_empty()) {
        let parts: Vec<&str> = step.splitn(3, ':').collect();
        if parts[0] == "shot" {
            let path = parts[1..].join(":");
            let png = with_tree(|t| t.snapshot());
            match png {
                Ok(bytes) => {
                    let _ = std::fs::write(&path, bytes);
                }
                Err(e) => eprintln!("day autodrive: snapshot failed: {e}"),
            }
            continue;
        }
        let node = with_tree(|t| t.find_by_id(parts[0]));
        let Some(node) = node else {
            eprintln!("day autodrive: id {:?} not found", parts[0]);
            continue;
        };
        // Gesture drivers (docs/shapes.md): tap fires at the node's local centre; drag runs a
        // Began→Changed→Ended sequence translated by dx,dy — exercising `.on_tap`/`.on_drag`
        // hit-testing through Day's own event path (the native recognizers deliver the same events).
        if parts.get(1) == Some(&"tap") {
            if let Some(f) = with_tree(|t| t.node_frame(node)) {
                let c = Point::new(f.size.width / 2.0, f.size.height / 2.0);
                tree::enqueue_event(tree::rnode_to_id(node), Event::Tap(c));
            }
            continue;
        }
        if parts.get(1) == Some(&"drag") {
            if let Some(f) = with_tree(|t| t.node_frame(node)) {
                let c = Point::new(f.size.width / 2.0, f.size.height / 2.0);
                let (dx, dy) = parts
                    .get(2)
                    .and_then(|s| s.split_once(':'))
                    .and_then(|(a, b)| Some((a.parse::<f64>().ok()?, b.parse::<f64>().ok()?)))
                    .unwrap_or((0.0, 0.0));
                let end = Point::new(c.x + dx, c.y + dy);
                let id = tree::rnode_to_id(node);
                tree::enqueue_event(
                    id,
                    Event::Drag {
                        phase: DragPhase::Began,
                        location: c,
                        translation: Point::ZERO,
                    },
                );
                tree::enqueue_event(
                    id,
                    Event::Drag {
                        phase: DragPhase::Changed,
                        location: end,
                        translation: Point::new(dx, dy),
                    },
                );
                tree::enqueue_event(
                    id,
                    Event::Drag {
                        phase: DragPhase::Ended,
                        location: end,
                        translation: Point::new(dx, dy),
                    },
                );
            }
            continue;
        }
        let ev = match (parts.get(1).copied(), parts.get(2).copied()) {
            (Some("press"), _) => Event::Pressed,
            (Some("text"), Some(v)) => Event::TextChanged(v.to_string()),
            (Some("toggle"), Some(v)) => Event::ToggleChanged(v == "true"),
            (Some("value"), Some(v)) => Event::ValueChanged(v.parse().unwrap_or(0.0)),
            (Some("select"), Some(v)) => Event::SelectionChanged(v.parse().unwrap_or(-1)),
            _ => continue,
        };
        tree::enqueue_event(tree::rnode_to_id(node), ev);
    }
}

#[cfg(test)]
mod posted_panic_tests {
    /// A panic inside a posted main-thread task must be CONTAINED (logged + runtime reset), never
    /// unwind into the native trampoline that posted it (`panic_cannot_unwind` → SIGABRT). This is
    /// the poster/scheduler twin of `pump_events`' containment.
    #[test]
    fn posted_panic_is_contained() {
        super::contain_posted_panic(Box::new(|| panic!("boom in a posted task")));
        // Reaching here IS the assertion: the panic did not propagate. The runtime was reset, so
        // subsequent reactive work still runs.
        let s = day_reactive::Signal::new(1i32);
        s.set(2);
        assert_eq!(s.get_untracked(), 2);
    }
}

/// Whether the platform is rendering in dark appearance (see `Toolkit::dark_mode`): the
/// branch apps take when painting custom OPAQUE surfaces so fills track the theme that the
/// default text colors already follow.
pub fn dark_mode() -> bool {
    tree::with_tree(|t| t.dark_mode())
}
