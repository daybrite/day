# Working on this Day app

This app uses native widgets through Day. Its starter has one main window, Welcome and Items
sections, and Settings in the desktop app menu or mobile navigation.

## Day documentation

Start with [For AI Agents](https://daybrite.dev/docs/for-agents), the concise reference for
Day's conventions, reactive patterns, APIs, and development workflow.

- [Machine-readable documentation index](https://daybrite.dev/llms.txt): discover the relevant
  guides and detailed references before implementing a feature.
- [API tour](https://daybrite.dev/docs/api-tour): examples of pieces, state, layout, navigation,
  localization, and accessibility.
- [Testing with dayscript](https://daybrite.dev/docs/dayscript): drive the app, assert behavior,
  and capture screenshots.

Use examples with the API supported by this project's Day dependency version. Check
`day <command> --help` for the installed CLI's options.

## Project map

- `src/lib.rs`: the window, typed routes, navigation, menus, and Settings and About through
  `day-piece-settings`.
- `src/model.rs`: the item model, the shared `Scene` state, its persistence, and unit tests.
- `src/pages.rs`: the Welcome page, the item list, and the item editor.
- `resource/locales/<locale>/app.ftl`: app-owned UI strings.
- `resource/`: bundled vectors, images, fonts, and data assets.
- `dayscript/demo.yaml`: the portable walkthrough and its screenshots.
- `Day.toml`: app identity, targets, window options, and permissions.
- `platform/android/AndroidManifest.xml`: optional Android manifest overlay; keep only app-specific
  declarations, following https://daybrite.dev/docs/platforms/android-mdc#manifest-customization.
- `platform/`: app-owned native sources and configuration; build projects are generated under `build/`.

## Conventions

- Add new user-facing strings, including errors and accessibility labels, to
  `resource/locales/<locale>/app.ftl` in every app locale. Reference them through auto-generated
  symbolic accessors such as `res::str::field_name()`, never raw unlocalized strings in UI code.
  Format messages on the UI thread when doing asynchronous work.
- Reference bundled resources through generated `res::images`, `res::vectors`, `res::fonts`,
  and `res::assets` constants. Mark monochrome UI glyphs with `.template()` so they follow the theme.
- Add accessibility information where native semantics are insufficient: localized `.a11y`
  labels for controls and meaningful images, heading roles for headings, and hints when an
  action needs explanation. Mark purely decorative images with `.decorative()`. Preserve native
  control roles and values; a scripting ID does not replace an accessible label.
- Give interactive pieces stable `.id(day::id!(snake_case_id))` names for scripting and
  accessibility.
- Use the `Space` scale (`Space::S`, `Space::L`, …) for gaps and padding, and semantic fonts
  (`Font::Headline`) for text, rather than point values.
- Keep shared state in `Scene`, accessed through `Scene::ambient()`. Use reactive bindings for
  controls and untracked reads in action callbacks when no subscription is intended.
- Prefer semantic pieces such as `form`, `section`, and `labeled` over custom layout.
- Add new pages to `src/pages.rs`, their typed routes and navigation to `src/lib.rs`, and
  portable steps to `dayscript/demo.yaml`, asserting text through localization keys.
- Keep test fixtures in the repository. Synthetic test text is separate from shipped UI text.

## Validation

Run `cargo fmt --all`, `cargo fmt --all -- --check`, `cargo test`, and
`cargo clippy --all-targets -- -D warnings`. Run `day lint` and build the affected targets.
Run the walkthrough with `day launch -t <target> --script dayscript/demo.yaml` and inspect the
screenshots of changed screens. Use `day drive -t <target> --steps-json '[…]'` to inspect a
running app. Available Day MCP tools provide equivalent launch and drive operations.
