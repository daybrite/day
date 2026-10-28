// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
//! DAV tokens pass through the same native and intercepted exchanges as standard HTTP verbs.
use day_part_http::{
    Client, Method, Request, Session,
    simulation::{Reply, Simulation},
    transport::PreparedBody,
};

const DAV: [Method; 9] = [
    Method::Options,
    Method::Propfind,
    Method::Proppatch,
    Method::Mkcol,
    Method::Copy,
    Method::Move,
    Method::Lock,
    Method::Unlock,
    Method::Report,
];
#[test]
fn method_tokens_round_trip() {
    for method in DAV {
        assert_eq!(Method::parse(method.as_str()), Some(method));
        assert_eq!(
            Method::parse(&method.as_str().to_ascii_lowercase()),
            Some(method)
        );
    }
    assert_eq!(Method::parse("PROPFIND\r\nX-Injected: yes"), None);
}
#[test]
fn dav_methods_and_xml_survive_interception() {
    let simulation = Simulation::new();
    simulation.handle(
        |_, _| true,
        |request| {
            assert!(DAV.iter().any(|m| m.as_str() == request.method));
            assert!(
                request
                    .headers
                    .iter()
                    .any(|(k, v)| k == "Depth" && v == "1")
            );
            assert!(matches!(request.body,PreparedBody::Bytes(b) if b.as_slice()==b"<propfind/>"));
            Ok(Reply::new(207, b"<multistatus/>".to_vec()))
        },
    );
    let session = Session::new();
    session.set_simulation(Some(simulation));
    let client = Client::builder().session(session).build();
    for method in DAV {
        let (tx, rx) = std::sync::mpsc::channel();
        let _flight = client.fetch_async(
            Request::new(method, "https://fixture.invalid/dav/")
                .header("Depth", "1")
                .body(b"<propfind/>".to_vec()),
            move |r| {
                tx.send(r).unwrap();
            },
        );
        assert_eq!(
            rx.recv_timeout(std::time::Duration::from_secs(5))
                .unwrap()
                .unwrap()
                .status,
            207
        );
    }
}
#[test]
fn native_transport_preserves_dav_tokens() {
    let server = day_part_http::testing::Server::start().unwrap();
    for method in DAV {
        let response = day_part_http::fetch(
            &Request::new(method, server.url("/echo")).body(b"<fixture/>".to_vec()),
        )
        .unwrap();
        assert_eq!(response.header("x-day-method"), Some(method.as_str()));
        assert_eq!(response.body, b"<fixture/>");
    }
}
