// @ts-check
import { defineConfig } from 'astro/config';
import mdx from '@astrojs/mdx';
import gallery from './integrations/gallery.mjs';
import rewriteInternalLinks from './plugins/rewrite-internal-links.mjs';

// Deployed to GitHub Pages on the custom domain https://daybrite.dev. A custom apex domain serves
// the repo at the root, so there is no base path (public/CNAME pins the domain). The `gallery`
// integration assembles the screenshots gallery from CI artifacts (or placeholders locally) before
// every dev/build; see integrations/gallery.mjs.
export default defineConfig({
  site: 'https://daybrite.dev',
  trailingSlash: 'ignore',
  // Minify CSS with esbuild rather than lightningcss. lightningcss mishandles the non-standard
  // `background-clip: text`: it strips the `-webkit-background-clip: text` prefix and narrows the
  // `@supports` guard, which regresses Safari/iOS and the older Chromium-based WebView /
  // QtWebEngine builds Day's own web view renders this site in (the hero gradient text rendered as
  // a filled rectangle). esbuild does not rewrite vendor prefixes or collapse `@supports`, so the
  // hand-written cross-browser gradient-text CSS ships intact.
  vite: {
    build: { cssMinify: 'esbuild' },
  },
  // mdx() lets individual docs pages pull in interactive components (e.g. the InstallPicker in
  // getting-started); plain .md remains the default for prose-only pages.
  integrations: [gallery(), mdx()],
  markdown: {
    // Shiki (build-time, zero client JS) for docs code fences; matches the CodeSample component.
    shikiConfig: { theme: 'night-owl', wrap: false },
    // Rewrite the internal reference docs' GitHub-native relative links to valid web URLs.
    rehypePlugins: [rewriteInternalLinks],
  },
});
