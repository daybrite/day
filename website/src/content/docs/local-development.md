---
navTitle: "Local development"
title: Developing Day and an app together
description: Build an app against a local day checkout with day patch, and the workflow for changing a framework crate and the app that demonstrates it in the same sitting.
order: 35
section: Contributing
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

An app's `Cargo.toml` takes the framework from crates.io:

```toml
[dependencies]
day = { package = "day-app", version = "0.6" }
```

Anyone who clones the app builds against the published crates. When you are changing a day crate
and the app screen that demonstrates it, the app has to build against your checkout instead;
`day patch` makes the switch:

```sh
cd my-app
day patch --local ../day     # build against the checkout at ../day
day patch --check            # verify: no patched crate still comes from crates.io

# More than one checkout: day and an external piece you are changing together. Each is
# recognized by what it carries (the `day-app` crate, or its manifest's `repository`).
day patch --local ../day --local ../day-piece-lottie

# The unreleased framework: every Day crate from the head of its repository's default
# branch, pinned to that commit. This is what CI builds on a push.
day patch --main

# A fork of day, for good: a committable table that takes day's crates from the fork for
# the whole graph, so external pieces build against the fork too, unchanged.
day patch --git https://github.com/acme/day.git@acme
rm .cargo/config.toml        # back to the published crates
```

## Trying a version for one build

`day patch` changes the project until you undo it. To build against another version once (to check
whether a branch fixes a bug, say), pass `--day-src` to `day build` or `day launch` instead:

```sh
day launch --day-src ../day                                               # a local checkout
day launch --day-src https://github.com/daybrite/day.git@experimental-nav  # a branch
day launch --day-src https://github.com/someone/day.git@fix-482            # a PR fork
```

`--patch-local` does the same for any checkout `day patch --local` would take, and repeats, so a
piece under development can ride along with the framework for one run:

```sh
day launch --patch-local ../day --patch-local ../day-piece-lottie         # day patch --local ×2, once
day launch --day-src https://github.com/daybrite/day.git@experimental-nav \
           --patch-local ../day-piece-lottie                               # a branch, plus the piece
```

Either flag computes the same `[patch]` table, hands it to that one cargo run, and leaves the
project as it found it: cargo rewrites `Cargo.lock` during the build and the CLI restores after it.
A git URL is cloned into the shared cache and cached per ref, so a second look at the same branch
skips the clone.

Each day-src also gets its own build tree, so you can leave two versions of the app running at
once and switching between them is an incremental compile. In a debug build the window titles say
which is which: `Day Rise (0.1.0+main-2d77edbf/appkit)` beside
`Day Rise (0.1.0+experimental-nav-9f8e7d6c/appkit)`.
[CLI & projects](/docs/reference/cli#trying-another-version-of-day-itself) has the full flag.

Reach for `day patch` when you're working on the framework and the app together over a session,
and `--day-src` or `--patch-local` when you're comparing.

## What it writes

`day patch --local <checkout>` writes the app's `.cargo/config.toml` with a Cargo
[`[patch]` table](https://doc.rust-lang.org/cargo/reference/overriding-dependencies.html)
mapping every day crate in the app's dependency graph to a path inside the checkout, and relocks
any of them that `Cargo.lock` still holds from crates.io (cargo would otherwise keep the locked
copy and ignore the table):

```toml
[patch.crates-io]
day-app = { path = "/home/you/src/day/crates/day" }
day-toolkit-material = { path = "/home/you/src/day/crates/day-toolkit-material" }
# … one entry per day crate the app resolves
```

The file is machine-local and stays out of git; the scaffold's `.gitignore` covers it. Delete the
file to switch back.

Writing that table by hand is risky because a missing entry does not fail: cargo resolves the
missing crate from crates.io, and the build mixes your checkout with a published release, so
part of it is not the code you are editing. The table is also hard to keep right:

- The crate set is bigger than the app's manifest. Toolkit backends like `day-toolkit-material` reach
  the app through the `day` umbrella crate's per-target dependency tables, so the app never
  names them. `day patch` starts from every crate the checkout carries, resolves the full graph
  for every platform Day targets with every backend feature on, and drops the entries cargo did
  not use.
- The crate set changes. Add a `day-part-*` dependency, or pull in a new framework crate,
  and a hand-maintained table is out of date, with no error to tell you so.

After writing the table, the command verifies it: it asks cargo for the resolved graph of
every platform and fails if a crate from a patched repository still comes from crates.io.

Cargo uses a `[patch]` entry only when its version is one the app accepts. Two cases fail the
check, and it reports both versions:

- The checkout is older than the newest release. Cargo selects the newest compatible version, so
  it takes the release. Update the checkout.
- The checkout carries a version your requirement does not accept, such as `0.7.0` while the
  app asks for `"0.6"`. Raise the requirement in `Cargo.toml`.

## Working on the framework and an app in parallel

This is the workflow for adding a feature to a day crate alongside its demonstration in an
app. [Day-Showcase](https://github.com/daybrite/Day-Showcase) is the framework's example
of the pattern (every new piece and part includes a showcase screen that exercises it),
but any app works the same way.

1. **Patch once.** In the app, run `day patch --local <path-to-day>`. Every later build
   compiles the framework from the checkout: edit a day crate, re-run `day launch` (or
   `day relaunch`), and the change is in the app, with cargo's usual incremental rebuild.
2. **Develop both sides.** Change the framework crate, change the app's screen, build,
   repeat. There is no publish step in the loop.
3. **Re-run `day patch` when the crate set changes.** A new day dependency in the app's
   `Cargo.toml`, or a new crate in the framework workspace that the app pulls in, needs a
   new table entry. Re-running the command rewrites the whole table.
4. **Land the framework change first.** The app's CI builds a push against day's `main`
   (`day patch --main`), so once the framework side merges, the app change builds there; land
   it second. Its nightly run builds against the newest release, so an app change that needs the
   new framework turns that run red until the next day release. That is the signal that the app
   is ahead of what users can build.
5. **Unpatch when you are done, or leave it.** `rm .cargo/config.toml` returns the app to the
   published crates. On a machine where you always develop against the checkout, leaving it in
   place is fine: git ignores the file, so it affects no one else.

`day patch --check` is the guard for step 4 and for CI. It writes nothing and exits non-zero
if a patched repository's crate in any platform's graph still resolves from crates.io. Day's CI
checks out Day-Showcase, points it at the commit under test (and its external pieces at their
own `main`), and runs exactly this check before building, so a green showcase build means the
showcase built against that commit, with nothing resolved from a release.

## What not to commit

- `.cargo/config.toml` — machine-local absolute paths. The scaffold gitignores it.
- `Cargo.lock` — a scaffolded Day app gitignores this too. A lock resolved while patched
  records the checkout's paths, which mean nothing on another machine, and an unpatched lock
  pins a git revision that falls behind a floating `main`.

If your app does commit its lockfile, regenerate it unpatched (delete `.cargo/config.toml`,
then run `cargo update`) before committing.
