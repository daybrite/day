// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
//! Raw external URL delivery; application handlers may consume data links before routing.
use std::{cell::RefCell, collections::VecDeque, rc::Rc, sync::Mutex};
static PENDING: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
day_reactive::tls_slots! {
    urls;
    static HANDLERS: RefCell<Vec<Rc<dyn Fn(&str) -> bool>>> = const { RefCell::new(Vec::new()) };
}
/// Register on the UI thread. Return true to consume a URL; false retains normal deep-link
/// routing. Delivery never performs a privileged action.
///
/// Handlers stack: the newest registration is asked first, and a URL it declines goes to the
/// ones before it, so a part that awaits a callback URL (a sign-in's redirect) can register
/// for the duration of its scope without displacing the application's own handler. Each
/// registration leaves the stack when the scope it was made in is disposed.
pub fn on_open_url(handler: impl Fn(&str) -> bool + 'static) {
    let handler: Rc<dyn Fn(&str) -> bool> = Rc::new(handler);
    HANDLERS.with(|h| h.borrow_mut().push(handler.clone()));
    day_reactive::Scope::current().on_cleanup(move || {
        let _ = HANDLERS.try_with(|h| {
            h.borrow_mut()
                .retain(|current| !Rc::ptr_eq(current, &handler));
        });
    });
    if day_reactive::has_main_poster() {
        day_reactive::on_main(drain);
    }
}
/// Platform intake. Keeps the original URL, including its scheme, query and escaping.
pub fn request_open_url(url: &str) {
    if url.is_empty() {
        return;
    }
    PENDING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push_back(url.into());
    if day_reactive::has_main_poster() {
        day_reactive::on_main(drain);
    }
}
pub(crate) fn drain() {
    drain_queue(&PENDING);
}
fn drain_queue(queue: &Mutex<VecDeque<String>>) {
    loop {
        let next = queue.lock().unwrap_or_else(|e| e.into_inner()).pop_front();
        let Some(url) = next else { break };
        // Newest first; a snapshot, so a handler may register or unregister another.
        let handlers: Vec<_> = HANDLERS.with(|h| h.borrow().iter().rev().cloned().collect());
        let consumed = handlers.iter().any(|h| h(&url));
        if !consumed {
            crate::request_route(&day_spec::route_of_url(&url));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_urls_are_queued_reentrant_and_scoped() {
        let seen = Rc::new(RefCell::new(Vec::new()));
        // No main poster exists in this unit test: cold data survives until the root installs.
        let queue = std::sync::Arc::new(Mutex::new(VecDeque::from([
            "feed:https://fixture.example/rss?q=a%26b".into(),
        ])));
        let again = queue.clone();
        let scope = day_reactive::Scope::child();
        let received = seen.clone();
        scope.enter(|| {
            on_open_url(move |url| {
                received.borrow_mut().push(url.to_owned());
                if url.starts_with("feed:") {
                    again
                        .lock()
                        .unwrap()
                        .push_back("atom:https://fixture.example/atom".into());
                }
                true
            })
        });
        drain_queue(&queue);
        queue
            .lock()
            .unwrap()
            .push_back("rss://fixture.example/rss".into());
        drain_queue(&queue);
        assert_eq!(
            *seen.borrow(),
            [
                "feed:https://fixture.example/rss?q=a%26b",
                "atom:https://fixture.example/atom",
                "rss://fixture.example/rss"
            ]
        );
        scope.dispose();
        assert!(HANDLERS.with(|h| h.borrow().is_empty()));
    }

    /// A later registration is asked first and may decline, which hands the URL on to the
    /// earlier one; disposing its scope removes only its own entry.
    #[test]
    fn handlers_stack_newest_first_and_leave_with_their_scope() {
        let app_seen = Rc::new(RefCell::new(Vec::new()));
        let part_seen = Rc::new(RefCell::new(Vec::new()));
        let queue = std::sync::Arc::new(Mutex::new(VecDeque::new()));
        let app_scope = day_reactive::Scope::child();
        let received = app_seen.clone();
        app_scope.enter(|| {
            on_open_url(move |url| {
                received.borrow_mut().push(url.to_owned());
                true
            })
        });
        let part_scope = day_reactive::Scope::child();
        let received = part_seen.clone();
        part_scope.enter(|| {
            on_open_url(move |url| {
                let mine = url.starts_with("myapp://auth");
                if mine {
                    received.borrow_mut().push(url.to_owned());
                }
                mine
            })
        });
        queue.lock().unwrap().extend([
            "myapp://auth?code=1".to_string(),
            "myapp://inbox".to_string(),
        ]);
        drain_queue(&queue);
        assert_eq!(*part_seen.borrow(), ["myapp://auth?code=1"]);
        assert_eq!(*app_seen.borrow(), ["myapp://inbox"]);
        part_scope.dispose();
        assert_eq!(HANDLERS.with(|h| h.borrow().len()), 1);
        queue
            .lock()
            .unwrap()
            .push_back("myapp://auth?code=2".into());
        drain_queue(&queue);
        assert_eq!(*app_seen.borrow(), ["myapp://inbox", "myapp://auth?code=2"]);
        app_scope.dispose();
        assert!(HANDLERS.with(|h| h.borrow().is_empty()));
    }
}
