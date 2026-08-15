// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0

// Astro integration: assemble the screenshots gallery before Astro reads any modules.
//
// Running in `astro:config:setup` (the earliest hook, fired for both `dev` and `build`) guarantees
// `src/data/gallery-manifest.json` and `public/gallery/**` exist before the gallery page imports
// them. On CI the images come from downloaded artifacts; locally they are placeholders.
import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { assembleGallery } from '../scripts/assemble-gallery.mjs';
import { assembleHeroShots } from '../scripts/hero-shots.mjs';
import { assembleDownloads } from '../scripts/assemble-downloads.mjs';
import { groups } from '../src/lib/internal-groups.mjs';

/** Regenerate reference.md's internal-docs tables from the shared curation
 *  (src/lib/internal-groups.mjs), between the GENERATED markers. Deterministic, and only
 *  rewritten on drift, so a committed tree stays clean. */
function assembleReferenceIndex() {
  const path = fileURLToPath(new URL('../src/content/docs/reference.md', import.meta.url));
  const begin = '<!-- BEGIN GENERATED: internal-docs-index (integrations/gallery.mjs, from src/lib/internal-groups.mjs) -->';
  const end = '<!-- END GENERATED: internal-docs-index -->';
  const src = readFileSync(path, 'utf8');
  const b = src.indexOf(begin);
  const e = src.indexOf(end);
  if (b < 0 || e < 0) return false;
  const body = groups
    .map((g) => {
      const rows = g.docs
        .map(([id, note]) => `| [${id}](/docs/internal/${id}) | ${note} |`)
        .join('\n');
      return `## ${g.heading}\n\n${g.blurb}\n\n| Page | Covers |\n|---|---|\n${rows}`;
    })
    .join('\n\n');
  const next = src.slice(0, b + begin.length) + '\n' + body + '\n' + src.slice(e);
  if (next !== src) {
    writeFileSync(path, next);
    return true;
  }
  return false;
}

/** @returns {import('astro').AstroIntegration} */
export default function gallery() {
  return {
    name: 'day-gallery',
    hooks: {
      'astro:config:setup': async ({ logger }) => {
        if (assembleReferenceIndex()) logger.info('regenerated reference.md internal-docs tables');
        const { hasArtifacts, unreadable, hidden } = await assembleGallery({ quiet: true });
        logger.info(
          hasArtifacts
            ? 'assembled screenshots gallery (published indexes and/or CI artifacts)'
            : 'no published index or screenshot artifacts — gallery uses placeholders (expected offline)',
        );
        // A capture that isn't a decodable PNG is dropped rather than shipped as a broken tile —
        // say so, or a failed screenshot step downstream looks like a shot nobody ever captured.
        for (const file of unreadable) logger.warn(`dropped unreadable capture: ${file}`);
        // A whole column with no captures is hidden rather than filled with placeholders. On CI
        // that means a walkthrough job delivered nothing, which the build log has to say — a
        // quietly narrower gallery reads as a design decision instead of a broken leg.
        if (hidden.length) logger.warn(`hidden gallery column(s), no captures: ${hidden.join(', ')}`);
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
