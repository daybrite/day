//! Target definitions: `<os>-<toolkit>` pairs (DESIGN.md §1) and their build/launch shapes.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TargetKind {
    Desktop,
    IosSim,
    Android,
}

#[derive(Clone, Copy, Debug)]
pub struct Target {
    pub name: &'static str,
    pub toolkit: &'static str,
    pub kind: TargetKind,
    /// Host OS that can build this target.
    pub host: &'static str,
}

pub const TARGETS: &[Target] = &[
    Target {
        name: "macos-appkit",
        toolkit: "appkit",
        kind: TargetKind::Desktop,
        host: "macos",
    },
    Target {
        name: "macos-gtk",
        toolkit: "gtk",
        kind: TargetKind::Desktop,
        host: "macos",
    },
    Target {
        name: "macos-qt",
        toolkit: "qt",
        kind: TargetKind::Desktop,
        host: "macos",
    },
    Target {
        name: "linux-gtk",
        toolkit: "gtk",
        kind: TargetKind::Desktop,
        host: "linux",
    },
    Target {
        name: "windows-winui",
        toolkit: "winui",
        kind: TargetKind::Desktop,
        host: "windows",
    },
    Target {
        name: "linux-qt",
        toolkit: "qt",
        kind: TargetKind::Desktop,
        host: "linux",
    },
    Target {
        name: "ios-uikit",
        toolkit: "uikit",
        kind: TargetKind::IosSim,
        host: "macos",
    },
    Target {
        name: "android-widget",
        toolkit: "widget",
        kind: TargetKind::Android,
        host: "any",
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
