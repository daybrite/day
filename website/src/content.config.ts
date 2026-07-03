import { defineCollection, z } from 'astro:content';
import { glob } from 'astro/loaders';

// Docs live as markdown under src/content/docs/. The Content Layer `glob` loader is the
// Astro-idiomatic way to turn a folder of markdown into a typed, queryable collection.
const docs = defineCollection({
  loader: glob({ pattern: '**/*.md', base: './src/content/docs' }),
  schema: z.object({
    title: z.string(),
    description: z.string(),
    // Sidebar order (ascending). Frontmatter is the single source of truth for doc ordering.
    order: z.number().default(99),
  }),
});

export const collections = { docs };
