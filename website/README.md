<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Day website

The marketing + documentation site for **Day**, built with [Astro](https://astro.build). Deployed
to GitHub Pages at <https://daybrite.dev>.

## Local development

From the repository root:

```sh
scripts/website.sh          # install deps + dev server at http://localhost:4321/
scripts/website.sh build    # production build into website/dist
scripts/website.sh preview  # build, then serve the production output
```

Or directly with npm inside `website/`: `npm install`, then `npm run dev` / `npm run build`.

> The gallery's screenshots are produced by CI, not locally. Local builds automatically show
> placeholder tiles — no artifacts required.

## Structure

```text
website/
├── astro.config.mjs        # site + base (/day) + the gallery integration
├── gallery.config.mjs      # ← the extensibility surface: suites (apps), platforms, curated shots
├── integrations/gallery.mjs# runs the assembly before every dev/build
├── scripts/assemble-gallery.mjs  # CI artifacts → public/gallery + src/data/gallery-manifest.json
├── src/
│   ├── components/         # Logo, Nav, Footer, CodeSample, ShotTile
│   ├── content/docs/       # documentation (markdown content collection)
│   ├── content.config.ts   # docs collection schema
│   ├── layouts/            # BaseLayout, DocsLayout
│   ├── pages/              # index (landing), gallery, docs/[...slug]
│   └── lib/site.ts         # site metadata + base-path URL helper
└── public/                 # favicon; public/gallery is generated
```

## Support-tier badges

Every target's support tier (Tier 1 supported … Tier 4 development) is recorded once in
`src/lib/platforms.mjs`, next to the rest of the platform table, and defined for readers at
`/docs/platforms#support-tiers`. To badge a target in a docs page, write a plain markdown link to
that anchor whose text starts with `Tier <n>`:

```text
[Tier 3](/docs/platforms#support-tiers)                 compact, for table cells
[Tier 3 · Experimental](/docs/platforms#support-tiers)  full, for prose and headings
```

`plugins/tier-badge.mjs` stamps the class, `data-tier`, and a tooltip at build time, and
`src/styles/global.css` styles the pill in the tier tint (`--tier`, the one iOS-palette hue no
platform accent uses). The repo's internal docs (`docs/*.md`) are read on GitHub as well, so they
use the absolute form `https://daybrite.dev/docs/platforms#support-tiers`, which the plugin matches
too. The eight `/docs/platforms/<target>` pages need no markup — their title row reads the tier
from the platform table.

**Changing a target's tier** edits `src/lib/platforms.mjs`, both tables in
`src/content/docs/platforms.md`, and the target table in `src/content/docs/overview.md`;
`grep -rn support-tiers src/content docs` finds every other badge.

## Admonitions

A call-out blockquote renders as a titled, tinted box with an icon (`plugins/admonitions.mjs`).
Two ways to write one:

```md
> [!WARNING] Experimental ([Tier 3](/docs/platforms#support-tiers))
> Body text. The title sits on the marker line, so it can carry links and code.

> **Which XAML.** Body text — a bold lead becomes the title, which is why the ~40 reference
> docs that open with "**Status: implemented** …" needed no edit.
```

Kinds: `note` (blue), `tip` (green), `question` (teal), `important` (amber), `warning` (red, and
GitHub's `CAUTION` maps here), `status` (neutral). `[!NOTE]` is treated as "no kind chosen", so a
title starting "Status…" still gets the quiet status box; any other marker wins. A blockquote with
neither a marker nor a bold lead stays a plain quote — that is what keeps the agent prompts in
`ai.md` and `getting-started.mdx` looking like quotations.

`[!NOTE|TIP|IMPORTANT|WARNING|CAUTION]` are GitHub's own markers, so the repo's `docs/*.md` (read
on GitHub as well as here) can use them — but there, put the bare marker on its own line with a
bold lead beneath it. A title on the marker line, and the `[!QUESTION]` / `[!STATUS]` kinds, are
ours alone and print literally on GitHub, so use them only under `src/content/docs/`.

Every box gets an id from its title (`#admonition-which-xaml`) for linking from outside, a `#`
permalink in its header that appears on hover, and a highlight when it is the link target.
`npm run linkcheck` validates those anchors like any other.

**Editing a plugin?** Astro caches rendered markdown in `.astro/`, so a plugin change alone does
not re-render pages that did not change — `npx astro build --force` (or `astro dev --force`)
clears it. A plain `npm run build` after a plugin edit will happily serve the old HTML.

## The gallery

The gallery is assembled from CI screenshot artifacts by an Astro integration
(`integrations/gallery.mjs`), which runs `scripts/assemble-gallery.mjs` before every build:

1. Each CI job uploads `screenshots-<platform>` (see the repo's `.github/workflows/ci.yml`).
2. The website job downloads all of them into `website/artifacts/` and runs the build.
3. The assembly copies each platform's curated shots into `public/gallery/…` and writes
   `src/data/gallery-manifest.json`; `src/pages/gallery.astro` renders it.
4. Locally (no artifacts) every shot becomes a placeholder tile.

**To add a sample app or a component-snapshot set:** add an entry to `suites` in
`gallery.config.mjs` (its `artifactPattern`, curated `shots`, and platforms). No other code
changes are required — the assembly and the gallery page are data-driven.

## Deployment

The `website` job in the repo's CI workflow builds this site **after** every platform has uploaded
its screenshots and deploys `website/dist` to GitHub Pages. Enable Pages with **Build and
deployment → Source → GitHub Actions** in the repository settings.
