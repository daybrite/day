// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0

// Astro integration: assemble the screenshots gallery before Astro reads any modules.
//
// Running in `astro:config:setup` (the earliest hook, fired for both `dev` and `build`) guarantees
// `src/data/gallery-manifest.json` exists before the gallery pages import it. The images come from
// the index each Day app's own site publishes, so this needs the network (or a warm `.cache/`).
import { assembleGallery } from '../scripts/assemble-gallery.mjs';
import { assembleHeroShots } from '../scripts/hero-shots.mjs';
import { assembleDownloads } from '../scripts/assemble-downloads.mjs';
import { verifyMarketingEvidence } from '../scripts/verify-marketing-evidence.mjs';

const SHOWCASE_APP_ID = 'Day-Showcase';

/** @returns {import('astro').AstroIntegration} */
export default function gallery() {
  return {
    name: 'day-gallery',
    hooks: {
      'astro:config:setup': async ({ logger }) => {
        // /examples retains the tutorial's recorded native output.
        await verifyMarketingEvidence();
        const { apps, captures, dropped, reasons, stale } = await assembleGallery({ quiet: true });
        logger.info(
          apps > 0
            ? `indexed ${captures} published screenshot(s) across ${apps} app(s)`
            : 'no app index could be read — the gallery is empty (expected offline)',
        );
        // An app left out is a site that could not be reached and has never been cached here. Say
        // so: a gallery that is quietly one app short reads as a curation decision.
        if (dropped.length) logger.warn(`app(s) left out of the gallery: ${dropped.join(', ')}`);
        // A cached index may be months old. The page says so too, but the build log is where a
        // publishing pipeline that stopped running gets noticed.
        if (stale.length) logger.warn(`app(s) shown from a cached index: ${stale.join(', ')}`);
        // Build the front-page hero carousel pool from the just-assembled gallery (falling back to
        // the live gallery for local previews). Only verified, non-blank screenshots are admitted.
        const { count } = await assembleHeroShots({ quiet: true });
        logger.info(`hero carousel: ${count} verified screenshot(s)`);
        // Publishing still requires the Showcase index for /showcase and its gallery, plus
        // a nonempty hero pool. The carousel can include other apps independently.
        //
        // ci.yml's `website` job sets DAY_REQUIRE_SHOWCASE on every run that deploys. `astro dev`, a local
        // build and a pull request's build go on rendering without the network, as they always
        // have: none of them publishes anything.
        if (process.env.DAY_REQUIRE_SHOWCASE) {
          const problems = [];
          if (dropped.includes(SHOWCASE_APP_ID)) {
            problems.push(`${SHOWCASE_APP_ID}'s gallery could not be built (${reasons[SHOWCASE_APP_ID] ?? 'no usable index'})`);
          } else if (stale.includes(SHOWCASE_APP_ID)) {
            // A cached index links images on a site that just failed to answer.
            problems.push(`${SHOWCASE_APP_ID}'s gallery is only a cached index — its site did not answer this build`);
          }
          if (count === 0) problems.push('the front-page carousel has no verified screenshot');
          if (problems.length) {
            throw new Error(
              `not building a site to publish: ${problems.join('; ')}. The last deploy stays live; ` +
                'rebuild once the app site serves its gallery again (DAY_REQUIRE_SHOWCASE).',
            );
          }
        }
        const downloads = await assembleDownloads({ quiet: true });
        logger.info(downloads.tag
          ? `showcase downloads: ${downloads.files} packages from ${downloads.tag}`
          : 'showcase downloads: no release reachable; package placeholders shown');
        if (downloads.missing.length) logger.warn(`no package in ${downloads.tag} for: ${downloads.missing.join(', ')}`);
      },
    },
  };
}
