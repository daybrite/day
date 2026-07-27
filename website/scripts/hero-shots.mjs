// Assemble the front-page hero carousel's screenshot pool.
//
// The hero shows real Day Showcase UI, one native rendering per platform, cross-fading with an
// animated platform caption. This script gathers the candidate images and — per the design — only
// admits a screenshot that (a) actually exists and (b) is NOT blank/solid (a capture that failed
// or a placeholder). Verification uses `sharp`'s per-channel standard deviation: a blank or
// single-colour image has ~0 stdev, real UI has plenty.
//
// Sources, in order of preference per (platform, shot):
//   1. `public/gallery/<suite>/<platform>/<variant>/<shot>.png` — the real CI artifacts, already
//      assembled by scripts/assemble-gallery.mjs (so production/CI needs no network).
//   2. `https://daybrite.dev/gallery/<suite>/<platform>/<variant>/<shot>.png` — the live gallery,
//      downloaded when local artifacts are placeholders (local dev previews get real images "to
//      build the page").
//
// Each admitted shot is the LIGHT capture; when the matching `dark` capture exists (and passes the
// same non-blank check plus a predominantly-dark check — see `isDark`) it is emitted alongside so
// the carousel can follow the site theme. A shot is never admitted on its dark capture alone, and
// `srcDark` is only written for files that exist and verified — the carousel falls back to light
// rather than pointing at a missing or defective image.
//
// Outputs : `public/hero/<platform>-<shot>.png`       (verified images, copied as static assets)
//           `public/hero/<platform>-<shot>-dark.png`  (only where a dark capture was verified)
//           `src/data/hero-shots.json`                (consumed by src/components/HeroCarousel.astro)
//
// Runnable standalone (`node scripts/hero-shots.mjs [--refresh]`) and from the Astro integration
// (integrations/gallery.mjs), after the gallery is assembled.

import { existsSync, mkdirSync, rmSync, readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join, resolve } from 'node:path';
import sharp from 'sharp';
import galleryConfig from '../gallery.config.mjs';

const HERE = dirname(fileURLToPath(import.meta.url));
const WEBSITE_ROOT = resolve(HERE, '..');
const LIVE_ORIGIN = 'https://daybrite.dev';

// The suite whose screenshots feed the hero (the one real sample app).
const SUITE_ID = galleryConfig.suites[0]?.id ?? 'showcase';

// The carousel shows only the "primary" target per OS — one canonical native toolkit each — not the
// secondary/cross ports (macos-qt, macos-gtk, windows-gtk, windows-qt) that also build in CI. Order
// here is the default (the client reshuffles anyway).
const PRIMARY_PLATFORMS = [
  'macos-appkit',
  'windows-winui',
  'linux-gtk',
  'linux-qt',
  'android-mdc',
  'ios-uikit',
];
// Signature baked into the manifest so the fast-path rebuilds when the primary set changes.
// The marker is the manifest format — bump it when the output shape or the caption/accent
// fields change so stale caches rebuild (v3: desktop-anchored toolkit names + per-target accents).
const PRIMARY_KEY = ['v3', ...PRIMARY_PLATFORMS].join(',');

// Carousel caption names — shorter than the gallery's toolkit strings, anchored to the desktop
// each toolkit is known by. Platforms not listed keep their gallery toolkit string.
const CAROUSEL_TOOLKIT = {
  'linux-gtk': 'GTK (GNOME)',
  'linux-qt': 'Qt (KDE)',
  'windows-winui': 'WinUI',
};

// Shots tried per platform, richest-looking UI first. The first (up to MAX_PER_PLATFORM) that pass
// verification are admitted, so a platform missing "home" still contributes via "controls", etc.
const PREFERRED_SHOTS = [
  'home', 'controls', 'canvas', 'stack-detail', 'text', 'tabs-one', 'resources', 'system', 'tweaks',
];
const MAX_PER_PLATFORM = 2;


/** True when the image decodes and is not blank/solid (real UI has high channel variance). */
async function isContentful(buf) {
  try {
    const img = sharp(buf, { failOn: 'none' });
    const meta = await img.metadata();
    if (!meta.width || !meta.height || meta.width < 80 || meta.height < 80) return false;
    const stats = await img.stats();
    const maxStdev = Math.max(...stats.channels.map((c) => c.stdev));
    // Solid/blank captures sit at ~0; genuine screenshots are well above. 8 is a comfortable floor.
    return maxStdev > 8;
  } catch {
    return false;
  }
}

/** True when the capture is predominantly dark. Guards against a "dark" artifact whose content
 *  pane rendered white (a capture defect seen on some toolkits) shipping as a dark hero shot.
 *  Genuine dark captures measure ~25–55 mean luminance, the defective ones ~145, and light sets
 *  ~230+ — 100 splits those populations with a wide margin either way. */
async function isDark(buf) {
  try {
    const stats = await sharp(buf, { failOn: 'none' }).stats();
    const mean = stats.channels.slice(0, 3).reduce((sum, c) => sum + c.mean, 0) / 3;
    return mean < 100;
  } catch {
    return false;
  }
}

/** Fetch a (platform, shot, theme) PNG: prefer the locally-assembled artifact, else the live
 *  gallery. Screenshots are per-variant since the themed capture sets landed; only light keeps
 *  the pre-variant flat path as a live-fallback for the transition window (those captures were
 *  light — a flat file must never be passed off as dark). */
async function obtain(platformId, shot, theme) {
  const rels =
    theme === 'dark'
      ? [`gallery/${SUITE_ID}/${platformId}/dark/${shot}.png`]
      : [
          `gallery/${SUITE_ID}/${platformId}/light/${shot}.png`,
          `gallery/${SUITE_ID}/${platformId}/${shot}.png`, // pre-variant layout (live fallback)
        ];
  for (const rel of rels) {
    const local = join(WEBSITE_ROOT, 'public', rel);
    if (existsSync(local)) return readFileSync(local);
  }
  for (const rel of rels) {
    try {
      const res = await fetch(`${LIVE_ORIGIN}/${rel}`);
      if (res.ok) return Buffer.from(await res.arrayBuffer());
    } catch {
      // try the next form
    }
  }
  return null;
}

/**
 * @param {{ quiet?: boolean, refresh?: boolean }} [opts]
 * @returns {Promise<{ count: number, manifestPath: string }>}
 */
export async function assembleHeroShots(opts = {}) {
  const outDir = join(WEBSITE_ROOT, 'public', 'hero');
  const manifestPath = join(WEBSITE_ROOT, 'src', 'data', 'hero-shots.json');
  const log = (m) => opts.quiet || console.log(`[hero] ${m}`);

  // Fast path: reuse a previous run's verified images unless a refresh is forced. Keeps `astro dev`
  // restarts instant and avoids re-downloading on every build once the pool exists.
  if (!opts.refresh && existsSync(manifestPath)) {
    try {
      const cached = JSON.parse(readFileSync(manifestPath, 'utf8'));
      if (
        cached.key === PRIMARY_KEY &&
        Array.isArray(cached.shots) &&
        cached.shots.length > 0 &&
        cached.shots.every(
          (s) =>
            existsSync(join(WEBSITE_ROOT, 'public', s.src)) &&
            (!s.srcDark || existsSync(join(WEBSITE_ROOT, 'public', s.srcDark))),
        )
      ) {
        log(`reusing ${cached.shots.length} cached hero shot(s) (pass --refresh to rebuild)`);
        return { count: cached.shots.length, manifestPath };
      }
    } catch {
      /* fall through and rebuild */
    }
  }

  rmSync(outDir, { recursive: true, force: true });
  mkdirSync(outDir, { recursive: true });
  mkdirSync(dirname(manifestPath), { recursive: true });

  // Normalise for the web: cap the longest side (the iOS captures are ~2600px tall) so the hero
  // stays light, and re-encode PNG. Never enlarge — desktop shots are already ~1000px.
  const normalise = (buf) =>
    sharp(buf, { failOn: 'none' })
      .resize({ width: 1000, height: 1000, fit: 'inside', withoutEnlargement: true })
      .png({ compressionLevel: 9 })
      .toBuffer();

  const shots = [];
  const platforms = PRIMARY_PLATFORMS
    .map((id) => galleryConfig.platforms.find((p) => p.id === id))
    .filter(Boolean);
  for (const platform of platforms) {
    let taken = 0;
    for (const shot of PREFERRED_SHOTS) {
      if (taken >= MAX_PER_PLATFORM) break;
      const buf = await obtain(platform.id, shot, 'light');
      if (!buf) continue;
      if (!(await isContentful(buf))) continue;
      const file = `${platform.id}-${shot}.png`;
      writeFileSync(join(outDir, file), await normalise(buf));
      const toolkit = CAROUSEL_TOOLKIT[platform.id] ?? platform.toolkit;
      const entry = {
        src: `hero/${file}`,
        // The gallery shot id — the carousel links each image to its row anchor (`/gallery#<shot>`).
        shot,
        os: platform.os,
        toolkit,
        // The per-target accent token suffix (--pf-<target>, global.css): linux-gtk and
        // linux-qt glow their own colors, not one shared "linux".
        accent: platform.id,
        alt: `The Day Showcase app running natively on ${platform.os} with ${toolkit}`,
      };
      // The matching dark capture, when it exists, is equally non-blank, and actually reads as
      // dark. `srcDark` is only written for a verified file — the carousel falls back to light
      // otherwise.
      const darkBuf = await obtain(platform.id, shot, 'dark');
      if (darkBuf && (await isContentful(darkBuf)) && (await isDark(darkBuf))) {
        const darkFile = `${platform.id}-${shot}-dark.png`;
        writeFileSync(join(outDir, darkFile), await normalise(darkBuf));
        entry.srcDark = `hero/${darkFile}`;
      }
      shots.push(entry);
      taken += 1;
    }
    if (taken === 0) log(`no non-blank screenshot found for ${platform.id} — skipped`);
  }

  writeFileSync(manifestPath, JSON.stringify({ key: PRIMARY_KEY, shots }, null, 2) + '\n');
  const platformCount = new Set(shots.map((s) => `${s.os}/${s.toolkit}`)).size;
  const darkCount = shots.filter((s) => s.srcDark).length;
  log(
    `verified ${shots.length} hero shot(s) across ${platformCount} native rendering(s), ` +
      `${darkCount} with a dark capture`,
  );
  return { count: shots.length, manifestPath };
}

// Standalone entry point.
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const refresh = process.argv.includes('--refresh');
  await assembleHeroShots({ refresh });
}
