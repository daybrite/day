// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0

// Astro integration: assemble the screenshots gallery before Astro reads any modules.
//
// Running in `astro:config:setup` (the earliest hook, fired for both `dev` and `build`) guarantees
// `src/data/gallery-manifest.json` exists before the gallery pages import it. The images come from
// each app's latest full GitHub release. Every startup checks release metadata online;
// verified asset bytes and optimized renditions are cached between builds.
import { assembleGallery } from '../scripts/assemble-gallery.mjs';
import { assembleHeroShots } from '../scripts/hero-shots.mjs';
import { assembleDownloads } from '../scripts/assemble-downloads.mjs';
import { verifyMarketingEvidence } from '../scripts/verify-marketing-evidence.mjs';

/** @returns {import('astro').AstroIntegration} */
export default function gallery() {
  return {
    name: 'day-gallery',
    hooks: {
      'astro:config:setup': async ({ logger }) => {
        // /examples retains the tutorial's recorded native output.
        await verifyMarketingEvidence();
        const { apps, captures } = await assembleGallery();
        logger.info(`indexed ${captures} full-release screenshot(s) across ${apps} app(s)`);
        // Build the front-page carousel from that same release manifest.
        // Only verified, non-blank screenshots are admitted.
        const { count, lightCount, darkCount } = await assembleHeroShots({ quiet: true });
        logger.info(`hero carousel: ${lightCount} light/default and ${darkCount} dark screenshot(s)`);
        // Every app must resolve a full release before reaching this point. Deployment
        // additionally requires at least one verified hero image.
        if (process.env.DAY_REQUIRE_SHOWCASE && count === 0) {
          throw new Error('Not publishing: the front-page carousel has no verified release screenshot (DAY_REQUIRE_SHOWCASE).');
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
