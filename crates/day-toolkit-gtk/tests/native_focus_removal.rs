// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Run on a desktop with G_DEBUG=fatal-criticals. Exercises the Win32 IME lifetime through
//! Day's actual remove/release paths, including a focused descendant and an unrelated input.
use day_spec::Toolkit;
use day_toolkit_gtk::Gtk;
use gtk4::prelude::*;
use std::time::{Duration, Instant};

fn pump() {
    let until = Instant::now() + Duration::from_millis(60);
    let context = gtk4::glib::MainContext::default();
    while Instant::now() < until {
        while context.pending() {
            context.iteration(false);
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn main() {
    gtk4::init().expect("native focus removal test needs a display");
    let mut toolkit = Gtk::new();
    let window = gtk4::Window::new();
    let host = gtk4::Fixed::new();
    let survivor = gtk4::Entry::new();
    host.put(&survivor, 0.0, 120.0);
    window.set_default_size(400, 240);
    window.set_child(Some(&host));
    window.present();
    pump();

    for cycle in 0..12 {
        let page = gtk4::Fixed::new();
        let input: gtk4::Widget = match cycle % 3 {
            0 => gtk4::Entry::new().upcast(),
            1 => gtk4::SpinButton::with_range(0.0, 10.0, 1.0).upcast(),
            _ => gtk4::TextView::new().upcast(),
        };
        input.set_size_request(240, 80);
        page.put(&input, 0.0, 0.0);
        host.put(&page, 0.0, 0.0);
        pump();
        assert!(input.grab_focus());
        pump();
        let focus = GtkWindowExt::focus(&window).expect("input must hold native focus");
        assert!(focus == input || focus.is_ancestor(&input));
        drop(focus);
        // Alternate removing the containing subtree and releasing the focused leaf itself.
        if cycle % 2 == 0 {
            toolkit.remove(host.upcast_ref(), page.upcast_ref());
        } else {
            toolkit.release(input.clone());
        }
        if cfg!(target_os = "windows") {
            assert!(
                GtkWindowExt::focus(&window).is_none(),
                "focus must be resigned synchronously before detaching the input"
            );
        }
        drop(input);
        toolkit.release(page.upcast());
        pump(); // Dispatch Win32 messages after disposing the old IME.

        assert!(survivor.grab_focus());
        pump();
        let unrelated = gtk4::Entry::new();
        host.put(&unrelated, 0.0, 0.0);
        toolkit.remove(host.upcast_ref(), unrelated.upcast_ref());
        toolkit.release(unrelated.upcast());
        let focus = GtkWindowExt::focus(&window).expect("another subtree keeps its focus");
        assert!(focus == survivor || focus.is_ancestor(&survivor));
    }
    GtkWindowExt::set_focus(&window, None::<&gtk4::Widget>);
    window.destroy();
    pump();
    println!("focused entry/spin/textview removal, disposal, and sibling focus passed");
}
