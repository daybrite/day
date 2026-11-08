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

## Documentation for coding agents

Astro generates `/llms.txt`, `/llms-full.txt`, and Markdown endpoints for the selected guides
on every build. Edit the canonical documentation and `tutorials/session-counter.yaml`, not the
generated files. `src/data/llm-docs.json` selects agent conventions and all six Session counter
lessons. The full bundle contains that selected app-building path; it is not a dump of every
reference page. The remainder of the agent reference stays available on its HTML page.

The generator resolves links to absolute canonical URLs, expands the same tutorial steps used
by the website, and preserves code exactly. Draft posts, contributor history, and unpublished
sources cannot enter this selection. Pages advertise `/llms.txt` with `rel="describedby"`;
selected guides also advertise their Markdown endpoint.

Production builds compile all six lesson stages for each of the three desktop configurations
with the mock toolkit, then replay the recipe's interaction assertions and final dayscript.
Day command syntax is checked against the CLI built from the same checkout. New shell commands
need an explicit validation assignment; installation and native launches run in native CI.
Day dependencies resolve to this checkout, matching the native CI main-compatibility lane.
Compilation covers the complete Rust files, generated Fluent accessors, and manifests. It
does not establish that platform SDKs, native rendering, packaging, or published crates work.
The existing `doctor-verify.yml` workflow executes the native walkthroughs on macOS, Windows,
and Linux, against both the checkout and published releases.

`npm run llms:verify` runs the compiler and interaction checks independently. Successful results
are cached under the repository's `.cache/docs/llm/` with hashes of the documentation, recipe,
validation code, framework sources, lockfile, and Rust toolchain. Changed inputs invalidate the
cache; failures stop the build. Use `npm run llms:verify -- --force` to bypass a successful
receipt. Cargo uses cached dependencies by default; set `CARGO_NET_OFFLINE=false` to allow
downloads on a fresh machine. Logs and the input hashes stay in that cache directory.

Run `npm run llms:test` after generator changes. `npm run llms:check` compares generated files
with their sources and checks their internal links after a build; the build runs this check too.
New selected code fences or executable files need validation coverage before publication.

## Blog and RSS

The header links to `/blog`. Posts use Astro's Markdown/MDX content collection in
`src/content/blog/`, with file-based URLs such as `/blog/coming-soon`. The blog index shows
newest posts first. Published posts also appear in the sitemap and Pagefind search index.

Create a post with frontmatter:

```yaml
---
title: Your post title
description: A short summary for the blog index and RSS feed.
pubDate: 2026-11-08
author: The Daybrite Project
# updatedDate: 2026-11-09
---
```

Write the body below the frontmatter. Use `.md` for prose or `.mdx` when embedding Astro
components. Keep filenames lowercase with hyphens. The publication date controls ordering;
it does not schedule publication. Keep unpublished work in the drafts folder instead.

### Local drafts

Create `src/content/blog/drafts/` and write posts there with the same frontmatter. The folder
and its contents are ignored by Git and excluded from the production content loader. A normal
`npm run dev` session includes them in the blog index with a draft label and previews them at
`/blog/drafts/<filename>`. Draft previews carry `noindex, nofollow`. Keep the local development
server private while it contains private drafts.

To publish, move the file out of `drafts/` into `src/content/blog/`. A post anywhere in the
collection may also use `draft: true`; such posts preview locally but never enter production
routes, RSS, sitemap, or search. This frontmatter flag does not prevent Git from tracking the
file; the ignored folder provides that protection. Remove the flag when publishing.

`npm run build` and `npm run preview` show the production version, without drafts. The RSS
feed and sitemap exclude drafts even in development. Draft frontmatter must be valid to
preview locally; unfinished files in `drafts/` cannot break a production build.

### Feed discovery and UI strings

`/rss.xml` is generated with `@astrojs/rss`. It contains each published post's title, summary,
publication date, absolute permalink/GUID, and complete article HTML in `content:encoded`.
Markdown and MDX use the same Astro content renderer as article pages. Feed HTML preserves
headings, lists, code, tables, and images; links and image URLs become absolute. Scripts,
site styling, and interactive behavior are removed. Embedded components should include useful
static content and a link to the article for any interaction that requires the website.
The blog and footer provide subscription links. `BaseLayout.astro` adds RSS autodiscovery to
normal pages, and the blog integration adds it to copied HTML during production builds.

New blog UI text lives in `src/locales/en.json`. The build generates symbolic `res.str`
accessors in `src/generated/strings.mjs`; edit the catalog, not the generated file. Use
`npm run strings:generate` after editing the catalog during a dev session. Post titles,
summaries, author names, and article bodies are publication content.

Run `npm run blog:test`, `npm run strings:check`, `npm run build`, and `npm run linkcheck`
after changing blog behavior. Check that a local draft is absent from `dist/`, the feed,
sitemap, and search results before publication.

## Analytics

Production builds include [GoatCounter](https://www.goatcounter.com/help/start) in
[`BaseLayout.astro`](src/layouts/BaseLayout.astro), shared by the marketing and documentation pages.
Pageviews go to `https://daybrite.goatcounter.com/count`; the dashboard is at
<https://daybrite.goatcounter.com>. The public counting endpoint needs no API key or account credentials.

The script is omitted in development. GoatCounter also
[ignores localhost pageviews](https://www.goatcounter.com/help/skip-dev) by default, including when
previewing a production build locally. Keep the [privacy page](src/pages/privacy.astro) consistent
with any tracking changes.

## Documentation authoring

Use the [style guide](../website/authoring/style-guide.md),
[templates](../website/authoring/templates/README.md), and
[authoring workflow](../website/authoring/authoring.md). From this directory,
`npm run docs:check` validates the initial standards scope and glossary structure;
`npm run docs:test` checks the editorial rules against synthetic fixtures.
Continue to build and run `npm run linkcheck` for rendered content changes.
The [maintenance checklist](../website/authoring/maintenance.md) covers review, evidence
freshness, platform-claim updates, and publication checks.

### Landing page copy review

The [copy review tool](authoring/copy-review/README.md) provides section-by-section alternatives,
inline editing, and saved choices. It is disabled by default and runs separately from the site.
Enable it for a local editing session from the repository root:

```sh
DAY_COPY_REVIEW=1 npm --prefix website run copy:review
```

Open `http://127.0.0.1:4335/`; stop with Ctrl-C. Ordinary site builds and previews never load
the review UI. Saved choices and generated snapshots stay under the tool’s ignored `.local/`
directory. The guide explains how to prepare new alternatives and preserve decisions.

## Content structure and routes

[`src/data/content-routes.json`](src/data/content-routes.json) assigns documentation sources their
route metadata, primary type, and audience. Blog posts use the separate collection described above.
Documentation source paths are independent of public URLs;
`published: false` retains a source in the catalog without publishing it:

- Public pages: `/`, `/features`, `/platforms`, `/gallery`, `/showcase`, `/blog`, `/examples`,
  and `/community`. The header is Gallery, Showcase, Blog, Docs, and Explore, followed by search and
  theme controls. Contributing and GitHub are in Explore.
- App documentation: `/docs`, with tutorials, how-to guides, reference, and explanation.
- Contributors: `/contributing`, with setup, architecture, extension tutorials, documentation
  standards, and history.

The existing Markdown/MDX guides remain in `src/content/docs/`. Repository documents stay in
`docs/` and use matching symlinks in `src/content/internal/`. Design records and backend implementation contracts remain unpublished;
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
new routes. The catalog and published blog collection drive `sitemap.xml`; the normal build indexes new routes with Pagefind.
The reference index is rendered from content metadata and no longer rewritten during builds.
Retired `/contributing/backend-contracts` URLs return 404; do not add redirects or aliases.
App testing, element IDs, and native-project configuration belong in the app documentation.

## Executable tutorials

The Session counter lessons share `tutorials/session-counter.yaml` with the scheduled CI runner.
Markdown references ordered recipe steps; the build expands each step into its platform-specific
instructions. The segmented picker chooses macOS, Windows, or Linux, using an explicit URL choice,
a saved preference, or a browser platform guess. Without JavaScript, all variants remain readable.

`npm run tutorials:check` verifies schema, source placement, rendered code, platform selection,
and runner failure behavior without loading a browser. The `doctor-verify` workflow runs
release instructions from the published recipe and a separate checkout compatibility check.
See the [authoring workflow](authoring/authoring.md#validate-app-examples) for local commands
and the scope of each check.

## Local development

From the repository root:

```sh
scripts/website.sh          # install deps + dev server at http://localhost:4321/
scripts/website.sh build    # production build into website/dist
scripts/website.sh preview  # build, then serve the production output
```

Or directly with npm inside `website/`: `npm install`, then `npm run dev` / `npm run build`.

`npm run build` builds the site. It publishes no Rust API reference: every API link goes to
docs.rs. After a build, `npm run docs:api` checks those links against rustdoc built the way docs.rs
builds it; that needs the Rust toolchain, jq, and Python 3 in addition to Node.js.

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
│   ├── pages/              # landing, examples/, shared [...slug], API reference page, sitemap
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
The landing carousel uses the reviewed app/screen/toolkit selections in `hero.config.mjs`.
Each slide links to its own gallery row and names its app. Set `framed: true` for captures
that already contain desktop window chrome. Review the actual images before changing a pick;
missing selections are logged, with no automatic substitution of another screen. The build
accepts light or unthemed captures and verified matching dark captures, creates compact WebP
images, and invalidates the cache when selections or published URLs change. Dark mode rotates
only through verified dark captures; apps with no dark capture remain in the light rotation.
Each variant retains its own dimensions, including small differences in native window chrome.
The client decodes the requested images before switching and keeps the current app when it is
available in the new theme. Run `node --test scripts/hero-theme.test.mjs` for selection checks.
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
pull-request, scheduled, manual, and repository-dispatch triggers. After the Astro build, the
website job builds rustdoc as docs.rs does (`scripts/build-api-docs.sh`, into
`target/api-check/`, with warnings denied) and checks every API link and its fragment against it
(`scripts/docs/check-api-links.py`) before upload. That build is never uploaded: API links go to
docs.rs. A local validation build does not run these deployment jobs.

To preview the artifact, run `npm --prefix website run build`, then
`npm --prefix website run linkcheck` and `npm --prefix website run docs:api` from the repository
root. Serve with `npm --prefix website run preview`; `scripts/website.sh preview` rebuilds the site
before serving. The API check can resolve Cargo dependencies; use an isolated copy for
preservation-sensitive reviews. See the
[maintenance procedure](../website/authoring/maintenance.md#assemble-and-review-a-publication-artifact)
for the full command sequence and evidence limits.

The website lane is independent of framework checks. Canonical non-PR runs, including a manual
`website-only` dispatch, can deploy. Do not dispatch one as a preview. Commit, push, and deployment
each need explicit authorization; this overhaul's phase approval does not provide it.

Enable Pages with **Build and deployment → Source → GitHub Actions** in the repository settings.

## Day Travel localization explorer

`LocaleExplorer.astro` reads Day Travel from the same generated gallery manifest as
`/gallery/Day-Travel`. It pairs `top` and `bottom` only when both captures match the
selected phone target, locale, and appearance. Desktop targets use `overview`.
Missing combinations are disabled; an image-loading failure retains the last complete
view. The controls support hover, tap, keyboard navigation, and reduced motion.
Screenshot appearance follows the site’s theme selector, including its system setting;
the explorer has no separate theme control. Phone screenshots appear without bezels.
Missing desktop chrome uses the shared `DeviceShell.astro` component; AppKit and WinUI
retain their captured title bars.

Day Travel's v0.1.0 project website was not deployed when its release was published.
Its `screenshotRelease` entry in `gallery.config.mjs` pins the released `screenshots.tar.xz`
by tag and SHA-256. The website build downloads it once into `.cache/release-gallery`,
checks the archive and individual image checksums, and generates lossless WebP renditions
under ignored `public/gallery-media`. The gallery and explorer share those URLs; app
source and screenshots do not need to be committed to this repository. A cold build
needs access to GitHub; subsequent builds reuse the verified archive and renditions.

To adopt another release, update the tag, metadata URL, and archive digest together.
Keep the old release pinned until its replacement has a complete, reviewed capture set.
Run `node --test scripts/travel-explorer.test.mjs` and
`python3 -B -m unittest discover -s scripts -p 'test_release_gallery.py'`, then build the site
and check hover/tap selection, both phone positions, and narrow-screen layout.

## Landing-page Rust example

`RustExample.astro` displays the `root()` function from `apps/website-controls/src/lib.rs`
beside that app running as WebAssembly in an iframe. Its slider, editable stepper, progress bar,
and Reset button share one signal. App text comes from generated Fluent accessors.

`integrations/live-demo.mjs` builds the app before Astro reads the component, on both `npm run
build` and dev-server startup. On a fresh checkout it runs `cargo run --locked -p day-cli --
build ... -t web-dom --profile release`: Cargo builds the CLI from this checkout first. No
installed CLI, published Day release, or artifact from another CI job is required. Rust and
the `wasm32-unknown-unknown` target are required; the website CI job installs the target explicitly.

The source fingerprint covers the app, framework crates, workspace lockfile, and builder. A
matching cached output avoids compiling Rust when editing page copy or CSS. After changing
the Rust app, restart the dev server to rebuild it. Generated assets go under ignored
`public/demos/shared-signal/<fingerprint>/`; the displayed source is extracted during the same
build. The iframe uses native `loading="lazy"`; the browser chooses when to fetch it, without a
custom intersection observer. It receives site-theme changes through a same-origin
message without reloading its state. Its host does not register a service worker.

After building the site, run `npm run demo:test`. This uses Chromium to test the actual Wasm
through keyboard and button input at desktop and mobile sizes, including numeric bounds,
the displayed source, lazy loading, and state preservation across theme changes. Install the
browser once with `npx playwright install chromium`. Set `DAY_DEMO_BROWSER=webkit` to run the
same checks in WebKit (install it with `npx playwright install webkit`). CI runs the Chromium
checks before uploading the site. For Rust edits, also run `cargo fmt` and `cargo clippy` with
`--manifest-path ../apps/website-controls/Cargo.toml`; the app is a standalone workspace.

The tutorial screenshots remain on `/examples`. `verifyMarketingEvidence()` continues to
check their source and image hashes on every site build and dev-server startup.
