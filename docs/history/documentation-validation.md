<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Documentation validation record — October 2026

These are scoped results from the October 2026 documentation overhaul, using the local Day 0.6.0
working tree. They record what was executed, not universal platform or installation coverage.
See the [migration decisions](documentation-overhaul.md) and the
[current validation commands](../../website/authoring/authoring.md) before extending the checks.

## Tutorial

Six lesson stages were extracted from the published Markdown, formatted, built, and exercised
with AppKit walkthroughs. The final app passed Clippy and its reader-facing dayscript at initial
and minimum window sizes. After the installation instructions changed to Git dependencies, the
six-stage check ran again with the documented manifest saved before local-path substitution.

The tutorial check covers first window, events, state, layout, navigation, and the complete app.
Intermediate walkthroughs are synthetic fixtures; the final script comes from the lesson.
The run used installed macOS tools and cached dependencies. It did not test an empty-cache
installation, remote Git revision, Windows, Linux, or a novice reader's completion.

## Recipes and priority guides

Twelve designated recipe modules were extracted with their manifests, resource files, Fluent
catalogs, and build scripts. They passed formatting, Cargo check, Clippy with warnings denied,
two storage/command contract tests, and seven mock interaction tests. These are bounded local
fixtures, not real permission prompts, HTTP service calls, app-store submissions, or platform
packaging tests.

The priority review covered reactivity, layout, navigation, resources, localization,
accessibility, storage, HTTP, permissions, commands, and packaging. Thirty-two additional Rust
fragments compiled with labeled synthetic context; ten further blocks were covered by complete
recipes. Fragment compilation does not turn an excerpt into a standalone app or prove native
behavior. Three bounded HTTP response tests used scripted transports without network requests.

The [original technical-review ledger](documentation/coverage.csv) records page-level review
coverage at that checkpoint. It is historical: later edits and successful editorial checks do
not automatically establish semantic API review. The final overhaul block ledger counted
449 Rust blocks, with 54 previously checked and 395 without a recorded compile pass.

## Platforms and distribution

The [platform evidence record](documentation/support.json) distinguishes source/configuration
review from execution. Local AppKit execution and the separate HarmonyOS compile check do not
certify all targets, packaging, signing, stores, or remote CI. HarmonyOS emulator acceptance
remains CI-only. No support tier was promoted by the documentation overhaul.

## Reproduction

The permanent validators are `scripts/docs/verify-tutorial.py`, `scripts/docs/verify-recipes.py`,
and `scripts/docs/verify-excerpts.py`. They create new isolated sessions and retain source hashes,
commands, and results under `.cache/docs/`. The full original logs, phase reports, and recovery
snapshots are retained in the external `daybrite/misc/docs-overhaul/` archive. A current change
must produce its own evidence rather than inherit a pass merely because its topic appears here.
