// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0

// @ts-check
import { defineConfig } from 'astro/config';
import { unified } from '@astrojs/markdown-remark';
import mdx from '@astrojs/mdx';
import gallery from './integrations/gallery.mjs';
import rewriteInternalLinks from './plugins/rewrite-internal-links.mjs';
import accentTargetCode from './plugins/accent-target-code.mjs';
import tierBadge from './plugins/tier-badge.mjs';
import admonitions from './plugins/admonitions.mjs';

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
    // Rewrite the internal reference docs' GitHub-native relative links to valid web URLs;
    // accent platform-target names in prose (plugins/accent-target-code.mjs); render links to
    // the tier definitions as tier badges (plugins/tier-badge.mjs); box call-out blockquotes as
    // admonitions (plugins/admonitions.mjs). tierBadge runs after the rewrite so it sees the
    // final href, and admonitions runs last so a badge inside a call-out is already a badge when
    // it moves into the title.
    //
    // Astro 7.2 deprecated `markdown.rehypePlugins` (and its remark siblings) in favor of naming
    // the processor explicitly: `unified()` IS the default remark/rehype pipeline, so this is the
    // same processing, declared through the factory that now owns the plugin lists. `shikiConfig`
    // stays at this level — it configures the highlighter, not the unified pipeline.
    processor: unified({
      rehypePlugins: [rewriteInternalLinks, accentTargetCode, tierBadge, admonitions],
    }),
  },
});
