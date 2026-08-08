// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! `day localize` — one locale set, every surface (DESIGN.md §16.5). A conventional Day project
//! spells its locales in FOUR places: `resource/locales/<tag>/` (the app's own Fluent
//! translations), `store/<tag>/` (the listing text, docs/store.md), the Xcode project's
//! `knownRegions` list, and `website/site.toml`'s `locales` array. Added by hand in one place,
//! a locale silently drifts out of the other three — so [`add`]/[`remove`] edit every surface
//! the project has at once, and [`survey`]/[`sync_findings`] give `day lint` the drift check.
//!
//! Everything here speaks DAY tags — strict BCP 47 (`en`, `fr-CA`, `zh-CN`). Each downstream
//! namespace converts at its own emission point: the stores' spellings at fastlane generation
//! (store.rs), Xcode's via [`xcode_region`] here, and Android's resource qualifiers
//! (`values-iw`, `values-in`) where android resources are emitted — never earlier, so no
//! checked-in tree carries a second spelling of the same locale.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::meta::Project;
use crate::term::{SUCCESS, WARN};
use anstream::eprintln;

/// Xcode's spelling of a Day locale tag.
///
/// Xcode speaks script-subtag Chinese (`zh-Hans` / `zh-Hant`) where Day keys by region
/// (`zh-CN` / `zh-TW`); every other tag passes through. This covers ONLY the `knownRegions`
/// namespace: the stores' spellings (Apple's `zh-Hans`, Play's legacy `iw-IL`) are converted
/// at fastlane emission by store.rs, and Android's resource qualifiers (`values-iw`,
/// `values-in`) are yet another namespace, converted where android resources are emitted —
/// not here.
pub fn xcode_region(tag: &str) -> String {
    match tag {
        "zh-CN" => "zh-Hans".to_string(),
        "zh-TW" => "zh-Hant".to_string(),
        other => other.to_string(),
    }
}

/// The Day tag for an Xcode region — the inverse of [`xcode_region`], so the survey speaks Day
/// tags no matter which spelling the file stores.
fn day_region(region: &str) -> String {
    match region {
        "zh-Hans" => "zh-CN".to_string(),
        "zh-Hant" => "zh-TW".to_string(),
        other => other.to_string(),
    }
}

fn shape_err(tag: &str) -> String {
    format!(
        "{tag:?} is not a Day locale tag — the shape is strict BCP 47: a lowercase 2–3 letter \
         language, then an optional `Xxxx` script and an optional `XX`/`999` region (en, fr-CA, \
         zh-Hans, es-419)"
    )
}

/// Check a Day locale tag: `language(-Script)?(-REGION)?`, with a 2–3 letter lowercase
/// language, an optional titlecase four-letter script, and an optional region (two uppercase
/// letters or three digits). Hand-rolled — a regex crate would be a new dependency for a
/// twenty-line check.
///
/// The legacy ISO-639 codes `iw`/`in`/`ji` are valid-shaped but rejected by name: every Day
/// surface keys by the modern tag, and the legacy spellings belong to single downstream
/// namespaces (Google Play, Android resources) that convert at emission.
pub fn validate_tag(tag: &str) -> Result<(), String> {
    let mut subtags = tag.split('-');
    // `split` always yields a first item; an empty tag falls through to the length check.
    let lang = subtags.next().unwrap_or("");
    if let Some(modern) = match lang {
        "iw" => Some("he"),
        "in" => Some("id"),
        "ji" => Some("yi"),
        _ => None,
    } {
        return Err(format!(
            "{lang:?} is the legacy ISO-639 code — Day uses the modern tag {modern:?} (the \
             stores and Android resources get their own spellings at generation time)"
        ));
    }
    if !((2..=3).contains(&lang.len()) && lang.bytes().all(|b| b.is_ascii_lowercase())) {
        return Err(shape_err(tag));
    }
    let mut next = subtags.next();
    if let Some(s) = next {
        // Optional script subtag: `Xxxx` (titlecase, exactly four letters).
        let script_ok = s.len() == 4
            && s.as_bytes()[0].is_ascii_uppercase()
            && s.bytes().skip(1).all(|b| b.is_ascii_lowercase());
        if script_ok {
            next = subtags.next();
        }
    }
    if let Some(r) = next {
        // Optional region subtag: `XX` (ISO 3166) or `999` (UN M.49, e.g. es-419).
        let region_ok = (r.len() == 2 && r.bytes().all(|b| b.is_ascii_uppercase()))
            || (r.len() == 3 && r.bytes().all(|b| b.is_ascii_digit()));
        if !region_ok {
            return Err(shape_err(tag));
        }
        next = subtags.next();
    }
    if next.is_some() {
        return Err(shape_err(tag));
    }
    Ok(())
}

/// The locale set of each surface, in Day tags. `None` = that surface is absent from the
/// project (no `store/`, no iOS host project, no website `locales` key), so it takes no part
/// in the sync contract. Fluent is the app's own translations and always surveyed — a project
/// without `resource/locales/` simply reports an empty list.
pub struct LocaleSurvey {
    pub fluent: Vec<String>,
    pub store: Option<Vec<String>>,
    pub xcode: Option<Vec<String>>,
    pub website: Option<Vec<String>>,
}

/// Read every surface's locale set from a project directory.
pub fn survey(project_root: &Path) -> LocaleSurvey {
    LocaleSurvey {
        fluent: dir_locales(&project_root.join("resource/locales")).unwrap_or_default(),
        store: dir_locales(&project_root.join("store")),
        xcode: pbx_regions(project_root),
        website: website_locales(project_root),
    }
}

/// The locale subdirectories of `dir`, or `None` when the directory itself is absent. The
/// pseudolocale is a development aid and never joins the sync contract — the same exclusion
/// store.rs applies to listings.
fn dir_locales(dir: &Path) -> Option<Vec<String>> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut v: Vec<String> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .filter(|t| t != "en-XA")
        .collect();
    v.sort();
    Some(v)
}

/// The scaffold's iOS project file — `day new app` writes exactly this path, and
/// `knownRegions` is the only locale list Xcode keeps in it.
fn pbxproj_path(project_root: &Path) -> PathBuf {
    project_root.join("platform/ios/DayApp.xcodeproj/project.pbxproj")
}

const PBX_REL: &str = "platform/ios/DayApp.xcodeproj/project.pbxproj";

/// Byte range of the entries between `knownRegions = (` and its closing `);`.
///
/// Textual on purpose: the block is the only part of the file locale work touches, and a full
/// pbxproj parser would be a dependency for a ten-line list. Assumes Xcode's own layout — one
/// entry per line — which is also what the scaffold ships.
fn pbx_block(text: &str) -> Option<(usize, usize)> {
    let start = text.find("knownRegions = (")?;
    let after = start + "knownRegions = (".len();
    let end = text[after..].find(");")? + after;
    Some((after, end))
}

/// The `knownRegions` entries as Xcode spellings, unquoted (`Base` included).
///
/// Xcode quotes any entry that is not a bare word, which is every hyphenated region
/// (`"zh-Hans"`, `"pt-BR"`). The quotes are pbxproj syntax rather than part of the tag, so they
/// come off here and go back on in [`pbx_quoted`] — otherwise a hyphenated region never compares
/// equal to its Day tag and reads as both missing and unknown at once.
fn pbx_entries(text: &str) -> Vec<String> {
    let Some((a, b)) = pbx_block(text) else {
        return Vec::new();
    };
    text[a..b]
        .lines()
        .map(|l| pbx_unquote(l.trim().trim_end_matches(',')))
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

/// One `knownRegions` entry with its pbxproj quoting removed.
fn pbx_unquote(entry: &str) -> &str {
    entry
        .strip_prefix('"')
        .and_then(|e| e.strip_suffix('"'))
        .unwrap_or(entry)
}

/// `region` spelled the way it must appear in the file: quoted unless it is a bare word.
///
/// Matches Xcode's own rule, which is what keeps the block round-trippable — the project opens
/// without Xcode rewriting the list on save.
fn pbx_quoted(region: &str) -> String {
    if region
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        region.to_string()
    } else {
        format!("\"{region}\"")
    }
}

/// The iOS project's regions as Day tags, or `None` when there is no iOS host project.
fn pbx_regions(project_root: &Path) -> Option<Vec<String>> {
    let text = std::fs::read_to_string(pbxproj_path(project_root)).ok()?;
    let mut v: Vec<String> = pbx_entries(&text)
        .iter()
        // `Base` is Xcode's base-internationalization pseudo-region, not a locale.
        .filter(|r| r.as_str() != "Base")
        .map(|r| day_region(r))
        .collect();
    v.sort();
    Some(v)
}

/// The pbxproj text with `region` inserted into `knownRegions` (before `Base`, matching the
/// scaffold's ordering), or `None` when it is already listed.
fn pbx_with_region(text: &str, region: &str) -> Result<Option<String>, String> {
    if pbx_entries(text).iter().any(|e| e == region) {
        return Ok(None);
    }
    let mut out = String::with_capacity(text.len() + region.len() + 8);
    let mut state = 0u8; // 0 = before the block, 1 = inside it, 2 = inserted
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim();
        if state == 1 && (trimmed == "Base," || trimmed == ");") {
            // Indentation copied from the neighbour so the file stays Xcode-shaped; entries
            // sit one level deeper than the closing `);`.
            let mut indent = line[..line.len() - line.trim_start().len()].to_string();
            if trimmed == ");" {
                indent.push('\t');
            }
            out.push_str(&indent);
            out.push_str(&pbx_quoted(region));
            out.push_str(",\n");
            state = 2;
        }
        if state == 0 && trimmed.starts_with("knownRegions = (") {
            state = 1;
        }
        out.push_str(line);
    }
    if state != 2 {
        return Err(format!("{PBX_REL} has no `knownRegions = (…);` block"));
    }
    Ok(Some(out))
}

/// The pbxproj text minus `region`'s entry line, or `None` when it is not listed.
fn pbx_without_region(text: &str, region: &str) -> Option<String> {
    let mut out = String::with_capacity(text.len());
    let mut state = 0u8; // 0 = before the block, 1 = inside it, 2 = past it
    let mut removed = false;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim();
        if state == 0 && trimmed.starts_with("knownRegions = (") {
            state = 1;
        } else if state == 1 {
            if trimmed == ");" {
                state = 2;
            } else if !removed && pbx_unquote(trimmed.trim_end_matches(',')) == region {
                removed = true;
                continue;
            }
        }
        out.push_str(line);
    }
    removed.then_some(out)
}

/// True when a site.toml line assigns `key` (`key = …`, ignoring leading whitespace) — a
/// commented-out `# key = …` example does not match.
fn is_key_line(line: &str, key: &str) -> bool {
    let t = line.trim_start();
    t.strip_prefix(key)
        .is_some_and(|rest| rest.trim_start().starts_with('='))
}

/// Split a single-line `locales = ["en", "fr"]` into (open-bracket index, close-bracket
/// index, entries). Day writes the array single-line; a hand-formatted multi-line array is
/// reported rather than mangled.
fn parse_locales_line(line: &str) -> Result<(usize, usize, Vec<String>), String> {
    let (Some(open), Some(close)) = (line.find('['), line.rfind(']')) else {
        return Err(
            "website/site.toml: the locales array is not on one line — put it on one line and \
             re-run, or edit it by hand"
                .into(),
        );
    };
    let entries = line[open + 1..close]
        .split(',')
        .map(|e| e.trim().trim_matches(['"', '\'']).to_string())
        .filter(|e| !e.is_empty())
        .collect();
    Ok((open, close, entries))
}

/// `website/site.toml`'s `locales` array, when the site has one. `None` both when there is no
/// website and when the key is absent: without the key the site derives its locale set from
/// `store/` at render time (daysite), so there is nothing to hold in sync — the array is
/// created by the first `day localize add`.
fn website_locales(project_root: &Path) -> Option<Vec<String>> {
    let text = std::fs::read_to_string(project_root.join("website/site.toml")).ok()?;
    let doc: toml::Value = toml::from_str(&text).ok()?;
    let mut v: Vec<String> = doc
        .get("locales")?
        .as_array()?
        .iter()
        .filter_map(|e| e.as_str().map(str::to_string))
        .collect();
    v.sort();
    Some(v)
}

/// The site.toml text with `tag` in its `locales` array, or `None` when already listed. The
/// array is created — right under the required `host =` key, seeded with the default locale —
/// the first time a locale is added: creation is the moment the website opts into the sync
/// contract (before that the site derives its locales from `store/` at render time).
fn site_with_locale(text: &str, tag: &str, default: &str) -> Result<Option<String>, String> {
    if text.lines().any(|l| is_key_line(l, "locales")) {
        let mut out = String::with_capacity(text.len() + tag.len() + 4);
        let mut done = false;
        for line in text.split_inclusive('\n') {
            if !done && is_key_line(line, "locales") {
                let (open, close, mut entries) = parse_locales_line(line)?;
                if entries.iter().any(|e| e == tag) {
                    return Ok(None);
                }
                entries.push(tag.to_string());
                out.push_str(&line[..open + 1]);
                out.push_str(&quote_join(&entries));
                out.push_str(&line[close..]);
                done = true;
            } else {
                out.push_str(line);
            }
        }
        return Ok(Some(out));
    }
    let mut entries: Vec<String> = vec![default.to_string()];
    if tag != default {
        entries.push(tag.to_string());
    }
    let new_line = format!("locales = [{}]\n", quote_join(&entries));
    let mut out = String::with_capacity(text.len() + new_line.len());
    let mut inserted = false;
    for line in text.split_inclusive('\n') {
        out.push_str(line);
        if !inserted && is_key_line(line, "host") {
            out.push_str(&new_line);
            inserted = true;
        }
    }
    if !inserted {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&new_line);
    }
    Ok(Some(out))
}

/// The site.toml text minus `tag` in its `locales` array, or `None` when it is not listed
/// (including when there is no array at all).
fn site_without_locale(text: &str, tag: &str) -> Result<Option<String>, String> {
    if !text.lines().any(|l| is_key_line(l, "locales")) {
        return Ok(None);
    }
    let mut out = String::with_capacity(text.len());
    let mut done = false;
    for line in text.split_inclusive('\n') {
        if !done && is_key_line(line, "locales") {
            let (open, close, entries) = parse_locales_line(line)?;
            if !entries.iter().any(|e| e == tag) {
                return Ok(None);
            }
            let kept: Vec<String> = entries.into_iter().filter(|e| e != tag).collect();
            out.push_str(&line[..open + 1]);
            out.push_str(&quote_join(&kept));
            out.push_str(&line[close..]);
            done = true;
        } else {
            out.push_str(line);
        }
    }
    Ok(Some(out))
}

fn quote_join(entries: &[String]) -> String {
    entries
        .iter()
        .map(|e| format!("{e:?}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Copy `src`'s files with extension `ext` into a fresh `dst`, prepending `header` (the
/// translate-me marker) when given. Returns how many files were copied; a missing `src`
/// copies nothing but still creates `dst`, so the surface exists to survey.
/// Rewrite the scaffold's opening-screen keys with their translations, leaving every other line
/// exactly as copied. Returns the new body and how many lines were translated.
///
/// Matching is on the key at the start of a line, so a commented-out key or a key inside a value
/// is untouched. `{app}` in the table is replaced with whatever the default locale already put
/// there, which keeps the project's own title rather than inventing one.
fn apply_starter(body: &str, starter: &'static [&'static str; 6]) -> (String, usize) {
    let keys = crate::starter_l10n::KEYS;
    // The app's title, read from `app_title` in the file being copied rather than guessed out of
    // a longer line: taking the last word of "Welcome to Day Sample" yields "Sample", which is
    // wrong for every multi-word title.
    let app = body
        .lines()
        .find_map(|l| l.strip_prefix("app_title = "))
        .unwrap_or("")
        .trim()
        .to_string();
    let mut n = 0usize;
    let out: Vec<String> = body
        .lines()
        .map(|line| {
            let Some((lhs, _)) = line.split_once(" = ") else {
                return line.to_string();
            };
            let Some(i) = keys.iter().position(|k| *k == lhs.trim_end()) else {
                return line.to_string();
            };
            n += 1;
            format!("{lhs} = {}", starter[i].replace("{app}", &app))
        })
        .collect();
    (
        out.join("\n") + if body.ends_with('\n') { "\n" } else { "" },
        n,
    )
}

fn copy_locale_files(
    src: &Path,
    dst: &Path,
    ext: &str,
    header: Option<&str>,
    starter: Option<&'static [&'static str; 6]>,
) -> Result<usize, String> {
    std::fs::create_dir_all(dst).map_err(|e| format!("{}: {e}", dst.display()))?;
    let Ok(entries) = std::fs::read_dir(src) else {
        return Ok(0);
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == ext))
        .collect();
    files.sort();
    let mut copied = 0usize;
    for p in files {
        let Some(name) = p.file_name() else { continue };
        let body = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
        let (body, translated) = match starter {
            Some(t) => apply_starter(&body, t),
            None => (body, 0),
        };
        let body = match header {
            // A file that arrived fully translated does not want a translate-me header, and one
            // that arrived partly translated wants the header to say which part.
            Some(h) if translated > 0 => format!(
                "{}{body}",
                h.replace(
                    "copied from",
                    &format!("{translated} starter string(s) translated; the rest copied from")
                )
            ),
            Some(h) => format!("{h}{body}"),
            None => body,
        };
        let to = dst.join(name);
        std::fs::write(&to, body).map_err(|e| format!("{}: {e}", to.display()))?;
        copied += 1;
    }
    Ok(copied)
}

/// Add `tag` to every surface the project HAS, idempotently: a surface that already lists the
/// tag is left alone, so re-running an interrupted add completes it. Returns one line per
/// change made, for the CLI to narrate.
pub fn add(project_root: &Path, tag: &str) -> Result<Vec<String>, String> {
    validate_tag(tag)?;
    let s = survey(project_root);
    let default = crate::store::default_locale(&s.fluent).unwrap_or_else(|| "en".to_string());
    let mut done = Vec::new();

    // Fluent — the app's own translations. New content is the default locale's files with a
    // translate-me header: a complete-but-untranslated locale the fluent lints then track,
    // rather than an empty directory nothing checks.
    let fluent_dir = project_root.join("resource/locales");
    let dst = fluent_dir.join(tag);
    if !dst.is_dir() {
        let header =
            format!("# TODO: translate — copied from {default}/ by `day localize add {tag}`.\n");
        let starter = crate::starter_l10n::starter_for(tag);
        let n = copy_locale_files(
            &fluent_dir.join(&default),
            &dst,
            "ftl",
            Some(&header),
            starter,
        )?;
        done.push(format!(
            "created resource/locales/{tag}/ ({n} file(s) copied from {default}/)"
        ));
    }

    // Store listing text — only when the project keeps one at all (store.rs's own rule: an app
    // that never ships to a store is not nagged about listings). Copied VERBATIM: a
    // translate-me header in listing text would upload.
    if s.store.is_some() {
        let store_dir = project_root.join("store");
        let dst = store_dir.join(tag);
        if !dst.is_dir() {
            let n = copy_locale_files(&store_dir.join(&default), &dst, "txt", None, None)?;
            done.push(format!(
                "created store/{tag}/ ({n} file(s) copied from {default}/)"
            ));
        }
    }

    // Xcode's knownRegions — its own spelling, inserted before `Base`.
    let pbx = pbxproj_path(project_root);
    if pbx.is_file() {
        let text = std::fs::read_to_string(&pbx).map_err(|e| format!("{}: {e}", pbx.display()))?;
        let region = xcode_region(tag);
        if let Some(updated) = pbx_with_region(&text, &region)? {
            std::fs::write(&pbx, updated).map_err(|e| format!("{}: {e}", pbx.display()))?;
            done.push(format!("added {region} to knownRegions in {PBX_REL}"));
        }
    }

    // The website's declared locale set.
    let site = project_root.join("website/site.toml");
    if site.is_file() {
        let text =
            std::fs::read_to_string(&site).map_err(|e| format!("{}: {e}", site.display()))?;
        if let Some(updated) = site_with_locale(&text, tag, &default)? {
            std::fs::write(&site, updated).map_err(|e| format!("{}: {e}", site.display()))?;
            done.push(format!(
                "added {tag:?} to the locales array in website/site.toml"
            ));
        }
    }
    Ok(done)
}

/// Remove `tag` from every surface — the inverse of [`add`]. The default locale is refused:
/// it is the root of the fallback chain (`res::locales::DEFAULT`) and the source `add` copies
/// from, so removing it strands every other locale — and the only locale is by definition it.
pub fn remove(project_root: &Path, tag: &str) -> Result<Vec<String>, String> {
    let s = survey(project_root);
    if crate::store::default_locale(&s.fluent).as_deref() == Some(tag) {
        return Err(format!(
            "{tag} is the app's default locale — every other locale falls back to it; make \
             another locale the default before removing it"
        ));
    }
    let mut done = Vec::new();
    for rel in [format!("resource/locales/{tag}"), format!("store/{tag}")] {
        let dir = project_root.join(&rel);
        if dir.is_dir() {
            std::fs::remove_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            done.push(format!("removed {rel}/"));
        }
    }
    let pbx = pbxproj_path(project_root);
    if pbx.is_file() {
        let text = std::fs::read_to_string(&pbx).map_err(|e| format!("{}: {e}", pbx.display()))?;
        let region = xcode_region(tag);
        if let Some(updated) = pbx_without_region(&text, &region) {
            std::fs::write(&pbx, updated).map_err(|e| format!("{}: {e}", pbx.display()))?;
            done.push(format!("removed {region} from knownRegions in {PBX_REL}"));
        }
    }
    let site = project_root.join("website/site.toml");
    if site.is_file() {
        let text =
            std::fs::read_to_string(&site).map_err(|e| format!("{}: {e}", site.display()))?;
        if let Some(updated) = site_without_locale(&text, tag)? {
            std::fs::write(&site, updated).map_err(|e| format!("{}: {e}", site.display()))?;
            done.push(format!(
                "removed {tag:?} from the locales array in website/site.toml"
            ));
        }
    }
    Ok(done)
}

/// Compare every PRESENT surface against the union of all of them: each (message, advice)
/// pair is one locale one surface lacks. The advice is per-surface and concrete — fixable
/// without opening the docs — and `day localize add` is always the one-command form.
pub fn sync_findings(survey: &LocaleSurvey) -> Vec<(String, String)> {
    let default = crate::store::default_locale(&survey.fluent).unwrap_or_else(|| "en".to_string());
    let mut present: Vec<(&str, &[String])> = vec![("resource/locales/", &survey.fluent)];
    if let Some(s) = &survey.store {
        present.push(("store/", s));
    }
    if let Some(s) = &survey.xcode {
        present.push(("platform/ios (knownRegions)", s));
    }
    if let Some(s) = &survey.website {
        present.push(("website/site.toml", s));
    }
    let mut union: BTreeSet<&str> = BTreeSet::new();
    for (_, set) in &present {
        union.extend(set.iter().map(String::as_str));
    }
    let mut out = Vec::new();
    for &tag in &union {
        let have: Vec<&str> = present
            .iter()
            .filter(|(_, set)| set.iter().any(|t| t == tag))
            .map(|(label, _)| *label)
            .collect();
        for (label, set) in &present {
            if set.iter().any(|t| t == tag) {
                continue;
            }
            let advice = match *label {
                "resource/locales/" => format!(
                    "run `day localize add {tag}`, or create resource/locales/{tag}/ by \
                     copying resource/locales/{default}/"
                ),
                "store/" => format!(
                    "run `day localize add {tag}`, or create store/{tag}/ by copying \
                     store/{default}/"
                ),
                "platform/ios (knownRegions)" => format!(
                    "add `{tag}` to knownRegions in {PBX_REL} (Xcode spelling: {})",
                    xcode_region(tag)
                ),
                _ => format!("add {tag:?} to the locales array in website/site.toml"),
            };
            out.push((
                format!(
                    "locale {tag} is in {} but missing from {label}",
                    have.join(", ")
                ),
                advice,
            ));
        }
        // A tag neither store can spell is drift of a different kind: `day store stage`
        // silently skips such a locale (store.rs), so say so while it is one tag old.
        if survey.store.is_some() && !crate::store::mappable(tag) {
            out.push((
                format!("locale {tag} has no App Store / Google Play locale in store::LOCALES"),
                format!(
                    "extend LOCALES in day-cli's store.rs with {tag}'s store spellings, or \
                     pick a supported tag — `day store stage` skips locales it cannot spell"
                ),
            ));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// `day localize …`
// ---------------------------------------------------------------------------

/// `day localize list` — each surface's locales, then the drift warnings. Informational: the
/// exit code is 0 even with findings (`day lint --strict` is the enforcing form).
fn list(project: &Project) -> i32 {
    let s = survey(&project.root);
    let show = |v: &[String]| -> String {
        if v.is_empty() {
            "(none)".into()
        } else {
            v.join(", ")
        }
    };
    crate::ops::status(
        "Fluent",
        &format!("{} — resource/locales/", show(&s.fluent)),
    );
    let opt = |label: &str, path: &str, set: &Option<Vec<String>>| match set {
        Some(v) => crate::ops::status(label, &format!("{} — {path}", show(v))),
        None => crate::ops::status(label, "(not present)"),
    };
    opt("Store", "store/", &s.store);
    opt("Xcode", "platform/ios knownRegions", &s.xcode);
    opt("Website", "website/site.toml locales", &s.website);
    let findings = sync_findings(&s);
    if findings.is_empty() {
        eprintln!("{SUCCESS}✓{SUCCESS:#} every present surface lists the same locales");
    } else {
        for (message, advice) in &findings {
            eprintln!("{WARN}warning{WARN:#} {message}\n        {advice}");
        }
        eprintln!(
            "{} finding(s) — `day localize add/remove` keeps the surfaces in step",
            findings.len()
        );
    }
    0
}

/// Apply [`add`] or [`remove`] to each requested tag, narrating what changed. The first
/// failing tag stops the run with exit 1; surfaces already updated stay updated — both
/// operations are idempotent, so re-running completes the rest.
fn edit(
    project: &Project,
    raw: &[String],
    op: fn(&Path, &str) -> Result<Vec<String>, String>,
) -> i32 {
    let tags = crate::cli::split_list(raw);
    if tags.is_empty() {
        eprintln!("error: no locale given (e.g. `day localize add fr`)");
        return 2;
    }
    for tag in &tags {
        match op(&project.root, tag) {
            Ok(lines) if lines.is_empty() => {
                crate::ops::status("Localize", &format!("{tag}: every surface already agrees"));
            }
            Ok(lines) => {
                for l in &lines {
                    crate::ops::status("Localize", l);
                }
            }
            Err(e) => {
                crate::ops::status("Error", &format!("{tag}: {e}"));
                return 1;
            }
        }
    }
    0
}

/// `day localize <list|add|remove>`.
pub fn run(project: &Project, cmd: &crate::cli::LocalizeCmd) -> i32 {
    match cmd {
        crate::cli::LocalizeCmd::List => list(project),
        crate::cli::LocalizeCmd::Add { locales } => edit(project, locales, add),
        crate::cli::LocalizeCmd::Remove { locales } => edit(project, locales, remove),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("day-localize-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    /// A minimal fake project with all four surfaces present, `en` only.
    fn fixture(name: &str) -> PathBuf {
        let root = scratch(name);
        std::fs::create_dir_all(root.join("resource/locales/en")).expect("locales");
        std::fs::write(root.join("resource/locales/en/app.ftl"), "hello = Hello\n").expect("ftl");
        std::fs::create_dir_all(root.join("store/en")).expect("store");
        std::fs::write(root.join("store/en/name.txt"), "Example\n").expect("name");
        std::fs::create_dir_all(root.join("platform/ios/DayApp.xcodeproj")).expect("xcodeproj");
        std::fs::write(
            pbxproj_path(&root),
            "\t\t\tdevelopmentRegion = en;\n\t\t\tknownRegions = (\n\t\t\t\ten,\n\t\t\t\tBase,\n\t\t\t);\n",
        )
        .expect("pbxproj");
        std::fs::create_dir_all(root.join("website")).expect("website");
        std::fs::write(
            root.join("website/site.toml"),
            "host = \"https://example.test/app\"\n",
        )
        .expect("site.toml");
        root
    }

    #[test]
    fn tag_validation_is_strict_bcp47_and_names_the_modern_tag() {
        for ok in ["en", "fr-CA", "zh-Hans", "es-419", "zh-CN", "sr-Latn-RS"] {
            assert!(validate_tag(ok).is_ok(), "{ok}");
        }
        for bad in ["EN", "english", "zh_CN", "en-us", "fr-", "a", ""] {
            assert!(validate_tag(bad).is_err(), "{bad}");
        }
        // The legacy ISO codes are rejected by NAME, pointing at the modern tag.
        for (legacy, modern) in [("iw", "he"), ("in", "id"), ("ji", "yi"), ("in-ID", "id")] {
            let err = validate_tag(legacy).expect_err(legacy);
            assert!(err.contains(&format!("{modern:?}")), "{err}");
        }
    }

    #[test]
    fn xcode_regions_use_script_subtag_chinese() {
        assert_eq!(xcode_region("zh-CN"), "zh-Hans");
        assert_eq!(xcode_region("zh-TW"), "zh-Hant");
        assert_eq!(xcode_region("fr"), "fr");
        // The survey maps back, so it always speaks Day tags.
        assert_eq!(day_region("zh-Hans"), "zh-CN");
        assert_eq!(day_region("zh-Hant"), "zh-TW");
        assert_eq!(day_region("pt-BR"), "pt-BR");
    }

    #[test]
    fn hyphenated_regions_round_trip_through_xcode_quoting() {
        // Xcode quotes every entry that is not a bare word. Reading has to take the quotes off
        // (or `"zh-Hans"` reads as a region nobody asked for, while zh-CN reads as missing) and
        // writing has to put them back.
        let text =
            "\t\t\tknownRegions = (\n\t\t\t\ten,\n\t\t\t\t\"zh-Hans\",\n\t\t\t\tBase,\n\t\t\t);\n";
        assert_eq!(pbx_entries(text), v(&["en", "zh-Hans", "Base"]));
        assert_eq!(pbx_quoted("zh-Hans"), "\"zh-Hans\"");
        assert_eq!(pbx_quoted("fr"), "fr");

        // Already listed under its quoted spelling, so adding it is a no-op…
        assert_eq!(pbx_with_region(text, "zh-Hans").expect("block"), None);
        // …and removing it finds the quoted line.
        let removed = pbx_without_region(text, "zh-Hans").expect("listed");
        assert_eq!(pbx_entries(&removed), v(&["en", "Base"]));
        // Adding it back quotes it again, returning the file to where it started.
        let added = pbx_with_region(&removed, "zh-Hans")
            .expect("block")
            .expect("not yet listed");
        assert_eq!(added, text);
    }

    #[test]
    fn sync_findings_flag_drift_in_both_directions() {
        // Fluent has a locale the store lacks…
        let s = LocaleSurvey {
            fluent: v(&["en", "fr"]),
            store: Some(v(&["en"])),
            xcode: None,
            website: None,
        };
        let f = sync_findings(&s);
        assert_eq!(f.len(), 1, "{f:?}");
        assert!(f[0].0.contains("missing from store/"), "{}", f[0].0);
        assert!(
            f[0].1.contains("day localize add fr") && f[0].1.contains("store/fr/"),
            "{}",
            f[0].1
        );

        // …and the other way round.
        let s = LocaleSurvey {
            fluent: v(&["en"]),
            store: Some(v(&["en", "fr"])),
            xcode: Some(v(&["en", "fr"])),
            website: None,
        };
        let f = sync_findings(&s);
        assert_eq!(f.len(), 1, "{f:?}");
        assert!(
            f[0].0.contains("missing from resource/locales/"),
            "{}",
            f[0].0
        );
        assert!(f[0].1.contains("day localize add fr"), "{}", f[0].1);

        // Xcode advice carries the Xcode spelling; website advice names the array.
        let s = LocaleSurvey {
            fluent: v(&["en", "zh-CN"]),
            store: None,
            xcode: Some(v(&["en"])),
            website: Some(v(&["en"])),
        };
        let f = sync_findings(&s);
        assert_eq!(f.len(), 2, "{f:?}");
        assert!(
            f.iter()
                .any(|(m, a)| m.contains("knownRegions") && a.contains("zh-Hans"))
        );
        assert!(
            f.iter()
                .any(|(m, a)| m.contains("website/site.toml") && a.contains("locales array"))
        );

        // A tag the stores cannot spell is drift too — but only when store/ exists at all.
        let s = LocaleSurvey {
            fluent: v(&["en", "kl"]),
            store: Some(v(&["en", "kl"])),
            xcode: None,
            website: None,
        };
        let f = sync_findings(&s);
        assert_eq!(f.len(), 1, "{f:?}");
        assert!(f[0].0.contains("kl") && f[0].1.contains("LOCALES"), "{f:?}");
        let s = LocaleSurvey {
            fluent: v(&["en", "kl"]),
            store: None,
            xcode: None,
            website: None,
        };
        assert!(sync_findings(&s).is_empty());
    }

    #[test]
    fn add_then_remove_round_trips_every_surface() {
        let root = fixture("roundtrip");
        let lines = add(&root, "fr").expect("add fr");
        assert_eq!(lines.len(), 4, "{lines:?}");
        let ftl = std::fs::read_to_string(root.join("resource/locales/fr/app.ftl")).expect("ftl");
        assert!(ftl.contains("hello = Hello"), "copied from en: {ftl}");
        assert!(
            ftl.starts_with("# TODO: translate"),
            "carries the translate-me header: {ftl}"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("store/fr/name.txt")).expect("name"),
            "Example\n"
        );
        let pbx = std::fs::read_to_string(pbxproj_path(&root)).expect("pbx");
        assert!(
            pbx.contains("\t\t\t\tfr,\n\t\t\t\tBase,"),
            "inserted before Base: {pbx}"
        );
        let site = std::fs::read_to_string(root.join("website/site.toml")).expect("site");
        assert!(site.contains("locales = [\"en\", \"fr\"]"), "{site}");

        // Idempotent: a second add changes nothing.
        assert!(add(&root, "fr").expect("re-add").is_empty());

        // Chinese takes the script-subtag spelling in the pbxproj and maps back in the survey.
        // Hyphenated, so it lands quoted — `fr` above is a bare word and does not.
        add(&root, "zh-CN").expect("add zh-CN");
        let pbx = std::fs::read_to_string(pbxproj_path(&root)).expect("pbx");
        assert!(
            pbx.contains("\"zh-Hans\",") && !pbx.contains("zh-CN,"),
            "{pbx}"
        );
        let s = survey(&root);
        assert_eq!(s.fluent, v(&["en", "fr", "zh-CN"]));
        assert_eq!(s.store, Some(v(&["en", "fr", "zh-CN"])));
        assert_eq!(s.xcode, Some(v(&["en", "fr", "zh-CN"])));
        assert_eq!(s.website, Some(v(&["en", "fr", "zh-CN"])));
        assert!(
            sync_findings(&s).is_empty(),
            "an add leaves nothing out of sync"
        );

        let lines = remove(&root, "fr").expect("remove fr");
        assert_eq!(lines.len(), 4, "{lines:?}");
        assert!(!root.join("resource/locales/fr").exists());
        assert!(!root.join("store/fr").exists());
        let pbx = std::fs::read_to_string(pbxproj_path(&root)).expect("pbx");
        assert!(!pbx.contains("fr,"), "{pbx}");
        let site = std::fs::read_to_string(root.join("website/site.toml")).expect("site");
        assert!(!site.contains("\"fr\""), "{site}");

        // The default locale is refused — everything else falls back to it.
        assert!(remove(&root, "en").is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Only the surfaces a project HAS are touched: no store/, no iOS host, no website ⇒ none
    /// invented.
    #[test]
    fn add_touches_only_present_surfaces() {
        let root = scratch("fluent-only");
        std::fs::create_dir_all(root.join("resource/locales/en")).expect("mkdir");
        std::fs::write(root.join("resource/locales/en/app.ftl"), "hello = Hello\n").expect("ftl");
        let lines = add(&root, "fr").expect("add");
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(root.join("resource/locales/fr/app.ftl").is_file());
        assert!(!root.join("store").exists());
        assert!(!root.join("website").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
