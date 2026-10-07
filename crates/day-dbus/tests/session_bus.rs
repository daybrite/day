// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Tests against a real bus daemon. They are ignored by default because they need one: start a
//! private session bus and point `DBUS_SESSION_BUS_ADDRESS` at it, then run them explicitly:
//!
//! ```sh
//! eval "$(dbus-launch --sh-syntax)"   # or: dbus-daemon --session --fork --print-address
//! cargo test -p day-dbus --test session_bus -- --ignored --test-threads=1
//! ```
//!
//! When `gdbus` and `dbus-send` are installed, the status item test also queries the item with
//! them, proving that a non-Day client reads what this crate serves. Set
//! `DAY_DBUS_REQUIRE_CLI=1` to fail instead of skipping when they are missing.

#![cfg(unix)]

use std::process::Command;
use std::sync::Mutex;
use std::sync::mpsc;
use std::time::Duration;

use day_dbus::launcher::{self, LauncherProps};
use day_dbus::sni::{self, Check, Event, Item, MenuEntry, Pixmap, StatusItem};
use day_dbus::{
    BUS_INTERFACE, BUS_NAME, BUS_PATH, Connection, Error, MethodError, NAME_REPLY_PRIMARY_OWNER,
    PEER_INTERFACE, PROPERTIES_INTERFACE, SignalFilter, Value,
};

const WAIT: Duration = Duration::from_secs(3);

fn connect() -> Connection {
    Connection::session().expect("a session bus (set DBUS_SESSION_BUS_ADDRESS)")
}

#[test]
#[ignore = "needs a session bus daemon"]
fn bus_calls_objects_and_signals() {
    let a = connect();
    let b = connect();
    assert!(a.unique_name().starts_with(':'));
    assert_ne!(a.unique_name(), b.unique_name());

    // ListNames includes the bus and both connections.
    let names = a
        .call(BUS_NAME, BUS_PATH, BUS_INTERFACE, "ListNames", &[])
        .expect("ListNames");
    let names: Vec<&str> = names[0]
        .as_array()
        .expect("as")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(names.contains(&BUS_NAME));
    assert!(names.contains(&a.unique_name()));
    assert!(names.contains(&b.unique_name()));

    // b owns a name and exports an object; a calls it.
    assert_eq!(
        b.request_name("dev.daybrite.DbusTest", 0)
            .expect("RequestName"),
        NAME_REPLY_PRIMARY_OWNER
    );
    assert!(a.name_has_owner("dev.daybrite.DbusTest"));
    assert!(!a.name_has_owner("dev.daybrite.Nobody"));
    let (seen_tx, seen_rx) = mpsc::channel();
    let seen_tx = Mutex::new(seen_tx);
    b.export("/dev/daybrite/Test", move |call| {
        match call.member.as_str() {
            "Echo" => Ok(call.args.clone()),
            "Fail" => Err(MethodError::new("dev.daybrite.Error.Nope", "asked to fail")),
            "Slow" => {
                std::thread::sleep(Duration::from_millis(600));
                Ok(Vec::new())
            }
            "Nested" => {
                // A blocking call from the reader thread is refused, not deadlocked.
                let refused = matches!(
                    call.connection
                        .call(BUS_NAME, BUS_PATH, BUS_INTERFACE, "ListNames", &[]),
                    Err(Error::ReaderThread)
                );
                Ok(vec![Value::Bool(refused)])
            }
            "Note" => {
                let _ = day_dbus_lock(&seen_tx).send(call.args.clone());
                Ok(Vec::new())
            }
            _ => Err(MethodError::unknown_method(call)),
        }
    })
    .expect("export");

    let args = vec![
        Value::Byte(1),
        Value::Struct(vec![Value::Byte(2), Value::Double(0.5)]),
        Value::props([
            ("n", Value::I16(-3)),
            ("v", Value::variant(Value::variant(Value::U64(9)))),
        ]),
        Value::array("(ii)", Vec::new()),
        Value::dict("o", "as", Vec::new()),
        Value::array(
            "(iiay)",
            vec![Value::Struct(vec![
                Value::I32(1),
                Value::I32(1),
                Value::bytes(&[0xff, 1, 2, 3]),
            ])],
        ),
        Value::path("/x/y"),
        Value::Signature("a{sv}".into()),
        Value::Bool(true),
        Value::I64(-1),
        Value::U16(7),
    ];
    let echoed = a
        .call(
            "dev.daybrite.DbusTest",
            "/dev/daybrite/Test",
            "dev.daybrite.Test",
            "Echo",
            &args,
        )
        .expect("Echo");
    assert_eq!(echoed, args);

    match a.call(
        "dev.daybrite.DbusTest",
        "/dev/daybrite/Test",
        "dev.daybrite.Test",
        "Fail",
        &[],
    ) {
        Err(Error::Remote { name, message }) => {
            assert_eq!(name, "dev.daybrite.Error.Nope");
            assert_eq!(message, "asked to fail");
        }
        other => panic!("expected a remote error, got {other:?}"),
    }
    match a.call(
        "dev.daybrite.DbusTest",
        "/no/such",
        "dev.daybrite.Test",
        "Echo",
        &[],
    ) {
        Err(Error::Remote { name, .. }) => {
            assert_eq!(name, "org.freedesktop.DBus.Error.UnknownObject")
        }
        other => panic!("expected UnknownObject, got {other:?}"),
    }
    match a.call("dev.daybrite.Nobody", "/", "dev.daybrite.Test", "Echo", &[]) {
        Err(Error::Remote { name, .. }) => {
            assert_eq!(name, "org.freedesktop.DBus.Error.ServiceUnknown")
        }
        other => panic!("expected ServiceUnknown, got {other:?}"),
    }
    let slow = a.call_with_timeout(
        "dev.daybrite.DbusTest",
        "/dev/daybrite/Test",
        "dev.daybrite.Test",
        "Slow",
        &[],
        Duration::from_millis(150),
    );
    assert!(matches!(slow, Err(Error::Timeout)), "{slow:?}");
    let nested = a
        .call(
            "dev.daybrite.DbusTest",
            "/dev/daybrite/Test",
            "dev.daybrite.Test",
            "Nested",
            &[],
        )
        .expect("Nested");
    assert_eq!(nested, vec![Value::Bool(true)]);
    a.call(
        b.unique_name(),
        "/dev/daybrite/Test",
        PEER_INTERFACE,
        "Ping",
        &[],
    )
    .expect("Peer.Ping");
    let xml = a
        .call(
            "dev.daybrite.DbusTest",
            "/",
            "org.freedesktop.DBus.Introspectable",
            "Introspect",
            &[],
        )
        .expect("Introspect /");
    assert!(
        xml[0]
            .as_str()
            .unwrap_or_default()
            .contains("<node name=\"dev\"/>")
    );

    // A call without a reply still arrives.
    a.call_no_reply(
        "dev.daybrite.DbusTest",
        "/dev/daybrite/Test",
        "dev.daybrite.Test",
        "Note",
        &[Value::str("fire and forget")],
    )
    .expect("call_no_reply");
    assert_eq!(
        seen_rx.recv_timeout(WAIT).expect("note"),
        vec![Value::str("fire and forget")]
    );

    // Signals: a subscribes, b emits.
    let (sig_tx, sig_rx) = mpsc::channel();
    let sig_tx = Mutex::new(sig_tx);
    a.add_match("type='signal',interface='dev.daybrite.Test'")
        .expect("AddMatch");
    let sub = a.on_signal(
        SignalFilter::new()
            .interface("dev.daybrite.Test")
            .member("Changed"),
        move |_, msg| {
            let _ = day_dbus_lock(&sig_tx).send((
                msg.sender.clone(),
                msg.path.clone(),
                msg.body.clone(),
            ));
        },
    );
    b.emit_signal(
        "/dev/daybrite/Test",
        "dev.daybrite.Test",
        "Changed",
        &[Value::U32(42)],
    )
    .expect("emit");
    let (sender, path, body) = sig_rx.recv_timeout(WAIT).expect("signal");
    assert_eq!(sender.as_deref(), Some(b.unique_name()));
    assert_eq!(path.as_deref(), Some("/dev/daybrite/Test"));
    assert_eq!(body, vec![Value::U32(42)]);
    a.remove_signal_handler(sub);

    // The launcher entry signal carries the app URI and the property dictionary.
    let (ln_tx, ln_rx) = mpsc::channel();
    let ln_tx = Mutex::new(ln_tx);
    a.add_match("type='signal',interface='com.canonical.Unity.LauncherEntry'")
        .expect("AddMatch launcher");
    a.on_signal(
        SignalFilter::new()
            .interface(launcher::INTERFACE)
            .member("Update"),
        move |_, msg| {
            let _ = day_dbus_lock(&ln_tx).send(msg.body.clone());
        },
    );
    launcher::update(
        &b,
        "dev.daybrite.Sample.desktop",
        LauncherProps {
            count: Some(3),
            progress: Some(0.25),
            urgent: None,
        },
    )
    .expect("launcher update");
    let body = ln_rx.recv_timeout(WAIT).expect("launcher signal");
    assert_eq!(
        body[0],
        Value::str("application://dev.daybrite.Sample.desktop")
    );
    assert_eq!(body[1].dict_get("count"), Some(&Value::I64(3)));
    assert_eq!(body[1].dict_get("count-visible"), Some(&Value::Bool(true)));
    assert_eq!(body[1].dict_get("progress"), Some(&Value::Double(0.25)));
    assert_eq!(body[1].dict_get("urgent"), Some(&Value::Bool(false)));

    // Closing fails later calls instead of hanging.
    b.close();
    assert!(matches!(
        b.call(BUS_NAME, BUS_PATH, BUS_INTERFACE, "ListNames", &[]),
        Err(Error::Disconnected)
    ));
}

fn day_dbus_lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A minimal `StatusNotifierWatcher` that records registrations.
fn fake_watcher() -> (Connection, mpsc::Receiver<(String, String)>) {
    let conn = connect();
    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(tx);
    conn.export(sni::WATCHER_PATH, move |call| match call.member.as_str() {
        "RegisterStatusNotifierItem" => {
            let service = call
                .args
                .first()
                .and_then(Value::as_str)
                .unwrap_or_default();
            let sender = call.sender.clone().unwrap_or_default();
            let _ = day_dbus_lock(&tx).send((sender, service.to_owned()));
            Ok(Vec::new())
        }
        _ => Err(MethodError::unknown_method(call)),
    })
    .expect("export watcher");
    assert_eq!(
        conn.request_name(sni::WATCHER_NAME, day_dbus::NAME_FLAG_DO_NOT_QUEUE)
            .expect("own watcher"),
        NAME_REPLY_PRIMARY_OWNER
    );
    (conn, rx)
}

fn cli_available(tool: &str) -> bool {
    let found = Command::new(tool).arg("--help").output().is_ok();
    if !found && std::env::var_os("DAY_DBUS_REQUIRE_CLI").is_some() {
        panic!("{tool} is required (DAY_DBUS_REQUIRE_CLI) but not installed");
    }
    if !found {
        eprintln!("skipping the {tool} interop checks: not installed");
    }
    found
}

fn run(tool: &str, args: &[&str]) -> String {
    let out = Command::new(tool).args(args).output().expect("spawn");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "{tool} {args:?} failed: {}{}",
        stdout,
        String::from_utf8_lossy(&out.stderr)
    );
    eprintln!("{tool} {}\n{stdout}", args.join(" "));
    stdout
}

#[test]
#[ignore = "needs a session bus daemon"]
fn status_item_registers_and_serves() {
    let (watcher, registrations) = fake_watcher();
    let item_conn = connect();
    assert!(sni::watcher_available(&item_conn));

    let (ev_tx, ev_rx) = mpsc::channel();
    let pixmap = Pixmap {
        width: 1,
        height: 1,
        argb: vec![0xff10_2030],
    };
    let item = StatusItem::new(
        &item_conn,
        Item {
            id: "dev.daybrite.Sample".into(),
            title: "Day Sample".into(),
            tooltip: "Sample status".into(),
            icon_name: "applications-system".into(),
            icon_pixmaps: vec![pixmap],
            ..Item::default()
        },
        vec![
            MenuEntry::item(1, "Open"),
            MenuEntry::item(2, "Mute").with_check(Check::On),
            MenuEntry::separator(3),
            MenuEntry::submenu(4, "More", vec![MenuEntry::item(5, "Deep")]),
            MenuEntry::item(6, "Unavailable").with_enabled(false),
        ],
        move |event| {
            let _ = ev_tx.send(event);
        },
    )
    .expect("StatusItem::new");
    let service = item.service().to_owned();
    assert!(
        service.starts_with("org.kde.StatusNotifierItem-"),
        "{service}"
    );

    let (sender, registered) = registrations.recv_timeout(WAIT).expect("registration");
    assert_eq!(registered, service);
    assert_eq!(sender, item_conn.unique_name());

    // Properties, as a host reads them.
    let all = watcher
        .call(
            &service,
            sni::ITEM_PATH,
            PROPERTIES_INTERFACE,
            "GetAll",
            &[Value::str(sni::ITEM_INTERFACE)],
        )
        .expect("GetAll");
    let props = &all[0];
    assert_eq!(props.dict_get("Title"), Some(&Value::str("Day Sample")));
    assert_eq!(
        props.dict_get("Id"),
        Some(&Value::str("dev.daybrite.Sample"))
    );
    assert_eq!(props.dict_get("Status"), Some(&Value::str("Active")));
    assert_eq!(
        props.dict_get("Category"),
        Some(&Value::str("ApplicationStatus"))
    );
    assert_eq!(props.dict_get("Menu"), Some(&Value::path(sni::MENU_PATH)));
    assert_eq!(props.dict_get("ItemIsMenu"), Some(&Value::Bool(false)));
    let pixmaps = props
        .dict_get("IconPixmap")
        .and_then(Value::as_array)
        .expect("pixmaps");
    assert_eq!(
        pixmaps[0],
        Value::Struct(vec![
            Value::I32(1),
            Value::I32(1),
            Value::bytes(&[0xff, 0x10, 0x20, 0x30])
        ])
    );
    let tooltip = props
        .dict_get("ToolTip")
        .and_then(Value::as_struct)
        .expect("tooltip");
    assert_eq!(tooltip[2], Value::str("Sample status"));
    let title = watcher
        .call(
            &service,
            sni::ITEM_PATH,
            PROPERTIES_INTERFACE,
            "Get",
            &[Value::str(sni::ITEM_INTERFACE), Value::str("Title")],
        )
        .expect("Get");
    assert_eq!(title, vec![Value::variant(Value::str("Day Sample"))]);
    let version = watcher
        .call(
            &service,
            sni::MENU_PATH,
            PROPERTIES_INTERFACE,
            "Get",
            &[Value::str(sni::MENU_INTERFACE), Value::str("Version")],
        )
        .expect("Get Version");
    assert_eq!(version, vec![Value::variant(Value::U32(3))]);

    // The menu layout.
    let reply = watcher
        .call(
            &service,
            sni::MENU_PATH,
            sni::MENU_INTERFACE,
            "GetLayout",
            &[Value::I32(0), Value::I32(-1), Value::strings::<&str>(&[])],
        )
        .expect("GetLayout");
    let Value::U32(revision) = reply[0] else {
        panic!("revision")
    };
    let root = reply[1].as_struct().expect("root");
    assert_eq!(root[0], Value::I32(0));
    let children = root[2].as_array().expect("children");
    assert_eq!(children.len(), 5);
    let mute = children[1].as_struct().expect("mute");
    assert_eq!(mute[1].dict_get("label"), Some(&Value::str("Mute")));
    assert_eq!(
        mute[1].dict_get("toggle-type"),
        Some(&Value::str("checkmark"))
    );
    assert_eq!(mute[1].dict_get("toggle-state"), Some(&Value::I32(1)));
    let more = children[3].as_struct().expect("more");
    assert_eq!(more[2].as_array().map(<[Value]>::len), Some(1));
    let groups = watcher
        .call(
            &service,
            sni::MENU_PATH,
            sni::MENU_INTERFACE,
            "GetGroupProperties",
            &[
                Value::array("i", vec![Value::I32(5)]),
                Value::strings(&["label"]),
            ],
        )
        .expect("GetGroupProperties");
    let row = groups[0].as_array().expect("rows")[0]
        .as_struct()
        .expect("row");
    assert_eq!(row[0], Value::I32(5));
    assert_eq!(row[1].dict_get("label"), Some(&Value::str("Deep")));

    // Clicks reach the callback; a disabled entry does not.
    let click = |id: i32| {
        watcher.call(
            &service,
            sni::MENU_PATH,
            sni::MENU_INTERFACE,
            "Event",
            &[
                Value::I32(id),
                Value::str("clicked"),
                Value::variant(Value::I32(0)),
                Value::U32(0),
            ],
        )
    };
    click(6).expect("Event on a disabled entry");
    click(5).expect("Event");
    assert_eq!(
        ev_rx.recv_timeout(WAIT).expect("menu event"),
        Event::MenuItem { id: 5 }
    );
    assert!(click(99).is_err());
    watcher
        .call(
            &service,
            sni::ITEM_PATH,
            sni::ITEM_INTERFACE,
            "Activate",
            &[Value::I32(10), Value::I32(20)],
        )
        .expect("Activate");
    assert_eq!(
        ev_rx.recv_timeout(WAIT).expect("activate"),
        Event::Activate { x: 10, y: 20 }
    );
    watcher
        .call(
            &service,
            sni::ITEM_PATH,
            sni::ITEM_INTERFACE,
            "Scroll",
            &[Value::I32(-2), Value::str("horizontal")],
        )
        .expect("Scroll");
    assert_eq!(
        ev_rx.recv_timeout(WAIT).expect("scroll"),
        Event::Scroll {
            delta: -2,
            horizontal: true
        }
    );

    // Updates emit the item's signals and bump the layout revision.
    let (sig_tx, sig_rx) = mpsc::channel();
    let sig_tx = Mutex::new(sig_tx);
    watcher
        .add_match(&format!(
            "type='signal',sender='{}'",
            item_conn.unique_name()
        ))
        .expect("AddMatch item");
    watcher.on_signal(
        SignalFilter::new().sender(item_conn.unique_name()),
        move |_, msg| {
            let _ = day_dbus_lock(&sig_tx)
                .send((msg.member.clone().unwrap_or_default(), msg.body.clone()));
        },
    );
    item.set_title("Renamed").expect("set_title");
    item.set_tooltip("New tip").expect("set_tooltip");
    item.set_status(sni::Status::NeedsAttention)
        .expect("set_status");
    item.set_icon("dialog-information", Vec::new())
        .expect("set_icon");
    item.set_menu(vec![MenuEntry::item(7, "Only")])
        .expect("set_menu");
    let mut got = Vec::new();
    for _ in 0..5 {
        got.push(sig_rx.recv_timeout(WAIT).expect("update signal"));
    }
    let members: Vec<&str> = got.iter().map(|(m, _)| m.as_str()).collect();
    assert_eq!(
        members,
        [
            "NewTitle",
            "NewToolTip",
            "NewStatus",
            "NewIcon",
            "LayoutUpdated"
        ]
    );
    assert_eq!(got[2].1, vec![Value::str("NeedsAttention")]);
    assert_eq!(got[4].1, vec![Value::U32(revision + 1), Value::I32(0)]);
    let title = watcher
        .call(
            &service,
            sni::ITEM_PATH,
            PROPERTIES_INTERFACE,
            "Get",
            &[Value::str(sni::ITEM_INTERFACE), Value::str("Title")],
        )
        .expect("Get renamed");
    assert_eq!(title, vec![Value::variant(Value::str("Renamed"))]);

    // Interop with non-Day clients.
    if cli_available("gdbus") {
        let out = run(
            "gdbus",
            &[
                "call",
                "--session",
                "--dest",
                &service,
                "--object-path",
                sni::ITEM_PATH,
                "--method",
                "org.freedesktop.DBus.Properties.GetAll",
                sni::ITEM_INTERFACE,
            ],
        );
        assert!(out.contains("'Title': <'Renamed'>"), "{out}");
        assert!(out.contains("'Menu': <objectpath '/MenuBar'>"), "{out}");
        let out = run(
            "gdbus",
            &[
                "call",
                "--session",
                "--dest",
                &service,
                "--object-path",
                sni::MENU_PATH,
                "--method",
                "com.canonical.dbusmenu.GetLayout",
                // "--" keeps gdbus from reading -1 as an option.
                "--",
                "0",
                "-1",
                "[]",
            ],
        );
        assert!(out.contains("'label': <'Only'>"), "{out}");
        let out = run(
            "gdbus",
            &[
                "introspect",
                "--session",
                "--dest",
                &service,
                "--object-path",
                sni::ITEM_PATH,
            ],
        );
        assert!(
            out.contains("interface org.kde.StatusNotifierItem"),
            "{out}"
        );
        run(
            "gdbus",
            &[
                "call",
                "--session",
                "--dest",
                &service,
                "--object-path",
                sni::MENU_PATH,
                "--method",
                "com.canonical.dbusmenu.Event",
                "7",
                "clicked",
                "<0>",
                "0",
            ],
        );
        assert_eq!(
            ev_rx.recv_timeout(WAIT).expect("gdbus click"),
            Event::MenuItem { id: 7 }
        );
    }
    if cli_available("dbus-send") {
        run(
            "dbus-send",
            &[
                "--session",
                "--print-reply",
                &format!("--dest={service}"),
                sni::ITEM_PATH,
                "org.freedesktop.DBus.Peer.Ping",
            ],
        );
        let out = run(
            "dbus-send",
            &[
                "--session",
                "--print-reply",
                &format!("--dest={service}"),
                sni::ITEM_PATH,
                "org.freedesktop.DBus.Properties.Get",
                "string:org.kde.StatusNotifierItem",
                "string:Status",
            ],
        );
        assert!(out.contains("NeedsAttention"), "{out}");
    }

    // A watcher that restarts gets the item registered again.
    watcher.close();
    drop(watcher);
    let (watcher2, registrations2) = fake_watcher();
    let (_, again) = registrations2.recv_timeout(WAIT).expect("re-registration");
    assert_eq!(again, service);

    // Dropping the item releases its name.
    drop(item);
    std::thread::sleep(Duration::from_millis(200));
    assert!(!watcher2.name_has_owner(&service));
    drop(item_conn);
}

#[test]
#[ignore = "needs a session bus daemon"]
fn single_instance_forwards_and_hands_over() {
    use day_dbus::instance::{self, Claimed};

    let app_id = "dev.daybrite.InstanceTest";
    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(tx);
    let first = instance::claim(app_id, vec!["ignored".into()], move |args| {
        let _ = day_dbus_lock(&tx).send(args);
    })
    .expect("first launch claims");
    assert_eq!(first.name(), app_id);

    // A second launch forwards its arguments and is told to exit.
    let args = vec!["--open".to_owned(), "a file.txt".to_owned(), String::new()];
    match instance::claim(app_id, args.clone(), |_| panic!("not the owner")) {
        Err(Claimed::Forwarded) => {}
        other => panic!("expected Forwarded, got {other:?}"),
    }
    assert_eq!(rx.recv_timeout(WAIT).expect("forwarded args"), args);

    // When the owner goes away, the next launch becomes the owner.
    drop(first);
    let (tx2, rx2) = mpsc::channel();
    let tx2 = Mutex::new(tx2);
    let second = instance::claim(app_id, Vec::new(), move |args| {
        let _ = day_dbus_lock(&tx2).send(args);
    })
    .expect("next launch claims");
    match instance::claim(app_id, vec!["x".into()], |_| {}) {
        Err(Claimed::Forwarded) => {}
        other => panic!("expected Forwarded, got {other:?}"),
    }
    assert_eq!(
        rx2.recv_timeout(WAIT).expect("forwarded to the new owner"),
        vec!["x"]
    );
    drop(second);

    // An owner that vanishes mid-forward (it disconnects instead of answering) is the race the
    // retry covers: the launch claims the freed name instead of giving up.
    let racer = connect();
    let name = instance::bus_name("dev.daybrite.InstanceRace");
    racer
        .export(instance::PATH, |call| {
            call.connection.close();
            Ok(Vec::new())
        })
        .expect("export racer");
    assert_eq!(
        racer
            .request_name(&name, day_dbus::NAME_FLAG_DO_NOT_QUEUE)
            .expect("own"),
        NAME_REPLY_PRIMARY_OWNER
    );
    let claim = instance::claim("dev.daybrite.InstanceRace", vec!["y".into()], |_| {})
        .expect("the retry claims the vanished owner's name");
    assert_eq!(claim.name(), name);

    // A name held by something that is not a Day instance leaves this launch unclaimed.
    let squatter = connect();
    let squat = instance::bus_name("dev.daybrite.InstanceSquat");
    assert_eq!(
        squatter
            .request_name(&squat, day_dbus::NAME_FLAG_DO_NOT_QUEUE)
            .expect("own"),
        NAME_REPLY_PRIMARY_OWNER
    );
    match instance::claim("dev.daybrite.InstanceSquat", Vec::new(), |_| {}) {
        Err(Claimed::NoBus(Error::Remote { name, .. })) => {
            assert_eq!(name, "org.freedesktop.DBus.Error.UnknownObject")
        }
        other => panic!("expected NoBus(UnknownObject), got {other:?}"),
    }
}
