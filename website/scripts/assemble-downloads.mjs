// Assemble the /showcase/ downloads: point each primary target at the showcase package attached
// to the latest GitHub release, and write the manifest the page renders (file name, byte size,
// download URL).
//
// Why the release and not this run's CI artifacts: only the release lane's macOS package is signed
// and NOTARIZED. The signing identity lives in an environment that admits `v*` tags alone, so a
// package built on a main push is unsigned by construction — and an unsigned .dmg is the one
// download here that macOS actively refuses to open. Serving the release assets keeps every card
// honest at the cost of tracking release cadence rather than main.
//
// Links are SYMBOLIC — `/releases/latest/download/<asset>`, which GitHub resolves at click time —
// so the page keeps working when the showcase releases and this site has not rebuilt. That is also
// why no SHA-256 is published: a digest read at build time would name a file the link no longer
// serves, and a wrong checksum is worse than none. The byte size is kept and presented as
// approximate, since a stale size misleads nobody about what to expect.
//
// Source : GET /repos/<owner>/<repo>/releases/latest — for the asset NAMES and sizes; the hrefs
//          are constructed, not taken from the response.
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

// Which release asset belongs to which platform card. `day pack` names every artifact
// `<stem>-<platform>-<toolkit>[-extra].<ext>` and the release lane flattens every combo's dist
// into one asset list, so the mapping keys on the combo — never on the stem, which is the app's
// and changes between apps. The extension anchors the match, which is also what keeps the
// provenance sidecars (`….dmg.buildinfo.json`) out of the cards. Order within a platform is the
// order the card lists them.
const PLATFORM_ASSETS = [
  { id: 'android-mdc', match: (n) => /-android-mdc\.(apk|aab)$/i.test(n) },
  { id: 'ios-uikit', match: (n) => /-ios-uikit\.ipa$/i.test(n) },
  { id: 'harmony-arkui', match: (n) => /-harmony-arkui\.hap$/i.test(n) },
  { id: 'macos-appkit', match: (n) => /-macos-appkit\.dmg$/i.test(n) },
  { id: 'windows-xaml', match: (n) => /-windows-xaml(-setup)?\.(msix|exe)$/i.test(n) },
  // Linux ships two formats. The card offers the AppImage — one executable carrying its own
  // toolkit, so `chmod +x` and run is the whole procedure — and falls back to the .flatpak for a
  // release predating the AppImage rather than showing an empty card.
  { id: 'linux-gtk', match: linuxMatcher('linux-gtk') },
  { id: 'linux-qt', match: linuxMatcher('linux-qt') },
];

/** Match a Linux target's preferred package: the AppImage when the release has one, else the flatpak. */
function linuxMatcher(target) {
  const appimage = new RegExp(`-${target}-.*\\.appimage$`, 'i');
  const flatpak = new RegExp(`-${target}-.*\\.flatpak$`, 'i');
  return (name, all) => (all.some((a) => appimage.test(a)) ? appimage.test(name) : flatpak.test(name));
}

/** Fetch JSON from the GitHub API, authenticated when a token is around (CI rate limits). */
async function api(path) {
  const headers = { accept: 'application/vnd.github+json' };
  const token = process.env.GITHUB_TOKEN || process.env.GH_TOKEN;
  if (token) headers.authorization = `Bearer ${token}`;
  const res = await fetch(`https://api.github.com${path}`, { headers });
  if (!res.ok) throw new Error(`GET ${path} — ${res.status} ${res.statusText}`);
  return res.json();
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

  const platforms = [];
  const missing = [];
  // The whole asset-name list is passed to each matcher: a target that ships two formats has to
  // know which ones this release actually carries before it can prefer one.
  const names = assets.map((a) => a.name);
  for (const { id, match } of PLATFORM_ASSETS) {
    const files = assets
      .filter((a) => match(a.name, names))
      .map((a) => ({
        name: a.name,
        bytes: a.size,
        // Not `a.browser_download_url`, which pins the tag this build happened to see.
        href: `https://github.com/${REPO}/releases/latest/download/${encodeURIComponent(a.name)}`,
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
