// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! A process that links only day-process starts its services and receives native notifications:
//! no window, widget tree or toolkit exists in this test binary. This is the shape of a
//! background worker or a push handler.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use day_process::linkme::distributed_slice;
use day_process::services::{
    NOTIFICATION_HANDLERS, NotificationEvent, SERVICES, Service, initialize, notification,
};

static STARTED: AtomicUsize = AtomicUsize::new(0);
static RECEIVED: Mutex<Vec<String>> = Mutex::new(Vec::new());

// A part registers its initializer and its notification handler exactly like this.
#[distributed_slice(SERVICES)]
#[linkme(crate = day_process::linkme)]
static WORKER: Service = Service {
    name: "test-worker",
    initialize: || {
        STARTED.fetch_add(1, Ordering::SeqCst);
    },
};

#[distributed_slice(NOTIFICATION_HANDLERS)]
#[linkme(crate = day_process::linkme)]
static HANDLER: fn(&NotificationEvent) = |event| {
    if let NotificationEvent::Received { payload, opened } = event {
        RECEIVED.lock().unwrap().push(format!("{payload}:{opened}"));
    }
};

/// One test function, so the assertions about "before" and "after" cannot race each other.
#[test]
fn services_start_once_and_notifications_arrive_without_a_ui() {
    assert!(SERVICES.iter().any(|service| service.name == "test-worker"));
    assert_eq!(
        STARTED.load(Ordering::SeqCst),
        0,
        "nothing starts before the host asks"
    );

    initialize();
    assert_eq!(STARTED.load(Ordering::SeqCst), 1);

    // A host may call it again (Android's headless entry point and the Activity both do); a
    // service still starts once per process, from whichever thread asked first.
    initialize();
    std::thread::spawn(initialize)
        .join()
        .expect("the second host thread finishes");
    assert_eq!(STARTED.load(Ordering::SeqCst), 1);

    // A native delegate delivers on its own thread, with no window in existence.
    std::thread::spawn(|| {
        notification(NotificationEvent::Received {
            payload: r#"{"id":7}"#.to_owned(),
            opened: true,
        });
    })
    .join()
    .expect("the delivery thread finishes");
    assert_eq!(*RECEIVED.lock().unwrap(), [r#"{"id":7}:true"#]);

    // Registration results reach the same handlers and are simply not this handler's concern.
    notification(NotificationEvent::Registered(vec![1, 2, 3]));
    assert_eq!(RECEIVED.lock().unwrap().len(), 1);
}

/// The route a tap asks for is held for a UI that does not exist yet, and the newest wins.
#[test]
fn a_route_request_is_buffered_for_a_ui_that_has_not_started() {
    use day_process::routes;

    routes::request("");
    assert_eq!(routes::requested(), None, "an empty route is not a request");
    routes::request("mail/inbox");
    std::thread::spawn(|| routes::request("clock/alarm"))
        .join()
        .expect("the tap thread finishes");
    assert_eq!(routes::requested().as_deref(), Some("clock/alarm"));
    assert_eq!(routes::take().as_deref(), Some("clock/alarm"));
    assert_eq!(routes::take(), None);
}
