//! Bundled custom fonts (DESIGN §18.4).
//!
//! A project ships font files under a top-level `fonts/` directory. `day build` stages them into
//! each platform's native font location (Android `res/font/`, the iOS DayPieces bundle +
//! `UIAppFonts`, ArkUI rawfile, loose `Resources/fonts` on the desktops) and each backend
//! registers them at startup so `Font::Custom("Family", pt)` resolves by the font's **family
//! name** — the name baked into the file's `name` table (what Font Book / fontconfig report), not
//! its file name.
//!
//! This module is the single source of truth shared by the CLI (staging-time family extraction,
//! identifier naming) and the runtimes (startup registration, family → file resolution):
//!
//! * [`parse_font_names`] — a minimal sfnt `name`-table reader for `.ttf`/`.otf`/`.ttc`.
//! * [`font_dir`] / [`bundled_fonts`] — locate the staged font files at runtime (`DAY_FONT_ROOT`
//!   under `day launch`, bundle-relative `Resources/fonts` when packed).
//! * [`resolve_font_file`] — map a requested family name to one of the bundled files, for
//!   backends whose native API wants a file path rather than a registered family (WinUI).
//! * [`font_ident`] — the identifier both the Android/ArkUI stagers and their runtimes derive
//!   from a family name (`"Special Elite"` → `special_elite`), so lookup needs no side table.

use std::path::PathBuf;

/// Font file extensions accepted under `fonts/`.
pub const FONT_EXTS: [&str; 3] = ["ttf", "otf", "ttc"];

/// The names a font file reports for itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontNames {
    /// The (typographic) family name — name ID 16, falling back to 1. This is what
    /// `Font::Custom` matches on.
    pub family: String,
    /// The PostScript name (name ID 6), when present — some platform APIs resolve by it.
    pub postscript: Option<String>,
}

fn u16be(data: &[u8], off: usize) -> Option<u16> {
    Some(u16::from_be_bytes(data.get(off..off + 2)?.try_into().ok()?))
}

fn u32be(data: &[u8], off: usize) -> Option<u32> {
    Some(u32::from_be_bytes(data.get(off..off + 4)?.try_into().ok()?))
}

/// Read the family and PostScript names from a raw font file (`.ttf`, `.otf`, or the first face
/// of a `.ttc`). A tiny sfnt reader — only the table directory and the `name` table are touched,
/// so it is safe to run on untrusted bytes (every access is bounds-checked).
pub fn parse_font_names(data: &[u8]) -> Option<FontNames> {
    // Font collections ('ttcf') front an array of face offsets; take the first face.
    let base = if u32be(data, 0)? == u32::from_be_bytes(*b"ttcf") {
        u32be(data, 12)? as usize
    } else {
        0
    };
    let version = u32be(data, base)?;
    let known = [
        0x0001_0000,                  // TrueType
        u32::from_be_bytes(*b"OTTO"), // CFF OpenType
        u32::from_be_bytes(*b"true"), // legacy Apple TrueType
    ];
    if !known.contains(&version) {
        return None;
    }
    let num_tables = u16be(data, base + 4)? as usize;
    let mut name_table = None;
    for i in 0..num_tables {
        let rec = base + 12 + i * 16;
        if u32be(data, rec)? == u32::from_be_bytes(*b"name") {
            let off = u32be(data, rec + 8)? as usize;
            let len = u32be(data, rec + 12)? as usize;
            name_table = Some(data.get(off..off.checked_add(len)?)?);
            break;
        }
    }
    let name = name_table?;
    let count = u16be(name, 2)? as usize;
    let storage = u16be(name, 4)? as usize;

    // Highest-scoring candidate per name ID. Windows/Unicode records (UTF-16BE) are preferred
    // over Mac Roman; the typographic family (ID 16) over the legacy family (ID 1).
    let mut family16: Option<(u8, String)> = None;
    let mut family1: Option<(u8, String)> = None;
    let mut postscript: Option<(u8, String)> = None;
    for i in 0..count {
        let rec = 6 + i * 12;
        let platform = u16be(name, rec)?;
        let encoding = u16be(name, rec + 2)?;
        let language = u16be(name, rec + 4)?;
        let name_id = u16be(name, rec + 6)?;
        let len = u16be(name, rec + 8)? as usize;
        let off = u16be(name, rec + 10)? as usize;
        if !matches!(name_id, 1 | 6 | 16) {
            continue;
        }
        let bytes = match name.get(storage + off..storage + off + len) {
            Some(b) => b,
            None => continue,
        };
        let (decoded, score) = match platform {
            // Windows (3) and Unicode (0): UTF-16BE. American English first among Windows.
            3 | 0 => {
                let units: Vec<u16> = bytes
                    .chunks_exact(2)
                    .map(|c| u16::from_be_bytes([c[0], c[1]]))
                    .collect();
                let s = String::from_utf16_lossy(&units);
                let score = if platform == 3 && language == 0x0409 {
                    3
                } else if platform == 3 {
                    2
                } else {
                    1
                };
                (s, score)
            }
            // Macintosh Roman: treat as Latin-1 (ASCII in practice).
            1 if encoding == 0 => (bytes.iter().map(|&b| b as char).collect(), 0),
            _ => continue,
        };
        let decoded = decoded.trim().to_string();
        if decoded.is_empty() {
            continue;
        }
        let slot = match name_id {
            16 => &mut family16,
            1 => &mut family1,
            _ => &mut postscript,
        };
        if slot.as_ref().is_none_or(|(s, _)| score > *s) {
            *slot = Some((score, decoded));
        }
    }
    let family = family16.or(family1)?.1;
    Some(FontNames {
        family,
        postscript: postscript.map(|(_, s)| s),
    })
}

/// The identifier the Android / ArkUI stagers derive from a family name, and that their runtimes
/// re-derive to look the font up (Android `R.font.<ident>`): lowercase, `[a-z0-9_]` only, with a
/// leading letter enforced. `"Special Elite"` → `special_elite`.
pub fn font_ident(family: &str) -> String {
    let mut s: String = family
        .chars()
        .map(|c| {
            let c = c.to_ascii_lowercase();
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if !s.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
        s.insert(0, 'r');
    }
    s
}

/// The directory holding the app's bundled font files at runtime: `DAY_FONT_ROOT` (the project's
/// `fonts/` under `day launch`), then bundle-relative locations next to the executable (macOS
/// `.app` `Resources/fonts`, files staged next to a desktop binary).
pub fn font_dir() -> Option<PathBuf> {
    if let Ok(root) = std::env::var("DAY_FONT_ROOT") {
        let p = PathBuf::from(root);
        if p.is_dir() {
            return Some(p);
        }
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        for rel in ["../Resources/fonts", "Resources/fonts", "fonts"] {
            let p = dir.join(rel);
            if p.is_dir() {
                return Some(p);
            }
        }
    }
    None
}

/// Every bundled font file ([`font_dir`] filtered to [`FONT_EXTS`]), sorted by file name. The
/// list backends iterate to register the app's fonts at startup.
pub fn bundled_fonts() -> Vec<PathBuf> {
    let Some(dir) = font_dir() else {
        return Vec::new();
    };
    font_files_in(&dir)
}

/// The font files directly under `dir`, sorted by file name (shared with the CLI's scanner).
pub fn font_files_in(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for e in entries.flatten() {
        let p = e.path();
        let ext = p
            .extension()
            .and_then(|x| x.to_str())
            .map(|x| x.to_ascii_lowercase());
        if p.is_file() && ext.is_some_and(|x| FONT_EXTS.contains(&x.as_str())) {
            out.push(p);
        }
    }
    out.sort();
    out
}

/// Resolve a requested family name to the bundled font file that provides it (case-insensitive,
/// PostScript name accepted too). For backends whose native font API takes a file path rather
/// than a registered family name.
pub fn resolve_font_file(family: &str) -> Option<PathBuf> {
    for path in bundled_fonts() {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        if let Some(names) = parse_font_names(&bytes)
            && (names.family.eq_ignore_ascii_case(family)
                || names
                    .postscript
                    .as_deref()
                    .is_some_and(|ps| ps.eq_ignore_ascii_case(family)))
        {
            return Some(path);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal single-table sfnt whose `name` table carries the given records
    /// (platform, encoding, language, name_id, utf16: bool, value).
    fn sfnt_with_names(records: &[(u16, u16, u16, u16, bool, &str)]) -> Vec<u8> {
        let mut strings: Vec<u8> = Vec::new();
        let mut recs: Vec<u8> = Vec::new();
        for &(plat, enc, lang, id, utf16, value) in records {
            let start = strings.len() as u16;
            if utf16 {
                for u in value.encode_utf16() {
                    strings.extend_from_slice(&u.to_be_bytes());
                }
            } else {
                strings.extend(value.bytes());
            }
            let len = strings.len() as u16 - start;
            for v in [plat, enc, lang, id, len, start] {
                recs.extend_from_slice(&v.to_be_bytes());
            }
        }
        let mut name = Vec::new();
        name.extend_from_slice(&0u16.to_be_bytes()); // format
        name.extend_from_slice(&(records.len() as u16).to_be_bytes());
        name.extend_from_slice(&((6 + recs.len()) as u16).to_be_bytes()); // storage offset
        name.extend(recs);
        name.extend(strings);

        let mut font = Vec::new();
        font.extend_from_slice(&0x0001_0000u32.to_be_bytes()); // sfnt version
        font.extend_from_slice(&1u16.to_be_bytes()); // numTables
        font.extend_from_slice(&[0; 6]); // searchRange etc (unused)
        let name_off = 12 + 16; // directory follows header, table follows directory
        font.extend_from_slice(b"name");
        font.extend_from_slice(&0u32.to_be_bytes()); // checksum
        font.extend_from_slice(&(name_off as u32).to_be_bytes());
        font.extend_from_slice(&(name.len() as u32).to_be_bytes());
        font.extend(name);
        font
    }

    #[test]
    fn parses_family_and_postscript() {
        let font = sfnt_with_names(&[
            (1, 0, 0, 1, false, "Mac Family"),
            (3, 1, 0x0409, 1, true, "Win Family"),
            (3, 1, 0x0409, 6, true, "WinFamily-Regular"),
        ]);
        let names = parse_font_names(&font).unwrap();
        assert_eq!(names.family, "Win Family"); // Windows en-US beats Mac Roman
        assert_eq!(names.postscript.as_deref(), Some("WinFamily-Regular"));
    }

    #[test]
    fn typographic_family_wins() {
        let font = sfnt_with_names(&[
            (3, 1, 0x0409, 1, true, "Legacy Light"),
            (3, 1, 0x0409, 16, true, "Legacy"),
        ]);
        assert_eq!(parse_font_names(&font).unwrap().family, "Legacy");
    }

    #[test]
    fn rejects_non_fonts() {
        assert!(parse_font_names(b"not a font at all").is_none());
        assert!(parse_font_names(&[]).is_none());
    }

    #[test]
    fn ident_rules() {
        assert_eq!(font_ident("Special Elite"), "special_elite");
        assert_eq!(font_ident("Pacifico"), "pacifico");
        assert_eq!(font_ident("3of9"), "r3of9");
    }
}
