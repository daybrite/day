// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
fn main() {
    use day_spec::{Platform, SystemColor, Toolkit, WindowOptions};
    use day_toolkit_appkit::AppKit;

    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_secs(60));
        eprintln!("native palette test timed out");
        std::process::exit(2);
    });
    AppKit::new().run(
        WindowOptions {
            title: "Palette fixture".into(),
            start_hidden: true,
            ..Default::default()
        },
        Box::new(|mut toolkit, _, _| {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut palettes = Vec::new();
                for dark in [false, true] {
                    toolkit.set_appearance(Some(dark));
                    let colors = SystemColor::ALL.map(|role| toolkit.system_color(role));
                    for color in colors {
                        for channel in [color.r, color.g, color.b, color.a] {
                            assert!(channel.is_finite() && (0.0..=1.0).contains(&channel));
                        }
                    }
                    let text = colors[SystemColor::Text as usize];
                    let background = colors[SystemColor::Background as usize];
                    assert_eq!(
                        text.r + text.g + text.b > background.r + background.g + background.b,
                        dark
                    );
                    palettes.push(colors);
                }
                assert_ne!(palettes[0], palettes[1]);
                toolkit.set_appearance(None);
            }));
            if result.is_ok() {
                println!("native palette resolves light and dark semantic colors");
            }
            std::process::exit(if result.is_ok() { 0 } else { 1 });
        }),
    );
}
