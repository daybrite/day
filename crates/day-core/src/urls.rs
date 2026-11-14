// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
//! Raw external URL delivery; application handlers may consume data links before routing.
//! Text shared to the app from the system share sheet arrives here too: a web link in it goes
//! the URL way, other text to [`on_shared_text`].
use std::{cell::RefCell, collections::VecDeque, rc::Rc, sync::Mutex};
static PENDING: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
/// Shared text with no web link in it, waiting for a handler.
static SHARED: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
day_reactive::tls_slots! {
    urls;
    static HANDLERS: RefCell<Vec<Rc<dyn Fn(&str) -> bool>>> = const { RefCell::new(Vec::new()) };
    static SHARED_HANDLERS: RefCell<Vec<Rc<dyn Fn(&str)>>> = const { RefCell::new(Vec::new()) };
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
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push_back(url.into());
    if day_reactive::has_main_poster() {
        day_reactive::on_main(drain);
    }
}
pub(crate) fn drain() {
    drain_queue(&PENDING);
}

/// Receive text the user shares to the app from another app's share sheet, on the UI thread
/// (Day.toml `share_target`, docs/deep-links.md "Receiving shared links").
///
/// Shared text that holds a web link never arrives here: its first `http`/`https` URL goes to
/// [`on_open_url`] instead, exactly as if the link had been opened with the app, which is what
/// a browser or a reader wants from "Share → App". This receives the rest: a search term, a
/// note, a quote. Text shared before any handler exists waits for the first registration.
///
/// The newest registration receives the text; it leaves when the scope it was made in is
/// disposed, and the one before it receives again.
pub fn on_shared_text(handler: impl Fn(&str) + 'static) {
    let handler: Rc<dyn Fn(&str)> = Rc::new(handler);
    SHARED_HANDLERS.with(|h| h.borrow_mut().push(handler.clone()));
    day_reactive::Scope::current().on_cleanup(move || {
        let _ = SHARED_HANDLERS.try_with(|h| {
            h.borrow_mut()
                .retain(|current| !Rc::ptr_eq(current, &handler));
        });
    });
    if day_reactive::has_main_poster() {
        day_reactive::on_main(drain_shared);
    }
}

/// Platform intake for the share sheet: the shared `text` and the `subject` that came with it
/// (a page title, a mail subject). The first web link in the text, else in the subject, goes to
/// [`request_open_url`]; otherwise the text (the subject when the text is empty) goes to
/// [`on_shared_text`].
pub fn request_shared_text(text: &str, subject: Option<&str>) {
    if let Some(url) = web_link_in(text).or_else(|| subject.and_then(web_link_in)) {
        request_open_url(url);
        return;
    }
    let text = if text.trim().is_empty() {
        subject.unwrap_or_default()
    } else {
        text
    };
    if text.trim().is_empty() {
        return;
    }
    SHARED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push_back(text.into());
    if day_reactive::has_main_poster() {
        day_reactive::on_main(drain_shared);
    }
}

fn drain_shared() {
    drain_shared_queue(&SHARED);
}

fn drain_shared_queue(queue: &Mutex<VecDeque<String>>) {
    loop {
        // Text waits in the queue until someone registers to receive it.
        let Some(handler) = SHARED_HANDLERS.with(|h| h.borrow().last().cloned()) else {
            return;
        };
        let next = queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pop_front();
        let Some(text) = next else { return };
        handler(&text);
    }
}

/// The first `http`/`https` link in shared text. Share sheets hand links over in every shape:
/// Chrome sends the bare URL, other apps "Title https://… via App", a mail client `<https://…>`.
/// The link ends at whitespace or a character URLs never contain, and loses the sentence
/// punctuation after it, and a closing bracket it did not open.
fn web_link_in(text: &str) -> Option<&str> {
    // ASCII lowercasing keeps every byte offset, so positions found here index `text`.
    let lower = text.to_ascii_lowercase();
    let mut from = 0;
    while let Some(found) = lower[from..].find("http") {
        let start = from + found;
        from = start + 4;
        let rest = &lower[start..];
        let scheme = if rest.starts_with("https://") {
            8
        } else if rest.starts_with("http://") {
            7
        } else {
            continue;
        };
        // Not the tail of a longer word ("xhttp://").
        if text[..start]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_ascii_alphanumeric())
        {
            continue;
        }
        let len = text[start..]
            .find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '`' | '\u{1f}'))
            .unwrap_or(text.len() - start);
        let mut link = &text[start..start + len];
        loop {
            let trimmed = link.trim_end_matches(['.', ',', ';', ':', '!', '?', '\'', '*']);
            let unbalanced = |open: char, close: char| {
                trimmed.ends_with(close)
                    && trimmed.matches(close).count() > trimmed.matches(open).count()
            };
            let trimmed = if unbalanced('(', ')') || unbalanced('[', ']') {
                &trimmed[..trimmed.len() - 1]
            } else {
                trimmed
            };
            if trimmed.len() == link.len() {
                break;
            }
            link = trimmed;
        }
        // A host, at least: "https://" alone, or "https:///x", is not a link.
        if link.len() > scheme && !link[scheme..].starts_with('/') {
            return Some(link);
        }
    }
    None
}
fn drain_queue(queue: &Mutex<VecDeque<String>>) {
    loop {
        let next = queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pop_front();
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
    fn shared_text_finds_the_web_link_in_it() {
        let cases = [
            (
                "https://example.org/a?b=c#d",
                Some("https://example.org/a?b=c#d"),
            ),
            ("  https://example.org\n", Some("https://example.org")),
            (
                "Read this: \"Title\" https://example.org/post. Via App",
                Some("https://example.org/post"),
            ),
            ("<HTTP://Example.org/x>", Some("HTTP://Example.org/x")),
            (
                "(see https://example.org/a_(b))",
                Some("https://example.org/a_(b)"),
            ),
            (
                "(see https://example.org/a)!",
                Some("https://example.org/a"),
            ),
            (
                "first http://a.example then https://b.example",
                Some("http://a.example"),
            ),
            (
                "xhttp://not.a.link https://yes.example",
                Some("https://yes.example"),
            ),
            ("ftp://example.org", None),
            ("https:// nothing", None),
            ("a quote with no link", None),
            ("http and https words", None),
        ];
        for (text, want) in cases {
            assert_eq!(web_link_in(text), want, "{text:?}");
        }
    }

    #[test]
    fn shared_text_waits_for_the_newest_handler() {
        let queue = Mutex::new(VecDeque::from(["early".to_string()]));
        // Nothing registered: the text stays.
        drain_shared_queue(&queue);
        assert_eq!(queue.lock().unwrap().len(), 1);
        let first = Rc::new(RefCell::new(Vec::new()));
        let second = Rc::new(RefCell::new(Vec::new()));
        let app = day_reactive::Scope::child();
        let seen = first.clone();
        app.enter(|| on_shared_text(move |t| seen.borrow_mut().push(t.to_owned())));
        drain_shared_queue(&queue);
        assert_eq!(*first.borrow(), ["early"]);
        let page = day_reactive::Scope::child();
        let seen = second.clone();
        page.enter(|| on_shared_text(move |t| seen.borrow_mut().push(t.to_owned())));
        queue.lock().unwrap().push_back("later".into());
        drain_shared_queue(&queue);
        assert_eq!(*second.borrow(), ["later"]);
        page.dispose();
        queue.lock().unwrap().push_back("last".into());
        drain_shared_queue(&queue);
        assert_eq!(*first.borrow(), ["early", "last"]);
        app.dispose();
        assert!(SHARED_HANDLERS.with(|h| h.borrow().is_empty()));
    }
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
            });
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
            });
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
            });
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
