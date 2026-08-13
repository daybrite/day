// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0

// Assemble the screenshots gallery into a static manifest.
//
// Inputs  : per suite, ONE of —
//           · the suite's published screenshot index (`suite.metadata`, `day screenshot index`'s
//             gallery.json): the manifest then references those hosted images by absolute
//             URL and nothing is copied — the app's own site owns the bytes, and adding
//             another Day app site's index adds another suite to the gallery;
//           · `<artifactsDir>/screenshots-<platform>/<variant>/<shot>.png` (from
//             `download-artifact pattern: screenshots-*`) — the fallback when there is no
//             index or the fetch fails. Variants are the themed / localized capture sets CI
//             produces per platform (light / dark / fr today), per gallery.config.mjs.
// Outputs : `public/gallery/<suite>/<platform>/<variant>/<shot>.png` (artifact mode only)
//           `src/data/gallery-manifest.json`   (consumed by src/pages/gallery.astro)
//
// The manifest is SHOT-major: the gallery renders one row per shot with every platform's tile
// in it, and each tile carries all of its variants so the page's theme/language selectors can
// swap images client-side without reloading.
//
// When no artifacts are present (local builds), every shot is emitted as a placeholder entry so
// the gallery layout is fully visible without any screenshots. The design is extensible: adding a
// sample app or a component-snapshot set is a gallery.config.mjs change, not a code change here.
//
// A run that DID capture something drops the platforms it captured nothing for: no column, no
// tiles, and their ids listed in the suite's `hidden` — a full column of placeholders reports a
// broken CI leg to a reader who came to compare toolkits.
//
// Runnable standalone (`node scripts/assemble-gallery.mjs [artifactsDir]`) and from the Astro
// integration (integrations/gallery.mjs). No third-party dependencies.

import { existsSync, mkdirSync, rmSync, copyFileSync, writeFileSync, readSync, openSync, closeSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join, resolve } from 'node:path';
import galleryConfig from '../gallery.config.mjs';

const HERE = dirname(fileURLToPath(import.meta.url));
const WEBSITE_ROOT = resolve(HERE, '..');

/** Read a PNG's pixel dimensions straight from the IHDR header (bytes 16..24), no dependency. */
function pngSize(file) {
  try {
    const fd = openSync(file, 'r');
    const buf = Buffer.alloc(24);
    readSync(fd, buf, 0, 24, 0);
    closeSync(fd);
    // 0x89 'PNG' magic, then IHDR at offset 16 = width (BE u32), 20 = height (BE u32).
    if (buf.readUInt32BE(0) !== 0x89504e47) return null;
    return { width: buf.readUInt32BE(16), height: buf.readUInt32BE(20) };
  } catch {
    return null;
  }
}

/** Locate one (shot, variant) PNG: the variant's directories in order, then flat (legacy). */
function findVariantShot(artifactDir, shotId, variant) {
  if (!existsSync(artifactDir)) return null;
  const named = `${shotId}.png`;
  for (const dir of variant.dirs) {
    const p = join(artifactDir, dir, named);
    if (existsSync(p)) return p;
  }
  if (variant.dirs.includes('default')) {
    const flat = join(artifactDir, named);
    if (existsSync(flat)) return flat;
  }
  return null;
}

/** Fetch a suite's published screenshot index (gallery.json) and key its entries by
 *  `(shot, platform, variant)` for the assembly loop. Returns null on any failure — the caller
 *  falls back to CI artifacts, so an unreachable site degrades rather than fails the build. */
async function fetchRemoteIndex(url, log) {
  try {
    const res = await fetch(url, { signal: AbortSignal.timeout(15_000) });
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const data = await res.json();
    if (!Array.isArray(data.screenshots)) throw new Error('no screenshots[] in the index');
    const byKey = new Map();
    for (const e of data.screenshots) {
      if (!e.url || !e.shot || !e.platform || !e.variant) continue;
      byKey.set(`${e.shot}|${e.platform}|${e.variant.toLowerCase()}`, e);
    }
    log(`using ${byKey.size} published screenshot(s) from ${url}`);
    return byKey;
  } catch (err) {
    log(`could not fetch ${url} (${err.message ?? err}) — falling back to CI artifacts`);
    return null;
  }
}

/** One (shot, platform, variant) from the remote index, trying the variant's directory names
 *  the way the artifact probe does (the index stores the CI `--variant` spelling verbatim). */
function findRemoteShot(remote, shotId, platformId, variant) {
  for (const dir of variant.dirs) {
    const e = remote.get(`${shotId}|${platformId}|${dir.toLowerCase()}`);
    if (e) return e;
  }
  return null;
}

/**
 * @param {{ artifactsDir?: string, quiet?: boolean }} [opts]
 * @returns {Promise<{ hasArtifacts: boolean, manifestPath: string, unreadable: string[], hidden: string[] }>}
 */
export async function assembleGallery(opts = {}) {
  const artifactsDir = resolve(WEBSITE_ROOT, opts.artifactsDir ?? process.env.GALLERY_ARTIFACTS_DIR ?? 'artifacts');
  const publicGallery = join(WEBSITE_ROOT, 'public', 'gallery');
  const dataDir = join(WEBSITE_ROOT, 'src', 'data');
  const log = (m) => opts.quiet || console.log(`[gallery] ${m}`);

  // Fresh output every run (stale screenshots must not linger).
  rmSync(publicGallery, { recursive: true, force: true });
  mkdirSync(dataDir, { recursive: true });

  // A suite with a published index (suite.metadata) references THOSE hosted screenshots —
  // absolute URLs straight into the manifest, no bytes copied into this site. Fetch failure
  // falls back to the CI-artifact path below, so the build never hinges on another site.
  const remoteBySuite = new Map();
  for (const suite of galleryConfig.suites) {
    if (suite.metadata) {
      remoteBySuite.set(suite.id, await fetchRemoteIndex(suite.metadata, log));
    }
  }

  let realShots = 0;
  const unreadable = [];
  /** `<suite>/<platform>` for every column dropped for want of captures (reported by the caller). */
  const hidden = [];
  const suites = galleryConfig.suites.map((suite) => {
    const remote = remoteBySuite.get(suite.id) ?? null;
    const suitePlatforms = suite.platforms
      .map((platformId) => galleryConfig.platforms.find((p) => p.id === platformId))
      .filter(Boolean);
    const captureCount = new Map(suitePlatforms.map((p) => [p.id, 0]));

    // SHOT-major: one entry per curated shot, holding every platform's variant set.
    const shots = suite.shots.map((shot) => {
      const byPlatform = suitePlatforms.map((platform) => {
        const artifactName = suite.artifactPattern.replace('{platform}', platform.id);
        const artifactDir = join(artifactsDir, artifactName);
        const variants = {};
        for (const variant of suite.variants) {
          // Published index first: the tile references the app site's own hosted image.
          if (remote) {
            const e = findRemoteShot(remote, shot.id, platform.id, variant);
            if (!e) continue;
            realShots += 1;
            variants[variant.id] = {
              src: e.url,
              width: e.width ?? undefined,
              height: e.height ?? undefined,
            };
            continue;
          }
          const found = findVariantShot(artifactDir, shot.id, variant);
          if (!found) continue;
          const rel = join('gallery', suite.id, platform.id, variant.id, `${shot.id}.png`);
          const dest = join(WEBSITE_ROOT, 'public', rel);
          mkdirSync(dirname(dest), { recursive: true });
          copyFileSync(found, dest);
          const size = pngSize(dest);
          if (!size) {
            // A zero-byte or non-PNG file (a screenshot step that failed on an emulator still
            // leaves one behind) would otherwise ship as a tile the browser can't decode. Drop it
            // so the shot falls back to another variant, or to its placeholder.
            rmSync(dest);
            unreadable.push(found);
            log(`skipping unreadable capture ${found}`);
            continue;
          }
          realShots += 1;
          variants[variant.id] = {
            src: rel.split('\\').join('/'), // POSIX for URLs, even on Windows runners
            width: size.width,
            height: size.height,
          };
        }
        const captured = Object.keys(variants).length > 0;
        if (captured) captureCount.set(platform.id, captureCount.get(platform.id) + 1);
        return {
          platform: platform.id,
          placeholder: !captured,
          variants,
        };
      });
      return { id: shot.id, label: shot.label, source: shot.source ?? null, byPlatform };
    });

    // A platform that captured NOTHING gets no column: a strip of "preview" placeholders down the
    // whole page says only that a CI leg produced no artifact, which is not what a reader came for.
    // The one exception is a build with no artifacts AT ALL (a local preview, `hasArtifacts` false
    // below) — there the placeholders ARE the point, so every column stays and the layout can be
    // seen without downloading anything.
    const withShots = suitePlatforms.filter((p) => (captureCount.get(p.id) ?? 0) > 0);
    const shownPlatforms = withShots.length > 0 ? withShots : suitePlatforms;
    const shown = new Set(shownPlatforms.map((p) => p.id));
    const dropped = suitePlatforms.filter((p) => !shown.has(p.id));
    if (dropped.length > 0) {
      // Never silently: a column vanishing from the gallery is a CI leg that stopped delivering,
      // and the build log is where that gets noticed.
      log(`hiding ${dropped.length} column(s) with no captures: ${dropped.map((p) => p.id).join(', ')}`);
      for (const p of dropped) hidden.push(`${suite.id}/${p.id}`);
    }
    // Tiles follow the columns, so the page's row/column indices (the lightbox's 2-D navigation)
    // stay contiguous and every tile's platform is still in `platforms`.
    const shownShots =
      dropped.length === 0
        ? shots
        : shots.map((s) => ({ ...s, byPlatform: s.byPlatform.filter((t) => shown.has(t.platform)) }));

    return {
      id: suite.id,
      label: suite.label,
      blurb: suite.blurb,
      // Which repository this suite's `source` paths are relative to; absent means this one, so
      // the page falls back to `site.repo`. The showcase moved out to daybrite/Day-Showcase, and
      // without carrying this through, every row header would link into a path daybrite/day no
      // longer has.
      sourceRepo: suite.sourceRepo ?? null,
      hero: suite.hero,
      variants: suite.variants.map(({ id, label }) => ({ id, label })),
      platforms: shownPlatforms.map((p) => ({
        id: p.id,
        label: p.label,
        os: p.os,
        toolkit: p.toolkit,
        captured: (captureCount.get(p.id) ?? 0) > 0,
        shotCount: captureCount.get(p.id) ?? 0,
      })),
      // The columns this run had nothing for, by id — the page shows none of them; the field
      // exists so a consumer can say what is missing rather than having to diff against the config.
      hidden: dropped.map((p) => p.id),
      shots: shownShots,
    };
  });

  const hasArtifacts = realShots > 0;
  const manifest = {
    // Only stamp a time when there is real content, to keep placeholder builds reproducible.
    generatedAt: hasArtifacts ? new Date().toISOString() : null,
    hasArtifacts,
    suites,
  };
  const manifestPath = join(dataDir, 'gallery-manifest.json');
  writeFileSync(manifestPath, JSON.stringify(manifest, null, 2) + '\n');

  const capturedPlatforms = suites.reduce((n, s) => n + s.platforms.filter((p) => p.captured).length, 0);
  log(
    hasArtifacts
      ? `assembled ${realShots} screenshot(s) across ${capturedPlatforms} platform-suite(s) from ${artifactsDir}`
      : `no artifacts under ${artifactsDir} — emitted placeholders for every shot (local build)`,
  );
  return { hasArtifacts, manifestPath, unreadable, hidden };
}

// Standalone entry point.
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const arg = process.argv[2];
  await assembleGallery(arg ? { artifactsDir: arg } : {});
}
