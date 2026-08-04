// Assemble the /showcase/ downloads: point each primary target at the showcase package attached
// to the latest GitHub release, and write the manifest the page renders (file name, byte size,
// SHA-256, download URL).
//
// Why the release and not this run's CI artifacts: only the release lane's macOS package is signed
// and NOTARIZED. The signing identity lives in an environment that admits `v*` tags alone, so a
// package built on a main push is unsigned by construction — and an unsigned .dmg is the one
// download here that macOS actively refuses to open. Serving the release assets keeps every card
// honest at the cost of tracking release cadence rather than main.
//
// Source : GET /repos/<owner>/<repo>/releases/latest, plus the release's own SHA256SUMS asset for
//          the digests (authoritative — the same file `gh release` publishes and users verify).
// Output : `src/data/downloads.json` (consumed by src/pages/showcase.astro).
//
// Nothing is copied into the site: the links go to GitHub, so the Pages artifact does not carry
// ~150 MB of installers. Without network (local builds, or a rate-limited API) the manifest is
// written empty and the page shows its placeholders — the same resilience as the gallery.

import { rmSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join, resolve } from 'node:path';

const HERE = dirname(fileURLToPath(import.meta.url));
const WEBSITE_ROOT = resolve(HERE, '..');
const REPO = process.env.DAY_RELEASE_REPO || 'daybrite/day';

// Which release asset belongs to which platform card. `day pack` names its output after the app
// title, and the release lane flattens every combo's dist into one asset list, so the mapping is
// by file name. Order within a platform is the order the card lists them.
const PLATFORM_ASSETS = [
  { id: 'android-mdc', match: (n) => /^showcase\.(apk|aab)$/i.test(n) },
  { id: 'ios-uikit', match: (n) => /\.ipa$/i.test(n) },
  { id: 'harmony-arkui', match: (n) => /\.hap$/i.test(n) },
  { id: 'macos-appkit', match: (n) => /\.dmg$/i.test(n) },
  { id: 'windows-xaml', match: (n) => /^showcase.*\.(msix|exe)$/i.test(n) },
  { id: 'linux-gtk', match: (n) => /^showcase-gtk-.*\.flatpak$/i.test(n) },
  { id: 'linux-qt', match: (n) => /^showcase-qt-.*\.flatpak$/i.test(n) },
];

/** Fetch JSON from the GitHub API, authenticated when a token is around (CI rate limits). */
async function api(path) {
  const headers = { accept: 'application/vnd.github+json' };
  const token = process.env.GITHUB_TOKEN || process.env.GH_TOKEN;
  if (token) headers.authorization = `Bearer ${token}`;
  const res = await fetch(`https://api.github.com${path}`, { headers });
  if (!res.ok) throw new Error(`GET ${path} — ${res.status} ${res.statusText}`);
  return res.json();
}

/** `name  digest` pairs from the release's SHA256SUMS asset, keyed by file name.
 *
 *  Keyed under BOTH the name the manifest records and the name GitHub serves it as: the checksums
 *  are computed on disk, where `day pack` names its output after the app title (`Day Showcase.dmg`),
 *  and GitHub rewrites spaces to dots when it accepts an asset (`Day.Showcase.dmg`). Without the
 *  second key the one notarized download on the page is the one with no checksum beside it. */
async function digests(url) {
  if (!url) return new Map();
  try {
    const res = await fetch(url, { headers: { accept: 'application/octet-stream' } });
    if (!res.ok) throw new Error(`${res.status} ${res.statusText}`);
    const text = await res.text();
    const map = new Map();
    for (const line of text.split('\n')) {
      const m = line.trim().match(/^([0-9a-f]{64})\s+\*?(.+)$/i);
      if (!m) continue;
      const [, digest, name] = m;
      map.set(name, digest.toLowerCase());
      map.set(name.replace(/\s/g, '.'), digest.toLowerCase());
    }
    return map;
  } catch {
    return new Map(); // the page renders a file without a checksum line rather than failing
  }
}

/**
 * @param {{ quiet?: boolean }} [opts]
 * @returns {{ platforms: number, files: number, tag: string | null, missing: string[] }}
 */
export async function assembleDownloads(opts = {}) {
  const manifestPath = join(WEBSITE_ROOT, 'src', 'data', 'downloads.json');
  const log = (m) => opts.quiet || console.log(`[downloads] ${m}`);
  // Packages used to be staged here and served from the site. Clear any left from a checkout that
  // predates this, so a stale installer cannot ship inside the Pages artifact.
  rmSync(join(WEBSITE_ROOT, 'public', 'downloads'), { recursive: true, force: true });

  let release = null;
  try {
    release = await api(`/repos/${REPO}/releases/latest`);
  } catch (e) {
    log(`no release data (${e.message}) — wrote an empty manifest (placeholders will show)`);
    writeFileSync(manifestPath, JSON.stringify({ release: null, platforms: [] }, null, 2) + '\n');
    return { platforms: 0, files: 0, tag: null, missing: [] };
  }

  const assets = release.assets ?? [];
  const sums = await digests(assets.find((a) => a.name === 'SHA256SUMS')?.browser_download_url);

  const platforms = [];
  const missing = [];
  for (const { id, match } of PLATFORM_ASSETS) {
    const files = assets
      .filter((a) => match(a.name))
      .map((a) => ({
        name: a.name,
        bytes: a.size,
        sha256: sums.get(a.name) ?? null,
        href: a.browser_download_url,
      }));
    if (files.length > 0) platforms.push({ id, files });
    else missing.push(id);
  }

  writeFileSync(
    manifestPath,
    JSON.stringify(
      {
        release: { tag: release.tag_name, url: release.html_url, publishedAt: release.published_at },
        platforms,
      },
      null,
      2,
    ) + '\n',
  );

  const total = platforms.reduce((n, p) => n + p.files.length, 0);
  log(`${total} package(s) across ${platforms.length} platform(s) from ${release.tag_name}`);
  // A platform the release did not ship renders its placeholder. Say which, because the cause is
  // usually a platform job that failed on the tag run rather than a deliberate omission.
  if (missing.length > 0) log(`no package in ${release.tag_name} for: ${missing.join(', ')}`);
  return { platforms: platforms.length, files: total, tag: release.tag_name, missing };
}

// Standalone entry point.
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  await assembleDownloads();
}
