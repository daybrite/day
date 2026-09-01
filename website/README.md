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

> The gallery's screenshots come from the apps' own websites, fetched at build time. The first
> build needs the network; after that a cached copy of each index under `.cache/` keeps working
> offline.

## Structure

```text
website/
├── astro.config.mjs        # site + base (/day) + the gallery integration
├── gallery.config.mjs      # ← the extensibility surface: which Day apps the gallery indexes
├── integrations/gallery.mjs# runs the assembly before every dev/build
├── scripts/assemble-gallery.mjs  # the apps' published gallery.json → src/data/gallery-manifest.json
├── src/
│   ├── components/         # Logo, Nav, Footer, CodeSample, DeviceShell, PlatformShots
│   ├── content/docs/       # documentation (markdown content collection)
│   ├── content.config.ts   # docs collection schema
│   ├── layouts/            # BaseLayout, DocsLayout
│   ├── pages/              # index (landing), gallery/ (hub + one page per app), docs/[...slug]
│   └── lib/site.ts         # site metadata + base-path URL helper
└── public/                 # favicon and static assets
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

`/gallery/` indexes every Day sample app; `/gallery/<App>/` is one app's screenshots, every
platform side by side. The images are **hosted by the apps themselves** — this site links them.

Each app's CI runs its dayscript walkthrough on every target it builds for, once per theme and
language, and its website publishes both the images and `day screenshot index`'s machine-readable
index of them at `<host>/gallery/gallery.json`. Before every build here, an Astro integration
(`integrations/gallery.mjs`) runs `scripts/assemble-gallery.mjs`, which fetches those indexes and
writes `src/data/gallery-manifest.json` for the pages to render.

The index describes itself, so a row's heading, its caption, the source file it links, the columns,
the themes and the languages all come from the app — an app that captures a new screen shows it
here on the next build, with no change in this repository.

**To add an app:** add an entry to `apps` in `gallery.config.mjs` with its label, blurb, repository
and index URL. The optional `order` / `labels` / `hide` keys are there for an app whose dayscript
carries thin metadata; a shot with no `title:` falls back to a label derived from its id. (The
better fix for a missing heading is a `title:` on that `screenshot:` step in the app's own
dayscript, which improves the app's own gallery too.)

An app whose site cannot be reached falls back to `.cache/gallery/<app>.json`, the last index that
fetched, and its page says so. With no cache either, the app is left out and the build log names
it. A build where nothing resolves still succeeds with an empty gallery, so a checkout with no
network can still render the pages.

## Deployment

`.github/workflows/website.yml` builds this site and deploys `website/dist` to GitHub Pages. It
runs on pushes touching `website/` or `docs/`, daily on a schedule (to pick up screenshots the
apps published since the last run), on `workflow_dispatch`, and on a `gallery-published`
`repository_dispatch` an app's CI can send. It builds its own copy of the rustdoc bundle into
`dist/api`, because Pages deploys one artifact and this is the workflow that deploys.

Enable Pages with **Build and deployment → Source → GitHub Actions** in the repository settings.
