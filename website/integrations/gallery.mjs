// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0

// Astro integration: assemble the screenshots gallery before Astro reads any modules.
//
// Running in `astro:config:setup` (the earliest hook, fired for both `dev` and `build`) guarantees
// `src/data/gallery-manifest.json` and `public/gallery/**` exist before the gallery page imports
// them. On CI the images come from downloaded artifacts; locally they are placeholders.
import { assembleGallery } from '../scripts/assemble-gallery.mjs';
import { assembleHeroShots } from '../scripts/hero-shots.mjs';
import { assembleDownloads } from '../scripts/assemble-downloads.mjs';

/** @returns {import('astro').AstroIntegration} */
export default function gallery() {
  return {
    name: 'day-gallery',
    hooks: {
      'astro:config:setup': async ({ logger }) => {
        const { hasArtifacts, unreadable } = assembleGallery({ quiet: true });
        logger.info(
          hasArtifacts
            ? 'assembled screenshots gallery from artifacts'
            : 'no screenshot artifacts found — gallery uses placeholders (expected for local builds)',
        );
        // A capture that isn't a decodable PNG is dropped rather than shipped as a broken tile —
        // say so, or a failed screenshot step downstream looks like a shot nobody ever captured.
        for (const file of unreadable) logger.warn(`dropped unreadable capture: ${file}`);
        // Build the front-page hero carousel pool from the just-assembled gallery (falling back to
        // the live gallery for local previews). Only verified, non-blank screenshots are admitted.
        const { count } = await assembleHeroShots({ quiet: true });
        logger.info(`hero carousel: ${count} verified screenshot(s)`);
        // Resolve the /showcase/ downloads from the latest GitHub release — the only packages
        // that are signed and notarized. No release reachable ⇒ placeholders, like the gallery.
        const dl = await assembleDownloads({ quiet: true });
        logger.info(
          dl.tag
            ? `showcase downloads: ${dl.files} package(s) across ${dl.platforms} platform(s) from ${dl.tag}`
            : 'showcase downloads: no release reachable — the page shows placeholders',
        );
        // A platform missing from the release is usually its job having failed on the tag run.
        if (dl.missing?.length) logger.warn(`no package in ${dl.tag} for: ${dl.missing.join(', ')}`);
      },
    },
  };
}
