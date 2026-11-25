// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

#[cfg(not(target_os = "macos"))]
fn main() {}

/// Synthetic document windows must expose the full client area below AppKit's chrome,
/// including before their first resize and after the native tab strip is added.
#[cfg(target_os = "macos")]
fn main() {
    use day_spec::{
        NodeId, Platform, Toolkit, WindowKind, WindowOpenReply, WindowOptions, WindowTabbing,
    };
    use day_toolkit_appkit::{AppKit, Handle};
    use objc2_app_kit::{NSTextField, NSView};
    use objc2_foundation::NSRect;
    use std::cell::RefCell;

    thread_local! {
        static RUNNING: RefCell<Option<(AppKit, Vec<Handle>)>> = const { RefCell::new(None) };
    }
    fn assert_client_origin(host: &Handle) {
        let window = host.window().unwrap();
        let inset = (host.frame().size.height - window.contentLayoutRect().size.height).max(0.0);
        assert!(inset > 0.0, "fixture must have native window chrome");
        assert!(
            (host.bounds().origin.y + inset).abs() < 0.5,
            "client origin must clear native chrome: bounds={:?}, inset={inset}",
            host.bounds()
        );
    }
    fn toast_label_in_window(view: &NSView) -> Option<NSRect> {
        if let Some(label) = view.downcast_ref::<NSTextField>()
            && label.stringValue().to_string() == "Fixture status"
        {
            return Some(view.convertRect_toView(view.bounds(), None));
        }
        view.subviews()
            .iter()
            .find_map(|child| toast_label_in_window(&child))
    }
    fn assert_toast_in_client(host: &Handle) {
        host.layoutSubtreeIfNeeded();
        let rect = toast_label_in_window(host).expect("native toast label");
        let client = host.window().unwrap().contentLayoutRect();
        assert!(rect.size.width > 0.0 && rect.size.height > 0.0);
        assert!(
            objc2_foundation::NSContainsRect(client, rect),
            "toast text must be visible inside the client area: {rect:?} in {client:?}"
        );
    }
    std::thread::spawn(|| {
        // Foundation may scan a large cached Cargo deps directory during native startup.
        std::thread::sleep(std::time::Duration::from_secs(60));
        eprintln!("native document layout timed out");
        std::process::exit(2);
    });
    AppKit::new().run(
        WindowOptions {
            title: "Fixture hidden owner".into(),
            start_hidden: true,
            ..Default::default()
        },
        Box::new(|mut toolkit, _, _| {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let hosts: Vec<_> = (0..2)
                    .map(|i| {
                        let options = WindowOptions {
                            title: format!("Fixture document {i}"),
                            start_hidden: true,
                            tabbing: WindowTabbing::Group("day.fixture.document-layout".into()),
                            ..Default::default()
                        };
                        let WindowOpenReply::Open(host) =
                            toolkit.open_window(NodeId(41 + i), &options, WindowKind::Normal)
                        else {
                            panic!("AppKit must open a native document window");
                        };
                        assert_client_origin(&host);
                        toolkit.show_toast(&host, "Fixture status");
                        assert_toast_in_client(&host);
                        host
                    })
                    .collect();
                toolkit.group_windows(&hosts);
                for host in &hosts {
                    assert_client_origin(host);
                    assert_toast_in_client(host);
                }
                RUNNING.with(|r| *r.borrow_mut() = Some((toolkit, hosts)));
            }));
            if result.is_err() {
                std::process::exit(1);
            }
            AppKit::post(Box::new(|| {
                let result = std::panic::catch_unwind(|| {
                    RUNNING.with(|r| {
                        let mut running = r.borrow_mut();
                        let (toolkit, hosts) = running.as_mut().unwrap();
                        for host in hosts.iter() {
                            assert_client_origin(host);
                        }
                        for host in hosts.iter() {
                            toolkit.close_window(host);
                        }
                    });
                });
                if result.is_ok() {
                    println!("native document client origins clear title and tab chrome");
                }
                std::process::exit(i32::from(result.is_err()));
            }));
        }),
    );
}
