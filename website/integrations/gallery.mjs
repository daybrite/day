// Astro integration: assemble the screenshots gallery before Astro reads any modules.
//
// Running in `astro:config:setup` (the earliest hook, fired for both `dev` and `build`) guarantees
// `src/data/gallery-manifest.json` and `public/gallery/**` exist before the gallery page imports
// them. On CI the images come from downloaded artifacts; locally they are placeholders.
import { assembleGallery } from '../scripts/assemble-gallery.mjs';
import { assembleHeroShots } from '../scripts/hero-shots.mjs';

/** @returns {import('astro').AstroIntegration} */
export default function gallery() {
  return {
    name: 'day-gallery',
    hooks: {
      'astro:config:setup': async ({ logger }) => {
        const { hasArtifacts } = assembleGallery({ quiet: true });
        logger.info(
          hasArtifacts
            ? 'assembled screenshots gallery from artifacts'
            : 'no screenshot artifacts found — gallery uses placeholders (expected for local builds)',
        );
        // Build the front-page hero carousel pool from the just-assembled gallery (falling back to
        // the live gallery for local previews). Only verified, non-blank screenshots are admitted.
        const { count } = await assembleHeroShots({ quiet: true });
        logger.info(`hero carousel: ${count} verified screenshot(s)`);
      },
    },
  };
}
