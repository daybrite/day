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

import { LinkChecker } from 'linkinator';
import { readdirSync } from 'node:fs';
import { join, relative } from 'node:path';

const DIST = 'dist';

function htmlFiles(dir, base = dir) {
  let out = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const p = join(dir, entry.name);
    if (entry.isDirectory()) out = out.concat(htmlFiles(p, base));
    else if (entry.name.endsWith('.html')) out.push(relative(base, p));
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

const checker = new LinkChecker();
const broken = [];
checker.on('link', (link) => {
  if (link.state === 'BROKEN') broken.push(link);
});

const result = await checker.check({
  path: paths,
  serverRoot: DIST,
  recurse: true,
  linksToSkip: ['^https?://(?!localhost)'],
});

if (broken.length === 0) {
  console.log(
    `linkcheck: OK — ${result.links.length} links across ${paths.length} pages, no broken internal links.`,
  );
  process.exit(0);
}

console.error(`linkcheck: ${broken.length} broken internal link(s) across ${paths.length} pages:\n`);
for (const b of broken) {
  console.error(`  [${b.status}] ${b.url}\n        on: ${b.parent}`);
}
process.exit(1);
