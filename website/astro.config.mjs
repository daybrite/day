// @ts-check
import { defineConfig } from 'astro/config';
import gallery from './integrations/gallery.mjs';

// Deployed to GitHub Pages on the custom domain https://daybrite.dev. A custom apex domain serves
// the repo at the root, so there is no base path (public/CNAME pins the domain). The `gallery`
// integration assembles the screenshots gallery from CI artifacts (or placeholders locally) before
// every dev/build; see integrations/gallery.mjs.
export default defineConfig({
  site: 'https://daybrite.dev',
  trailingSlash: 'ignore',
  integrations: [gallery()],
  markdown: {
    // Shiki (build-time, zero client JS) for docs code fences; matches the CodeSample component.
    shikiConfig: { theme: 'night-owl', wrap: false },
  },
});
