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

Product pages are outside Diátaxis. Contributor pages still need a primary type. Keep design
records and proposals in the repository with `published: false` in the content route catalog;
do not publish or link to them from the website. Public reference pages describe current contracts.
The [templates](templates/README.md) are drafting aids, not new public routes.

## Write for the reader

Lead with the outcome and prerequisites. Define unfamiliar terms at first use and link to the
canonical glossary entry. Do not use Piece, Part, widget, control, or window as synonyms.

Start every reference page with one or two original sentences defining the topic and its useful
role. Explain the component or capability before naming its crate or API. Keep the description
independent of an input device or operating system unless the topic itself is platform-specific.
Design guidance can inform the concepts, but write the introduction in Day's voice rather than
copying or closely paraphrasing another guide. Replace a repeated definition instead of adding
another one. Follow the introduction with Day-specific setup and contracts, preserving platform
limits and warnings near the affected behavior.
Keep titles focused on the capability; omit parenthetical packaging labels such as “external
piece” or “headless capability crate.” For a crate reference, follow the introduction with
“Add to your app”: explain only the setup needed beyond a normal Day app.
For built-in controls, state that the control is included in `day-pieces` and re-exported through
`day::prelude::*`, with no additional crate or control feature required. Do not repeat the base
`day-app` dependency or the `day new` installation instructions. For optional components and
parts, name and link the required crate or feature and show only the additional Cargo or
`Day.toml` configuration. Use registry versions for published dependencies. Then
summarize availability by platform/toolkit. Distinguish native, composed, placeholder, and
unavailable behavior. A feature declaration or successful build alone does not establish runtime
support. Use the standard target icons and tints alongside readable platform names.
Introduce the task before signatures, toolkit machinery, or examples. Do not lead with
“Status: implemented,” “Shipped,” dates, or test-run summaries. Put concrete limitations beside
the affected API or platform, in plain language. Keep an early warning only when an experimental
or unavailable feature would change whether the reader should use the page.
Omit visible `DESIGN §…` citations and architectural section numbers in headings. Use a relevant
public guide link when readers need context. Maintainer citations may remain in HTML comments.

Use descriptive link labels. Identify a linked API or task instead of `click here`.
For internal documentation links, omit `.md` and `.mdx` from the visible label: write
`[element IDs](/docs/reference/element-ids)` or “the element-ID reference.” The file
extension belongs only in an actual source-file reference, not in the name of a website page.
Keep repository-relative Markdown destinations when needed for GitHub rendering; only the label
loses its extension. Keep heading
levels sequential. Write useful image alt text, and explain diagrams in surrounding prose.
A decorative image needs an explicit accessibility review; the Markdown check rejects
empty alt text. Review keyboard use, focus visibility, and contrast when changing rendered UI.

## Link related documentation

When prose or a table names another Day capability, crate, guide, or documented concept, link
that reference to its relevant page. For example, write
“Like [`day-part-device`](/docs/reference/battery) and
[`day-part-connectivity`](/docs/reference/network)” rather than leaving the crate names as plain text.
Use the guide for a capability or crate reference; use Rustdoc for an exact function, type,
method, or variant. Keep explicit links to source files when discussing their implementation.

Use root-relative destinations for guides, such as `/docs/reference/battery`. Use absolute
Rustdoc links, such as `https://docs.rs/day-part-haptics/latest/day_part_haptics/fn.is_supported.html`, so local
previews open the published API documentation. Preserve
query strings and heading fragments. External projects with no Day guide may link to their
repository or documentation; do not invent a local destination. Do not link a page's own name
back to itself, or insert Markdown links into code fences, commands, resource names, or filenames.
Use links in the explanation beside a code example so the code stays copyable.

Check cross-references across the whole page, including comparisons, “see also” text, and tables.
The [crate-to-guide map](../src/data/reference-crates.json) records destinations shared with
generated tables. Update it when adding or moving a crate's guide. Run the rendered website and
guide-to-Rustdoc link checks after changing references; checking that a destination looks plausible
is not enough.

## Name and label toolkits

Use **toolkit**, not **backend**, for Day's UI implementations: “Toolkit implementations”
and “Toolkit notes.” Keep actual API identifiers, feature names, source paths, and URLs unchanged.
“Backend” remains appropriate for a server, storage provider, or other non-UI implementation.

In platform tables, headings, and notes, use the human-readable names (AppKit, UIKit, GTK, Qt,
Android, WinUI, ArkUI, or Web). Keep `web-dom` when listing full target IDs alongside names such
as `macos-appkit` or `android-material`, and in commands, configuration, paths, and URLs.
Use **WinUI** for the current Windows UI toolkit and **Windows** for OS services shared across
toolkits. Reserve **XAML** for markup, geometry, API terminology, and explicitly identified
legacy system XAML. Preserve identifiers such as `day-toolkit-xaml-sys`, `xaml`, and `Microsoft.UI.Xaml`.
Link current WinUI controls to the Windows App SDK API reference (`Microsoft.UI.Xaml`), not
the older UWP reference (`Windows.UI.Xaml`). Historical design records retain their original
platform names and execution claims.
The Markdown renderer adds the site's platform icons
and light/dark tint colors throughout headings, table cells, bold labels, and inline target IDs,
including lowercase feature labels and names inside links or mixed Markdown. It also marks the
first mention of each target in each paragraph of a platform-specific section (for example,
“Toolkit implementations” or “Toolkit notes”). Use a bold lead such as `**UIKit:**` for a
target-specific list item; do not add emoji or hand-picked colors. Linux and Apple family labels
carry both relevant target marks rather than implying a single toolkit. Review the rendered
per-toolkit sections, not just the installation/availability summary at the top of the page.
When one OS has several toolkits, name the affected toolkit rather than implying they all behave
alike. Keep the visible text: icons and color must never be the only way to identify a target.

Omit the deprecated system XAML (`windows-xaml`) target from public reference platform notes,
tables, and navigation. Describe current Windows behavior for WinUI; do not transfer legacy
target limitations or verification claims to WinUI.

Omit framework verification reports, test counts, and host-specific build diaries from public
reference pages. Retain instructions that help readers test their apps. Extract relevant limits
into standard admonitions beside the affected feature, with a bold platform/toolkit lead:

```markdown
> [!WARNING]
> **Windows / WinUI: alpha selection.**
> Native alpha selection is unverified. Test transparent color selection before relying on it.
```

Use `NOTE` for a behavior difference and `WARNING` for a limitation that could affect correctness
or release readiness. Preserve uncertainty; removing a test report must not turn unverified
behavior into a support claim.

## Link platform guidance and native APIs

In platform realization sections, link to the platform or toolkit maintainer's documentation.
Include relevant design or usage guidance as well as API references: for haptics, link Apple's
[haptics guidance](https://developer.apple.com/design/human-interface-guidelines/playing-haptics)
and Android's [haptics principles](https://developer.android.com/develop/ui/views/haptics/haptics-principles),
then link native types such as
[`UIImpactFeedbackGenerator`](https://developer.apple.com/documentation/uikit/uiimpactfeedbackgenerator)
and [`VibrationEffect`](https://developer.android.com/reference/android/os/VibrationEffect)
where the mapping names them.

Link to the exact API when possible, or its owning type when a stable member URL is unavailable.
Check the destination, version, and namespace: GTK 4 differs from GTK 3, system XAML differs
from WinUI, and identically named Android and HarmonyOS APIs are not interchangeable.
Use official documentation or the maintainer's documentation repository, not third-party
tutorials or search results. Keep links next to the relevant table entry or explanation and
leave code fences copyable. A platform link explains native behavior; it does not establish
that Day supports every feature of that API. Preserve Day's support limits alongside it.

## Follow the existing source conventions

- App docs live in `website/src/content/docs/`. Their front matter supplies `title` and
  `description`; the layout supplies the H1. Start body headings at H2. Inspect
  [the collection schema](../../website/src/content.config.ts) before adding metadata.
  Public routes, audiences, and primary types live in the single
  [content route catalog](../../website/src/data/content-routes.json); source paths do not
  determine public URLs. Product and contributor guides also use this collection.
- Assign every public reference page to one task group in
  [reference-navigation.json](../src/data/reference-navigation.json). The sidebar and reference
  index share these groups; adding an uncategorized page fails the architecture check.
  Use a short `navTitle` in front matter when the sidebar needs less detail than the page title.
  Use plurals for categories of components or objects, such as Buttons, Pickers, and Text areas.
  Keep concepts, activities, and technology names in their natural form, such as Progress,
  Scrolling, and SwiftUI. Apply this convention consistently within each group.
  Navigation entries sort alphabetically by their sidebar label within each group. App and
  contributor tutorials retain their lesson sequence through `order` instead.
- Existing internal docs remain canonical in `docs/`, with one H1 and no required front matter.
  Every Markdown page, including nested design and history records, needs a matching symlink in
  `website/src/content/internal/` and a route catalog entry. `published: false` keeps a source
  out of routes, navigation, search, and the sitemap. Sidebar placement follows audience and topic.
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

## Specific headings and toolkit names

Do not use `Why it matters`. Name the subject or action, such as “Verify a release against its source.”
Avoid toolkit inventory counts and ordinal labels. Name the relevant toolkits or link to the
platform table; keep technical constraints such as one toolkit per binary. Component references
explain app behavior, setup, examples, and limits. Keep design alternatives, patch protocols,
regression histories, and contributor task lists in contributor documentation.
