<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Contributing to Day

Day is a young project. Its architecture is still moving, and the most useful contribution
right now is a conversation, not a patch.

## Start with a conversation

- For an idea, a question, or a design concern, open a thread in
  [GitHub Discussions](https://github.com/daybrite/day/discussions).
- For a bug, file a [GitHub Issue](https://github.com/daybrite/day/issues) with the target
  platform, the Day version, and steps to reproduce — a failing `dayscript` file is the best
  reproduction there is.

**Please do not open a pull request without a discussion or issue that concluded a patch is
wanted.** A change that looks small on the surface often crosses nine backends, the docs, and
the design record, and reviewing an unexpected patch costs more than talking the change through
first. PRs without a linked conversation are likely to be closed with a pointer to this file.

## When a patch is agreed on

Once a discussion lands on "yes, send a patch":

1. Reference the discussion or issue in the PR description.
2. Build and run through the `day` CLI (`day build`, `day launch`), not bare `cargo`.
3. To exercise your change in a real app (the showcase, or your own), build that app against
   your checkout with `day patch --local` —
   [Developing Day and an app together](https://daybrite.dev/docs/local-development) covers the
   workflow. A framework feature usually lands with a Day-Showcase screen that demonstrates it,
   as a second PR that follows the framework change.
4. Before pushing, run `cargo fmt --all`, `scripts/ci/lint.sh` (the full fmt + clippy matrix CI
   runs), and `cargo test` for the crates you touched.
5. Update the documentation the change affects in the same PR: the relevant `docs/*.md` page,
   and the `DESIGN.md` section that describes what you changed.

## Platform support tiers

Every `(OS, toolkit)` target sits in a support tier, from Tier 1 (supported: thoroughly tested,
highest attention to quality) down to Tier 4 (development: compatibility combinations no app
ships on). The [Platform support](https://daybrite.dev/docs/platforms#support-tiers) page defines
all four and says where each target stands today.

A tier records the maintenance a target has, not a verdict on the platform, so a target moves up
when people show up to keep it there. Taking a platform up a tier means taking on the work that
tier promises:

- an owner for the backend who reviews the patches that touch it;
- the showcase walkthrough run on real hardware before a release, not only in CI;
- triage for that platform's bug reports, with reproductions the rest of us can run;
- its toolchain kept green in CI as SDKs, signing, and packaging move.

If you or your team can commit to that, open a
[Discussion](https://github.com/daybrite/day/discussions) naming the target and what you can
cover. Tiers move down the same way: a target whose maintainer moves on, or whose CI leg stays
red, drops until someone picks it up.

## Contribution terms

By submitting a contribution to this repository — a pull request, a patch, or any other
material intended for inclusion — you agree to the following:

1. The contribution is your own work, or you otherwise have the right to submit it under these
   terms.
2. Copyright in the contribution is transferred to The Daybrite Project upon submission.
3. Your contribution is published under the project's licenses (below), and the submission
   record is public and permanent.

A single copyright holder keeps the licensing of every file uniform and lets the project act on
license questions for the work as a whole. If you cannot agree to the transfer, say so in the
discussion before any code is written — do not open a PR.

## Licenses

- Code is licensed under the Mozilla Public License 2.0 — see [LICENSE](./LICENSE).
- The documentation (`docs/`) and the website (`website/`) are licensed under Creative Commons
  Attribution-ShareAlike 4.0 — see [docs/LICENSE](./docs/LICENSE) and
  [website/LICENSE](./website/LICENSE).
- Source files carry a `Copyright © The Daybrite Project` line and an
  `SPDX-License-Identifier` header naming the license that applies to them.
