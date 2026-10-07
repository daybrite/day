---
title: App websites
description: Set up a localized app website, customize daysite or own its Astro pages, and publish with Day's GitHub Actions workflow and an optional custom domain.
order: 32.6
section: Build & ship
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

Day's shared [GitHub Actions workflow](/docs/github-actions) can publish a complete app website:
localized app descriptions, download and store links, screenshot carousels, galleries, and an
optional hosted web app. The [daysite template](https://github.com/daybrite/daysite) reads your
existing app and store metadata. You can keep its default appearance, change its styles, select
a reusable theme, or build your own Astro pages while retaining the publication data.

Existing Day apps keep their current appearance and configuration. Customization is optional;
you do not need to fork daysite or copy its source into your app.

## Set up the website

`day new` creates `website/site.toml` and `website/theme.css`, unless you pass `--no-website`.
For an existing app, create `website/site.toml` next to `Day.toml`:

```toml
host = "https://YOUR-OWNER.github.io/YOUR-REPOSITORY"
```

Replace both placeholders, preserving the repository name's capitalization. For a repository
named `YOUR-OWNER.github.io`, use `https://YOUR-OWNER.github.io` without a repository path.
An optional custom domain uses its own URL, such as `https://my-app.example.org`.
`host` sets the canonical URL, sitemap, social metadata, and Astro base path.

The site gets its app identity and supported targets from `Day.toml`, localized listing text
from `store/`, and captures from [dayscripts](/docs/dayscript). Keep those sources current
instead of duplicating their content in website configuration. A native-only app can have a
website too; a web build is optional.

Common optional settings are:

```toml
host = "https://YOUR-OWNER.github.io/YOUR-REPOSITORY"
accent-color = "#3B82F6"
default-theme = "system" # light, dark, or system
default-platform = "web"
show-gallery = true
show-permissions = true
show-store-badges = true
footer = "© {year} Your organization"
```

Use a platform your app actually builds. A visitor's device, saved choice, or bookmarked
platform can override `default-platform`. The footer is publication text you supply.
See the [daysite configuration reference](https://github.com/daybrite/daysite#sitetoml)
for all settings, including search, icon effects, and release-channel choices.

## Publish with the built-in workflow

The generated `.github/workflows/ci.yml` already calls
[`daybrite/actions`' reusable app workflow](https://github.com/daybrite/actions/blob/main/.github/workflows/dayapp.yml).
Keep that workflow and its build inputs; a separate Pages build workflow is unnecessary.
The following is a minimal example for a repository that needs to add the shared workflow:

```yaml
name: Build app and website
on:
  push:
    branches: [main] # Replace with your default branch.
    tags: ['v*']
  pull_request:
  workflow_dispatch:
permissions:
  contents: read
jobs:
  app:
    uses: daybrite/actions/.github/workflows/dayapp.yml@v1
    permissions:
      contents: write
      pages: write
      id-token: write
    secrets: inherit
    with:
      targets: all
      scripts: auto
      locales: all
      themes: "light dark"
      deploy-web: true # Use false if you do not want to host a web build.
```

Choose targets and signing settings for your app as described in
[GitHub Actions](/docs/github-actions). `scripts: auto` runs the app's dayscripts;
`locales` and `themes` select the capture variants. `deploy-web: true` requires a web build
when a displayed development channel needs one, so omit it or use `false` for a native-only app.
For an app below the repository root, also set `project-path`; its website directory belongs
inside that project.

The presence of `website/site.toml` enables the full website automatically. Leave
`deploy-website` unset for that behavior. `deploy-website: "false"` disables publication for
that workflow call; use it on additional calls in the same run that build another variant.
Forcing `deploy-website: "true"` still requires the website configuration and app metadata.

### Enable GitHub Pages once

1. Open the app repository's **Settings → Pages**.
2. Under **Build and deployment**, select **Source → GitHub Actions**. Day supplies the
   workflow, so you can skip GitHub's suggested starter workflows.
3. Ensure the workflow job grants `pages: write` and `id-token: write`, as above.
4. Under **Settings → Environments → github-pages**, review deployment branch and tag rules.
   If you use selected rules, allow your default branch and release tags such as `v*`.
   GitHub can create this environment on the first deployment.
5. Run the workflow on the default branch, or let the next default-branch push run it.
   After its website build and deployment succeed, follow the deployment URL in Actions or Pages.

See GitHub's [publishing-source setup](https://docs.github.com/en/pages/getting-started-with-github-pages/configuring-a-publishing-source-for-your-github-pages-site)
and [custom workflow requirements](https://docs.github.com/en/pages/getting-started-with-github-pages/using-custom-workflows-with-github-pages)
for repository and organization policies that may apply.

The workflow generates and uploads a Pages artifact. Keep source configuration and theme
files in Git; the generated `dist/` directory and a `gh-pages` branch are unnecessary.

### What each publication contains

Normally the website deploys after successful default-branch builds and release-tag builds.
Pull requests and other branches build the app without replacing the live website.
`web-deploy-tag-pattern` can restrict automatic website publication to matching tags; consult
the [actions website reference](https://github.com/daybrite/actions#project-website-daysite)
before changing that filter.

| Channel | Source | Default location |
| --- | --- | --- |
| Latest release | That release's packages, screenshots, and web build | `/<locale>/` |
| Development | The current workflow's packages, captures, and web build | `/<locale>/main/` |

Before the first release, the development build occupies the locale root. A channel picker
appears when there is more than one channel. Pre-release channels can also be enabled through
`site.toml`. A release page only offers artifacts that its release actually carries;
development downloads are hosted with the site.

## Customize the site

Customization has three layers: daysite defaults, a selected reusable theme, and your project's
overrides. Later component choices win; styles and public asset directories are combined.
Project `theme.css` is applied last. Config paths belong to the directory containing that config,
so a theme can be used from a different checkout without copying its files.

### Change colors and styles

Start with the settings in `site.toml`. For additional styles, edit `website/theme.css`;
it is included automatically. For example, a project can supply a system font stack:

```css
body {
  font-family: Georgia, "Times New Roman", serif;
}
```

Treat selectors that reach into a component's markup as an implementation dependency.
For substantial changes to its structure, override the component instead.

### Select a reusable theme

Add a theme table **after** the top-level site settings:

```toml
host = "https://YOUR-OWNER.github.io/YOUR-REPOSITORY"

[theme]
repository = "appfair/appsite"
ref = "main"
```

The [App Fair example](https://github.com/appfair/appsite) supplies its own branding, footer,
appfair.net link, and Markdown journal while retaining daysite's app functionality.
The theme repository must be published before CI can check it out. Pin a tested tag or commit
for controlled theme upgrades; `ref` defaults to `main`.

Alternatively, commit a local theme inside your app repository:

```toml
[theme]
path = "./theme" # The directory website/theme/, relative to site.toml.
```

Choose `repository` or `path`. Each selected theme supplies `daysite.config.mjs`.
The workflow installs theme and project website dependencies from their `package-lock.json`
files. A customization with npm dependencies must include its lockfile.

This feature requires actions and daysite revisions that include customization support.
The workflow's `daysite-version` input defaults to `main` and can pin the renderer independently
of the theme. An older pinned renderer continues to build the default site, but must be updated
when adopting customization. Existing default sites need no configuration changes.

### Override components and add pages

Create `website/daysite.config.mjs` to override the selected theme or the defaults:

```js
export default {
  apiVersion: 1,
  components: {
    Header: './src/components/Header.astro',
    Footer: './src/components/Footer.astro',
  },
  customCss: ['./src/styles/brand.css'],
};
```

Create the referenced files too. Overrides receive the original component's props.
Import the selected component as `@daysite/components/Header`; import the original as
`daysite/components/Header.astro` when wrapping it, which avoids recursively selecting yourself.
The original header exposes `brand` and `nav` slots. The layout exposes `head`,
`before-content`, and `after-content` slots in addition to page content.
Wrapping the header preserves the language, theme, and QR controls. Replacing the whole layout
lets you own those features, along with SEO and document structure.

For your own pages, add `srcDir: './src'` to the customization module. Normal Astro routes in
`website/src/pages/` can then coexist with daysite's generated app and gallery routes.
To own a default URL, disable its pattern with `disabledRoutes` or replace it through `routes`;
do not create competing routes at the same URL.

A theme or project can also supply a standard `astro.config.mjs` (or another Astro-supported
config extension), using daysite's `createDaysiteConfig()` factory and adding its own Astro
integrations. The project configuration takes precedence over the theme configuration.
Preserve the workflow's `DAYSITE_PUBLIC_DIR` and `DAYSITE_OUT_DIR` when using the factory:
those connect generated app assets to the uploaded site artifact.

Use [`daysite/docs/customization.md`](https://github.com/daybrite/daysite/blob/main/docs/customization.md)
for the full component registry, factory example, route contract, and data APIs.
It also explains generated localization and asset accessors for theme-owned UI;
App Fair demonstrates those alongside its components.

### Build a blog or a completely different website

Your source tree is an ordinary Astro project. Use Markdown, MDX, content collections,
RSS, or other compatible Astro integrations as needed. App Fair's journal demonstrates
[content collections](https://docs.astro.build/en/guides/content-collections/) and
[static dynamic routes](https://docs.astro.build/en/guides/routing/).
Its sample posts are English publication content; additional post languages are yours to provide.

For a complete redesign, disable the default routes and compose your own pages from
`daysite/data`, its route helpers, and whichever components you keep. App descriptions,
artifacts, permissions, captures, and channel records remain available independently of the UI.
GitHub Pages serves static output: prerender pages and use browser-side services for features
that need a backend. Keep theme dependencies compatible with the renderer's Astro version.

## Preview and check locally

Install Node.js 22.18 or newer in the 22.x line, the Day CLI, and Git. From your app directory:

```sh
cd website
git clone https://github.com/daybrite/daysite .daysite
npm --prefix .daysite ci
node .daysite/scripts/install-customization.mjs
node .daysite/scripts/preview.mjs
```

For a repository theme, clone it separately and set `DAYSITE_THEME` to its absolute checkout
path **before** the install and preview commands. This variable can also try a theme without
editing `site.toml`. Local `theme.path` selections are resolved automatically.

The plain preview assembles one channel from your checkout and local dayscript captures.
Run your app's screenshot dayscripts first if you want local galleries. For release and
development channels assembled from GitHub, install and authenticate `gh`, then run:

```sh
node .daysite/scripts/preview.mjs --ci
```

Once publication data has been generated, check and build the selected renderer:

```sh
node .daysite/scripts/build-site.mjs check
node .daysite/scripts/build-site.mjs build
```

Neither command deploys. The launcher selects the project Astro configuration, then the theme
configuration, then daysite's default. `DAYSITE_CONFIG` can point a separate daysite checkout
at your app's `website/site.toml`; `DAYSITE_PUBLIC_DIR` and `DAYSITE_OUT_DIR` allow isolated
staging and output for multiple previews.

Ignore `website/.daysite/`, website `node_modules/`, `.astro/`, and `dist/`, and the generated
app index, channel, and gallery JSON files. Keep your customization modules, components,
content, styles, and dependency lockfiles tracked. See the generated project's
[`gitignore` template](https://github.com/daybrite/day/blob/main/crates/day-cli/templates/app/_gitignore).

Daysite's CI builds and browser-tests both the default and a custom theme fixture. App Fair's
CI separately builds its theme and journal. For your own theme, add an equivalent build check
and browser checks for its routes, mobile layout, locales, and preserved app features.

## Use an optional custom domain

First publish successfully at the repository's default Pages URL. Then choose a domain you
control, such as `my-app.example.org`:

1. In the app repository's **Settings → Pages → Custom domain**, enter the hostname without
   `https://` or a path and save it.
2. At your DNS provider, point that hostname to GitHub Pages. For the example subdomain, create
   a `CNAME` record named `my-app` with value `YOUR-OWNER.github.io`; omit the repository path.
3. Change `website/site.toml` to `host = "https://my-app.example.org"` and rebuild the site.

For an apex domain such as `example.org`, use your provider's `ALIAS`/`ANAME` support or
GitHub's published `A` records. Follow GitHub's
[custom-domain instructions](https://docs.github.com/en/pages/configuring-a-custom-domain-for-your-github-pages-site/managing-a-custom-domain-for-your-github-pages-site)
for the current addresses and optional `www` redirects.

When GitHub's DNS check and certificate provisioning complete, enable **Enforce HTTPS** in
Pages settings. See the [HTTPS guide](https://docs.github.com/en/pages/getting-started-with-github-pages/securing-your-github-pages-site-with-https)
if the option is not yet available.

You can also verify domain ownership through your account or organization's **Settings → Pages**
using GitHub's DNS TXT challenge. Keep that TXT record after verification; see
[domain verification](https://docs.github.com/en/pages/configuring-a-custom-domain-for-your-github-pages-site/verifying-your-custom-domain-for-github-pages).

For an Actions-based deployment, GitHub ignores an artifact's `CNAME` file: configuring `host`
or generating that file does not change the repository's Pages domain setting. Configure GitHub
and DNS as well. A custom-domain site normally uses `/` as its base, without the repository name.
Daysite's portable resource links continue to work when the site moves; rebuild to update its
canonical metadata, sitemap, and generated domain file.

## Troubleshooting

| Symptom | Check |
| --- | --- |
| The website job is skipped | Confirm `website/site.toml` is under the app's `project-path`, the current ref qualifies, and `deploy-website` is not `"false"`. |
| A release-tag deploy is skipped or refused | Allow release tags in the `github-pages` environment's deployment rules. |
| Pages deployment reports a permission error | Check the caller job's `pages: write` and `id-token: write`, the Pages publishing source, and organization Actions policies. |
| Downloads or screenshots are missing | Check which channel you are viewing and whether its release or workflow produced those artifacts. |
| The web build is required but missing | Include `web-dom` in the targets, or disable `deploy-web` for a site that does not host it. |
| A theme checkout or install fails | Check its repository/ref, publication status, local path, and committed npm lockfile. |
| Customization reports an unsupported renderer | Update the pinned `daysite-version` and ensure the reusable actions revision supports customization. |
| Project Pages links or canonical URLs are wrong | Check `host`, including repository capitalization and path; preserve the factory's staging/output settings in your Astro configuration. |
| A custom domain does not work | Check the repository's Pages domain setting and DNS records separately from `site.toml`. |
