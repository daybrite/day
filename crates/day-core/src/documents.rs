// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
//! File activation is data delivery, never route navigation. See docs/documents.md.
use std::{cell::RefCell, collections::VecDeque, rc::Rc, sync::Mutex};
static PENDING: Mutex<VecDeque<Vec<String>>> = Mutex::new(VecDeque::new());
day_reactive::tls_slots! {
    documents;
    static HANDLER: RefCell<Option<Rc<dyn Fn(Vec<String>)>>> = const { RefCell::new(None) };
}
/// Install the application's file-open handler. Called on the UI thread; pending cold-start
/// activations are delivered on the next UI turn. Register once in the application root.
pub fn on_open_files(handler: impl Fn(Vec<String>) + 'static) {
    let handler: Rc<dyn Fn(Vec<String>)> = Rc::new(handler);
    let previous = HANDLER.with(|h| h.borrow_mut().replace(handler.clone()));
    drop(previous);
    day_reactive::Scope::current().on_cleanup(move || {
        // The app root can outlive this slot during thread-local teardown.
        let removed = HANDLER.try_with(|h| {
            if h.borrow()
                .as_ref()
                .is_some_and(|current| Rc::ptr_eq(current, &handler))
            {
                h.borrow_mut().take()
            } else {
                None
            }
        });
        drop(removed);
    });
    if day_reactive::has_main_poster() {
        day_reactive::on_main(drain_open_files);
    }
}
/// Platform intake, callable before launch and from worker threads. Locators must be local
/// paths or file URLs (mobile providers stage granted content before calling this).
pub fn request_open_files(files: Vec<String>) {
    let files: Vec<_> = files
        .into_iter()
        .filter(|p| {
            if p.split_once(':').is_some_and(|(scheme, _)| {
                scheme.len() > 1
                    && scheme
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"+.-".contains(&b))
            }) && !p.starts_with("file:")
                && !std::path::Path::new(p).is_absolute()
            {
                crate::request_open_url(p);
                false
            } else {
                !p.is_empty()
            }
        })
        .collect();
    if files.is_empty() {
        return;
    }
    PENDING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push_back(files);
    if day_reactive::has_main_poster() {
        day_reactive::on_main(drain_open_files);
    }
}
pub(crate) fn drain_open_files() {
    loop {
        // A callback may replace or dispose its registration. Re-resolve between batches.
        let Some(handler) = HANDLER.with(|h| h.borrow().clone()) else {
            return;
        };
        let next = PENDING
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pop_front();
        let Some(files) = next else { break };
        handler(files);
    }
}
/// Desktop file associations pass `--day-open-file <path>` once per file, or `--day-open-files`
/// followed by filenames. Existing positional files also support shell drops onto executables;
/// arbitrary flags, nonexistent paths and navigation URLs are ignored.
pub(crate) fn launch_files(args: impl Iterator<Item = String>) -> Vec<String> {
    let mut args = args.peekable();
    let mut files = Vec::new();
    while let Some(arg) = args.next() {
        if arg == "--day-open-files" {
            files.extend(args);
            break;
        }
        if arg == "--day-open-url" {
            if let Some(url) = args.next() {
                files.push(url);
            }
        } else if !arg.starts_with('-') && arg.contains("://") {
            files.push(arg);
        } else if arg == "--day-open-file" {
            if let Some(path) = args.next() {
                files.push(path)
            }
        } else if !arg.starts_with('-') && std::path::Path::new(&arg).is_file() {
            files.push(arg);
        }
    }
    files
}

/// Deliver the launch arguments a second copy of the app forwarded to this one
/// (docs/deep-links.md "Single-instance forwarding"): the files, URLs and routes it was started
/// with arrive as if this process had been launched with them, and the app's front window comes
/// forward. Backend-facing; call on the UI thread.
pub fn forward_launch_args(args: Vec<String>) {
    request_open_files(launch_files(args.into_iter()));
    if let Some(window) = crate::windows::focused_window().or_else(crate::windows::initial_window) {
        window.focus();
    }
}

/// The link a launcher shortcut for `route` passes on a desktop command line
/// (`--day-open-url <link>`): `<scheme>://<route>` with the app's URL scheme, which `day build`
/// hands the binary as `DAY_APP_SCHEME`, else the bare route. Either form reaches the route
/// through the deep-link intake.
pub fn launch_link(route: &str) -> String {
    let scheme = std::env::var("DAY_APP_SCHEME")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| option_env!("DAY_APP_SCHEME").map(str::to_owned));
    match scheme {
        Some(scheme) => format!("{scheme}://{route}"),
        None => route.to_owned(),
    }
}

/// Backend bootstrap hook: claim launch arguments once. GTK passes them to GApplication
/// so it can forward an open request to an existing process; other backends let core deliver.
#[doc(hidden)]
pub fn take_launch_files() -> Vec<String> {
    static TAKEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if TAKEN.swap(true, std::sync::atomic::Ordering::Relaxed) {
        Vec::new()
    } else {
        launch_files(std::env::args().skip(1))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cold_warm_reentrant_delivery_and_scope_cleanup() {
        let received = Rc::new(RefCell::new(Vec::new()));
        request_open_files(vec!["cold.epub".into()]);
        drain_open_files(); // No consumer yet: the activation must remain queued.
        let scope = day_reactive::Scope::child();
        let seen = received.clone();
        scope.enter(|| {
            on_open_files(move |files| {
                if files == ["cold.epub"] {
                    request_open_files(vec!["reentrant.epub".into()]);
                }
                seen.borrow_mut().extend(files);
            })
        });
        drain_open_files();
        request_open_files(vec!["warm.epub".into()]);
        drain_open_files();
        assert_eq!(
            *received.borrow(),
            ["cold.epub", "reentrant.epub", "warm.epub"]
        );
        scope.dispose();
        request_open_files(vec!["after-dispose.epub".into()]);
        drain_open_files();
        assert_eq!(received.borrow().len(), 3);
        let second = day_reactive::Scope::child();
        let seen = received.clone();
        second.enter(|| on_open_files(move |files| seen.borrow_mut().extend(files)));
        drain_open_files();
        assert_eq!(received.borrow().last().unwrap(), "after-dispose.epub");
        second.dispose();
    }
    #[test]
    fn association_arguments_are_explicit_and_preserve_spaces() {
        assert_eq!(
            launch_files(
                [
                    "app",
                    "--day-open-file",
                    "/tmp/book name.epub",
                    "--day-open-files",
                    "a.epub",
                    "b.epub"
                ]
                .into_iter()
                .map(str::to_owned)
            ),
            ["/tmp/book name.epub", "a.epub", "b.epub"]
        );
        assert_eq!(
            launch_files(
                ["https://example.org", "--debug"]
                    .into_iter()
                    .map(str::to_owned)
            ),
            ["https://example.org"]
        );
        assert_eq!(
            launch_files(
                ["--day-open-url", "feed:https://example.org/rss?q=a%26b"]
                    .into_iter()
                    .map(str::to_owned)
            ),
            ["feed:https://example.org/rss?q=a%26b"]
        );
    }
}
