// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0

import { defineCollection, z } from 'astro:content';
import { glob } from 'astro/loaders';

// Docs live as markdown under src/content/docs/. The Content Layer `glob` loader is the
// Astro-idiomatic way to turn a folder of markdown into a typed, queryable collection.
// `.mdx` is included so a page can embed components such as the platform setup picker.
const docs = defineCollection({
  loader: glob({ pattern: '**/*.{md,mdx}', base: './src/content/docs' }),
  schema: z.object({
    title: z.string(),
    // Sidebar label, when the page title is too long for a 232px column. The page itself, the
    // browser tab, the docs index card and the prev/next pager all keep `title`.
    navTitle: z.string().optional(),
    description: z.string(),
    // Sequence within tutorials; other navigation groups sort by their sidebar labels.
    order: z.number().default(99),
    // Retained editorial category; content-routes.mjs owns sidebar grouping and placement.
    section: z.string().default('Docs'),
  }),
});

// Canonical repository docs, including nested design and history records, publish through
// symlinks. Front matter is optional: metadata falls back to the first heading and body.
const internal = defineCollection({
  loader: glob({ pattern: '**/*.md', base: './src/content/internal' }),
  schema: z.object({
    title: z.string().optional(),
    navTitle: z.string().optional(),
    description: z.string().optional(),
    order: z.number().optional(),
  }),
});

// Canonical contributor instructions and standards also publish through symlinks.
const repository = defineCollection({
  loader: glob({ pattern: '**/*.md', base: './src/content/repository' }),
  schema: z.object({ title: z.string().optional(), description: z.string().optional(), order: z.number().optional() }),
});

export const collections = { docs, internal, repository };
