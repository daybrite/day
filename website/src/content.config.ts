import { defineCollection, z } from 'astro:content';
import { glob } from 'astro/loaders';

// Docs live as markdown under src/content/docs/. The Content Layer `glob` loader is the
// Astro-idiomatic way to turn a folder of markdown into a typed, queryable collection.
// `.mdx` is included so a page can embed a component (getting-started's install picker).
const docs = defineCollection({
  loader: glob({ pattern: '**/*.{md,mdx}', base: './src/content/docs' }),
  schema: z.object({
    title: z.string(),
    description: z.string(),
    // Sidebar order (ascending). Frontmatter is the single source of truth for doc ordering.
    order: z.number().default(99),
    // Sidebar section heading. Sections appear in the order their first page appears
    // (by `order`), so frontmatter stays the single source of truth for the whole nav.
    section: z.string().default('Docs'),
  }),
});

// The framework's internal reference docs. These are the repo's top-level `docs/*.md`, symlinked
// into `src/content/internal/` so `docs/` stays the single source of truth (see the symlinks). They
// have NO frontmatter — each opens with an `# H1` — so the schema is fully lenient: every field is
// optional and titles are derived from the filename / first heading at render time.
const internal = defineCollection({
  loader: glob({ pattern: '**/*.md', base: './src/content/internal' }),
  schema: z.object({
    title: z.string().optional(),
    description: z.string().optional(),
    order: z.number().optional(),
  }),
});

export const collections = { docs, internal };
