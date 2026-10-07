// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Remembered window frames (docs/windows.md "Remembered frames"): `WindowOptions::
//! remember_frame` keys a window's last size, position and maximized state, kept in one small
//! file in the app's configuration directory.
//!
//! The file is `day-windows.txt` under `$XDG_CONFIG_HOME/<app id>/` when that variable is set (on
//! every platform), else the platform's own configuration directory: `~/.config` on Linux,
//! `~/Library/Application Support` on macOS, `%APPDATA%` on Windows. One line per key:
//! `key<TAB>x<TAB>y<TAB>width<TAB>height<TAB>maximized`.

use std::path::PathBuf;

use day_spec::Rect;

/// A remembered window: its outer frame and whether it was maximized.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Remembered {
    pub frame: Rect,
    pub maximized: bool,
}

fn app_id() -> Option<String> {
    std::env::var("DAY_APP_ID")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| option_env!("DAY_APP_ID").map(str::to_owned))
}

fn config_root() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    if cfg!(target_os = "windows") {
        return std::env::var_os("APPDATA").map(PathBuf::from);
    }
    let home = PathBuf::from(std::env::var_os("HOME")?);
    if cfg!(target_os = "macos") {
        Some(home.join("Library/Application Support"))
    } else {
        Some(home.join(".config"))
    }
}

fn store() -> Option<PathBuf> {
    Some(config_root()?.join(app_id()?).join("day-windows.txt"))
}

fn parse(line: &str) -> Option<(&str, Remembered)> {
    let mut f = line.split('\t');
    let key = f.next()?;
    let mut num = || f.next()?.parse::<f64>().ok();
    let (x, y, w, h) = (num()?, num()?, num()?, num()?);
    let maximized = f.next()? == "1";
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    Some((
        key,
        Remembered {
            frame: Rect::new(x, y, w, h),
            maximized,
        },
    ))
}

fn read_all() -> Vec<String> {
    store()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|s| s.lines().map(str::to_owned).collect())
        .unwrap_or_default()
}

/// The frame remembered under `key`, if any.
pub fn load(key: &str) -> Option<Remembered> {
    read_all()
        .iter()
        .find_map(|l| parse(l).filter(|(k, _)| *k == key).map(|(_, r)| r))
}

/// Remember `frame` under `key`. Best effort: a read-only or missing configuration directory
/// costs the app its remembered frame, never an error.
pub fn save(key: &str, r: Remembered) {
    let Some(path) = store() else { return };
    let mut lines: Vec<String> = read_all()
        .into_iter()
        .filter(|l| parse(l).is_none_or(|(k, _)| k != key))
        .collect();
    lines.push(format!(
        "{key}\t{}\t{}\t{}\t{}\t{}",
        r.frame.origin.x,
        r.frame.origin.y,
        r.frame.size.width,
        r.frame.size.height,
        u8::from(r.maximized)
    ));
    if let Some(dir) = path.parent()
        && std::fs::create_dir_all(dir).is_err()
    {
        return;
    }
    if let Err(e) = std::fs::write(&path, lines.join("\n") + "\n") {
        log::debug!("remembered window frame not saved: {e}");
    }
}

/// Whether `frame` is usefully on one of `work_areas`: at least a 64×32-point patch of it,
/// enough to grab the window and drag it back. With no displays reported, every frame passes.
pub fn on_screen(frame: Rect, work_areas: &[Rect]) -> bool {
    work_areas.is_empty()
        || work_areas.iter().any(|a| {
            let w = (frame.origin.x + frame.size.width).min(a.origin.x + a.size.width)
                - frame.origin.x.max(a.origin.x);
            let h = (frame.origin.y + frame.size.height).min(a.origin.y + a.size.height)
                - frame.origin.y.max(a.origin.y);
            w >= 64.0 && h >= 32.0
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_round_trips_and_a_bad_one_is_skipped() {
        let (k, r) = parse("main\t10\t20\t800\t600\t1").unwrap();
        assert_eq!(k, "main");
        assert_eq!(r.frame.size, day_spec::Size::new(800.0, 600.0));
        assert!(r.maximized);
        assert!(parse("main\t10\t20\t0\t600\t0").is_none());
        assert!(parse("main\tx").is_none());
    }

    #[test]
    fn a_frame_off_every_display_is_not_on_screen() {
        let display = [Rect::new(0.0, 0.0, 1440.0, 900.0)];
        let inside = Rect::new(100.0, 100.0, 800.0, 600.0);
        let gone = Rect::new(3000.0, 100.0, 800.0, 600.0);
        let sliver = Rect::new(1420.0, 100.0, 800.0, 600.0);
        assert!(on_screen(inside, &display));
        assert!(!on_screen(gone, &display));
        assert!(!on_screen(sliver, &display));
        assert!(on_screen(gone, &[]));
    }
}
