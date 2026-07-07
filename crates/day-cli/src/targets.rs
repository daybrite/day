//! Target definitions: `<os>-<toolkit>` pairs (DESIGN.md §1) and their build/launch shapes.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TargetKind {
    Desktop,
    IosSim,
    Android,
    /// HarmonyOS Next / ArkUI: a Rust cdylib (`libentry.so`) loaded by an ArkTS host and mounted
    /// into a NodeContent, packaged into a `.hap` (see apps/day-arkui-demo/harmony). Cross-compiled
    /// with the OpenHarmony NDK (`OHOS_NDK_HOME`); packaged/signed/run via DevEco Studio or hvigor.
    HarmonyOs,
}

#[derive(Clone, Copy, Debug)]
pub struct Target {
    pub name: &'static str,
    pub toolkit: &'static str,
    pub kind: TargetKind,
    /// Host OS that can build this target.
    pub host: &'static str,
    /// Human-friendly label for pickers/menus (e.g. `day new`'s interactive target chooser).
    pub label: &'static str,
    /// Not yet production-ready — surfaced with an `[EXPERIMENTAL]` tag in menus.
    pub experimental: bool,
}

// Ordered for presentation (mobile first, then desktops grouped by OS, experimental last) — this is
// the order the `day new` interactive target menu shows. `find()` is by name and `day.yaml` defaults
// are string literals, so the order is purely cosmetic elsewhere.
pub const TARGETS: &[Target] = &[
    Target {
        name: "ios-uikit",
        toolkit: "uikit",
        kind: TargetKind::IosSim,
        host: "macos",
        label: "iOS",
        experimental: false,
    },
    Target {
        name: "android-widget",
        toolkit: "widget",
        kind: TargetKind::Android,
        host: "any",
        label: "Android",
        experimental: false,
    },
    Target {
        name: "macos-appkit",
        toolkit: "appkit",
        kind: TargetKind::Desktop,
        host: "macos",
        label: "macOS (AppKit)",
        experimental: false,
    },
    Target {
        name: "macos-gtk",
        toolkit: "gtk",
        kind: TargetKind::Desktop,
        host: "macos",
        label: "macOS (GTK)",
        experimental: false,
    },
    Target {
        name: "macos-qt",
        toolkit: "qt",
        kind: TargetKind::Desktop,
        host: "macos",
        label: "macOS (Qt)",
        experimental: false,
    },
    Target {
        name: "linux-gtk",
        toolkit: "gtk",
        kind: TargetKind::Desktop,
        host: "linux",
        label: "Linux (GTK)",
        experimental: false,
    },
    Target {
        name: "linux-qt",
        toolkit: "qt",
        kind: TargetKind::Desktop,
        host: "linux",
        label: "Linux (Qt)",
        experimental: false,
    },
    Target {
        name: "windows-winui",
        toolkit: "winui",
        kind: TargetKind::Desktop,
        host: "windows",
        label: "Windows (WinUI)",
        experimental: false,
    },
    Target {
        name: "windows-qt",
        toolkit: "qt",
        kind: TargetKind::Desktop,
        host: "windows",
        label: "Windows (Qt)",
        experimental: false,
    },
    Target {
        name: "windows-gtk",
        toolkit: "gtk",
        kind: TargetKind::Desktop,
        host: "windows",
        label: "Windows (GTK)",
        experimental: false,
    },
    Target {
        name: "ohos-arkui",
        toolkit: "arkui",
        kind: TargetKind::HarmonyOs,
        host: "any",
        label: "OpenHarmony ArkUI",
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
/// menu and the fallback when a non-interactive `day new app` gets no `--toolkit`.
pub fn host_default() -> &'static str {
    match host_os() {
        "linux" => "linux-gtk",
        "windows" => "windows-winui",
        _ => "macos-appkit",
    }
}
