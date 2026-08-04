---
title: Reproducible builds
description: What Day guarantees about rebuilding the same commit twice, what breaks per platform, and how to verify a build yourself with diffoscope.
order: 34
section: Build & ship
---

A build is reproducible when the same source, built twice, produces the same bytes. Day builds the
compiled code of every app reproducibly: rebuild a commit in a different directory, on a different
day, and the machine code that comes out is identical. The containers those binaries ship in —
`.dmg`, `.apk`, `.ipa`, `.hap`, `.msix`, `.flatpak` — are reproducible on some platforms and not on
others, and this page says which, and why.

Day's CI checks this on every push to `main`. Each platform-toolkit job has a follow-up `validate`
job that first installs and launches the shipped artifact on a clean runner, then rebuilds the same
commit from a second checkout at a different path and compares the two.

## Why it matters

If you can rebuild an artifact and get the same bytes someone else got, you can check that a binary
corresponds to the source it claims to come from. Without that, a published binary is something you
take on trust: you cannot tell a clean build from one where a compromised build machine, a
substituted dependency, or a modified toolchain inserted something the source never contained.

Reproducibility turns that trust into something you can test. Anyone can rebuild and compare, so a
tampered artifact has to survive independent verification rather than a single signature. That is
the argument the [Reproducible Builds project](https://reproducible-builds.org) makes in full, and
its [documentation](https://reproducible-builds.org/docs/) is the reference for the general
techniques — `SOURCE_DATE_EPOCH`, path normalization, archive metadata, and the rest. Day follows
those conventions rather than inventing its own.

Reproducibility is not a substitute for signing. A signature says who built an artifact;
reproducibility says the artifact matches its source. You want both.

## What Day guarantees

Day's CI grades a rebuild in two tiers, and they carry different weight.

**Payload** — the compiled code, extracted from whatever container ships it. A mismatch here fails
the build. It means the same sources produced different machine code, or a build path leaked into
the binary.

**Container** — the shipped file itself. A mismatch is reported but does not fail, because what
remains after Day normalizes archive timestamps is either a linker build ID or a signature, and
neither is something a build controls.

`day pack` normalizes what it can before handing a tree to an archiver: every file and directory
gets a fixed modification time, taken from `SOURCE_DATE_EPOCH` when you set it and otherwise
2020-01-01T00:00:00Z. The default is not the Unix epoch because ZIP's timestamp field cannot encode
anything before 1980, and an out-of-range value gets clamped back into per-run variance.

Day also sets `codegen-units = 1` and `lto = "fat"` in the release profile. Both matter for
reproducibility; the LTO choice is explained under Linux below.

## Signing sets a ceiling

Two Day artifacts cannot be byte-identical no matter what the build does, and it is worth knowing
why before you go looking for a bug.

A signed `.hap` uses `SHA256withECDSA`. ECDSA picks a random value per signature, so signing the
same bytes twice produces two different signatures. A released `.dmg` is stapled: `xcrun stapler
staple` writes an Apple-issued notarization ticket into the file, and Apple issues a new one per
submission.

For both, the payload tier is the guarantee. Verify the code, not the wrapper.

## Per-platform caveats

Reproducibility needs cooperation from the Rust dependency graph *and* from the platform's own build
tools. Day controls the first and configures the second where the tool allows it. Where a tool
offers no control, this section says so.

### macos-appkit

The compiled binary is reproducible. The `.dmg` is not.

Every Mach-O carries an `LC_UUID`. Apple documents it in [TN3178: Checking for and resolving build
UUID problems](https://developer.apple.com/documentation/technotes/tn3178-checking-for-and-resolving-build-uuid-problems), which states that the linker derives it from a hash of the built code
specifically to promote reproducible builds, and that Apple's tools "strive to support reproducible
builds within the constraints imposed by the Mach-O file format".

In practice the UUID still changes when the same commit is built in a different directory, which
moves 16 bytes of the executable plus the signature covering them — TN3178 notes that a signature
covers the build UUID. Day's check normalizes the UUID before comparing, so the payload tier passes
and the container tier reports the difference. Treat a differing UUID as expected rather than a bug.

The linker's `-no_uuid` option would remove it and make the binary reproducible, and TN3178 says
plainly why not to: an image without a build UUID cannot be matched to its `.dSYM`, so Apple's crash
reporter cannot symbolicate it. Day keeps the UUID. There is also no way to set one after the fact —
per TN3178, no Apple command does that.

`hdiutil` is the harder problem. Two DMGs built from identical, timestamp-normalized input differ by
628 bytes uncompressed under APFS — UUIDs plus a Fletcher-64 checksum on every block — or 151 bytes
under HFS+, spread across GPT GUIDs and their CRC32s, the volume header's dates and UUID, and
per-file creation dates in the catalog. `hdiutil` stamps copy time as each file's creation date, so
normalizing the source tree does not reach them. UDZO compression then turns any of that into a
whole-file difference, because a change in compressed length shifts every chunk after it. Day does
not attempt to rewrite DMG internals.

Day sets `ZERO_AR_DATE=1` on every Apple toolchain invocation, which stops `libtool` and `ld64`
writing file modification times into static archives and debug maps. TN3178 is Apple's reference for
build UUIDs specifically; for the wider set of linker and archiver flags, [LLVM's deterministic
builds post](https://blog.llvm.org/2019/11/deterministic-builds-with-clang-and-lld.html) covers the
ground Apple's documentation does not.

### ios-uikit

The unsigned `.ipa` is byte-reproducible, including the container. Getting there took three fixes,
each hidden behind the last.

Xcode's release defaults leave a **debug map** in the linked binary: one `N_OSO` entry per object
file, each holding that file's absolute path under `SYMROOT`. The showcase app carried 267 of them.
Day passes `DEPLOYMENT_POSTPROCESSING=YES STRIP_INSTALLED_PRODUCT=YES STRIP_STYLE=debugging`, which
strips the map. Xcode runs `dsymutil` before `strip`, so the `.dSYM` still appears and symbolication
still works; `STRIP_STYLE=debugging` keeps the symbol table so in-process backtraces resolve.

Xcode 14 added **ObjC selector stubs**, where the compiler emits `_objc_msgSend$<selector>`
references and the linker synthesizes an `__objc_stubs` section. That leaves two `__got` slots for
`_objc_msgSend` with byte-identical contents, and which consumer gets which slot is not stable. Day
disables the optimization with `-fno-objc-msgsend-selector-stubs`, which leaves one slot. For a
Swift-heavy app this also makes the binary slightly smaller.

`ditto -c -k` copies each file's modification time into the ZIP and has no flag to suppress it, so
Day stamps the staging tree before archiving.

`LC_UUID` still varies, as on macOS — see [TN3178](https://developer.apple.com/documentation/technotes/tn3178-checking-for-and-resolving-build-uuid-problems) and the macos-appkit section above.

### linux-gtk and linux-qt

The compiled binary is reproducible. The `.flatpak` bundle is not compared.

ThinLTO makes an internal symbol external so it can be inlined across modules, and renames it with a
`.llvm.<hash>` suffix to avoid collisions. That suffix was not stable across build directories: two
builds of the same commit on one machine differed in exactly two symbol names, which cascaded into
the GNU build ID and the string table size. Day's release profile uses `lto = "fat"`, which merges
everything into one module so no cross-module promotion happens and the suffix is never emitted.
Cargo's `trim-paths`, the more targeted fix, is [still
unstable](https://rust-lang.github.io/rfcs/3127-trim-paths.html) as of Cargo 1.97.

A `.flatpak` is an OSTree bundle that ordinary archivers cannot open, so Day's check compares the
ELF binary staged before bundling rather than the bundle. `flatpak-builder` honors
`SOURCE_DATE_EPOCH` from version 1.3.1, and Day exports it; OSTree's own support for it is [an open
issue](https://github.com/ostreedev/ostree/issues/2385). See the [flatpak-builder command
reference](https://docs.flatpak.org/en/latest/flatpak-builder-command-reference.html).

### android-mdc

The compiled `.so` files are reproducible. The `.apk` and `.aab` are close but not yet identical.

Gradle stamps each ZIP entry with the file's modification time and walks the tree in filesystem
order. Day's app template sets `isPreserveFileTimestamps = false` and `isReproducibleFileOrder =
true` on every archive task, which is the documented fix — see [Gradle's reproducible archives
guidance](https://docs.gradle.org/current/userguide/working_with_files.html) and
[reproducible-builds.org on the JVM](https://reproducible-builds.org/docs/jvm/).

Day ships a fixed dev keystore rather than generating one per project, which is what Android's own
`debug.keystore` does. A freshly minted key meant a dev-tier `.apk` could never be reproducible,
because two builds signed identical bytes with different keys. It also means a build from one
machine can now upgrade an install from another, which Android otherwise refuses when a signature
changes.

What remains is inside the APK Signing Block. Zip entries and the central directory come out
byte-identical, and `apksigner` produces identical output given the same key and input, so this is
not an inherent property of APK signing the way ECDSA is for HarmonyOS. It is unresolved rather than
impossible. Note that v2 and v3 signatures cover every byte of the file, so an APK has to be
identical *before* signing for any of this to hold — the constraint F-Droid documents in its
[reproducible builds guide](https://f-droid.org/docs/Reproducible_Builds/).

### harmony-arkui

The compiled `.so` is reproducible. A signed `.hap` cannot be.

`hvigor` assembles and emits the `.hap` itself, so there is no staging tree for Day to stamp. Day
patches the archive's timestamps in place instead, and does it before signing. A `.hap` signature
covers the local file headers, so rewriting them afterwards would invalidate it.

`SHA256withECDSA` is where this stops. Two haps of byte-identical content differ only in the signing
block, and that difference is the signature itself.

One caveat is worth knowing even if you never check reproducibility: `DAY_OHOS_ARCH` takes
precedence over any connected device. Before that, `day pack` built for whatever emulator or handset
happened to be attached, so a hap packed next to a running x86_64 emulator shipped x86_64 while the
same commit packed elsewhere shipped arm64. A distribution build should not change shape because
something was plugged in.

### windows-xaml

The staged payload is reproducible.

The Microsoft linker writes the wall clock into the PE header's `TimeDateStamp` and into the debug
directory. Day passes `/Brepro`, which substitutes a hash of the input. Those 24 bytes were the entire
difference between two Windows builds.

For the NSIS installer, Day's generated script sets `SetDateSave off`, which stops NSIS storing and
restoring each file's modification date. The `/SOLID lzma` compressor it already used is
deterministic.

The `.msix` and the `-setup.exe` are built from one staged payload directory, and Day stamps that
tree once before either container is built.

### Rust dependencies

None of the above helps if a crate in your dependency graph is itself nondeterministic. A `build.rs`
is ordinary Rust code and can embed a timestamp, an absolute path, a hostname, or a random value
into generated source. If your app stops reproducing after adding a dependency, that is the first
place to look.

Two things are worth knowing about rustc itself. `codegen-units = 1` is the documented baseline for
deterministic output, and Day sets it. Dependency source paths under `~/.cargo/registry` appear in
panic locations inside the binary, so two machines with different home directories produce different
binaries even when everything else matches — Day's CI compares builds on the same runner image,
where that path is constant. `rust-lang/rust#129080` is the [standing list of reproducibility
hazards](https://github.com/rust-lang/rust/issues/129080) and worth watching.

## What ships alongside the artifact

`day pack` writes two files into `build/day/dist` next to the artifact itself:

| File | What it records |
| --- | --- |
| `day-sbom.cdx.json` / `day-sbom.spdx.json` | every dependency that went into the build, plus the repository and commit it came from |
| `<target>.buildinfo.json` | the target and profile, the host OS and architecture, the exact version of every tool that participated, and the SHA-256 of each artifact produced |

The SBOM answers *what went in*. The `.buildinfo` answers *what built it*:

```json
{
  "schema": "1.0",
  "target": "macos-appkit",
  "profile": "release",
  "host": { "os": "macos", "arch": "aarch64" },
  "tools": [
    { "key": "rust",  "name": "rustc", "version": "rustc 1.97.0 (2d8144b78 2026-07-07)",
      "install": "rustup toolchain install <version> && rustup override set <version>" },
    { "key": "xcode", "name": "Xcode", "version": "Xcode 26.6",
      "install": "https://developer.apple.com/download/all/?q=Xcode — install, then sudo xcode-select -s ..." }
  ],
  "artifacts": [
    { "name": "Day Showcase.dmg",
      "sha256": "62e8dd94235e3e36202cd0d4a0be2084f24ba4be037d9ae9c93e7baa5d929e9e" }
  ]
}
```

Each tool carries an `install` hint, so a machine that cannot reproduce the build can be told what
to change. The `.buildinfo` is always a sidecar and never embedded: tool versions differ per machine,
so baking them into the artifact would make the artifact itself unreproducible.

On `linux-gtk` and `linux-qt` a second file, `<source>_<version>_<arch>.buildinfo`, is written in
Debian's [deb822 `.buildinfo` format](https://wiki.debian.org/ReproducibleBuilds/BuildinfoFiles)
alongside the JSON one, so a Debian maintainer has everything the distribution's own tooling expects.

Keep both files with the artifact when you publish it. Without the SBOM there is no commit to
rebuild from, and without the `.buildinfo` there is no way to tell whether your machine matches the
one that built it.

## Checking an artifact you did not build

The recorded SHA-256 makes the cheapest check a hash comparison. If you downloaded a release and its
`.buildinfo`, you can confirm the file is the one the publisher meant to ship without building
anything:

```sh
shasum -a 256 "Day Showcase.dmg"
python3 -c "import json;print(json.load(open('macos-appkit.buildinfo.json'))['artifacts'][0]['sha256'])"
```

That establishes the artifact is intact. It does not establish that it was built from the source it
claims — the publisher computed both the file and the hash. To check *that*, rebuild it and compare
the result against the artifact you were given, which is what `day rebuild` does.

## Verifying with `day rebuild`

Point `day rebuild` at any artifact that has its SBOM and `.buildinfo` beside it — one you built, or
one you downloaded from someone else. It reads that information back and does the whole check:

```sh
day rebuild "My App.dmg"
```

It finds the SBOM shipped with the artifact, reads the repository and commit that produced it,
compares your installed tool versions against the ones recorded at build time, clones that commit
into a temporary directory, packs it again, and compares the two artifacts.

```
 Environment 6 tool(s) match the artifact
    Cloning https://github.com/you/my-app @ 342a2be1606d
 Rebuilding macos-appkit (release) in /tmp/day-rebuild-My App/src/apps/my-app
    Payload identical
  Container differs
            99517b21e367c5b0… vs 62e8dd94235e3e36…
```

The two verdicts are the ones described above. `Payload` covers the compiled code pulled out of
whatever container it ships in, and a mismatch there exits non-zero. `Container` covers the shipped
file byte for byte, and it differs on formats that embed a signature or a build UUID, which is why
it only reports. Pass `--strict` to also fail when the payload could not be compared at all, which
is what you want in CI: a container the machine cannot open means the code went unverified.

The environment check runs before the clone, so a machine that cannot reproduce the artifact says so
immediately rather than after a long build. It never installs anything — it reports what is missing
and what to run:

```
      Error the build environment does not match the artifact:
  rust: this machine has rustc 1.97.0 (2d8144b78 2026-07-07)
      the artifact was built with rustc 1.90.0 (1159e78c4 2025-09-14)
      rustup toolchain install 1.90.0

  Install the versions above, or re-run with --force-tool=<name> for each tool you want to
  ignore (--force-tool=all ignores every mismatch).
```

`--force-tool` takes a tool name, repeats, and accepts `all`. It exists for experiments. A forced
rebuild that differs tells you nothing, because the difference may be the tool you forced.

A rebuild needs the commit to exist in the repository, so an artifact packed from a working tree
with uncommitted changes is refused. Nothing describes what went into it.

## Verifying a build yourself

`day rebuild` is the short path. Doing it by hand is worth knowing when you want to compare
something it does not handle, or to see the difference rather than a verdict. Build the app twice
in two different directories and compare.

```sh
git clone <your-app> app-a && git clone <your-app> app-b
( cd app-a && day pack -p macos-appkit )
( cd app-b && day pack -p macos-appkit )
```

Two separate directories matter. Building twice in the same place will not catch a build path that
leaked into the binary, which is one of the most common causes of a reproducibility failure.

Compare the results with [diffoscope](https://diffoscope.org), which recurses into archives and
decodes binary formats instead of reporting that two files differ:

```sh
diffoscope "app-a/build/day/dist/My App.dmg" "app-b/build/day/dist/My App.dmg"
```

Install it with `brew install diffoscope`, `apt install diffoscope`, or `pip install diffoscope`.
On Windows most of its external comparators are unavailable and it falls back to a binary diff.

Set `SOURCE_DATE_EPOCH` to the same value for both builds if you want the archive timestamps to
match a specific date rather than Day's default:

```sh
SOURCE_DATE_EPOCH=$(git log -1 --format=%ct) day pack -p android-mdc
```

On Apple platforms you can check the build UUID directly, which is the difference diffoscope will
report most often. TN3178 documents `dwarfdump`:

```sh
dwarfdump --uuid "app-a/build/day/pack/macos-appkit/My App.app/Contents/MacOS/myapp"
dwarfdump --uuid "app-b/build/day/pack/macos-appkit/My App.app/Contents/MacOS/myapp"
```

Each architecture in a universal binary has its own UUID. `otool -l <binary> | grep -A1 LC_UUID`
shows the same load command if you prefer.

Read the output against the sections above before filing a bug. A differing `LC_UUID` on Apple
platforms, a differing signature block, or a differing `.dmg` is expected. A differing `.so`, `.exe`,
or Mach-O executable, once the UUID is accounted for, is not.

Day's CI runs `day rebuild --strict` against every artifact it ships, on a clean runner, for all six
packing platforms. The same command is available to you, and it applies the same two-tier comparison
described above.
