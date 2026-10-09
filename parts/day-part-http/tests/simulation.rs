//! Synthetic requests: no publisher or external server is contacted.
use day_part_http::{
    Client, Cookies, Request, Session,
    simulation::{Conditions, ManualClock, Reply, Simulation},
};
use std::{
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};
fn setup() -> (Client, Session, Simulation, ManualClock) {
    let clock = ManualClock::default();
    let sim = Simulation::with_clock(Arc::new(clock.clone()));
    let session = Session::new();
    session.set_simulation(Some(sim.clone()));
    let client = Client::builder()
        .session(session.clone())
        .cookies(Cookies::jar())
        .build();
    (client, session, sim, clock)
}
#[test]
fn real_client_redirects_and_cookies_flow_through_handlers() {
    let (c, session, s, clock) = setup();
    s.route("GET", "https://fixture.example/start", |_| {
        Ok(Reply::new(302, Vec::new())
            .header("Location", "/end")
            .header("Set-Cookie", "test=1; Path=/"))
    });
    s.route("GET", "https://fixture.example/end", |r| {
        assert!(
            r.headers
                .iter()
                .any(|(k, v)| k.eq_ignore_ascii_case("cookie") && v == "test=1")
        );
        Ok(Reply::new(200, b"done".to_vec()))
    });
    let (tx, rx) = mpsc::channel();
    let _flight = c.fetch_async(Request::get("https://fixture.example/start"), move |r| {
        tx.send(r).unwrap()
    });
    clock.advance(Duration::ZERO);
    assert_eq!(rx.try_recv().unwrap().unwrap().text(), "done");
    assert_eq!(session.statistics().started, 2);
    assert_eq!(session.statistics().downloaded_bytes, 4);
    assert_eq!(session.statistics().active, 0);
}
#[test]
fn pause_cancel_and_strict_unmatched_never_reach_network() {
    let (c, session, s, clock) = setup();
    s.route("GET", "https://fixture.example/", |_| {
        Ok(Reply::new(200, vec![1; 32768]))
    });
    s.set_conditions(Conditions {
        paused: true,
        ..Default::default()
    });
    let (tx, rx) = mpsc::channel();
    let flight = c.fetch_async(Request::get("https://fixture.example/"), move |r| {
        tx.send(r).unwrap()
    });
    clock.advance(Duration::from_secs(1));
    assert!(rx.try_recv().is_err());
    flight.cancel();
    assert!(matches!(
        rx.try_recv(),
        Err(mpsc::TryRecvError::Disconnected)
    ));
    clock.advance(Duration::from_secs(1));
    assert_eq!(session.statistics().failed, 1);
    s.set_conditions(Conditions::default());
    let (tx, rx) = mpsc::channel();
    let _f = c.fetch_async(Request::get("https://unexpected.invalid/"), move |r| {
        tx.send(r).unwrap()
    });
    clock.advance(Duration::ZERO);
    assert!(rx.try_recv().unwrap().is_err());
}
#[test]
fn bandwidth_and_partial_failures_account_only_delivered_bytes() {
    let (c, session, s, clock) = setup();
    s.route("GET", "https://fixture.example/", |_| {
        let mut r = Reply::new(200, vec![1; 32768]);
        r.fail_after_chunks = Some(1);
        Ok(r)
    });
    s.set_conditions(Conditions {
        bytes_per_second: Some(16384),
        ..Default::default()
    });
    let (tx, rx) = mpsc::channel();
    let _f = c.fetch_async(Request::get("https://fixture.example/"), move |r| {
        tx.send(r).unwrap()
    });
    clock.advance(Duration::from_millis(999));
    assert!(rx.try_recv().is_err());
    clock.advance(Duration::from_millis(1001));
    assert!(rx.try_recv().unwrap().is_err());
    assert_eq!(session.statistics().downloaded_bytes, 16384);
    assert_eq!(session.statistics().failed, 1);
}
#[test]
fn seeded_failures_do_not_depend_on_other_urls() {
    fn run(noise: bool) -> Vec<bool> {
        let (c, _, s, clock) = setup();
        s.handle(|_, _| true, |_| Ok(Reply::new(200, vec![])));
        s.set_conditions(Conditions {
            seed: 42,
            failure_rate: 0.5,
            ..Default::default()
        });
        for _ in 0..20 {
            let _f = c.fetch_async(Request::get("https://fixture.example/a"), |_| {});
            if noise {
                let _f = c.fetch_async(Request::get("https://fixture.example/b"), |_| {});
            }
            clock.advance(Duration::ZERO);
        }
        s.requests()
            .iter()
            .filter(|r| r.url.ends_with("/a"))
            .map(|r| r.injected_failure)
            .collect()
    }
    let a = run(false);
    assert!(a.contains(&true) && a.contains(&false));
    assert_eq!(a, run(true));
}
#[test]
fn simulation_isolated_between_sessions_and_changes_apply_to_existing_client() {
    let (c, _, s, clock) = setup();
    s.route("GET", "https://fixture.example/", |_| {
        Ok(Reply::new(200, b"one".to_vec()))
    });
    let (other, _, other_sim, other_clock) = setup();
    other_sim.route("GET", "https://fixture.example/", |_| {
        Ok(Reply::new(200, b"two".to_vec()))
    });
    for (client, clock, body) in [(c, clock, "one"), (other, other_clock, "two")] {
        let (tx, rx) = mpsc::channel();
        let _f = client.fetch_async(Request::get("https://fixture.example/"), move |r| {
            tx.send(r).unwrap()
        });
        clock.advance(Duration::ZERO);
        assert_eq!(rx.try_recv().unwrap().unwrap().text(), body);
    }
}
#[test]
fn websocket_echo_respects_demand_and_updates_statistics() {
    let (c, session, s, clock) = setup();
    s.websocket_echo("wss://fixture.example/echo");
    let (tx, rx) = mpsc::channel();
    c.websocket_async(Request::get("wss://fixture.example/echo"), move |r| {
        tx.send(r).unwrap()
    });
    clock.advance(Duration::ZERO);
    let socket = rx.try_recv().unwrap().unwrap();
    let received = Arc::new(Mutex::new(Vec::new()));
    let output = received.clone();
    let sender = socket.sender();
    socket.read_async(move |m| {
        if let Some(Ok(day_part_http::Message::Text(t))) = m {
            output.lock().unwrap().push(t);
        }
        true
    });
    sender.send_async(day_part_http::Message::Text("hello".into()), |r| r.unwrap());
    clock.advance(Duration::ZERO);
    assert_eq!(*received.lock().unwrap(), vec!["hello"]);
    sender.close(1000, "done");
    assert_eq!(session.statistics().uploaded_bytes, 5);
    assert_eq!(session.statistics().downloaded_bytes, 5);
    assert_eq!(session.statistics().active, 0);
}
#[cfg(all(
    feature = "reqwest",
    not(target_arch = "wasm32"),
    not(target_env = "ohos")
))]
#[test]
fn switch_existing_client_between_native_and_reqwest_and_echo_websocket() {
    use day_part_http::{Provider, testing::Server};
    let server = Server::start().unwrap();
    let session = Session::new();
    let c = Client::builder().session(session.clone()).build();
    for p in [Provider::Native, Provider::Reqwest, Provider::Native] {
        session.set_provider(p);
        let (tx, rx) = mpsc::channel();
        let _f = c.fetch_async(Request::get(server.url("/")), move |r| tx.send(r).unwrap());
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(10))
                .unwrap()
                .unwrap()
                .text(),
            "day-http-ok"
        );
    }
    session.set_provider(Provider::Reqwest);
    let (tx, rx) = mpsc::channel();
    c.websocket_async(Request::get(server.ws_url("/ws/echo")), move |r| {
        tx.send(r).unwrap()
    });
    let socket = rx.recv_timeout(Duration::from_secs(10)).unwrap().unwrap();
    let sender = socket.sender();
    let (tx, rx) = mpsc::channel();
    socket.read_async(move |m| {
        if let Some(Ok(day_part_http::Message::Text(t))) = m {
            tx.send(t).unwrap();
        }
        true
    });
    sender.send_async(day_part_http::Message::Text("echo".into()), |r| r.unwrap());
    assert_eq!(rx.recv_timeout(Duration::from_secs(10)).unwrap(), "echo");
    sender.close(1000, "done");
}

#[test]
fn manual_clock_drives_idle_and_total_deadlines() {
    let (client, session, simulation, clock) = setup();
    simulation.route("GET", "https://fixture.example/slow", |_| {
        let mut reply = Reply::new(200, vec![0; 32768]);
        reply.delay = Duration::from_secs(10);
        Ok(reply)
    });
    let (tx, rx) = mpsc::channel();
    let _flight = client.fetch_async(
        Request::get("https://fixture.example/slow").timeout_total(Duration::from_secs(2)),
        move |r| {
            tx.send(r).unwrap();
        },
    );
    clock.advance(Duration::from_secs(2));
    assert!(matches!(
        rx.try_recv().unwrap(),
        Err(day_part_http::HttpError::Timeout)
    ));
    simulation.set_conditions(Conditions {
        bytes_per_second: Some(1),
        ..Default::default()
    });
    simulation.route("GET", "https://fixture.example/body", |_| {
        Ok(Reply::new(200, vec![0; 32]))
    });
    let (tx, rx) = mpsc::channel();
    let _flight = client.fetch_async(
        Request::get("https://fixture.example/body").timeout(Duration::from_secs(1)),
        move |r| {
            tx.send(r).unwrap();
        },
    );
    clock.advance(Duration::from_secs(1));
    assert!(matches!(
        rx.try_recv().unwrap(),
        Err(day_part_http::HttpError::Timeout)
    ));
    assert_eq!(session.statistics().active, 0);
}

#[test]
fn disabling_interception_does_not_replace_an_active_response() {
    let (client, session, simulation, clock) = setup();
    simulation.route("GET", "https://fixture.example/", |_| {
        let mut r = Reply::new(200, b"kept".to_vec());
        r.delay = Duration::from_secs(1);
        Ok(r)
    });
    let (tx, rx) = mpsc::channel();
    let _flight = client.fetch_async(Request::get("https://fixture.example/"), move |r| {
        tx.send(r).unwrap();
    });
    session.set_simulation(None);
    clock.advance(Duration::from_secs(1));
    assert_eq!(rx.try_recv().unwrap().unwrap().text(), "kept");
    assert_eq!(session.statistics().simulated_downloaded_bytes, 4);
}

#[test]
fn existing_default_client_observes_global_interception() {
    let client = Client::new();
    let clock = ManualClock::default();
    let simulation = Simulation::with_clock(Arc::new(clock.clone()));
    simulation.route("GET", "https://global-fixture.invalid/", |_| {
        Ok(Reply::new(200, b"global".to_vec()))
    });
    let global = Session::global();
    global.set_simulation(Some(simulation));
    let (tx, rx) = mpsc::channel();
    let _f = client.fetch_async(Request::get("https://global-fixture.invalid/"), move |r| {
        tx.send(r).unwrap()
    });
    clock.advance(Duration::ZERO);
    assert_eq!(rx.try_recv().unwrap().unwrap().text(), "global");
    global.set_simulation(None);
}

#[cfg(all(
    feature = "reqwest",
    not(target_arch = "wasm32"),
    not(target_env = "ohos")
))]
#[test]
fn reqwest_streaming_upload_and_cancellation_are_observed() {
    use day_part_http::{Provider, testing::Server};
    let server = Server::start().unwrap();
    let session = Session::new();
    session.set_provider(Provider::Reqwest);
    let client = Client::builder().session(session.clone()).build();
    let (tx, rx) = mpsc::channel();
    let _f = client.fetch_async(
        Request::post(server.url("/echo"), vec![7; 65536]),
        move |r| tx.send(r).unwrap(),
    );
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(10))
            .unwrap()
            .unwrap()
            .body,
        vec![7; 65536]
    );
    let stats = session.statistics();
    assert_eq!(stats.uploaded_bytes, 65536);
    assert_eq!(stats.downloaded_bytes, 65536);
    let (tx, rx) = mpsc::channel();
    let flight = client.fetch_async(Request::get(server.url("/delay/5000")), move |r| {
        let _ = tx.send(r);
    });
    flight.cancel();
    assert_eq!(session.statistics().active, 0);
    assert_eq!(session.statistics().failed, 1);
    assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());
}
