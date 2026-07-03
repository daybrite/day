// @ts-check
import { defineConfig } from 'astro/config';
import gallery from './integrations/gallery.mjs';

// Deployed to GitHub Pages at https://daybrite.github.io/day/ — hence the `/day` base path.
// The `gallery` integration assembles the screenshots gallery from CI artifacts (or emits
// placeholders locally) before every dev/build; see integrations/gallery.mjs.
export default defineConfig({
  site: 'https://daybrite.github.io',
  base: '/day',
  trailingSlash: 'ignore',
  integrations: [gallery()],
  markdown: {
    // Shiki (build-time, zero client JS) for docs code fences; matches the CodeSample component.
    shikiConfig: { theme: 'night-owl', wrap: false },
  },
});
