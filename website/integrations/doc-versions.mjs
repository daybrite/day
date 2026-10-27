// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
import { checkRepository } from '../scripts/doc-dependency-versions.mjs';

export default function docVersions() {
  return {
    name: 'day-documentation-versions',
    hooks: {
      'astro:config:setup': ({ command }) => {
        // Run before content rendering and gallery network work, including direct astro builds.
        if (command === 'build') checkRepository();
      },
    },
  };
}
