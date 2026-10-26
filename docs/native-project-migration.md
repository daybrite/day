---
navTitle: "Native projects"
title: "Configure native projects"
description: "Choose generated or persistent native projects and preserve app configuration, resources, and signing."
---

<!-- Copyright © The Daybrite Project; SPDX-License-Identifier: CC-BY-SA-4.0 -->
# Configure native projects

Day can generate native build projects while preserving app-owned sources and configuration.
Keep a persistent host when its custom targets, build phases, plugins, or resource setup are
not represented by the generator.

## Choose a generated or persistent host

Persistent Apple projects must contain Xcode 27's `project.xcproj`. Day no longer reads
PBX projects, migrates inline build settings, injects PBX resources or phases, or creates
crate-named aliases for the staged Rust archive. Use the current template's `libdayapp.a`
and linker response file. Convert a customized project with Xcode 27 and review its resource
paths and callbacks, or remove the project folder to use the generated host.

`day project migrate-xcode` and `day prepare --migrate` have been removed. Old icon lockfiles
are not read; remove obsolete generated icons and update custom project references manually.
HarmonyOS custom hosts use `platform/harmony/` and identity overrides use `[app.harmony]`;
the old `ohos` spellings are rejected. SDK, permission and signing keys keep their OHOS names.

For app-specific Android declarations, keep an optional overlay at
`platform/android/AndroidManifest.xml`. Day's Gradle plugin merges it above the host and piece
contributions in every app variant, using Android's standard rules. This path alone does not
select a persistent host. Keep only custom declarations and explicit merge markers; do not copy
the complete old host manifest. See
[manifest customization](/docs/reference/platforms/android-material#manifest-customization).

## Preserve app configuration

Before removing a host project, inventory its targets, build phases, native packages, launch
arguments, environment settings, permissions, themes, signing settings, and icon sources.
Preserve a recoverable copy outside disposable build output: `day clean` removes that output.
Do not discard uncommitted native-project changes.

App-owned Swift sources, plists, string catalogs, and `.xcconfig` files remain inputs to generated
Apple hosts. Unmodified `platform/<macos|ios>/day-cli.sh` launchers can also be removed: Day
writes the default launcher beside the generated Xcode project. Keep customized launchers;
Day uses and preserves them. Persisted Xcode projects get a missing launcher recreated beside
their source configuration during preparation. Preserve custom Android resources as source overlays:

```toml
[android]
cleartext-domains = ["127.0.0.1"]
edge-to-edge = true
resource-overlays = ["platform/android/res"]

[harmony]
vendor = "daybrite"
```

`cleartext-traffic = true` permits HTTP from arbitrary hosts when an app needs it. Otherwise,
use exact-host `cleartext-domains` exceptions; the base policy denies cleartext. Overlays
replace generated resources, and regeneration prunes removed overlay files. Arbitrary themes
remain native Android XML.

Android settings apply only to generated hosts. A persistent Gradle project retains its own
policy and resources; preparation and builds warn if `[android]` configuration would be ignored.
HarmonyOS vendor metadata applies to staged persistent hosts as well as generated hosts.

Check custom icon pipelines before removing native projects. Preserve adaptive Android artwork,
Apple asset catalogs, and custom product names until equivalent generated outputs are verified.
Permissions and shortcuts should remain in their supported Day.toml declarations and localized
resource catalogs rather than being copied from previously generated platform output.

## Validate the generated host

With the matching local or released CLI, run `day prepare`, `day prepare --check`, and `day lint`.
Inspect the generated policy, resources, permissions, and platform identity. Then build and run
each affected target; preparation alone does not verify runtime behavior. Compare packaged
resources and icons with the preserved originals. Check signing and release packaging too.

`day open -t <target>` opens the selected generated project. Restoring a persistent project
under `platform/` makes that project take precedence again. Keep backups until the migration
has passed the app's required validation and release process.
