<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Documentation overhaul — October 2026

This record preserves the decisions and remaining work from the documentation and website
overhaul completed in October 2026. It is a historical record, not a declaration that every
retained example or platform has been tested.

## Result

App documentation is organized into tutorials, how-to guides, reference, and explanation.
Framework development, backend contracts, design proposals, and history have separate contributor
routes. Canonical repository documents publish through symlinks; the route catalog determines
their public locations without duplicating prose.

The homepage retains its varied native screenshot carousel. `/gallery`, its per-app pages, and
`/showcase` coexist with `/examples`. Gallery and Showcase were restored after an intermediate
revision removed them. The header is Gallery, Showcase, Docs, and Explore; Contributing and
GitHub are inside Explore. App creators install the CLI with the Git-based Cargo command,
with PATH troubleshooting kept separate from the primary installation step.

The API artifact builds after Astro so the site build cannot delete it. A manifest-driven check
requires all documented crate entry pages before upload. Site link checks and API item-link
checks remain separate.

## Accepted decisions

- Review common app-building tasks first and track the remaining technical corpus explicitly.
- Provide equivalent macOS, Windows, and Linux beginner entry paths, with separate installation
  validation. The initial local walkthrough covers AppKit; the other paths remain follow-up work.
- Lead with supported targets and retain experimental, development, and deprecated combinations
  in the technical reference. A support tier is not proof of a successful runtime test.
- Use repository evidence for technical claims. The existing gallery and Showcase release inputs
  were explicitly restored; that is not permission to invent screenshot provenance or comparisons.
- Allow English literals in the public website during this overhaul. Day app examples and embedded
  app UI still require generated Fluent accessors, generated resource constants, and symbolic IDs.
- Defer named documentation/support ownership. Record technical review per change without
  assigning a permanent owner by inference.
- Fix current internal links without adding public redirects for old external URLs. Build-time
  link migrations and Rustdoc's generated aliases are separate mechanisms.

## Evidence and remaining work

The Phase 7 local site build passed for 230 pages and 3,454 internal links. The API bundle
contained 25 crate entries. Chrome checks covered the maintenance page at desktop and phone
widths, themes, real search, keyboard focus, and navigation into and out of the API reference.
These counts describe that checkpoint, not every future build.

The example ledger at that checkpoint recorded 449 Rust blocks: 54 with prior validation and
395 without a recorded compilation pass. The six tutorial stages used a saved documented
manifest and a local-framework fixture. That evidence does not verify an empty-cache Git
installation or all native targets. Marc subsequently reviewed and accepted the documentation
and tutorial instructions.

Remaining work includes the retained technical corpus, Windows/Linux beginner setup parity,
fresh-host installations, screen-reader and native accessibility sessions, additional browser
engines, physical devices, and distribution testing. Public website localization and named
ownership retain their explicit deferrals. Record subsequent verification against its own
revision rather than editing these historical counts.

## Permanent maintenance tools

Use the [authoring workflow](../../website/authoring/authoring.md),
[style guide](../../website/authoring/style-guide.md), and
[maintenance checklist](../../website/authoring/maintenance.md). Reusable validators live in
`scripts/docs/`; they create fresh disposable sessions under `.cache/docs/` without depending
on the overhaul workspace.

The full phase reports, decisions, ledgers, and local recovery/build material were moved outside
the Day repository to the maintainer's `daybrite/misc/docs-overhaul/` archive. They are not inputs
to site builds or CI. This compact record preserves the accepted direction inside the repository.
The previously accepted lockfile incident is closed and is not an outstanding review decision.
