//! The sample miniapps, embedded in the superapp binary and materialized to disk on first
//! launch (docs/lite.md §12). Materialized packages install through the NORMAL local-origin
//! path — the bundled catalog is a real catalog whose origins happen to live on disk, so
//! the install/disclosure/update flow exercised here is exactly the remote one.

use std::path::{Path, PathBuf};

pub struct BundledApp {
    pub id: &'static str,
    pub files: &'static [(&'static str, &'static str)],
}

pub const BUNDLED: &[BundledApp] = &[
    BundledApp {
        id: "dev.daybrite.lite.weather",
        files: &[
            (
                "manifest.json",
                include_str!("../miniapps/weather/manifest.json"),
            ),
            ("app.ts", include_str!("../miniapps/weather/app.ts")),
            ("icon.svg", include_str!("../miniapps/weather/icon.svg")),
            (
                "i18n/en.ftl",
                include_str!("../miniapps/weather/i18n/en.ftl"),
            ),
            (
                "i18n/fr.ftl",
                include_str!("../miniapps/weather/i18n/fr.ftl"),
            ),
            (
                "i18n/ar.ftl",
                include_str!("../miniapps/weather/i18n/ar.ftl"),
            ),
            (
                "i18n/zh-CN.ftl",
                include_str!("../miniapps/weather/i18n/zh-CN.ftl"),
            ),
        ],
    },
    BundledApp {
        id: "dev.daybrite.lite.todo",
        files: &[
            (
                "manifest.json",
                include_str!("../miniapps/todo/manifest.json"),
            ),
            ("app.ts", include_str!("../miniapps/todo/app.ts")),
            ("icon.svg", include_str!("../miniapps/todo/icon.svg")),
            ("i18n/en.ftl", include_str!("../miniapps/todo/i18n/en.ftl")),
            ("i18n/fr.ftl", include_str!("../miniapps/todo/i18n/fr.ftl")),
            ("i18n/ar.ftl", include_str!("../miniapps/todo/i18n/ar.ftl")),
            (
                "i18n/zh-CN.ftl",
                include_str!("../miniapps/todo/i18n/zh-CN.ftl"),
            ),
        ],
    },
    BundledApp {
        id: "dev.daybrite.lite.tictactoe",
        files: &[
            (
                "manifest.json",
                include_str!("../miniapps/tictactoe/manifest.json"),
            ),
            ("app.ts", include_str!("../miniapps/tictactoe/app.ts")),
            ("icon.svg", include_str!("../miniapps/tictactoe/icon.svg")),
            (
                "i18n/en.ftl",
                include_str!("../miniapps/tictactoe/i18n/en.ftl"),
            ),
            (
                "i18n/fr.ftl",
                include_str!("../miniapps/tictactoe/i18n/fr.ftl"),
            ),
            (
                "i18n/ar.ftl",
                include_str!("../miniapps/tictactoe/i18n/ar.ftl"),
            ),
            (
                "i18n/zh-CN.ftl",
                include_str!("../miniapps/tictactoe/i18n/zh-CN.ftl"),
            ),
        ],
    },
];

/// Write every bundled package under `root/bundled/<id>/`, returning each id's origin path.
/// Files are rewritten every launch so a superapp update refreshes its samples.
pub fn materialize(root: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    for app in BUNDLED {
        let dir = root.join("bundled").join(app.id);
        let mut ok = true;
        for (rel, content) in app.files {
            let dest = dir.join(rel);
            if let Some(parent) = dest.parent()
                && std::fs::create_dir_all(parent).is_err()
            {
                ok = false;
                break;
            }
            if std::fs::write(&dest, content).is_err() {
                ok = false;
                break;
            }
        }
        if ok {
            out.push((app.id.to_string(), dir));
        }
    }
    out
}
