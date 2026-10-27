# daybrite writing style guide

For public-facing text: the docs site, READMEs, website copy, app-store listings, release notes,
changelogs, marketing, and UI microcopy. The aim is plain, specific writing.

Follow this whenever you write or edit that kind of text. Code comments follow the rules in
[Code comments](#code-comments) below; the rest of the guide is good advice for them too.

## Principles

- Write for a reader in a hurry. Lead with the point; put the payoff in the first sentence.
- Be specific. Name the thing, the number, the file, the command. Cut adjectives that carry no
  information.
- Prefer the plain word. If a shorter, more common word works, use it.
- Active voice, present tense: "Day builds the tree once," not "The tree is built once by Day."
- Don't sell. Say what it does and what it costs; let the reader decide.
- One idea per sentence. Prefer short sentences.
- Cut the last paragraph when it just restates the first.
- Begin reference pages with one or two original sentences defining the topic and its useful
  role. Follow with Day-specific setup and contracts. Keep the opening independent of an input
  device or operating system unless the topic itself is platform-specific. Omit
  implementation-status banners; explain concrete limitations beside the affected feature or platform.
- Link native API names and platform-specific behavior to first-party documentation, including
  design guidance where useful. Verify the target and toolkit version. See the
  [documentation linking guidelines](website/authoring/style-guide.md#link-platform-guidance-and-native-apis).

## Minimize LLM-isms

These words and constructions read as machine-generated filler. Do not use them. This list is a
starting point, not the whole set — when a phrase sounds like padding or self-praise, cut it.

**Self-praise and vibes:** genuine, genuinely, honest, honestly, "honest assessment", "to be
honest", truly, seamless, seamlessly, seam / seams (as a metaphor or as jargon for a boundary:
name the trait, shim, or FFI boundary instead), elegant, powerful, robust, delightful, magical,
buttery-smooth, bespoke, first-class (as praise), proven / battle-tested (say what test it passed
instead), "source of truth".

**Marketing inflation:** unleash, unlock, elevate, empower, supercharge, revolutionize,
game-changer, cutting-edge, state-of-the-art, best-in-class, world-class, next-generation, blazing
fast, lightning-fast.

**Cliché metaphors:** journey, tapestry, landscape, realm, beacon, testament, "testament to", "at
the heart of", "in the world of", load-bearing (say what actually depends on the thing instead),
hard-won (the reader can't verify the winning; state the lesson itself).

**Filler openers and transitions:** "It's worth noting that", "worth knowing", "It's important to
note", "In today's … world", "Let's dive in", "dive into", "deep dive", "delve", "delve into", "at
the end of the day", "in practice" as an opener, "simply" as filler, "the whole trick / story /
setup / point", "that is the point", "earns its keep", "on sight", and furthermore / moreover used
as decoration.

**Vague quantities and hedges:** arguably, essentially, fundamentally, basically, "a plethora of",
"a myriad of", "a wide range of", countless, "a variety of" (name the count or the items instead).

**Inflated verbs:** leverage, utilize, facilitate, harness, streamline, foster, curate, boast /
boasts, showcase (in prose), architect (as a verb).

**Formula sentences:** "Not only X, but also Y", "It's not just X — it's Y", "Whether you're X or
Y", and rule-of-three lists built for rhythm rather than content.

### Sentence-level slop patterns

These constructions read as machine-generated even when every word passes the lists above. Each
was caught in review; the replacements are the settled rulings — follow them.

- **Dramatic colon setups.** "The one difference that colors everything else: …" — don't
  announce a fact, state it. Fold the point into the paragraph with no lead-in.
- **Verbless fragments and aphorisms.** "Same shape; two differences." / "Same philosophy, one
  difference of degree." / "Structurally near-identical." Write complete sentences: "There are
  two differences." "The structure is nearly identical; the runtime behavior isn't."
- **Headline-style headings ("X, Y-ed").** "The mental model, mapped" / "A counter, twice" —
  headings name their content: "Flutter terms in Day", "The same counter in both".
- **No-X, no-Y constructs, pairs and triples alike.** "no VM, no bridge layer, and no separate
  layout tree" / "no drafts, no Save button" — rewrite as prose saying what IS there and why:
  "there is no VM in between, and layout is computed in the same process." / "each control
  writes the store directly, so every edit persists as it is made." A lone negation is fine;
  it is the listed absences that read as filler.
- **Cleft pivots.** "What differs is what happens on tap." → one plain contrast sentence: "Both
  build the same UI; they handle the tap differently."
- **Pivot fragments as glue.** "The limits differ, though." / "They part ways in two places." →
  a flat topic sentence: "Each framework limits styling differently."
- **Docs personified as truth-tellers.** "Styling states exactly where that line is" / "Why Day
  treats that choice honestly" → name the content: "Styling lists what you can restyle and what
  stays native." "Why Day lists what each approach costs and when Flutter wins."
- **Honesty labels.** "Be honest about the cost:", "The honest trade:", "an honest gap" — drop
  the label and state the cost: "Day has no hot reload."
- **Reader-feeling predictions.** "will feel familiar", "reads on sight", "the R. experience
  carries over" → the factual comparison itself: "Flutter and Day use the same layout: thin
  per-platform hosts next to shared code."
- **Personification and superlative drama.** "Flutter's ecosystem grew Provider…", "agree on
  the most important decision" → plain subjects, no superlative: "Flutter needs Provider or
  Riverpod to scope rebuilds; Day doesn't." "React Native and Day both render the platform's
  real widgets."
- **"Deliberately", "by design", "on purpose", "intentional" as vouching.** "There is
  deliberately no anonymous constructor" → cut the adverb and state the reason: "There is no
  anonymous constructor, because every web view needs an id for its events." A heading like
  "Deferred (deliberately)" is just "Deferred".
- **Counted colon setups.** "Three things to notice:" / "Two rules keep this stable:" → allowed
  only when a real numbered list follows; otherwise fold the items into prose.
- **Self-congratulation about what the reader did not write.** "You wrote zero platform code."
  → say what the code does: "The piece runs on supported toolkits from this one file."
- **ALL-CAPS emphasis.** "only ORDER changes reload", "the ONLY correct gate", "a crate's OWN
  piece" → lowercase, everywhere, prose and code comments alike. If the sentence is ambiguous
  without the stress, rewrite it so the words carry the distinction ("only a change to the order
  reloads the list"); *italics* is the last resort in prose and is not available in a comment.
  Capitals are for acronyms (UI, API), identifiers and constants (in `monospace`), environment
  variables, and comment markers at the start of a line (`TODO`, `SAFETY:`). A comment in all
  caps is an extraordinary event, reserved for a warning that must not be missed.
- **"Own" as a stress marker.** "the toolkit's own window", "its own GOT slot", "chrome of its
  own" → the possessive already says it: "the toolkit's window", "a GOT slot". Keep "own" only
  where it carries a contrast that the sentence needs ("Day's own pop calls, not UIKit's").
- **Single "no X" tag lines.** "No server needed.", "no project required" are the absence list
  in miniature → say what is there: "Notifications are scheduled on the device."

### Plain-word swaps

| Instead of | Write |
|---|---|
| leverage, utilize | use |
| facilitate | help, let |
| in order to | to |
| prior to | before |
| due to the fact that | because |
| in the event that | if |
| a number of | some, several, or the exact count |
| functionality | the feature, or what it does |
| performant | fast (with a number) |
| robust | reliable, well-tested (and say why) |
| commence | start |
| assist | help |
| additionally | also, and |

Use **toolkit**, not **backend**, when referring to a Day UI implementation. Preserve exact
identifiers and paths; “backend” can still describe a server or storage provider. Label platform
notes and tables with the standard target icons and tints, following the
[toolkit labeling guidelines](website/authoring/style-guide.md#name-and-label-toolkits).
Keep verification reports out of public references; retain app-facing limitations in platform-labeled
admonitions and keep useful instructions for testing apps.

## Code comments

Comments in source files (`//`, `///`, `//!`, `#`, `/* */`, and the comments inside the scaffold
templates that `day new` writes) are read by users of the code, so the rules that make prose
sound human apply to them too:

- Emphasis is lowercase (see the slop patterns above). All-caps in a comment is for acronyms,
  identifiers and constants, environment variables, and the markers `TODO`, `FIXME`, `NOTE:`,
  and `SAFETY:`. A section banner is sentence case: `// --- Part ---`.
- "Own" stays out unless it carries a contrast, and clauses are joined with a period, a
  semicolon, a colon, or a conjunction rather than an em dash (see Grammar and mechanics).
- The banned words and constructions above apply. In comments the frequent ones are "seam"
  for a boundary (name the trait, shim, or FFI function instead), "deliberately"/"by design"/
  "on purpose" as vouching (state the reason instead), "honest", "genuine", "simply", "the whole
  point", "source of truth", "for free", "smoke test", the "no X, no Y" list, and "not just X
  but Y".
- State the fact and the reason in plain sentences. Dramatic colon setups, verbless aphorisms,
  and rhetorical questions belong to marketing copy, and not to ours.
- Keep lines at or under 100 characters, matching `rustfmt`'s default width.
- Scaffold and template comments stay terse and link to the public docs instead of explaining
  inline: what the file is, one example, a `https://daybrite.dev/docs/...` link.
- Historical records (`DESIGN.md` Part II, changelogs) narrate change; everything else describes
  the code as it is now, not the diff that produced it.

Internal documentation link labels omit `.md` and `.mdx`; use a page name or route label.
Keep extensions only when referring to actual source files. Reference titles omit packaging
parentheses; place crate dependencies and platform availability after the introduction, following
the [documentation structure guide](website/authoring/style-guide.md#write-for-the-reader).

## Tone

- Link references to other documented Day capabilities, crates, and guides to their relevant
  internal pages. Link exact API symbols to Rustdoc. Use root-relative site URLs (`/docs/...`,
  `/api/...`) for site content, and keep code examples copyable. Follow the
  [documentation cross-reference rules](website/authoring/style-guide.md#link-related-documentation).

- Confident, not boastful. Back every claim with a specific.
- Address the reader as "you" in guides and tutorials. Use the third person for reference docs.
- Dry humor is fine in small doses. No exclamation marks in reference material.
- Don't apologize for the software or hedge every sentence.
- Describe Day by what it does. When a sentence defines Day against another technology ("not a
  webview", "no bundled browser", "unlike Electron"), rewrite it around Day's own behavior.
  Stay neutral and friendly toward other stacks, web views included. Migration guides may name
  the other framework, describe a mechanism difference factually, and keep a balanced "pick X
  when / pick Day when" pair. Comparison tables and cards on the marketing pages list what each
  approach is good at; the costs belong on the Why Day page, stated as facts.

## Grammar and mechanics

- American English spelling: color, localize, behavior. One exception: **cancelled** keeps the
  double l. `HttpError::Cancelled` is public API and GitHub Actions' own `cancelled()` is not ours
  to rename, so spelling the prose "canceled" would leave every sentence disagreeing with the
  identifier it describes. American usage accepts both.
- In `day/`, this rule is enforced rather than remembered: `typos` runs over the whole tree in CI
  (the `spelling` job), configured by `day/typos.toml` with `locale = "en-us"`. Run `typos` before
  you push, and `typos --write-changes` to apply the fixes it can make itself. Exceptions belong in
  that file, each with a line saying why.
- Oxford comma.
- Sentence case for headings and titles. Keep product and proper names as written: Day, Day Skies,
  DayScript, `Day.toml`, Open-Meteo, AppKit, UIKit, GTK, Qt, WinUI, ArkUI, HarmonyOS.
- Numerals: spell out zero through nine in prose; use digits for 10 and up, versions, measurements,
  and anything technical (a 10-day forecast, 3 backends, `forecast_days=10`).
- Put commands, file names, flags, and identifiers in `monospace`.
- Contractions are fine (it's, don't); they read as natural speech.
- Use the em dash rarely. A dash that splices two clauses reads as machine-written; use a
  period, a semicolon, a colon, "because"/"so", or parentheses, whichever the sentence needs.
  Never write ` -- ` for a dash. The same rule holds in code comments.

## Structure

- Front-load. The first sentence says what the page is for.
- Short paragraphs, one topic each.
- Headings that describe content. Avoid "Overview" / "Introduction" when a real title fits.
- Never use `Why it matters` as a heading or prose transition. Name the topic, behavior,
  or decision directly, such as “Verify a release against its source.”
- Do not count or number the toolkit inventory: avoid “the ninth toolkit” or “all eight
  toolkits.” Name the relevant toolkits or link to the platform support table. Keep actual
  technical constraints, such as selecting one toolkit per binary, explicit.
- Show a runnable example instead of describing one.

## Before / after

> Day genuinely offers a seamless, powerful way to leverage native widgets — it's worth noting that
> this is a game-changer for cross-platform development.

Rewrite:

> Day renders your UI with real native widgets — `UILabel` on iOS, a Material button on Android —
> from one Rust codebase.

---

> In order to utilize the mock data functionality, simply set the environment variable.

Rewrite:

> To use mock data, set `WEATHER_MOCK=1`.
