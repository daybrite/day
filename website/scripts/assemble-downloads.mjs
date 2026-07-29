// Assemble the /showcase/ downloads: copy each primary target's packaged showcase app out of
// the CI artifacts and write the manifest the page renders (file name, byte size, SHA-256).
//
// Sources: `website/artifacts/showcase-dist-<combo>/…` — the per-combo `day pack` outputs the
// platform jobs upload on every push (the same artifacts the release lane publishes on tags).
// Local builds usually have none: the manifest is still written (empty), and the page shows
// its "packaged by CI" placeholders — the same resilience as the gallery.
//
// Outputs: `public/downloads/<combo>/<file>` (static passthrough) and
//          `src/data/downloads.json` (consumed by src/pages/showcase.astro).

import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync, copyFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join, resolve } from 'node:path';

const HERE = dirname(fileURLToPath(import.meta.url));
const WEBSITE_ROOT = resolve(HERE, '..');

// The primary platform-toolkit pairs, in display order. The secondary desktop combos
// (macos-gtk/qt, windows-gtk/qt) are deliberately absent even when CI uploads them.
const PRIMARY = [
  'macos-appkit',
  'ios-uikit',
  'android-mdc',
  'windows-xaml',
  'linux-gtk',
  'linux-qt',
  'harmony-arkui',
];

// Installable payloads only — pack.json/log droppings in the dist dirs are skipped.
const EXTENSIONS = ['.dmg', '.ipa', '.zip', '.apk', '.aab', '.flatpak', '.msix', '.exe', '.hap'];

/**
 * @param {{ quiet?: boolean }} [opts]
 * @returns {{ platforms: number, files: number }}
 */
export function assembleDownloads(opts = {}) {
  const outDir = join(WEBSITE_ROOT, 'public', 'downloads');
  const manifestPath = join(WEBSITE_ROOT, 'src', 'data', 'downloads.json');
  const log = (m) => opts.quiet || console.log(`[downloads] ${m}`);

  rmSync(outDir, { recursive: true, force: true });
  const platforms = [];
  for (const combo of PRIMARY) {
    const src = join(WEBSITE_ROOT, 'artifacts', `showcase-dist-${combo}`);
    if (!existsSync(src)) continue;
    const files = [];
    for (const name of readdirSync(src).sort()) {
      const path = join(src, name);
      if (!statSync(path).isFile()) continue;
      if (!EXTENSIONS.some((e) => name.toLowerCase().endsWith(e))) continue;
      const bytes = readFileSync(path);
      mkdirSync(join(outDir, combo), { recursive: true });
      copyFileSync(path, join(outDir, combo, name));
      files.push({
        name,
        bytes: bytes.length,
        sha256: createHash('sha256').update(bytes).digest('hex'),
      });
    }
    if (files.length > 0) platforms.push({ id: combo, files });
  }
  writeFileSync(manifestPath, JSON.stringify({ platforms }, null, 2));
  const total = platforms.reduce((n, p) => n + p.files.length, 0);
  log(
    platforms.length > 0
      ? `assembled ${total} package(s) across ${platforms.length} platform(s)`
      : 'no showcase-dist artifacts — wrote an empty manifest (placeholders will show)',
  );
  return { platforms: platforms.length, files: total };
}
