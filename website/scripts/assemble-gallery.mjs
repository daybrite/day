// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0

// Assemble the screenshots gallery into a static manifest.
//
// Inputs: each repository's latest published full GitHub release. Drafts and prereleases
// are excluded. Indexes and screenshot bytes come from the SAME verified release assets.
// Outputs: src/data/gallery-manifest.json and optimized public/gallery-media images.
// The gallery, localization explorer, and carousel consume that shared manifest.
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
import { mkdirSync, writeFileSync, renameSync, readdirSync, rmSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join, resolve } from 'node:path';
import galleryConfig from '../gallery.config.mjs';
import { loadReleaseGallery } from './release-gallery.mjs';
import { platformsById } from '../src/lib/platforms.mjs';

const HERE = dirname(fileURLToPath(import.meta.url));
const WEBSITE_ROOT = resolve(HERE, '..');
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
  // The CLI retains this historical spelling as an alias for the same backend.
  if (platform === 'android-mdc') platform = 'android-material';
  if (device && platformsById[`${platform}-${device}`]) return `${platform}-${device}`;
  return platform;
}

/** The key a capture is stored under: the two dimensions the app actually varied. Either may be
 *  absent from an index (most apps capture one theme, some one language), and `default` stands in
 *  so a single ladder covers every app. */
function captureKey(theme, locale) {
  return `${theme || 'default'}|${locale || 'default'}`;
}

// Fail closed: a failed latest-release lookup/download leaves the previous generated
// manifest untouched instead of silently publishing branch captures or an older release.
async function loadIndex(app, log) {
  return loadReleaseGallery(app, log);
}

/** Turn one published index into the app's manifest entry. */
export function assembleApp(app, index, stale, source) {
  // Group locally materialized release captures by (shot, column, theme+locale).
  const byShot = new Map();
  const columnShots = new Map();
  const themes = [];
  const locales = [];
  let captures = 0;
  for (const s of index.screenshots) {
    const src = s.url;
    if (!src || !s.shot || !s.platform) continue;
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
    tile[key] = { src, width: s.width ?? undefined, height: s.height ?? undefined };
    captures += 1;
    columnShots.set(column, (columnShots.get(column) ?? 0) + 1);
  }

  // Column order: the index lists platforms in the Day target vocabulary's order, and a device
  // refinement follows the target it refines.
  const columns = [];
  for (const target of index.platforms ?? []) {
    const platform = columnFor(target);
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

  // An app's own site hosts its web-dom build at /webapp/ (the `webapp` default in its
  // website/site.toml), so every app whose index carries a web-dom column has a live build at
  // that address without naming it here. `web` in the config is for a build hosted elsewhere.
  const site = app.site ?? index.site ?? null;
  const web =
    app.web ??
    (site && (index.platforms ?? []).includes('web-dom') ? `${site.replace(/\/$/, '')}/webapp/` : null);

  return {
    id: app.id,
    label: app.label,
    blurb: app.blurb,
    repo: app.repo,
    site,
    web,
    stores: app.stores ?? {},
    // Flattened to one shot→fragment map for the page: the shots whose id is their own route,
    // then the explicit map (which may override one, or `null` it out as unreachable).
    webRoutes: web
      ? { ...Object.fromEntries((app.webShots ?? []).map((s) => [s, s])), ...(app.webRoutes ?? {}) }
      : null,
    stale,
    indexGeneratedAt: index.generated ?? null,
    // Provenance for the exact full release supplying every image in this app.
    indexURL: source?.url ?? null,
    fromMain: false,
    release: { tag: source.tag, url: source.releaseURL, publishedAt: source.publishedAt, archiveDigest: source.digest },
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
 * @returns {Promise<{ manifestPath: string, apps: number, captures: number }>}
 */
export async function assembleGallery(opts = {}) {
  const dataDir = join(WEBSITE_ROOT, 'src', 'data');
  const log = (m) => opts.quiet || console.log(`[gallery] ${m}`);
  mkdirSync(dataDir, { recursive: true });

  const apps = [];
  for (const app of galleryConfig.apps) {
    const loaded = await loadIndex(app, log);
    const entry = assembleApp(app, loaded.data, loaded.stale, loaded.source);
    if (entry.counts.captures === 0) {
      throw new Error(`${app.id}: latest full release contains no usable screenshots`);
    }
    log(
      `${app.id}: ${entry.counts.shots} screen(s), ${entry.counts.captures} capture(s) ` +
        `on ${entry.counts.columns} target(s), from the ${loaded.source?.channel ?? 'unknown'} index`,
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
  const temporary = `${manifestPath}.${process.pid}.tmp`;
  writeFileSync(temporary, JSON.stringify(manifest, null, 2) + '\n');
  renameSync(temporary, manifestPath);
  // Only generated release images live here. Retain the active asset identity for each
  // app; obsolete renditions must not accumulate in every future Pages deployment.
  const media = join(WEBSITE_ROOT, 'public', 'gallery-media');
  const referenced = new Set(apps.flatMap(app => app.shots.flatMap(shot => shot.byColumn.flatMap(column =>
    Object.values(column.captures).map(image => image.src)))));
  const active = new Set([...referenced].map(path => path.split('/').slice(2, 5).join('/')));
  // Release archives also contain hidden test states and duplicate device captures.
  // Publish only images actually referenced by the curated gallery manifest.
  for (const identity of active) {
    for (const file of readdirSync(join(media, identity))) {
      if (!referenced.has(`/gallery-media/${identity}/${file}`)) rmSync(join(media, identity, file));
    }
  }
  for (const app of readdirSync(media, { withFileTypes: true })) {
    if (!app.isDirectory()) continue;
    for (const tag of readdirSync(join(media, app.name), { withFileTypes: true })) {
      if (!tag.isDirectory()) continue;
      const release = `${app.name}/${tag.name}`;
      if (![...active].some(path => path.startsWith(release + '/'))) {
        rmSync(join(media, release), { recursive: true, force: true });
      } else for (const identity of readdirSync(join(media, release))) {
        if (!active.has(`${release}/${identity}`)) rmSync(join(media, release, identity), { recursive: true, force: true });
      }
    }
  }
  log(`indexed ${captures} full-release screenshot(s) across ${apps.length} app(s)`);
  return { manifestPath, apps: apps.length, captures };
}

// Standalone entry point.
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  await assembleGallery();
}
