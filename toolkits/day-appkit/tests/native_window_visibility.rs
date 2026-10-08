// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

#[cfg(not(target_os = "macos"))]
fn main() {}

/// Synthetic windows: verify the actual launch ordering, including the run-loop turn after
/// `ready`, where an unconditional order-front used to override an app's hidden startup.
#[cfg(target_os = "macos")]
fn main() {
    use day_appkit::{AppKit, Handle};
    use day_spec::{
        NodeId, Platform, Toolkit, WindowChange, WindowKind, WindowOpenReply, WindowOptions,
    };
    use std::cell::RefCell;

    thread_local! {
        static RUNNING: RefCell<Option<(AppKit, Handle)>> = const { RefCell::new(None) };
    }

    // A missing main-loop callback must fail rather than leave a native test running forever.
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_secs(15));
        eprintln!("native window visibility timed out");
        std::process::exit(2);
    });

    AppKit::new().run(
        WindowOptions {
            title: "Fixture hidden host".into(),
            start_hidden: true,
            ..Default::default()
        },
        Box::new(|toolkit, host, _| {
            assert!(!host.window().unwrap().isVisible());
            RUNNING.with(|running| *running.borrow_mut() = Some((toolkit, host)));
            AppKit::post(Box::new(|| {
                let result = std::panic::catch_unwind(|| {
                    RUNNING.with(|running| {
                        let mut running = running.borrow_mut();
                        let (toolkit, host) = running.as_mut().unwrap();
                        let window = host.window().unwrap();
                        assert!(
                            !window.isVisible(),
                            "launch must not reveal the hidden host"
                        );
                        toolkit.apply_window(host, &WindowChange::Visible(true));
                        assert!(window.isVisible(), "explicit show still works");
                        toolkit.apply_window(host, &WindowChange::Visible(false));
                        assert!(!window.isVisible());

                        for hidden in [false, true] {
                            let options = WindowOptions {
                                title: "Fixture preferences".into(),
                                start_hidden: hidden,
                                ..Default::default()
                            };
                            let WindowOpenReply::Open(prefs) =
                                toolkit.open_window(NodeId(41), &options, WindowKind::Preferences)
                            else {
                                panic!("AppKit must open a native preferences window")
                            };
                            let window = prefs.window().unwrap();
                            assert_eq!(window.isVisible(), !hidden);
                            toolkit.focus_window(&prefs);
                            assert!(window.isVisible(), "on-demand preferences can be revealed");
                            toolkit.close_window(&prefs);
                            assert!(!window.isVisible());
                        }
                    });
                });
                if result.is_ok() {
                    println!(
                        "native hidden launch, explicit show, and preferences visibility passed"
                    );
                }
                std::process::exit(if result.is_ok() { 0 } else { 1 });
            }));
        }),
    );
}
