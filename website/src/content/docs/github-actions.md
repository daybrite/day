---
title: GitHub Actions
description: Build and test a Day app in CI, publish release packages, and deploy its website with the shared dayapp workflow.
order: 32.5
section: Build & ship
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

The reusable [`dayapp.yml`](https://github.com/daybrite/actions/blob/main/.github/workflows/dayapp.yml)
workflow installs Day and the platform toolchains, builds your app, runs dayscripts, captures
screenshots and packages supported targets. Version tags also produce GitHub releases. Store
uploads and website deployment use the same build outputs.

## Add the workflow

Create `.github/workflows/ci.yml` in your app repository, or adapt the file supplied by your
template:

```yaml
name: App CI

on:
  push:
    branches: [main]
    tags: ["v[0-9]+.[0-9]+.[0-9]+*"]
  pull_request:
  workflow_dispatch:

permissions:
  contents: write

jobs:
  app:
    uses: daybrite/actions/.github/workflows/dayapp.yml@main
    secrets: inherit
    with:
      targets: macos-appkit, windows-xaml, linux-gtk, ios-uikit, android-mdc, web-dom
      scripts: auto
      locales: all
      themes: light dark
      deploy-website: "false"
      upload-ios: "false"
      upload-macos: "false"
      upload-play: "false"
```

Change `main` if your default branch has another name. Keep only the targets your app supports.
`targets` is required; `scripts: auto` runs the project's `dayscript/*.yaml` files, or
`scripts/*.yaml` when that is the script directory. Use a space-separated list of paths to run
selected tests, or `none` to build without running tests.

This example publishes GitHub release assets but disables store uploads and Pages. Enable those
after completing their setup below. `contents: write` permits release creation and asset uploads;
fork pull requests normally receive a read-only token and no repository secrets.

Dependencies must resolve on the runner. Commit `Cargo.lock` and use registry or Git dependencies
instead of paths to another local checkout. `day-version` defaults to the latest published CLI;
set a version or commit when the app needs a particular Day revision. Leave `update-day-deps`
off to keep the versions in the lockfile.

> [!NOTE] Runner requirements
> The current shared workflow requests the `xcode-27` runner label for macOS builds, iOS builds
> and Apple upload jobs. Your repository must have access to that label. It is selected in the
> workflow's preflight job, not by the caller's operating system. See the
> [runner reference](https://github.com/daybrite/actions#targets-and-runners) before enabling
> those targets.

## Read the results

Open the repository's **Actions** tab and select a run. The jobs execute in this order:

1. **Preflight** checks formatting and prepares the build matrix. `preflight-checks: fmt clippy check test`
   also enables the other Rust checks; only `fmt` runs by default.
2. **Build jobs** run `day lint`, build, package and execute the selected dayscripts. Each selected
   locale/theme combination runs separately. Mobile targets use phone and tablet profiles by default.
3. **Release and deployment jobs** consume the packages and screenshots after the required builds succeed.

Download `dist-<target>` for packages and `screenshots-<target>` for captures. Additional mobile
profiles have separate screenshot artifacts. Branch and pull-request packages use development
signing or remain unsigned; they are test artifacts. See [signing tiers](/docs/packaging#signing-tiers).

If a job fails, fix the reported error or use GitHub's **Re-run failed jobs** for a transient
failure. Release and website jobs wait for the required build jobs. `tolerate-failures` is off
by default; enabling it permits failures in the workflow's designated best-effort steps.

## Create a release

Update the app version, build number and release notes, then commit the changes. From that commit:

```sh
git tag -a v1.0.1 -m "Release 1.0.1"
git push origin v1.0.1
```

The tag run attaches packages, checksums, build provenance, screenshot bundles, `gallery.json`
and `storefront.json` to the GitHub release. Screenshot validation can fail the release if a
declared store listing lacks required captures. The default release mode publishes immediately.
Set `release-mode: pre-release` for a public preview, or `release-mode: draft` for a private draft.

These modes control the GitHub release only. They do not delay configured store uploads. See
[App Store Submissions](/docs/app-store-submissions) for credentials and upload lanes.

For a pre-release that will later become the latest release, add this event alongside `push`:

```yaml
  release:
    types: [released]
```

Promoting the pre-release then rebuilds and deploys the website without uploading to stores again.
Do not add this event for the default `publish` mode; it would trigger a redundant run.

## Deploy to GitHub Pages

Add the deployment permissions to the caller:

```yaml
permissions:
  contents: write
  pages: write
  id-token: write
```

In the repository settings, select **Pages → Source → GitHub Actions**. Under
**Environments → github-pages → Deployment branches and tags**, allow the default branch and
release tags such as `v*`.

Choose what to deploy in the job's `with` block:

| Output | Configuration |
| --- | --- |
| App website with listings, downloads and screenshots | Add `website/site.toml` and remove `deploy-website: "false"`; the workflow detects it automatically. |
| An existing Astro website | Set `deploy-website: "true"` with a `website/` directory. |
| Web app only | Keep `deploy-website: "false"`, include `web-dom` in `targets`, and set `deploy-web: true`. |

The generated site uses the [daysite template](https://github.com/daybrite/daysite).
Deployment normally follows a successful default-branch push; the app website also deploys for
release tags. A `github-pages` environment that excludes tags prevents release-tag deployment.
See the [website workflow reference](https://github.com/daybrite/actions#project-website-daysite)
for `site.toml`, deployment filters and multiple workflow calls.

## Other workflow inputs

| Input | Use |
| --- | --- |
| `project-path` | Build a project below the repository root. |
| `target-scripts` | Override the test scripts for particular targets. |
| `ios-devices`, `android-devices` | Choose simulator/emulator profiles, orientations and artifact names. |
| `flavors`, `release-flavor`, `store-flavor` | Build and distribute [app variants](/docs/flavors). |
| `target-packages`, `target-setup` | Install additional packages or run target-specific setup. |
| `signing-environment` | Use a separate environment for macOS release signing and notarization. |
| `validate-rebuild` | Rebuild packages from their recorded provenance and compare results. |

See the [input reference](https://github.com/daybrite/actions#inputs) for defaults and constraints.
The repository also provides [composite actions](https://github.com/daybrite/actions#composite-actions)
for toolchain setup, signing and store uploads when you need a custom workflow.
