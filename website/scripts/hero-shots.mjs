// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0

// Build a reviewed set of gallery images for the landing carousel. The gallery index supplies
// capture URLs and dimensions; hero.config.mjs chooses the app, screen, and toolkit. Generated
// images stay local to the website, including captures supplied by a pinned release bundle.
import { existsSync, mkdirSync, rmSync, readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join, resolve } from 'node:path';
import { createHash } from 'node:crypto';
import sharp from 'sharp';
import galleryConfig from '../gallery.config.mjs';
import { heroPicks } from '../hero.config.mjs';

const WEBSITE_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const CAROUSEL_TOOLKIT = {
  'linux-gtk': 'GTK (GNOME)', 'linux-qt': 'Qt (KDE)',
  'windows-winui': 'WinUI 3', 'web-dom': 'DOM',
};

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

async function obtain(capture) {
  if (!capture?.src) return null;
  try {
    if (capture.src.startsWith('/')) {
      const publicRoot = join(WEBSITE_ROOT, 'public');
      const path = resolve(publicRoot, '.' + capture.src);
      if (!path.startsWith(publicRoot + '/')) return null;
      return readFileSync(path);
    }
    const response = await fetch(capture.src, { signal: AbortSignal.timeout(30_000) });
    return response.ok ? Buffer.from(await response.arrayBuffer()) : null;
  } catch { return null; }
}

/** @param {{ quiet?: boolean, refresh?: boolean }} [opts] */
export async function assembleHeroShots(opts = {}) {
  const outDir = join(WEBSITE_ROOT, 'public', 'hero');
  const manifestPath = join(WEBSITE_ROOT, 'src/data/hero-shots.json');
  const log = message => opts.quiet || console.log(`[hero] ${message}`);
  const gallery = JSON.parse(readFileSync(join(WEBSITE_ROOT, 'src/data/gallery-manifest.json'), 'utf8'));
  const candidates = heroPicks.map(pick => {
    const app = gallery.apps.find(app => app.id === pick.app);
    const row = app?.shots.find(shot => shot.id === pick.shot);
    const captures = row?.byColumn.find(column => column.column === pick.platform)?.captures;
    // An unthemed capture is valid for the light/default slide. Never substitute it for dark.
    const light = captures?.['light|en'] ?? captures?.['light|default'] ?? captures?.['default|en'];
    const dark = captures?.['dark|en'] ?? captures?.['dark|default'];
    return { ...pick, label: app?.label, light, dark };
  });
  // Changes to the reviewed selection or its published URLs invalidate the generated pool.
  const key = createHash('sha256').update(JSON.stringify(['curated-v1', candidates])).digest('hex');
  if (!opts.refresh && existsSync(manifestPath)) {
    try {
      const cached = JSON.parse(readFileSync(manifestPath, 'utf8'));
      if (cached.key === key && cached.shots?.length && cached.shots.every(s =>
        existsSync(join(WEBSITE_ROOT, 'public', s.src)) &&
        (!s.srcDark || existsSync(join(WEBSITE_ROOT, 'public', s.srcDark))))) {
        log(`reusing ${cached.shots.length} reviewed hero shots`);
        return { count: cached.shots.length, manifestPath };
      }
    } catch { /* Rebuild a missing or incomplete cache. */ }
  }
  mkdirSync(outDir, { recursive: true });
  const normalize = async bytes => {
    const { data, info } = await sharp(bytes)
      .resize({ width: 1000, height: 1000, fit: 'inside', withoutEnlargement: true })
      .webp({ quality: 90 }).toBuffer({ resolveWithObject: true });
    return { data, width: info.width, height: info.height };
  };
  const shots = [];
  const files = new Map();
  for (const pick of candidates) {
    const platform = galleryConfig.platforms.find(p => p.id === pick.platform);
    const bytes = await obtain(pick.light);
    if (!platform || !bytes || !(await isContentful(bytes))) {
      console.warn(`[hero] unavailable reviewed capture: ${pick.app}/${pick.platform}/${pick.shot}`);
      continue;
    }
    const stem = `${pick.app}-${pick.platform}-${pick.shot}`;
    const light = await normalize(bytes), filename = `${stem}.webp`;
    files.set(filename, light.data);
    const toolkit = CAROUSEL_TOOLKIT[pick.platform] ?? platform.toolkit;
    const entry = {
      src: `hero/${filename}`, width: light.width, height: light.height,
      app: pick.app, label: pick.label, shot: pick.shot, framed: !!pick.framed,
      gallery: `gallery/${pick.app}#${pick.shot}`,
      os: platform.os, toolkit, accent: pick.platform,
      alt: `${pick.label} on ${platform.os} with ${toolkit}`,
    };
    const darkBytes = await obtain(pick.dark);
    if (darkBytes && await isContentful(darkBytes) && await isDark(darkBytes)) {
      const dark = await normalize(darkBytes);
      // The frame must stay the same size when the site changes appearance.
      if (dark.width === light.width && dark.height === light.height) {
        const filename = `${stem}-dark.webp`;
        files.set(filename, dark.data);
        entry.srcDark = `hero/${filename}`;
      }
    }
    shots.push(entry);
  }
  // Keep the previous files until their replacements are ready.
  rmSync(outDir, { recursive: true, force: true });
  mkdirSync(outDir, { recursive: true });
  for (const [file, bytes] of files) writeFileSync(join(outDir, file), bytes);
  mkdirSync(dirname(manifestPath), { recursive: true });
  writeFileSync(manifestPath, JSON.stringify({ key, shots }, null, 2) + '\n');
  log(`verified ${shots.length} reviewed screenshots from ${new Set(shots.map(s => s.app)).size} apps`);
  return { count: shots.length, manifestPath };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  await assembleHeroShots({ refresh: process.argv.includes('--refresh') });
}
