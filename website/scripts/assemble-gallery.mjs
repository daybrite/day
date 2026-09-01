// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0

// Assemble the screenshots gallery into a static manifest.
//
// Inputs  : each app's published screenshot index (`app.metadata` in gallery.config.mjs — the
//           `gallery.json` that `day screenshot index` writes and every Day app site serves at
//           `<host>/gallery/gallery.json`). The index carries absolute image URLs, so this site
//           REFERENCES the app's own hosted screenshots: one copy of the bytes, owned by the app
//           that captured them, and daybrite.dev's build waits on nobody else's CI.
// Outputs : `src/data/gallery-manifest.json`  (src/pages/gallery/index.astro, gallery/[app].astro,
//                                              components/PlatformShots.astro, hero-shots.mjs)
//           `.cache/gallery/<app>.json`        (the last index that fetched, gitignored)
//
// The manifest is SHOT-major per app: one row per captured screen holding every column's tile, and
// each tile carrying all of its (theme, locale) captures so the page's selectors can swap images
// client-side without reloading. Rows, columns, themes and languages all come from the index —
// an app that captures a new screen shows it on the next build with no change here.
//
// A COLUMN is a target, split by device where the app captured more than one form factor and the
// platform table names the refinement (`ios-uikit` + `ios-uikit-ipad`). An unknown device folds
// into its target's own column rather than inventing one the rest of the site cannot name.
//
// An app whose index cannot be fetched falls back to the cached copy of its last successful
// fetch, and is DROPPED (loudly) when there is no cache either: a page of placeholders would say
// only that a fetch failed, which is not what a reader came for. A build where NO app resolves
// still succeeds, with an empty gallery — that is a local checkout with no network, and the
// pages have to be able to render.
//
// Runnable standalone (`node scripts/assemble-gallery.mjs`) and from the Astro integration
// (integrations/gallery.mjs). No third-party dependencies.

import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join, resolve } from 'node:path';
import galleryConfig from '../gallery.config.mjs';
import { platformsById } from '../src/lib/platforms.mjs';

const HERE = dirname(fileURLToPath(import.meta.url));
const WEBSITE_ROOT = resolve(HERE, '..');
const CACHE_DIR = join(WEBSITE_ROOT, '.cache', 'gallery');

/** `san-francisco` → `San Francisco`: the row heading for a shot whose dayscript declared no
 *  `title:`. The same derivation `day screenshot index` applies, so a thin index and a rich one
 *  read alike. */
function derivedLabel(id) {
  return id.replace(/[-_]+/g, ' ').replace(/\b\w/g, (c) => c.toUpperCase());
}

/** Resolve one of the index's localized text maps for this (English) site: the English entry,
 *  else any entry — a French-only caption beats no caption. */
function english(text) {
  if (!text) return null;
  if (typeof text === 'string') return text;
  return text.en ?? Object.values(text).find((v) => typeof v === 'string') ?? null;
}

/** The column a capture belongs in. A device the platform table names as a refinement of the
 *  target (`ios-uikit` + `ipad` → `ios-uikit-ipad`) gets its own column; every other device —
 *  the app's primary phone, an unnamed profile — folds into the target's own column. */
function columnFor(platform, device) {
  if (device && platformsById[`${platform}-${device}`]) return `${platform}-${device}`;
  return platform;
}

/** The key a capture is stored under: the two dimensions the app actually varied. Either may be
 *  absent from an index (most apps capture one theme, some one language), and `default` stands in
 *  so a single ladder covers every app. */
function captureKey(theme, locale) {
  return `${theme || 'default'}|${locale || 'default'}`;
}

/** Fetch an app's published index, caching the last good copy so a later build survives an
 *  unreachable site (and so a local checkout works offline once it has fetched). */
async function loadIndex(app, log) {
  const cacheFile = join(CACHE_DIR, `${app.id}.json`);
  try {
    const res = await fetch(app.metadata, { signal: AbortSignal.timeout(20_000) });
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const data = await res.json();
    if (!Array.isArray(data.screenshots)) throw new Error('no screenshots[] in the index');
    mkdirSync(CACHE_DIR, { recursive: true });
    writeFileSync(cacheFile, JSON.stringify(data));
    return { data, stale: false };
  } catch (err) {
    const why = err?.message ?? err;
    if (existsSync(cacheFile)) {
      try {
        log(`${app.id}: ${why} — using the cached index from the last good fetch`);
        return { data: JSON.parse(readFileSync(cacheFile, 'utf8')), stale: true };
      } catch {
        /* fall through to the drop below */
      }
    }
    log(`${app.id}: ${why} — no cached index, so the app is left out of this build`);
    return null;
  }
}

/** Turn one published index into the app's manifest entry. */
function assembleApp(app, index, stale) {
  // Group every usable capture by (shot, column, theme+locale). `url` is what this site links;
  // an index published by a site with no configured host has none, and contributes nothing.
  const byShot = new Map();
  const columnShots = new Map();
  const themes = [];
  const locales = [];
  let captures = 0;
  for (const s of index.screenshots) {
    if (!s.url || !s.shot || !s.platform) continue;
    const column = columnFor(s.platform, s.device);
    if (app.platforms && !app.platforms.includes(column)) continue;
    if (app.hide?.includes(s.shot)) continue;
    if (s.theme && !themes.includes(s.theme)) themes.push(s.theme);
    if (s.locale && !locales.includes(s.locale)) locales.push(s.locale);
    const tiles = byShot.get(s.shot) ?? new Map();
    byShot.set(s.shot, tiles);
    const tile = tiles.get(column) ?? {};
    tiles.set(column, tile);
    const key = captureKey(s.theme, s.locale);
    if (key in tile) continue; // first capture of a combination wins, in the index's own order
    tile[key] = { src: s.url, width: s.width ?? undefined, height: s.height ?? undefined };
    captures += 1;
    columnShots.set(column, (columnShots.get(column) ?? 0) + 1);
  }

  // Column order: the index lists platforms in the Day target vocabulary's order, and a device
  // refinement follows the target it refines.
  const columns = [];
  for (const platform of index.platforms ?? []) {
    for (const id of [platform, ...[...columnShots.keys()].filter((c) => c !== platform && c.startsWith(`${platform}-`)).sort()]) {
      if (columnShots.has(id) && !columns.includes(id)) columns.push(id);
    }
  }
  for (const id of columnShots.keys()) if (!columns.includes(id)) columns.push(id);

  // Row order: the config's pinned ids first (for an app whose dayscript order reads oddly),
  // then the index's own — which is the dayscript's declaration order, not alphabetical.
  const indexShots = new Map((index.shots ?? []).map((s) => [s.id, s]));
  const ordered = [
    ...(app.order ?? []).filter((id) => byShot.has(id)),
    ...[...indexShots.keys()].filter((id) => byShot.has(id) && !app.order?.includes(id)),
    ...[...byShot.keys()].filter((id) => !indexShots.has(id) && !app.order?.includes(id)),
  ];
  if (app.hero && ordered.includes(app.hero)) {
    ordered.splice(ordered.indexOf(app.hero), 1);
    ordered.unshift(app.hero);
  }

  const shots = ordered.map((id) => {
    const meta = indexShots.get(id);
    const tiles = byShot.get(id);
    return {
      id,
      label: app.labels?.[id] ?? english(meta?.title) ?? derivedLabel(id),
      caption: english(meta?.caption),
      source: meta?.source ?? null,
      byColumn: columns
        .filter((c) => tiles.has(c))
        .map((c) => ({ column: c, captures: tiles.get(c) })),
    };
  });

  // Themes read light-before-dark; languages keep the index's order, English first where it is
  // captured, because that is the page's own language.
  themes.sort((a, b) => (a === 'light' ? -1 : b === 'light' ? 1 : a.localeCompare(b)));
  if (locales.includes('en')) locales.splice(0, 0, ...locales.splice(locales.indexOf('en'), 1));

  return {
    id: app.id,
    label: app.label,
    blurb: app.blurb,
    repo: app.repo,
    site: app.site ?? index.site ?? null,
    web: app.web ?? null,
    // Flattened to one shot→fragment map for the page: the shots whose id is their own route,
    // then the explicit map (which may override one, or `null` it out as unreachable).
    webRoutes: app.web
      ? { ...Object.fromEntries((app.webShots ?? []).map((s) => [s, s])), ...(app.webRoutes ?? {}) }
      : null,
    // When the app's site went unreachable this build, the page says so rather than presenting a
    // possibly-months-old set as current.
    stale,
    indexGeneratedAt: index.generated ?? null,
    themes,
    locales,
    columns: columns.map((id) => {
      const p = platformsById[id] ?? {};
      return {
        id,
        label: p.chip ?? p.toolkit ?? id,
        os: p.osShort ?? p.os ?? id,
        toolkit: p.toolkitLong ?? p.toolkit ?? id,
        shotCount: columnShots.get(id) ?? 0,
      };
    }),
    counts: { shots: shots.length, captures, columns: columns.length },
    // The hub's card carousel: a diagonal through the grid, so consecutive slides differ in BOTH
    // the screen and the platform rather than showing one screen twelve ways.
    cover: coverOf(shots, themes, locales),
    shots,
  };
}

/** Up to six representative captures for the app's hub card. */
function coverOf(shots, themes, locales, max = 6) {
  const want = captureKey(themes.includes('light') ? 'light' : themes[0], locales[0]);
  const out = [];
  for (let i = 0; i < shots.length && out.length < max; i++) {
    const shot = shots[i];
    // Step the column with the row so the strip walks platforms as it walks screens.
    for (let n = 0; n < shot.byColumn.length; n++) {
      const tile = shot.byColumn[(i + n) % shot.byColumn.length];
      const img = tile.captures[want] ?? Object.values(tile.captures)[0];
      if (!img) continue;
      out.push({ ...img, shot: shot.id, label: shot.label, column: tile.column });
      break;
    }
  }
  // Landscape first. The card's stage is wide, so a phone capture fills a third of it — fine as
  // the carousel passes through, wrong as the still every visitor sees before it starts moving.
  // A stable partition, so the walk's screen-and-platform variety survives the reorder.
  const portrait = (s) => (s.width && s.height && s.height > s.width ? 1 : 0);
  return out.map((s, i) => [s, i]).sort((a, b) => portrait(a[0]) - portrait(b[0]) || a[1] - b[1]).map(([s]) => s);
}

/**
 * @param {{ quiet?: boolean }} [opts]
 * @returns {Promise<{ manifestPath: string, apps: number, captures: number, dropped: string[], stale: string[] }>}
 */
export async function assembleGallery(opts = {}) {
  const dataDir = join(WEBSITE_ROOT, 'src', 'data');
  const log = (m) => opts.quiet || console.log(`[gallery] ${m}`);
  mkdirSync(dataDir, { recursive: true });

  const dropped = [];
  const stale = [];
  const apps = [];
  for (const app of galleryConfig.apps) {
    const loaded = await loadIndex(app, log);
    if (!loaded) {
      dropped.push(app.id);
      continue;
    }
    if (loaded.stale) stale.push(app.id);
    const entry = assembleApp(app, loaded.data, loaded.stale);
    if (entry.counts.captures === 0) {
      log(`${app.id}: its index describes no linkable screenshot — left out`);
      dropped.push(app.id);
      continue;
    }
    log(
      `${app.id}: ${entry.counts.shots} screen(s), ${entry.counts.captures} capture(s) ` +
        `on ${entry.counts.columns} target(s)`,
    );
    apps.push(entry);
  }

  const captures = apps.reduce((n, a) => n + a.counts.captures, 0);
  const manifest = {
    // Only stamp a time when something was indexed, to keep an empty build reproducible.
    generatedAt: apps.length > 0 ? new Date().toISOString() : null,
    apps,
  };
  const manifestPath = join(dataDir, 'gallery-manifest.json');
  writeFileSync(manifestPath, JSON.stringify(manifest, null, 2) + '\n');
  log(
    apps.length > 0
      ? `indexed ${captures} published screenshot(s) across ${apps.length} app(s)`
      : 'no app index could be read — the gallery is empty (expected offline, on a cold checkout)',
  );
  return { manifestPath, apps: apps.length, captures, dropped, stale };
}

// Standalone entry point.
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  await assembleGallery();
}
