// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0

// Post-build HTML link checker for the built site (dist/).
//
// Validates every internal link on EVERY generated page — not just pages reachable by crawling from
// the homepage (the internal reference docs are intentionally absent from the top nav, so a crawl
// would miss them). Each page is seeded explicitly and resolution is faithful to production:
// `serverRoot: dist` serves directory pages with the same trailing-slash semantics as GitHub Pages,
// so a relative link that would 404 in the browser 404s here too.
//
// External links (http/https to any host other than the local test server) are SKIPPED: they are
// flaky in CI and outside our control. Internal (relative / root-absolute) links are checked strictly;
// a single broken one fails the build.
//
// FRAGMENTS are checked by the second pass here, not by linkinator: it clears `url.hash` before
// requesting a link, so `/docs/platforms#nope` passes as long as the page exists. Since every
// in-page anchor on this site is a generated heading id, a renamed heading silently breaks every
// link into it — which is how `docs/navigation.md` pointed at `#stacks-pushpop-navigation` for a
// heading whose id is `stack-pushpop-with-a-value-path`. The pass resolves each internal
// `…#fragment` against the ids of the page it lands on. A missing PAGE is left to linkinator, so
// each failure is reported once.

import { LinkChecker } from 'linkinator';
import { readdirSync, readFileSync } from 'node:fs';
import { join, relative, posix } from 'node:path';

const DIST = 'dist';

function htmlFiles(dir, base = dir) {
  let out = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const p = join(dir, entry.name);
    if (entry.isDirectory()) {
      // The Rust API reference is a generated bundle, not ours to lint, and it is absent in
      // local builds (dist/api, from `cargo doc` — rustdoc validates its own links).
      const rel = relative(base, p);
      if (rel === 'api') continue;
      out = out.concat(htmlFiles(p, base));
    } else if (entry.name.endsWith('.html')) out.push(relative(base, p));
  }
  return out;
}

let paths;
try {
  paths = htmlFiles(DIST);
} catch {
  console.error(`linkcheck: cannot read ${DIST}/ — run \`npm run build\` first.`);
  process.exit(2);
}
if (paths.length === 0) {
  console.error(`linkcheck: no HTML found in ${DIST}/ — run \`npm run build\` first.`);
  process.exit(2);
}

/** The URL a built file is served at: `docs/cli/index.html` -> `/docs/cli`, `404.html` -> `/404`. */
const urlOf = (file) =>
  '/' + file.replace(/index\.html$/, '').replace(/\.html$/, '').replace(/\/$/, '');

/** Every id (and legacy `<a name>`) a built page offers as an anchor target. */
function anchorsOf(html) {
  const out = new Set();
  for (const m of html.matchAll(/\sid="([^"]+)"/g)) out.add(m[1]);
  for (const m of html.matchAll(/<a[^>]+name="([^"]+)"/g)) out.add(m[1]);
  return out;
}

/** Internal `…#fragment` links whose target page exists but offers no such anchor. */
function brokenFragments(files) {
  const anchors = new Map(files.map((f) => [urlOf(f), anchorsOf(readFileSync(join(DIST, f), 'utf8'))]));
  const found = [];
  const seen = new Set();
  let checked = 0;
  for (const file of files) {
    const from = urlOf(file);
    for (const [, href] of readFileSync(join(DIST, file), 'utf8').matchAll(/href="([^"]*#[^"]*)"/g)) {
      if (/^(https?:|mailto:|tel:)/.test(href) || href.includes('/api/')) continue;
      const [target, fragment] = href.split('#');
      if (!fragment) continue; // a bare `#` is a no-op link, not an anchor
      const page = target
        ? target.startsWith('/')
          ? target.replace(/\/$/, '') || '/'
          : posix.normalize(posix.join(posix.dirname(from + '/'), target)).replace(/\/$/, '')
        : from; // same-page link
      const ids = anchors.get(page);
      checked++;
      if (!ids) continue; // the PAGE is missing — linkinator reports that, and better
      const key = `${from} ${href}`;
      if (!ids.has(decodeURIComponent(fragment)) && !seen.has(key)) {
        seen.add(key);
        found.push({ from, href });
      }
    }
  }
  return { found, checked };
}

const checker = new LinkChecker();
const broken = [];
checker.on('link', (link) => {
  if (link.state === 'BROKEN') broken.push(link);
});

const result = await checker.check({
  path: paths,
  serverRoot: DIST,
  recurse: true,
  // Skip external links (flaky, not ours) and any link INTO the generated /api/ rustdoc
  // reference, which is absent in local builds.
  linksToSkip: ['^https?://(?!localhost)', '://[^/]+/api/'],
});

const fragments = brokenFragments(paths);

if (broken.length === 0 && fragments.found.length === 0) {
  console.log(
    `linkcheck: OK — ${result.links.length} links across ${paths.length} pages ` +
      `(${fragments.checked} of them into an anchor), no broken internal links.`,
  );
  process.exit(0);
}

if (broken.length > 0) {
  console.error(`linkcheck: ${broken.length} broken internal link(s) across ${paths.length} pages:\n`);
  for (const b of broken) {
    console.error(`  [${b.status}] ${b.url}\n        on: ${b.parent}`);
  }
}
if (fragments.found.length > 0) {
  console.error(`linkcheck: ${fragments.found.length} link(s) to an anchor that does not exist:\n`);
  for (const f of fragments.found) {
    console.error(`  [no such id] ${f.href}\n        on: ${f.from}`);
  }
}
process.exit(1);
