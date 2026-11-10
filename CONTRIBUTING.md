<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Contributing to Day

Contributions are currently invitation-only. Open pull requests are not accepted at this time.

You are welcome to start a discussion or file an issue in any
[Daybrite repository](https://github.com/daybrite). For Day itself:

- Use [Discussions](https://github.com/daybrite/day/discussions) for questions, ideas, and
  design concerns.
- Report bugs in [Issues](https://github.com/daybrite/day/issues). Include the platform,
  Day version, and steps to reproduce. A small example or failing DayScript walkthrough helps.

## Show, don't tell

If you'd like to be invited to contribute, publish a novel, interesting app built with Day.
Share something people can try, and keep improving it as you learn from their feedback.

We look for people who intend to maintain their apps over time and help maintain the parts of
Day those apps depend on. That includes reproducing bugs, testing fixes on the platforms they
use, and taking responsibility for ongoing framework work. Share your app and what you're
prepared to maintain in a discussion.

## For invited contributors

Agree on the scope and submission process with a maintainer before starting. Read the
[contribution terms](#contribution-terms) below.

### Preparing the patch

1. Link the agreed discussion or issue when submitting your work.
2. Install the repository hooks once per clone with `bash scripts/install-hooks.sh`.
3. Build and launch apps through `day build` and `day launch`. Use `day patch --local` to
   test against your checkout; see [local development](/contributing/local-development).
   Include a Day-Showcase example for a new framework feature, submitted after the framework change.
4. Run `cargo fmt --all` and `bash scripts/ci/check-ready.sh` against all outstanding changes
   before reporting work ready to commit or push. The gate checks formatting, generated
   matrices, host Clippy with test targets and warnings denied, and website source checks
   against a fresh Rustdoc build. Install the locked website dependencies with
   `npm --prefix website ci` on a fresh checkout and after package-lock changes. Regenerate stale tables
   with their `scripts/ci/*-matrix.sh` generators and rerun the gate.
5. Run `bash scripts/ci/lint.sh` for the available platform Clippy checks and `cargo test` for
   the crates you touched. Report any checks you could not run. The hooks and build tests do
   not replace these checks.
6. Update the affected documentation and `DESIGN.md` in the same PR as a framework change.

For documentation changes, follow the [writing style guide](STYLE_GUIDE.md) and the
[authoring workflow](website/authoring/authoring.md), which links to the page conventions,
templates, and validation commands.

### Dependency policy before Day 1.0

Daybrite-owned crates and apps use `"0.*"` for dependencies on Day and other Daybrite-owned
0.x crates, including external parts and pieces. Preserve path sources and workspace
inheritance. Package release versions remain concrete; third-party requirements are unchanged.
Test-only path dependencies stripped during publication omit the version field to avoid
publishing reverse dependencies and creating cycles.

Source and behavior changes between 0.x releases are expected. Apps commit `Cargo.lock`, build
with `--locked`, and test dependency updates before shipping. A library's lockfile does not pin
its consumers' dependencies. Preserve the wildcard policy through 0.x releases; revisit it at 1.0.

## Creating extensions and crates

You can publish independent extensions without an invitation. A
[part](/docs/reference/glossary#part) adds a capability without UI; a
[piece](/docs/reference/glossary#piece) adds reusable UI. The
[extension model](/contributing/extending) explains the implementation choices.

Scaffold with `day new part <name>` or `day new piece <name>`, then follow the
[part](/contributing/tutorials/part), [native-piece](/contributing/tutorials/native-piece), or
[composite-piece](/contributing/tutorials/composite-piece) tutorial. Test each supported target
and document platform limits and permissions. Localize user-facing text and use generated
resource constants for bundled assets.

### Naming

The `day-` prefix is reserved for crates hosted under the
[daybrite organization](https://github.com/daybrite) and forks of those repositories.
Independent extensions use `dayext-`.

Please respect the Day™ name and this naming convention. We may ask maintainers to rename
crates that imply an affiliation with the Daybrite project.

Use lowercase, hyphen-separated names:

| Extension | Naming pattern | Example |
| --- | --- | --- |
| Platform-specific part | `dayext-part-platform-feature` | `dayext-part-linux-power-profile` |
| Toolkit-specific piece | `dayext-piece-toolkit-feature` | `dayext-piece-gtk-source-editor` |
| Part supporting multiple platforms | `dayext-part-feature` | `dayext-part-power-profile` |
| Piece supporting multiple toolkits, or a composite piece | `dayext-piece-feature` | `dayext-piece-rating` |

Use an OS name such as `linux` for `platform`, or a toolkit such as `gtk` for `toolkit`.
Omit that segment for shared implementations. Other extension crates use `dayext-feature`.
The examples above illustrate names, not published crates.

## Platform support tiers

[Platform support](/docs/reference/platforms#support-tiers) describes the maintenance each
target receives. To help maintain a platform, open a discussion describing what you can cover:

- Review changes to its toolkit implementation.
- Run showcase walkthroughs on real hardware before releases.
- Reproduce and triage platform bugs.
- Keep its CI builds working as SDKs, signing, and packaging requirements change.

A target's tier can rise or fall as its maintenance and testing coverage changes.

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
