<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Maintain Day documentation

Update documentation alongside the behavior it describes. Use the
[authoring workflow](authoring.md) for commands and the [style guide](style-guide.md) for prose,
examples, and source placement. This checklist applies to documentation, public website copy,
screenshots, and generated API reference.

## Responsibility and freshness

Named documentation and support-claim ownership remains unassigned, as chosen in the
[maintainer review](../../docs/history/documentation-overhaul.md#accepted-decisions).
Do not infer an owner from authorship or a Git history entry. For each proposed change, identify
who reviewed its technical evidence in the review record. A reviewer for one change does not
become the permanent owner of that area.

Review the affected documentation when an API, CLI command, backend, dependency requirement,
packaging workflow, or route changes, and before a release that relies on those claims.
Evidence becomes stale when its relevant inputs change; a recent date alone does not make it
valid. Record the source revision and any uncommitted inputs, host/toolkit and tool versions,
command, date, result, and limitations. Preserve prior evidence rather than rewriting its result.
For an unchanged example, compare its source hash before citing earlier validation.

Use PASS for an executed successful check, FAIL for an observed defect, and NOT RUN for missing
validation. Separate compilation, native execution, installation, and accessibility results.
When evidence is unavailable, qualify the claim and record a follow-up with the affected source,
the missing check, and the condition for closing it. Do not advance its verification date.

## Review a documentation change

- [ ] Link the agreed issue or discussion required by [CONTRIBUTING](../../CONTRIBUTING.md).
      State the reader's task and whether the page is a tutorial, how-to, reference, explanation,
      or marketing page. Keep proposals separate from shipped contracts.
- [ ] Edit the canonical source. Update the route catalog, publishing symlink, sidebar placement,
      current internal links, and sitemap when adding or moving a page. Keep a migration record;
      old external URLs do not receive redirects under the accepted policy.
- [ ] Verify API signatures, feature gates, defaults, errors, lifecycle, threading, and target
      limits against the current implementation. A successful prose check is not API review.
- [ ] Compile designated runnable examples with their imports, manifests, resources, and Fluent
      catalogs. Label excerpts and synthetic harness context. Use generated `res::str` accessors,
      generated resource constants, and symbolic element IDs in app UI. The public website's
      English-copy exception does not apply to app examples or embedded app UI.
- [ ] Run the affected editorial, architecture, source-link, symlink, build, and rendered-link
      checks from the authoring workflow. Review headings, metadata, keyboard focus, search,
      dropdowns, themes, and narrow-screen layout when their shared templates change.
- [ ] Check screenshot provenance and claim evidence. Review the rendered page, including its
      gallery or example links; do not replace a gallery with an unrelated teaching sample.
- [ ] Record exact commands and outcomes, unrun checks, changed files, and remaining issues.
      Run the whole-tree readiness gate before describing the work as ready to commit or push.
      Commit and push each require Marc's explicit permission; documentation edits grant neither.

## Update platform claims

1. Identify the exact OS/toolkit combination and claim: API availability, maintenance tier,
   compile success, native behavior, packaging, or store distribution. These need different evidence.
2. Inspect the implementation, feature gates, tests, and checked-in CI configuration. For runtime
   claims, supply dated results for the relevant host/device, SDK, app revision, and framework
   revision. A configured CI job is not a successful CI run. HarmonyOS emulator acceptance stays
   in CI; local work may compile-check it.
3. Record limitations and evidence before proposing a tier change. Follow the support commitments
   in [CONTRIBUTING](../../CONTRIBUTING.md#platform-support-tiers); this process assigns no owner
   and authorizes no tier promotion by itself.
4. Update [platform metadata](../../website/src/lib/platforms.mjs), both support tables in
   [the platform guide](../../website/src/content/docs/platforms.md), and affected platform,
   compatibility, onboarding, and marketing pages. Check all `support-tiers` links. Regenerate
   affected duty, coverage, and recorder matrices using their existing scripts.
5. Keep supported targets prominent while retaining experimental, development, and deprecated
   targets in the technical reference. Run architecture, build, links, and matrix checks, and
   record unavailable platform validation as NOT RUN.

Repository-only reviews do not establish external service availability, competitor facts, remote
CI results, or store acceptance. Keep unsupported claims qualified. The explicit restoration of
the existing gallery and Showcase release inputs is not blanket permission for external research.

## Maintain images and examples

The homepage carousel, `/gallery`, and `/showcase` serve different purposes from `/examples`.
Preserve them when updating tutorial examples. The gallery configuration and published app indexes
supply screenshot paths and capture metadata. Record missing app/framework revisions or capture
dates as unknown; a plausible image is not proof that a particular code sample produced it.

Local builds can use cached gallery indexes and placeholder release cards. They do not verify
current remote releases. Keep the `DAY_REQUIRE_SHOWCASE` publication check enabled; do not remove
it to make an offline build look publishable. Check package URLs against the release manifest
when release metadata is refreshed, and distinguish link rendering from installation testing.

## Assemble and review a publication artifact

Use the deployment details in [the website README](../../website/README.md#deployment).
From the repository root, with the documented Node and Rust tools and dependencies available:

```sh
npm --prefix website run build:force
npm --prefix website run linkcheck
npm --prefix website run preview
```

These commands build and serve locally; they do not publish. The site build may fetch existing
gallery/release inputs and rewrite generated data. The API script clears `target/doc`, runs Cargo,
and replaces the destination bundle. Use an isolated checkout/copy when preserving local build
output or lockfile edits. The full build generates Rustdoc after Astro, then checks the bundle
and links into it. A later `build:site` or bare Astro build clears `dist`, including the API
bundle; rerun `build:api` afterward. `scripts/website.sh preview` rebuilds both outputs; use the
direct preview command above to inspect an existing artifact.

The artifact completeness check requires the gallery and Showcase pages, search entry files,
sitemap, custom-domain file, API bridge, root API landing page, and every crate
entry from the shared manifest. The API link checker validates guide-to-API links as well as
Rustdoc item destinations and fragments; Rustdoc warnings fail
the build. The completeness check itself does not replace either check.
The website link checker excludes Rustdoc and external URLs; review those separately. Search
indexes the Astro site before Rustdoc is added and is not evidence of full API-item search.

Before any authorized publication, review the exact artifact, its screenshot/release inputs,
known failures, and unrun checks. The current CI website lane can deploy independently of the
framework lane; a green website build does not certify the framework. A manual website-only run
on `main` in the canonical repository can publish, so it is not a harmless preview command.
Manual runs on other branches build and check the site without deploying. Review approval
does not authorize a commit, push, workflow dispatch, or deployment. Named ownership can remain
deferred while the maintenance process and evidence are available for review.
