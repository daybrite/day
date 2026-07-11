// Assemble the screenshots gallery from CI artifacts into a static manifest + copied images.
//
// Inputs  : `<artifactsDir>/screenshots-<platform>/<variant>/<shot>.png` (from `download-artifact
//           pattern: screenshots-*`), described by gallery.config.mjs. Variants are the themed /
//           localized capture sets CI produces per platform (light / dark / fr today).
// Outputs : `public/gallery/<suite>/<platform>/<variant>/<shot>.png` (copied static assets)
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

/**
 * @param {{ artifactsDir?: string, quiet?: boolean }} [opts]
 * @returns {{ hasArtifacts: boolean, manifestPath: string }}
 */
export function assembleGallery(opts = {}) {
  const artifactsDir = resolve(WEBSITE_ROOT, opts.artifactsDir ?? process.env.GALLERY_ARTIFACTS_DIR ?? 'artifacts');
  const publicGallery = join(WEBSITE_ROOT, 'public', 'gallery');
  const dataDir = join(WEBSITE_ROOT, 'src', 'data');
  const log = (m) => opts.quiet || console.log(`[gallery] ${m}`);

  // Fresh output every run (stale screenshots must not linger).
  rmSync(publicGallery, { recursive: true, force: true });
  mkdirSync(dataDir, { recursive: true });

  let realShots = 0;
  const suites = galleryConfig.suites.map((suite) => {
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
          const found = findVariantShot(artifactDir, shot.id, variant);
          if (!found) continue;
          const rel = join('gallery', suite.id, platform.id, variant.id, `${shot.id}.png`);
          const dest = join(WEBSITE_ROOT, 'public', rel);
          mkdirSync(dirname(dest), { recursive: true });
          copyFileSync(found, dest);
          realShots += 1;
          const size = pngSize(dest) ?? {};
          variants[variant.id] = {
            src: rel.split('\\').join('/'), // POSIX for URLs, even on Windows runners
            width: size.width ?? null,
            height: size.height ?? null,
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

    return {
      id: suite.id,
      label: suite.label,
      blurb: suite.blurb,
      hero: suite.hero,
      variants: suite.variants.map(({ id, label }) => ({ id, label })),
      platforms: suitePlatforms.map((p) => ({
        id: p.id,
        label: p.label,
        os: p.os,
        toolkit: p.toolkit,
        captured: (captureCount.get(p.id) ?? 0) > 0,
        shotCount: captureCount.get(p.id) ?? 0,
      })),
      shots,
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
  return { hasArtifacts, manifestPath };
}

// Standalone entry point.
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const arg = process.argv[2];
  assembleGallery(arg ? { artifactsDir: arg } : {});
}
