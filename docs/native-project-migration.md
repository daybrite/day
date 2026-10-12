<!-- Copyright © The Daybrite Project; SPDX-License-Identifier: CC-BY-SA-4.0 -->
# Native project migration

Day can generate native build projects while preserving app-owned sources and configuration.
Keep a persistent host when its custom targets, build phases, plugins, or resource setup are
not represented by the generator.

## Release ordering

Land the Day generator and manifest support before removing app host projects or adding
new `[android]` and `[harmony]` fields. Older CLIs reject unknown manifest fields. A matching
minor-version compatibility label alone does not prove that a CLI contains these changes.

Validate dependent pieces against the updated Day Git revision, then distribute a CLI release
containing the support before rolling out downstream app migrations. CI jobs resolving Day
from Git main need the framework changes on that branch first. Keep host-project removal
separate from unrelated engine changes so each can be reviewed and validated independently.

## Preserve app configuration

Before removing a host project, inventory its targets, build phases, native packages, launch
arguments, environment settings, permissions, themes, signing settings, and icon sources.
Preserve a recoverable copy outside disposable build output: `day clean` removes that output.
Do not discard uncommitted native-project changes.

App-owned Swift sources, plists, string catalogs, `.xcconfig` files, and CLI wrappers remain
inputs to generated Apple hosts. Preserve custom Android resources as source overlays:

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

The historical workspace audit and retained exceptions are recorded in
[the October 2026 audit](https://github.com/daybrite/day/blob/main/docs/history/native-project-migration-2026-10.md).
