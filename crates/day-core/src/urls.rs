// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
//! Raw external URL delivery; application handlers may consume data links before routing.
use std::{cell::RefCell, collections::VecDeque, rc::Rc, sync::Mutex};
static PENDING: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
day_reactive::tls_slots! {
    urls;
    static HANDLER: RefCell<Option<Rc<dyn Fn(&str) -> bool>>> = const { RefCell::new(None) };
}
/// Register on the UI thread in the application root. Return true to consume a URL;
/// false retains normal deep-link routing. Delivery never performs a privileged action.
pub fn on_open_url(handler: impl Fn(&str) -> bool + 'static) {
    let handler: Rc<dyn Fn(&str) -> bool> = Rc::new(handler);
    HANDLER.with(|h| *h.borrow_mut() = Some(handler.clone()));
    day_reactive::Scope::current().on_cleanup(move || {
        let _ = HANDLER.try_with(|h| {
            if h.borrow()
                .as_ref()
                .is_some_and(|current| Rc::ptr_eq(current, &handler))
            {
                h.borrow_mut().take();
            }
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
        let consumed = HANDLER
            .with(|h| h.borrow().clone())
            .is_some_and(|h| h(&url));
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
        assert!(HANDLER.with(|h| h.borrow().is_none()));
    }
}
