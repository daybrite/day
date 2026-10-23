<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Day website

The marketing + documentation site for **Day**, built with [Astro](https://astro.build). Deployed
to GitHub Pages at <https://daybrite.dev>.

App project websites use the separate [daysite template](https://github.com/daybrite/daysite).
The [App websites guide](src/content/docs/websites.md) covers their configuration, reusable
themes, custom Astro pages, built-in Actions deployment, and GitHub Pages/custom-domain setup.
This directory maintains Day's own documentation and marketing site.

## Documentation authoring

Use the [style guide](../website/authoring/style-guide.md),
[templates](../website/authoring/templates/README.md), and
[authoring workflow](../website/authoring/authoring.md). From this directory,
`npm run docs:check` validates the initial standards scope and glossary structure;
`npm run docs:test` checks the editorial rules against synthetic fixtures.
Continue to build and run `npm run linkcheck` for rendered content changes.
The [maintenance checklist](../website/authoring/maintenance.md) covers review, evidence
freshness, platform-claim updates, and publication checks.

## Content structure and routes

[`src/data/content-routes.json`](src/data/content-routes.json) assigns every content source one
route metadata, primary type, and audience. Source paths are independent of public URLs;
`published: false` retains a source in the catalog without publishing it:

- Public pages: `/`, `/features`, `/platforms`, `/gallery`, `/showcase`, `/examples`,
  and `/community`. The header is Gallery, Showcase, Docs, and Explore, followed by search and
  theme controls. Contributing and GitHub are in Explore.
- App documentation: `/docs`, with tutorials, how-to guides, reference, and explanation.
- Contributors: `/contributing`, with setup, architecture, backend contracts, documentation
  standards, and history.

The existing Markdown/MDX guides remain in `src/content/docs/`. Repository documents stay in
`docs/` and use matching symlinks in `src/content/internal/`. Design records remain unpublished;
their source files and symlinks stay in place. Contributor standards publish through `src/content/repository/`.
Edit the canonical source, never a copied website version. Keep titles and descriptions in
front matter; the route catalog owns publication. Reference groups live in
[`reference-navigation.json`](src/data/reference-navigation.json), shared by the sidebar and index.
Pages within each group sort alphabetically by `navTitle`, falling back to `title`. Tutorial
groups retain their lesson sequence using `order`. Short sidebar labels leave full page titles intact.

For a new repository document, add its symlink, catalog entry, and topic placement in
[`content-routes.json`](src/data/content-routes.json). Run `npm run docs:architecture`,
`python3 -B ../scripts/ci/doc-links.py`, and `bash ../scripts/ci/docs-symlinks.sh` from this directory.
The checks reject missing sources, duplicate routes, unplaced pages, and wrong publishing targets.

[`plugins/content-links.mjs`](plugins/content-links.mjs) resolves repository-relative links from
their canonical source path. [`route-migrations.json`](src/data/route-migrations.json) translates
source-era site links; it does not publish old routes or redirects. Update authored links to the
new routes. The catalog also drives `sitemap.xml`; the normal build indexes new routes with Pagefind.
The reference index is rendered from content metadata and no longer rewritten during builds.

## Local development

From the repository root:

```sh
scripts/website.sh          # install deps + dev server at http://localhost:4321/
scripts/website.sh build    # production build into website/dist
scripts/website.sh preview  # build, then serve the production output
```

Or directly with npm inside `website/`: `npm install`, then `npm run dev` / `npm run build`.

`npm run build` and `npm run build:force` include the Rust API reference and validate links into
it. They require the Rust toolchain, jq, and Python 3 in addition to Node.js. `npm run build:site`
builds only the guide site, for a quick content check. To browse API links from the development
server, run `npm run api:dev` before `npm run dev`; rerun it after changing Rust APIs.

> Builds assemble the apps’ published gallery indexes and Day Showcase release metadata. A warm
> checkout can reuse gallery indexes and hero images offline; unavailable release metadata produces
> download placeholders. Dependencies must already be installed for an offline build.

## Structure

```text
website/
├── astro.config.mjs        # daybrite.dev at the domain root + gallery integration
├── gallery.config.mjs      # ← the extensibility surface: which Day apps the gallery indexes
├── integrations/gallery.mjs# assembles galleries and the hero; verifies tutorial evidence
├── scripts/assemble-gallery.mjs  # combines the apps’ published screenshot indexes
├── src/
│   ├── components/         # Logo, Nav, Footer, CodeSample, DeviceShell, PlatformShots
│   ├── content/docs/       # authored guides; route catalog determines public location
│   ├── content/internal/   # symlinks to canonical docs/, including design and history
│   ├── content/repository/# symlinks to contributor instructions and standards
│   ├── content.config.ts   # docs collection schema
│   ├── layouts/            # BaseLayout, DocsLayout
│   ├── pages/              # landing, examples/, shared [...slug], API bridge, sitemap
│   └── lib/site.ts         # site metadata + base-path URL helper
└── public/                 # favicon and static assets
```

## Support-tier badges

Every target's support tier (Tier 1 supported … Tier 5 deprecated) is recorded once in
`src/lib/platforms.mjs`, next to the rest of the platform table, and defined for readers at
`/docs/reference/platforms#support-tiers`. To badge a target in a docs page, write a plain markdown link to
that anchor whose text starts with `Tier <n>`:

```text
[Tier 3](/docs/reference/platforms#support-tiers)                 compact, for table cells
[Tier 3 · Experimental](/docs/reference/platforms#support-tiers)  full, for prose and headings
```

`plugins/tier-badge.mjs` stamps the class, `data-tier`, and a tooltip at build time, and
`src/styles/global.css` styles the pill in the tier tint (`--tier`, the one iOS-palette hue no
platform accent uses). The repo's internal docs (`docs/*.md`) are read on GitHub as well, so they
use the absolute form `https://daybrite.dev/docs/reference/platforms#support-tiers`, which the plugin matches
too. The nine `/docs/reference/platforms/<target>` pages need no markup — their title row reads the tier
from the platform table.

**Changing a target's tier** edits `src/lib/platforms.mjs`, both tables in
`src/content/docs/platforms.md`, and the target table in `src/content/docs/overview.md`;
`grep -rn support-tiers src/content docs` finds every other badge.

## Admonitions

A call-out blockquote renders as a titled, tinted box with an icon (`plugins/admonitions.mjs`).
Two ways to write one:

```md
> [!WARNING] Experimental ([Tier 3](/docs/reference/platforms#support-tiers))
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

## Showcase downloads

`/showcase` provides the screenshot carousel, platform package cards, store links, and CLI launch
instructions. `scripts/assemble-downloads.mjs` runs during dev/build and reads the latest release
metadata from `daybrite/Day-Showcase` (`DAY_RELEASE_REPO` can override the repository). It writes
`src/data/downloads.json`; package links point to GitHub release assets, rather than copying
installers into the site. A failed metadata request produces explicit package placeholders.

## Gallery and screenshot evidence

`/gallery` shows the screenshot carousels for the apps in `gallery.config.mjs`.
`/gallery/<app>` compares the same screens across captured platforms, languages, and themes.
The landing page uses `HeroCarousel.astro` to rotate through Day Showcase captures with their
platform shells, captions, and matching dark captures where available. Each slide links directly
to its screenshot row in the gallery.

`integrations/gallery.mjs` runs `scripts/assemble-gallery.mjs` and `scripts/hero-shots.mjs` before
every dev/build. Gallery indexes come from the apps’ published sites, with saved indexes under
`.cache/gallery/` available for offline builds. Cached indexes are labeled when used. Gallery
images remain hosted by their apps; the hero downloads and caches its images under `public/hero/`.
Run `node scripts/hero-shots.mjs --refresh` to rebuild the hero pool. A cold build needs access to
the app sites; a warm checkout can build from its cached indexes and hero images.

`DAY_REQUIRE_SHOWCASE=1` prevents publishing a build without a current Showcase index and a
nonempty verified hero pool. The existing deployment workflow sets it. Screenshot validation
rejects blank images; it does not certify every target or prove source-revision equivalence.
The gallery preserves capture labels and links to the app’s source, which may have advanced since
capture. Restoring this existing gallery and its published screenshot inputs was explicitly
requested after Phase 6; the earlier removal is superseded.

`/examples` retains the tutorial code and its locally captured AppKit screenshots. Their
`public/images/tutorial/provenance.json` records source hashes and capture conditions.
The integration also runs `scripts/verify-marketing-evidence.mjs` for those captures.
`node --test scripts/marketing-evidence.test.mjs` checks that changed image bytes, app behavior,
Fluent text, or target attribution invalidate that evidence. Generate replacement tutorial
captures with `python3 -B scripts/docs/capture-tutorial.py`, inspect them, then update the images
and provenance together.

Public website English literals are explicitly permitted for this overhaul. Day app examples
and embedded app UI still require generated localization and resource accessors.

## Deployment

The `website` and `website-deploy` jobs in [ci.yml](../.github/workflows/ci.yml) build and deploy
`website/dist` to GitHub Pages. The website build runs on non-tag workflow runs; deployment is
restricted to the canonical repository and excludes pull requests. The workflow includes push,
pull-request, scheduled, manual, and repository-dispatch triggers. The website job builds the
Rustdoc bundle into `dist/api` **after** the Astro build, because Astro clears `dist` and Pages
deploys one combined artifact. `node website/scripts/check-api-bundle.mjs` (from the repository
root) checks the site, API bridge, and all crate entry pages before upload. A separate Python check validates item-level Rustdoc links, and Rustdoc builds with warnings
denied. A local validation build does not run these deployment jobs.

To preview that combined artifact, run `npm --prefix website run build`, then
`npm --prefix website run linkcheck` from the repository root. Serve with
`npm --prefix website run preview`; `scripts/website.sh preview` rebuilds the combined site
and API reference before serving. The API generator clears `target/doc` and can resolve Cargo dependencies; use
an isolated copy for preservation-sensitive reviews. See the
[maintenance procedure](../website/authoring/maintenance.md#assemble-and-review-a-publication-artifact)
for the full command sequence and evidence limits.

The website lane is independent of framework checks. Canonical non-PR runs, including a manual
`website-only` dispatch, can deploy. Do not dispatch one as a preview. Commit, push, and deployment
each need explicit authorization; this overhaul's phase approval does not provide it.

Enable Pages with **Build and deployment → Source → GitHub Actions** in the repository settings.
