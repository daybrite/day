# day website

The marketing + documentation site for **day**, built with [Astro](https://astro.build). Deployed
to GitHub Pages at <https://daybrite.github.io/day/>.

## Local development

From the repository root:

```sh
scripts/website.sh          # install deps + dev server at http://localhost:4321/day/
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
