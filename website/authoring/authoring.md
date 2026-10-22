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
rendered checks. Source lint does not verify API semantics, root-relative site routes, or external
URLs. These checks use existing locked dependencies; they are not full Vale or markdownlint runs.

## Validate app examples

On macOS with the documented AppKit tools, build the local CLI and verify the tutorial:

```sh
cargo build --locked -p day-cli
python3 -B scripts/docs/verify-tutorial.py --run-native
```

The validator extracts the exact lesson files, builds six stages, runs AppKit walkthroughs, and
lints the final app. Omit `--run-native` to compile without launching. It saves the documented
Git-dependency manifest before substituting local framework paths in its isolated fixture.
This checks lesson code against the checkout, not a fresh Git installation or remote revision.

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

Follow [the artifact assembly procedure](maintenance.md#assemble-and-review-a-publication-artifact)
to add the API reference after the site build. Then check the API's internal links with:

```sh
python3 -B scripts/docs/check-api-links.py
```

Both Python rendered checkers accept an optional path to another built site directory. They
create their own result directory and require no prior validation session. Review responsive
layout, search, keyboard focus, themes, and navigation when shared templates change.

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
