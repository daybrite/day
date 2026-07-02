//! day doctor — per-target toolchain diagnosis (DESIGN.md §16.5, v0).

use std::process::Command;

fn have(cmd: &str, args: &[&str]) -> Option<String> {
    Command::new(cmd).args(args).output().ok().and_then(|o| {
        if o.status.success() {
            Some(
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .next()
                    .unwrap_or("")
                    .to_string(),
            )
        } else {
            None
        }
    })
}

pub fn run() -> i32 {
    let mut failures = 0;
    let mut check = |name: &str, detail: Option<String>, fix: &str| match detail {
        Some(d) => eprintln!("\x1b[32m✓\x1b[0m {name:<14} {d}"),
        None => {
            eprintln!("\x1b[31m✗\x1b[0m {name:<14} missing — {fix}");
            failures += 1;
        }
    };

    check(
        "rust",
        have("cargo", &["--version"]),
        "install rust (brew install rust or rustup)",
    );
    check(
        "rustup-ios",
        have(
            "bash",
            &[
                "-c",
                "ls ~/.rustup/toolchains/*/lib/rustlib | grep -m1 aarch64-apple-ios-sim",
            ],
        ),
        "rustup target add aarch64-apple-ios-sim",
    );
    check(
        "rustup-android",
        have(
            "bash",
            &[
                "-c",
                "ls ~/.rustup/toolchains/*/lib/rustlib | grep -m1 aarch64-linux-android",
            ],
        ),
        "rustup target add aarch64-linux-android",
    );
    check(
        "cargo-ndk",
        have("cargo", &["ndk", "--version"]),
        "cargo install cargo-ndk",
    );
    check(
        "gtk4",
        have("pkg-config", &["--modversion", "gtk4"]),
        "brew install gtk4",
    );
    check(
        "qt6",
        have("pkg-config", &["--modversion", "Qt6Widgets"]),
        "brew install qt",
    );
    check("xcode", have("xcodebuild", &["-version"]), "install Xcode");
    check(
        "simulator",
        have(
            "bash",
            &["-c", "xcrun simctl list devices booted | grep -m1 Booted"],
        ),
        "boot a simulator: xcrun simctl boot <device>",
    );
    check(
        "emulator",
        have("bash", &["-c", "adb devices | grep -m1 -w device"]),
        "start an Android emulator",
    );
    check(
        "jdk21",
        have("/opt/homebrew/opt/openjdk@21/bin/java", &["--version"]),
        "brew install openjdk@21 (JDK 26 breaks AGP)",
    );
    if failures == 0 { 0 } else { 3 }
}
