<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Day documentation style guide

Follow the [writing style guide](../../STYLE_GUIDE.md) for tone, wording, grammar, and code comments.
This guide adds documentation structure, routing, and example requirements. Follow repository
contributor instructions first. The [authoring workflow](authoring.md) supplies commands and
scope; the [shared glossary](../../website/src/lib/glossary.mjs) supplies definitions.

## Choose one purpose

Choose one primary Diátaxis type before drafting. Keep app-developer instructions separate
from framework implementation details and proposals. Link between types instead of copying
whole explanations. Preserve useful technical depth when splitting a page.

| Type | Reader needs | Include | Keep elsewhere |
| --- | --- | --- | --- |
| Tutorial | Learn through a working sequence | Outcome, prerequisites, environment, steps, complete example, expected result, next step, troubleshooting | Exhaustive options |
| How-to | Complete one concrete task | Applicability, prerequisites, procedure, verified example, caveats, verification, related reference | A general introduction |
| Reference | Look up an exact contract | Signature, parameters, defaults, returns, errors, lifecycle, threading, target limits, examples | Unimplemented proposals |
| Explanation | Understand a model or tradeoff | Question, mental model, mechanics, alternatives, related tasks | A long procedure |
| Product | Evaluate a capability | User problem, specific capability, evidence, limitations, next action | Unsupported superiority claims |

Product pages are outside Diátaxis. Contributor pages still need a primary type. Label design
proposals as proposals, identify implementation status, and link to shipped contracts separately.
The [templates](templates/README.md) are drafting aids, not new public routes.

## Write for the reader

Lead with the outcome and prerequisites. Define unfamiliar terms at first use and link to the
canonical glossary entry. Do not use Piece, Part, widget, control, or window as synonyms.

Use descriptive link labels. Identify a linked API or task instead of `click here`. Keep heading
levels sequential. Write useful image alt text, and explain diagrams in surrounding prose.
A decorative image needs an explicit accessibility review; the Markdown check rejects
empty alt text. Review keyboard use, focus visibility, and contrast when changing rendered UI.

## Follow the existing source conventions

- App docs live in `website/src/content/docs/`. Their front matter supplies `title` and
  `description`; the layout supplies the H1. Start body headings at H2. Inspect
  [the collection schema](../../website/src/content.config.ts) before adding metadata.
  Public routes, audiences, and primary types live in the single
  [content route catalog](../../website/src/data/content-routes.json); source paths do not
  determine public URLs. Product and contributor guides also use this collection.
- Existing internal docs remain canonical in `docs/`, with one H1 and no required front matter.
  Every Markdown page, including nested design and history records, needs a matching symlink in
  `website/src/content/internal/`, a route catalog entry. Sidebar placement follows the route and audience.
  The [symlink check](../../scripts/ci/docs-symlinks.sh) verifies both directions.
  Do not duplicate its prose on the website.
- Contributor standards stay canonical in `website/authoring/`. The style guide and authoring workflow publish
  through symlinks under `/contributing/documentation/`. Draft templates remain repository-only.
- Use fenced code blocks with a language. Use `text` for conceptual output or pseudocode.
  Keep snippets distinct from complete, runnable examples.
- Link website docs to their catalog routes under `/docs/tutorials/`, `/docs/how-to/`,
  `/docs/reference/`, `/docs/explanation/`, or `/contributing/`. In published docs, link repository files
  to GitHub `blob/main` and directories to `tree/main`, as the website instructions require.
  Repository-only standards may use relative source links. A valid path is not proof of a claim.
- Keep GitHub-compatible admonition markers on their own line in canonical internal docs.
  Follow the [website README](../../website/README.md) for site-only extensions.

## Verify code and commands

Verify every public symbol against implementation, facade re-exports, and feature gates in the
current checkout. Comments and earlier docs can be stale. Record the revision, target, command,
and result in the change description or maintenance record. Keep audit status out of app-facing
prose; explain concrete platform limits where they affect the reader. Include imports, manifests, resource
catalogs, and setup commands for designated runnable examples. Test the exact example readers
receive; a nearby integration test is supporting evidence, not a compilation result for it.

Label fragments “Conceptual fragment; not standalone” or “API excerpt; not a complete app”.
Never mark an unexecuted example as tested. Separate PASS, FAIL, and NOT RUN, with the reason
for each missing check. Verify command flags against the CLI parser and, when allowed, help
output. Do not invent APIs, defaults, errors, lifecycle rules, or platform support.

All app-owned UI strings must use generated localized accessors such as `res::str::loading_text()`.
Provide the matching catalog key; this identifier is illustrative until generated for that app.
Use generated `res::images`, `res::vectors`, `res::fonts`, and `res::assets` constants for bundled
resources. Do not bypass them with literal resource names or include paths. Embedded web UI
receives translations from generated accessors, with no English fallback. Format localized
messages on the UI thread after asynchronous work. Open bundled web documents through their
generated AssetDir constant; links inside the document stay relative to it.

Mark synthetic fixtures and user-supplied or protocol text explicitly. A teaching example is
not an exemption from localization or resource rules. Explain pedagogical numeric shortcuts
and replace them with production guidance where needed. The
[maintainer decision](../../docs/history/documentation-overhaul.md#accepted-decisions)
defers public website localization and allows English literals there for this overhaul.
That exception does not apply to Day app examples or embedded app UI.

## Support claims with evidence

Distinguish API presence, runtime capability, maintenance tier, successful compilation, and
runtime testing. Specify OS/toolkit target, version or revision, limitations, verification date,
and source/test evidence. Unknown support stays unknown. Read the current
[platform metadata](../../website/src/lib/platforms.mjs), implementation, and CI configuration
before updating claims; a tier declaration does not establish that a platform passed tests.

For screenshots, record image source, app source revision, framework revision, platform, capture
procedure/date, and attribution. Mark missing provenance unverified. Never attribute a capture
to an example from appearance alone. Competitor comparisons need dated, reliable sources and
fair, specific claims. Repository-only work must leave external facts unverified.

## Apply checks and human review

The [authoring workflow](authoring.md) defines the executable editorial scope and limitations.
Mechanical checks cover selected prose, Markdown structure, and local source links; existing
site checks cover rendered internal links. Semantic accuracy, accessibility, screenshot
provenance, localization correctness, and example compilation require separate review.
The human-readable guide is canonical; keep agent instructions short and link here.
