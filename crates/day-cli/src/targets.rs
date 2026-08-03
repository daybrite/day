//! Target definitions: `<os>-<toolkit>` pairs (DESIGN.md §1) and their build/launch shapes.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TargetKind {
    Desktop,
    IosSim,
    Android,
    /// HarmonyOS Next / ArkUI: a Rust cdylib (`libentry.so`) loaded by an ArkTS host and mounted
    /// into a NodeContent, packaged into a `.hap` (see apps/day-arkui-demo/platform/ohos). Cross-compiled
    /// with the OpenHarmony NDK (`OHOS_NDK_HOME`); packaged/signed/run via DevEco Studio or hvigor.
    HarmonyOs,
    /// web-dom (DESIGN.md §9, docs/web.md): a wasm32 cdylib plus the day-dom host page,
    /// assembled into a servable `dist/`; `day launch` serves it over loopback and opens
    /// the default browser.
    Web,
}

#[derive(Clone, Copy, Debug)]
pub struct Target {
    pub name: &'static str,
    pub toolkit: &'static str,
    pub kind: TargetKind,
    /// The platform key: the `platform/<os>/` scaffold dir, the `[app.<os>]` override table, and
    /// the per-platform namespace generally. NOT derivable from `name` — `harmony-arkui`'s
    /// platform key is `ohos` (the scaffold dir, signing table, and `day ohos` all predate the
    /// target's rename and keep the OS's own name). Deriving this by splitting the target name
    /// is what silently broke `day new`'s HarmonyOS scaffold when the target was renamed.
    pub os: &'static str,
    /// Host OS that can build this target.
    pub host: &'static str,
    /// Human-friendly label for pickers/menus (e.g. `day new`'s interactive target chooser).
    pub label: &'static str,
    /// Not yet production-ready — surfaced with an `[EXPERIMENTAL]` tag in menus.
    pub experimental: bool,
}

// Ordered for presentation (mobile first, then desktops grouped by OS, experimental last) — this is
// the order the `day new` interactive target menu shows. `find()` is by name and `Day.toml` defaults
// are string literals, so the order is purely cosmetic elsewhere.
pub const TARGETS: &[Target] = &[
    Target {
        name: "ios-uikit",
        toolkit: "uikit",
        kind: TargetKind::IosSim,
        os: "ios",
        host: "macos",
        label: "iOS",
        experimental: false,
    },
    Target {
        name: "android-mdc",
        toolkit: "mdc",
        kind: TargetKind::Android,
        os: "android",
        host: "any",
        label: "Android",
        experimental: false,
    },
    Target {
        name: "macos-appkit",
        toolkit: "appkit",
        kind: TargetKind::Desktop,
        os: "macos",
        host: "macos",
        label: "macOS (AppKit)",
        experimental: false,
    },
    Target {
        name: "macos-gtk",
        toolkit: "gtk",
        kind: TargetKind::Desktop,
        os: "macos",
        host: "macos",
        label: "macOS (GTK)",
        experimental: false,
    },
    Target {
        name: "macos-qt",
        toolkit: "qt",
        kind: TargetKind::Desktop,
        os: "macos",
        host: "macos",
        label: "macOS (Qt)",
        experimental: false,
    },
    Target {
        name: "linux-gtk",
        toolkit: "gtk",
        kind: TargetKind::Desktop,
        os: "linux",
        host: "linux",
        label: "Linux (GTK)",
        experimental: false,
    },
    Target {
        name: "linux-qt",
        toolkit: "qt",
        kind: TargetKind::Desktop,
        os: "linux",
        host: "linux",
        label: "Linux (Qt)",
        experimental: false,
    },
    Target {
        name: "windows-xaml",
        toolkit: "xaml",
        kind: TargetKind::Desktop,
        os: "windows",
        host: "windows",
        label: "Windows (XAML)",
        experimental: false,
    },
    Target {
        name: "windows-qt",
        toolkit: "qt",
        kind: TargetKind::Desktop,
        os: "windows",
        host: "windows",
        label: "Windows (Qt)",
        experimental: false,
    },
    Target {
        name: "windows-gtk",
        toolkit: "gtk",
        kind: TargetKind::Desktop,
        os: "windows",
        host: "windows",
        label: "Windows (GTK)",
        experimental: false,
    },
    Target {
        name: "harmony-arkui",
        toolkit: "arkui",
        kind: TargetKind::HarmonyOs,
        os: "ohos",
        host: "any",
        label: "OpenHarmony ArkUI",
        experimental: false,
    },
    Target {
        name: "web-dom",
        toolkit: "dom",
        kind: TargetKind::Web,
        os: "web",
        host: "any",
        label: "Web (DOM)",
        experimental: true,
    },
];

pub fn find(name: &str) -> Option<&'static Target> {
    TARGETS.iter().find(|t| t.name == name)
}

pub fn host_os() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "other"
    }
}

/// The default target for the current host — the sensible preselection for `day new app`'s target
/// menu, the fallback when a non-interactive `day new app` gets no `--toolkit`, and what
/// `day launch`/`day build` run when given no `-p`.
///
/// Each OS has one obvious native answer except Linux, where the toolkit follows the DESKTOP the
/// user is actually running: a Qt desktop gets `linux-qt`, everything else `linux-gtk`. Getting
/// this wrong is not cosmetic — a GTK build under Plasma (or vice versa) is the one that looks
/// foreign, which is the whole thing Day exists to avoid.
pub fn host_default() -> &'static str {
    match host_os() {
        "linux" => linux_default_desktop(),
        "windows" => "windows-xaml",
        _ => "macos-appkit",
    }
}

/// Qt or GTK for the running Linux desktop.
///
/// `XDG_CURRENT_DESKTOP` is the freedesktop-specified answer and is colon-separated for
/// derivatives (`ubuntu:GNOME`), so every component is checked; `DESKTOP_SESSION` is the older
/// fallback still set by some display managers. Unknown or unset means a plain GTK build, which is
/// the safer default: GTK is present on more Linux systems than Qt, and a headless/CI shell has no
/// desktop to match anyway.
fn linux_default_desktop() -> &'static str {
    desktop_toolkit(&[
        std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default(),
        std::env::var("XDG_SESSION_DESKTOP").unwrap_or_default(),
        std::env::var("DESKTOP_SESSION").unwrap_or_default(),
    ])
}

/// The toolkit for a set of desktop-identifying strings — pure, so the mapping is testable
/// without mutating the process environment.
fn desktop_toolkit(values: &[String]) -> &'static str {
    const QT_DESKTOPS: [&str; 6] = ["kde", "plasma", "lxqt", "deepin", "razor", "trinity"];
    for value in values {
        for part in value.split(':') {
            let part = part.trim().to_ascii_lowercase();
            if QT_DESKTOPS.iter().any(|d| part.contains(d)) {
                return "linux-qt";
            }
        }
    }
    "linux-gtk"
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every default names a real target — a typo here would only surface as a launch failure on
    /// the one OS that hits that arm.
    /// The desktops a Linux user actually runs, as their session variables report them.
    #[test]
    fn linux_desktops_map_to_their_toolkit() {
        let v = |s: &str| vec![s.to_string()];
        for qt in ["KDE", "plasma", "KDE:plasma", "LXQt", "Deepin"] {
            assert_eq!(desktop_toolkit(&v(qt)), "linux-qt", "{qt}");
        }
        for gtk in [
            "GNOME",
            "ubuntu:GNOME",
            "XFCE",
            "MATE",
            "Cinnamon",
            "sway",
            "",
        ] {
            assert_eq!(desktop_toolkit(&v(gtk)), "linux-gtk", "{gtk}");
        }
        // The first variable may be empty while a later one names the desktop.
        assert_eq!(
            desktop_toolkit(&["".into(), "".into(), "plasmawayland".into()]),
            "linux-qt"
        );
        // Nothing set at all (a CI shell, a bare TTY) is GTK, not a panic.
        assert_eq!(desktop_toolkit(&[]), "linux-gtk");
    }

    #[test]
    fn host_defaults_name_real_targets() {
        for name in ["macos-appkit", "windows-xaml", "linux-gtk", "linux-qt"] {
            assert!(find(name).is_some(), "{name} is not in the target table");
        }
        assert!(find(host_default()).is_some());
    }
}
