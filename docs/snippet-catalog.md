---
title: "Documentation recipe fixtures"
description: "Repository-only setup for compiling documentation examples."
---

# Documentation recipe fixtures

This file supports the documentation validators and is not published as a website page.
Reader-facing guidance is in [Localization](/docs/reference/localization#reference-example-messages).
The shared sample catalog is [reference-messages.ftl](../website/public/examples/reference-messages.ftl).
The excerpt validator reads that file directly; individual recipes supply their own catalogs.

## Technical recipes

The task-guide modules are designated compilable examples. Each guide supplies a complete
`src/<module>.rs` and its Fluent messages. Compile them together with this setup in a new
`apps/technical-recipes/` directory directly under the Day checkout. These are library recipes,
not standalone apps. Keep every dependency on the same checkout and run the validator against
the Day revision your app uses.

Create `Cargo.toml`:

<!-- technical-file: Cargo.toml -->
```toml
[package]
name = "technical-recipes"
version = "0.1.0"
edition = "2024"

[workspace]

[lib]
name = "dayapp"

[dependencies]
day = { package = "day-app", path = "../../crates/day", features = ["mock"] }
day-part-http = { path = "../../crates/day-part-http" }
day-part-filesystem = { path = "../../crates/day-part-filesystem" }
day-part-permissions = { path = "../../crates/day-part-permissions" }
day-part-local-notify = { path = "../../crates/day-part-local-notify" }

[build-dependencies]
day-build = { path = "../../crates/day-build" }
```

Create `Day.toml` (the title is packaging metadata; visible messages use the catalog):

<!-- technical-file: Day.toml -->
```toml
schema = 1

[app]
id = "dev.daybrite.docs.technical-recipes"
title = "Technical recipes"
build = 1
targets = ["macos-appkit"]

[permissions]
notifications = true
```

Create `build.rs`:

<!-- technical-file: build.rs -->
```rust
fn main() {
    day_build::prebuild_project().expect("day-build: prebuild");
}
```

Create `src/lib.rs`:

<!-- technical-file: src/lib.rs -->
```rust
day::resources!();

pub mod accessibility_recipe;
pub mod commands;
pub mod documents;
pub mod events;
pub mod grid_recipe;
pub mod http;
pub mod layout_recipe;
pub mod localization_recipe;
pub mod navigation_recipe;
pub mod notify;
pub mod permissions;
pub mod resource_recipe;
pub mod shapes_recipe;
pub mod storage;
pub mod tree_recipe;
```

Create `resource/locales/en/app.ftl` with `app_title = Technical recipes`, followed by the catalog
blocks from [HTTP](/docs/how-to/guide-http),
[storage](/docs/how-to/guide-storage),
[permissions](/docs/how-to/guide-permissions),
[notifications](/docs/how-to/guide-notifications),
[commands](/docs/how-to/guide-commands), and
[documents](/docs/how-to/guide-documents). Copy each guide's marked complete
module into the corresponding source file. Also copy the modules and catalogs from
[events/state](/docs/explanation/reactivity#handle-an-event-and-update-state),
[layout](/docs/explanation/layout#build-a-labeled-settings-row),
[navigation](/docs/how-to/navigation#share-state-between-sections),
[localization](/docs/how-to/localization#bind-complete-messages-to-state),
[accessibility](/docs/how-to/accessibility#expose-a-changing-value), and
[resources](/docs/how-to/resources#handle-a-missing-bundled-file), and
[trees](/docs/reference/tree#show-hierarchical-data) and
[grids](/docs/reference/grid#authoring).
Create the resource guide's `resource/assets/recipe_data.json` fixture too. No additional helper
types are needed.

From the app directory, run `cargo check --all-targets` and
`cargo clippy --all-targets -- -D warnings`. From the repository root, the automated equivalent is:

```sh
python3 -B scripts/docs/verify-recipes.py
```

The validator extracts these exact blocks into a fresh repository-local workspace, compiles and
lints every designated module, and runs synthetic command/prefs/file and mock interaction tests.
Day dependencies resolve to this checkout. Third-party dependencies use Cargo's offline cache by
default; set `CARGO_NET_OFFLINE=false` to allow downloads on a fresh machine, as website CI does.
The validator verifies the original repository lock remains unchanged. The examples do not send
HTTP requests, post notifications, display permission prompts, or open user documents.
The mock facade does not replace the OS implementation inside every part.
The fixture store paths are redirected under the validator's workspace.

All other Rust fences in the technical corpus are API excerpts or integration fragments unless
explicitly designated by a tutorial or technical-example marker. They are not implicitly covered
by this compilation result. The priority-guide excerpts have a separate compiler check:

```sh
python3 -B scripts/docs/verify-excerpts.py
```

That check supplies synthetic app functions, types, and resource fixtures for omitted context.
It compiles the published blocks without making them standalone apps. The
[validation record](history/documentation-validation.md#recipes-and-priority-guides) summarizes
the checked recipes, priority excerpts, and remaining unverified corpus.
