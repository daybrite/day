// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

#[cfg(not(target_os = "macos"))]
fn main() {}

// Synthetic menu fixtures. Run on the process main thread, including startup's default menu.
#[cfg(target_os = "macos")]
fn main() {
    use day_appkit::AppKit;
    use day_spec::{
        MenuBarRole, MenuItem as M, MenuRole as R, Platform, Shortcut, Toolkit, WindowOptions,
    };
    use objc2::{MainThreadMarker, sel};
    use objc2_app_kit::{NSApplication, NSEventModifierFlags as Flags, NSMenu};

    fn inspect(app: &NSApplication, menu: &NSMenu) {
        let application = menu.itemAtIndex(0).unwrap().submenu().unwrap();
        for (selector, key, modifiers) in [
            (sel!(hide:), "h", Flags::Command),
            (
                sel!(hideOtherApplications:),
                "h",
                Flags::Command | Flags::Option,
            ),
            (sel!(unhideAllApplications:), "", Flags::Command),
        ] {
            let matching: Vec<_> = application
                .itemArray()
                .into_iter()
                .filter(|item| item.action() == Some(selector))
                .collect();
            assert_eq!(matching.len(), 1);
            let item = &matching[0];
            assert_eq!(item.keyEquivalent().to_string(), key);
            assert_eq!(item.keyEquivalentModifierMask(), modifiers);
            let target = item.target().unwrap();
            let application: &objc2::runtime::AnyObject = app;
            assert!(std::ptr::eq(&*target, application));
            assert!(!item.title().is_empty());
        }
        let services = app.servicesMenu().expect("registered native Services menu");
        assert!(
            application
                .itemArray()
                .into_iter()
                .any(|item| item.submenu().as_ref() == Some(&services))
        );
        for menu in [menu, &application] {
            let items = menu.itemArray();
            assert!(!items.firstObject().unwrap().isSeparatorItem());
            assert!(!items.lastObject().unwrap().isSeparatorItem());
            let mut separator = false;
            for item in items {
                assert!(!(separator && item.isSeparatorItem()));
                separator = item.isSeparatorItem();
            }
        }
    }

    fn command(role: Option<R>, action: u64) -> M {
        M::Action {
            id: None,
            action,
            label: "Fixture command".into(),
            shortcut: None,
            enabled: true,
            checked: None,
            role,
            icon: None,
        }
    }

    AppKit::new().run(
        WindowOptions {
            title: "Fixture menu app".into(),
            start_hidden: true,
            ..Default::default()
        },
        Box::new(|mut toolkit, _, _| {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let app = NSApplication::sharedApplication(MainThreadMarker::new().unwrap());
                let default_menu = app
                    .mainMenu()
                    .unwrap()
                    .itemAtIndex(0)
                    .unwrap()
                    .submenu()
                    .unwrap();
                let quit = default_menu
                    .itemArray()
                    .into_iter()
                    .find(|item| item.action() == Some(sel!(terminate:)))
                    .unwrap();
                assert_eq!(quit.keyEquivalent().to_string(), "q");
                assert_eq!(quit.keyEquivalentModifierMask(), Flags::Command);
                assert!(quit.target().is_none());
                inspect(&app, &app.mainMenu().unwrap());
                for locale in ["en", "fr"] {
                    day_l10n::locale().set(locale.into());
                    let mut quit = command(Some(R::Quit), 42);
                    if let M::Action {
                        shortcut, enabled, ..
                    } = &mut quit
                    {
                        *shortcut = Some(Shortcut::new("q").shift());
                        *enabled = false;
                    }
                    toolkit.set_app_menu(&[
                        M::Submenu {
                            label: "Fixture app roles".into(),
                            role: None,
                            items: vec![
                                M::Separator,
                                M::Submenu {
                                    label: "Fixture nested roles".into(),
                                    role: None,
                                    items: vec![
                                        command(Some(R::About), 41),
                                        M::Separator,
                                        command(Some(R::Preferences), 43),
                                        M::Separator,
                                        quit,
                                    ],
                                },
                                M::Separator,
                                command(Some(R::Quit), 99),
                            ],
                        },
                        M::Submenu {
                            label: "Fixture File".into(),
                            role: Some(MenuBarRole::File),
                            items: vec![
                                M::Separator,
                                command(None, 44),
                                command(Some(R::CloseWindow), 0),
                                M::Separator,
                                command(Some(R::Quit), 100),
                                M::Separator,
                            ],
                        },
                    ]);
                    let menu = app.mainMenu().unwrap();
                    inspect(&app, &menu);
                    assert!(
                        !menu
                            .itemArray()
                            .into_iter()
                            .any(|item| item.title().to_string() == "Fixture app roles")
                    );
                    let application = menu.itemAtIndex(0).unwrap().submenu().unwrap();
                    let tags: Vec<_> = application
                        .itemArray()
                        .into_iter()
                        .map(|item| item.tag())
                        .filter(|id| *id != 0)
                        .collect();
                    assert_eq!(tags, [41, 43, 42]);
                    let quit = application
                        .itemArray()
                        .into_iter()
                        .find(|item| item.tag() == 42)
                        .unwrap();
                    assert_eq!(quit.action(), Some(sel!(fire:)));
                    assert_eq!(
                        quit.keyEquivalentModifierMask(),
                        Flags::Command | Flags::Shift
                    );
                    assert!(!quit.isEnabled());
                    let hide = application
                        .itemArray()
                        .into_iter()
                        .find(|item| item.action() == Some(sel!(hide:)))
                        .unwrap();
                    assert!(hide.title().to_string().starts_with(if locale == "fr" {
                        "Masquer "
                    } else {
                        "Hide "
                    }));
                    let file = menu
                        .itemArray()
                        .into_iter()
                        .find(|item| item.title().to_string() == "Fixture File")
                        .unwrap()
                        .submenu()
                        .unwrap();
                    assert_eq!(file.numberOfItems(), 2);
                    let close = file.itemAtIndex(1).unwrap();
                    assert_eq!(close.action(), Some(sel!(performClose:)));
                    assert_eq!(close.keyEquivalent().to_string(), "w");
                    assert_eq!(close.keyEquivalentModifierMask(), Flags::Command);
                    assert!(close.target().is_none());
                    assert_eq!(file.itemAtIndex(0).unwrap().tag(), 44);
                }
                println!("native default and rebuilt application menus passed");
            }));
            std::process::exit(if result.is_ok() { 0 } else { 1 });
        }),
    );
}
