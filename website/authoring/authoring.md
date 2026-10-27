<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Author Day documentation

Use the [style guide](style-guide.md), [maintenance checklist](maintenance.md), and
[page templates](templates/README.md). Follow [CONTRIBUTING](../../CONTRIBUTING.md) and the
[website conventions](../README.md). The [shared glossary](../src/lib/glossary.mjs) is the
terminology source. For automated edits, also follow any local `AGENTS.md` instructions supplied
with your checkout; those files are not part of the published documentation.

## Prepare a change

1. Confirm the checkout, branch, requested scope, and existing changes. Preserve unrelated work.
2. Choose a reader and primary page type. Keep proposals separate from shipped contracts.
3. Verify symbols, commands, platform limits, and image attribution against current evidence.
   Record missing evidence instead of inventing it.
4. Edit the canonical source. For a new page, add its route catalog entry, publishing symlink
   when required, navigation placement, title, and description.
5. Run the applicable checks and review the rendered result. Record exact commands, results,
   source revision, environment, and limitations in the change description.

## Editorial and structural checks

Use Node.js 22.12.0 or newer and npm, Python 3, and Bash. From the repository root:

```sh
npm --prefix website ci
npm --prefix website run docs:check
npm --prefix website run docs:test
npm --prefix website run docs:architecture
python3 -B scripts/ci/doc-links.py
bash scripts/ci/docs-symlinks.sh
```

`npm ci` installs locked dependencies and replaces the website's installed packages. Skip it when
matching dependencies already exist. No global editorial package is required.

`docs:check` discovers all Markdown under `docs/`, `website/src/content/docs/`, and
`website/authoring/`, plus the contribution guide. New pages are included automatically. It checks Markdown structure, fence languages,
selected prose rules, descriptive links, image alt text, local source destinations, and glossary
structure. In a Git checkout, local links must point to tracked or new non-ignored source files;
ignored files on an author's machine cannot satisfy a public link. Code and HTML comments are
excluded from prose rules. Check additional Markdown with:

```sh
npm --prefix website run docs:check -- docs/history/documentation-overhaul.md
```

Explicit file arguments replace the default scope. MDX and Astro components require build and
rendered checks. After shortening a crate README or removing its headings, also run the default
`npm --prefix website run docs:check`: checking only the edited README does not check links from
other pages into its removed sections. Move detailed contracts into their canonical reference
page before removing them from a README.
Source lint does not verify API semantics, root-relative site routes, or external
URLs. These checks use existing locked dependencies; they are not full Vale or markdownlint runs.

## Validate app examples

The six Session counter lessons use `website/tutorials/session-counter.yaml` for all commands,
file examples, platform variants, and UI assertions. Markdown places steps using
`<!-- tutorial-step: lesson-id/step-id -->`; keep them in recipe order. Add executable examples
to the YAML, not Markdown fences. The website renderer and CI runner use the same parser.

Run the source and rendering checks without a browser:

```sh
npm --prefix website run tutorials:check
```

The `doctor-verify` workflow runs the scaffold/package checks and tutorial walkthroughs together
at 00:00 and 12:00 UTC daily, or through a manual workflow dispatch. Tutorial jobs cover
macOS/AppKit, Windows/WinUI, and Ubuntu/GTK in two modes, with separate results and evidence.
Relevant pull requests run only the tutorial source and rendering checks in this workflow.
The **release** mode downloads the recipe published with daybrite.dev, installs Day fresh from
crates.io, and executes the reader commands unchanged. The **main** mode uses this checkout's
recipe and explicitly substitutes the checkout's CLI and framework dependencies. It reports
compatibility with the candidate sources, not a published-reader result.

To run the release instructions from the checkout on a prepared Mac:

```sh
node website/scripts/tutorial-walkthrough.mjs --platform macos --mode release
```

Use `windows` or `linux` on the corresponding host. Add `--source published` to test the recipe
currently served by daybrite.dev. Use `--dry-run` to export the resolved plan without executing
it. `--out <new-directory>` selects an unused session directory; otherwise it uses a temporary
location. Each run isolates its Cargo home, build output, and app data. SDKs and Rust are
prerequisites; these checks do not validate their installation procedures.

Commands execute in order in Bash or PowerShell, preserving directory and environment changes.
File steps write or append the displayed bytes. For an interactive launch, the harness starts
the exact command, then attaches with `day drive` to check the recipe's expected behavior and
take a screenshot. It stops the app afterward. This verifies scripted interactions, not
physical input or the OS quit shortcut. A failed assertion ends the run; assertions are not
retried by the harness. The evidence directory records the recipe hash, commands, steps,
results, resolved lockfile, and screenshots. CI uploads only that directory, not the Cargo cache
or session authentication tokens.

For a faster local source-compatibility check using existing Cargo caches:

```sh
cargo build --locked -p day-cli
python3 -B scripts/docs/verify-tutorial.py --run-native
```

This validator also reads the YAML files and assertions, but substitutes local dependencies
and supplies its own build and launch commands. It does not certify the reader's installation
sequence. Omit `--run-native` to compile only.

Validate the designated recipes and priority excerpts with:

```sh
python3 -B scripts/docs/verify-recipes.py
python3 -B scripts/docs/verify-excerpts.py
```

Recipes compile and lint exact modules with their catalogs and resources, then run isolated
storage, command, and mock interaction tests. Excerpts use explicitly synthetic context for
omitted functions, types, images, and font metadata; they remain fragments rather than complete
native apps. All three validators create new sessions under `.cache/docs/`, copy the workspace
lockfile, and check that the root lockfile stays unchanged. They use cached dependencies offline;
a missing dependency is a failed check, not a passed or skipped example.

To refresh the tutorial captures on the documented macOS host, run
`python3 -B scripts/docs/capture-tutorial.py`. Review the captures and resulting provenance record
before including them. Gallery images and Showcase downloads have separate sources.

## Build and inspect the site

A site build can fetch gallery/release inputs and rewrite generated data. Use an isolated copy
for a preservation-sensitive review. From the repository root:

```sh
npm --prefix website run build:force
python3 -B scripts/docs/check-site.py
npm --prefix website run linkcheck
```

The build checks MDX and Astro output and generates the Pagefind search index. Force a rebuild
after Markdown plugin edits. The rendered audit checks metadata, headings, IDs, image alt
attributes, and sitemap membership. The link checker excludes external URLs and Rustdoc.

The full build includes Rustdoc and validates guide-to-API links and internal API links.
Follow [the artifact assembly procedure](maintenance.md#assemble-and-review-a-publication-artifact)
when running the separate `build:site` and `build:api` steps. To repeat the API link check:

```sh
python3 -B scripts/docs/check-api-links.py
```

Both Python rendered checkers accept an optional path to another built site directory. They
create their own result directory and require no prior validation session. Review responsive
layout, search, keyboard focus, themes, and navigation when shared templates change.

Link API names in prose and function tables directly to their generated item page, for example
[`is_supported()`](https://daybrite.dev/api/day_part_haptics/fn.is_supported.html). Use absolute `https://daybrite.dev/api/` links so local previews open published Rustdoc
and link methods and variants to their specific anchors. Link references to other capabilities
to their internal guides, following the [cross-reference rules](style-guide.md#link-related-documentation). Keep code fences
copyable Rust, with the corresponding API links in the explanation beside the example. Verify
the destination in the combined build rather than guessing a Rustdoc path. Check the receiver's
actual type: identical method names such as `on_submit` and `header` can have different owners.
External crates use versioned `https://docs.rs/<package>/<version>/<library>/` item links.
Keep their released versions, guide URLs, and API destinations in
[external-crates.json](../src/data/external-crates.json). Do not add them to `api-crates.json`:
that catalog builds APIs from this workspace. When updating an external guide, check the actual
published source and verify its docs.rs item pages and fragments; the local API checker only
covers the bundled Rustdoc. Exclude unpublished packages and empty placeholder releases.

The site marks item links with a dashed underline; keep ordinary
Markdown links in the source rather than adding presentation markup.

After assembling Rustdoc, `npm run docs:api-references` (from `website/`) checks every published
Markdown and MDX source for unlinked calls, dot-methods, and qualified method references whose
names occur in the bundle. CI and `build:api` run it automatically. It reports missing links;
it never guesses an owner or inserts a link. Bare words without callable syntax still need
editorial review because they can also name fields, CSS values, commands, or concepts.

Generated application accessors have no framework item page. Other reviewed non-API references
(native calls, fields, private helpers, historical proposals, and separately hosted crates) are
listed by exact source and code text in `api-reference-exceptions.json`, with reasons. The check
rejects stale and duplicate exceptions. Resolve a real public API reference rather than adding
an exception for it. `scripts/docs/check-api-links.py` remains the destination check: it validates
the final file and exact fragment, including Rustdoc re-export redirects.

## Hand off a change

Run `bash scripts/ci/check-ready.sh` against the full working tree before reporting it ready to
commit or push. Follow root instructions for formatting, stale matrices, and backend Clippy.
HarmonyOS emulator acceptance runs in CI, not locally.

Distinguish PASS, FAIL, and NOT RUN. Include changed files, evidence, and remaining limitations.
Marc's explicit permission is required separately for commits and pushes. A request for changes
or a passing validation run does not authorize publication.

The website CI job compiles the designated recipes and priority excerpts against the checked-out
Day sources: main HEAD for a main-branch run, or the candidate revision for a pull request.
It does not use published Day crates. The validators use cached Cargo dependencies locally;
set `CARGO_NET_OFFLINE=false` to permit third-party dependency downloads on a fresh machine.
`CARGO_HOME` and `CARGO_TARGET_DIR` can isolate the dependency and build caches for that check.
Record wider example-review gaps in the maintenance ledger, not as repeated notices on app
documentation pages.

## Dependency versions in examples

`npm run docs:versions` checks published documentation, crate READMEs, authoring guides, and
machine-readable tutorial recipes. Every Astro production build runs this check before rendering;
`docs:architecture` runs it in CI too. A stale Day dependency fails with its source location.

Core crate versions come from the workspace manifests. Examples may use the current minor
release line or the full workspace version. External crate versions come from
`src/data/external-crates.json`; additional documented dependencies live in
`src/data/doc-dependencies.json`. Update those catalogs when adopting a verified release.
External examples use the catalog's full version, or its minor shorthand when the patch is zero.
The check runs offline; it does not infer releases from crates.io during a build.

The check covers Cargo dependency entries, explicit `--day-version` arguments, and versioned
Day API links. Unpublished historical records and synthetic validator test inputs may retain
older versions. Path and workspace dependencies have no registry requirement to compare.

Shared reference messages live in `public/examples/reference-messages.ftl`. Localization links
to that downloadable file, and the excerpt compiler reads it directly. The unpublished
`docs/snippet-catalog.md` retains the recipe build setup; it is not a documentation destination.
