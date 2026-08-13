---
title: Developing Day and an app together
description: Build an app against a local day checkout with day patch, and the workflow for changing a framework crate and the app that demonstrates it in the same sitting.
order: 35
section: Build & ship
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

An app's `Cargo.toml` resolves the framework from git:

```toml
[dependencies]
day = { git = "https://github.com/daybrite/day.git" }
```

That is the right declaration for CI and for anyone who clones the app — no day checkout
required. It is the wrong resolution when you are changing the framework and the app in the
same sitting: a new capability in a day crate, and the screen in your app that demonstrates
it. For that, the app has to build against your checkout, and `day patch` makes the switch:

```sh
cd my-app
day patch --local ../day     # build against the checkout at ../day
day patch --check            # verify: no day crate resolves from git
rm .cargo/config.toml        # back to the git dependency
```

## What it writes

`day patch --local <checkout>` writes the app's `.cargo/config.toml` with a Cargo
[`[patch]` table](https://doc.rust-lang.org/cargo/reference/overriding-dependencies.html)
mapping every day crate in the app's dependency graph to a path inside the checkout:

```toml
[patch."https://github.com/daybrite/day.git"]
day = { path = "/home/you/src/day/crates/day" }
day-android = { path = "/home/you/src/day/toolkits/day-android" }
# … one entry per day crate the app resolves
```

The file is machine-local and stays out of git — the scaffold's `.gitignore` covers it, and
CI resolves the git dependency exactly as a user's build would. Delete the file to switch
back.

You could write that table by hand, and the reason not to is that a missing entry does not
fail. Cargo quietly resolves the missing crate from its git cache, and the build mixes your
checkout with a published release — it compiles, it runs, and part of it is not the code you
are editing. Two things make a hand-written table hard to keep right:

- The crate set is bigger than the app's manifest. Toolkit backends like `day-android` reach
  the app through the `day` umbrella crate's per-target dependency tables, so the app never
  names them. `day patch` resolves the full graph for every platform Day targets, with every
  backend feature on, and patches whatever it finds.
- The crate set changes. Add a `day-part-*` dependency, or pull in a new framework crate,
  and a hand-maintained table is stale — with no error to tell you so.

After writing the table, the command verifies it: it asks cargo for the resolved graph of
every platform and fails if any `day*` crate still comes from git.

## Working on the framework and an app in parallel

This is the workflow for adding a feature to a day crate alongside its demonstration in an
app. [Day-Showcase](https://github.com/daybrite/Day-Showcase) is the framework's own example
of the pattern — every new piece and part lands with a showcase screen that exercises it —
but any app works the same way.

1. **Patch once.** In the app, run `day patch --local <path-to-day>`. Every later build
   compiles the framework from the checkout: edit a day crate, re-run `day launch` (or
   `day relaunch`), and the change is in the app, with cargo's usual incremental rebuild.
2. **Develop both sides.** Change the framework crate, change the app's screen, build,
   repeat. There is no publish step in the loop.
3. **Re-run `day patch` when the crate set changes.** A new day dependency in the app's
   `Cargo.toml`, or a new crate in the framework workspace that the app pulls in, needs a
   new table entry. Re-running the command rewrites the whole table.
4. **Land the framework change first.** The app's git dependency names no revision, and a
   scaffolded app commits no `Cargo.lock`, so CI and fresh clones resolve day's current
   `main`. Once the framework side merges, the app change builds everywhere; land it second.
5. **Unpatch when you are done — or don't.** `rm .cargo/config.toml` returns the app to the
   git dependency. On a machine where you always develop against the checkout, leaving it in
   place is fine: git ignores the file, so it affects no one else.

`day patch --check` is the guard for step 4 and for CI. It writes nothing and exits non-zero
if any day crate in any platform's graph still resolves from git. Day's own CI checks out
Day-Showcase, points it at the commit under test, and runs exactly this check before
building — so a green showcase build means the showcase built against that commit, not
against whatever the git cache held.

## What not to commit

- `.cargo/config.toml` — machine-local absolute paths. The scaffold gitignores it.
- `Cargo.lock` — a scaffolded Day app gitignores this too. A lock resolved while patched
  records the checkout's paths, which mean nothing on another machine, and an unpatched lock
  pins a git revision that goes stale against a floating `main`.

If your app does commit its lockfile, regenerate it unpatched — delete `.cargo/config.toml`,
then run `cargo update` — before committing.
