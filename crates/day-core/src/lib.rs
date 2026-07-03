//! day-core — the Piece model, realized tree, mounter, layout engine, and event routing
//! (DESIGN.md §5, §7). Build-once: pieces are constructed exactly once; all dynamism flows
//! through reactive bindings (day-reactive) writing to the thread-local tree.

mod build;
mod layout;
pub mod list;
mod nav;
mod present;
mod tree;

pub use build::*;
pub use layout::*;
pub use list::{BuiltRow, ListDriver, install_list, list_reload};
pub use nav::*;
pub use present::*;
pub use tree::*;

use day_spec::{Platform, WindowOptions};

/// Launch a day app on the given platform backend: sets up the reactive scheduler and the
/// cross-thread poster, mounts the root piece into the window's content container, runs the
/// initial layout, and installs the turn-end layout callback (§3.3). The backend then owns
/// the native main loop.
pub fn launch_with<P: Platform>(
    backend: P,
    options: WindowOptions,
    root_piece: impl FnOnce() -> AnyPiece + 'static,
) {
    // Reactive plumbing rides the platform's main-loop poster.
    day_reactive::install_main_poster(|f| P::post(f));
    day_reactive::install_scheduler(|| {
        P::post(Box::new(|| {
            day_reactive::flush_sync();
        }))
    });

    P::run(
        backend,
        options,
        Box::new(move |mut toolkit, root_handle, size| {
            day_spec::Toolkit::set_event_sink(&mut toolkit, Box::new(tree::enqueue_event));
            let tree = Tree::new(toolkit, root_handle, size);
            let root = tree.root();
            tree::install_tree(Box::new(tree));

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
            // day's own event path once the native loop starts (delayed past first allocation
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

/// `DAY_AUTODRIVE="<id>:press;<id>:text:Ada;<id>:value:80;<id>:toggle:true;shot:/tmp/x.png"` —
/// synthesized day events by element id, plus window snapshots.
fn autodrive(spec: &str) {
    use day_spec::Event;
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
