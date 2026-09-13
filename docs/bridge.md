---
title: "daybridge"
description: "Foreign-language implementations of a Rust API: how Swift, Kotlin, and ArkTS arms are declared, generated, and kept in parity."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# daybridge: foreign-language implementations of a Rust API (§15.6)

> [!NOTE]
> **Status: v1 shipped through phase 7 (2026-08).** The synchronous surface described here is in
> the tree: every arm language and both generators, with `day-part-speech` as the reference
> crate, since 2026-09 in a separate repository ([day-part-speech](https://github.com/daybrite/day-part-speech)). What remains is [phase 8](#implementation-phases), migrating the other synchronous parts,
> and phase 9's gates; async, callbacks, and streams are [after v1](#after-v1). The type table,
> the ownership rule, the threading rule, and the naming derivation are the API surface; changing
> them now invalidates every arm written against them.

A **bridge** lets one Rust function have implementations in Swift, Kotlin, Java, ArkTS, JavaScript,
C, or C++, chosen per target at build time. The Rust signature is the contract; the foreign code is
an implementation of it. Calling code sees an ordinary Rust function and contains no platform
conditionals.

```rust
day_part_speech::speak("Rain later")?;   // AVSpeechSynthesizer, TextToSpeech, speechSynthesis, …
```

## When to use one

**Only when the platform's API is unreachable from Rust.** That is a smaller set than it looks:

| Platform | Reachable from Rust today | Needs a bridge |
|---|---|---|
| macOS, iOS | `objc2`, C frameworks (IOKit, AVFoundation) | rarely; a delegate-heavy API reads better in Swift |
| Linux | `std`, C libraries via `#[link]` | rarely |
| Windows | Win32 via `#[link]`, `windows` crate | COM-heavy APIs where C++ is materially shorter |
| HarmonyOS | the ArkUI/BasicServicesKit **C** APIs | **yes** for anything ArkTS-only (Core Speech Kit, Web, Map) |
| Android | nothing; the platform is Java | **yes**, for every platform API |
| Web | nothing; wasm cannot touch the DOM or browser APIs | **yes** |

`parts/day-part-battery` is the reference for the negative case: five of its six arms are Rust
because IOKit, `GetSystemPowerStatus`, sysfs, and `libohbattery_info.so` are C APIs. Writing those
in a foreign language would add a toolchain to the build and buy nothing.

## The file

Everything bridge-related in a crate lives inside one `day_bridge::bridge!` block. The macro
discards its body; the generated Rust arrives through an `include!` of `OUT_DIR/day-bridge/mod.rs`.

```rust
day_bridge::bridge! {
    // 1. the contract — the only definition of the API
    #[day_bridge::declare]
    extern "day" {
        fn speak_native(text: &str) -> Result<(), day_bridge::Error>;
        fn stop_native();
    }

    // 2. an implementation, inline — `body` alone, or with the file-level preamble it needs
    #[day_bridge::impl(java, platforms = [android])]
    java!(
        prelude = r#"
            import android.speech.tts.TextToSpeech;
            import dev.daybrite.day.bridge.DayBridge;
        "#,
        body = r#"
            public static void speak_native(String text) { … }
            public static void stop_native() { … }
        "#,
    );

    // 3. an arm with no preamble is the sole argument
    #[day_bridge::impl(js, platforms = [web])]
    js!(r#"
        export function speak_native(text) { speechSynthesis.speak(new SpeechSynthesisUtterance(text)); }
    "#);

    // 4. or an implementation in its own file, staged like platform/android/java is today
    #[day_bridge::impl(swift, platforms = [ios, macos], src = "platform/Speech.swift")]

    // 5. the arm that keeps `cargo test` and day-mock compiling
    #[day_bridge::impl(rust, platforms = [other])]
    fn speak_native(_text: &str) -> Result<(), day_bridge::Error> {
        Err(day_bridge::Error::Unsupported)
    }
}
```

**Inline or file.** Inline arms use a raw string, because foreign code must still lex as Rust
tokens inside a macro body and idiomatic JavaScript and ArkTS do not: a backtick is not a Rust
token at all, and `'zh-CN'` lexes as a malformed lifetime. A raw string accepts anything.
**Use a file (`src = "…"`) once an arm passes roughly 25 lines**, where an editor, a formatter,
and the language's test runner start to matter more than keeping the crate in one file.

**The prelude holds imports and nothing else**, and belongs to the arm that needs it. A `package`,
`namespace`, or `module` declaration in one is an error, not a passthrough: those are identity, the
build integration derives them (below), and a hand-written one that disagrees is the drift this
system exists to remove.

It is a separate argument rather than the first lines of the body because on the JVM the body is
*inside* the generated class (`package …; imports; final class DayXBridge { your arm }`), and Java
and Kotlin imports cannot live in a class body. Swift, JavaScript, ArkTS, C and C++ have no such
split and mostly need no `prelude` at all; write the arm alone and pass it as the sole argument.
Scoping it to the arm rather than to the language keeps a Linux-only `#include` out of a
Windows arm's translation unit when one crate has two arms in the same language.

**Any hash count works.** `r#"…"#` ends at the first `"#`, which appears in ordinary JavaScript
(`querySelector("#speech")`), CSS selectors and C format strings. Write those arms as `r##"…"##`;
the parser counts hashes exactly as rustc does.

## Names

Everything is derived from the crate name, so nothing has to be kept in agreement by hand.

| Thing | Derivation | Example (`day-part-speech`) |
|---|---|---|
| Symbol prefix | `day_bridge_<crate_snake>_<fn>` | `day_bridge_day_part_speech_speak_native` |
| JNI package | `dev.daybrite.day.bridge.<crate_snake>` | `dev.daybrite.day.bridge.day_part_speech` |
| JVM class | `Day<CrateCamel>Bridge` — a Kotlin `object`, or a Java `final class` of statics | `DayPartSpeechBridge` |
| Java file | `<Class>.java`, because javac requires the match | `DayPartSpeechBridge.java` |
| Swift file | `<CrateCamel>Bridge.swift` in `DayPieces` | `DayPartSpeechBridge.swift` |
| ES module | `build/day/web/bridge/<crate>.js` | `bridge/day-part-speech.js` |
| ArkTS module | `<crate>/Index.ets` under the ohos host | `day-part-speech/Index.ets` |
| C/C++ unit | `OUT_DIR/day-bridge/<crate>.c` \| `.cpp` | — |

**A bridged function has one name in every language: the declared one.** `speak_native` is
`speak_native` in Rust, Swift, Kotlin, Java, ArkTS, JavaScript, and C alike. That is not
idiomatic on the JVM or in Swift. In exchange, one grep finds the declaration, every arm, and the
generated adapter, and no one has to map a name in their head while reading a stack trace.
Expect a Kotlin or Swift linter to have an opinion; the
generated adapters carry the suppression where a project's linter needs one.

**A collision is a build error.** Two crates with the same package name (a vendored copy beside a
git dependency, say) derive the same prefix, and the CLI fails the build naming both manifest
paths rather than hashing the source path into the symbol. A hash would make the failure silent and
the symbol unreadable in a crash log; renaming one crate is a two-minute fix that keeps every
generated name predictable, which is what the naming table is for.

## Types

The v1 surface. A declaration using anything outside this table fails the crate's build (day-build validates it before generating anything); phase 9 adds the same check to `day lint`, so a mistake surfaces before a compile.

| Rust | Swift | Kotlin | Java | ArkTS | JavaScript | C / C++ |
|---|---|---|---|---|---|---|
| `bool` | `Bool` | `Boolean` | `boolean` | `boolean` | `boolean` | `bool` / `int32_t` |
| `i32`, `i64` | `Int32`, `Int64` | `Int`, `Long` | `int`, `long` | `number` | `number` | `int32_t`, `int64_t` |
| `f32`, `f64` | `Float`, `Double` | `Float`, `Double` | `float`, `double` | `number` | `number` | `float`, `double` |
| `&str` (argument) | `String` | `String` | `String` | `string` | `string` | `const char*` (UTF-8) |
| `String` (return) | `String` | `String` | `String` | `string` | `string` | `char*`, callee-allocated |
| `&[u8]` (argument) | `Data` | `ByteArray` | `byte[]` | `Uint8Array` | `Uint8Array` | `const uint8_t*` + `size_t` |
| `Vec<u8>` (return) | `Data` | `ByteArray` | `byte[]` | `Uint8Array` | `Uint8Array` | `uint8_t*` + out `size_t` |
| `#[day_bridge::data] struct` of the above | `struct` | `data class` | `final class` | interface | object | `struct` |
| `Result<T, day_bridge::Error>` | `throws` | exception | exception | thrown | thrown | status code + out-param |

The table leaves out these rules:

- **A value comes back as `Result`.** A declaration returns nothing or
  `Result<T, day_bridge::Error>`, never a bare `T`: any arm can fail, so `fn ready() -> bool` is
  a build error and `fn ready() -> Result<bool, day_bridge::Error>` is the spelling. The arms
  themselves return plain `T` (a JVM `boolean`, an ArkTS `boolean`) and throw for the error.
- **`&str` is UTF-8 by default, and UTF-16 only where an arm asks.** C and C++ arms receive
  `const char*` unless the arm opts in with `encoding = "utf16"`, which makes the generated
  adapter convert and pass `const char16_t*`. Windows is not special-cased: a UTF-8 C library on
  Windows stays natural, and an arm calling a wide-character API asks for what it needs. The
  declaration never changes either way.

  ```rust
  #[day_bridge::impl(cpp, platforms = [windows], encoding = "utf16", link = ["ole32", "sapi"])]
  ```
- **`#[day_bridge::data]` structs are POD**: fields from this table only; v1 allows neither
  nesting nor `Option`. A struct is copied across the boundary, never shared.
- **A struct is not versioned.** It crosses by layout, so changing a field is a breaking change to
  every arm at once, which is safe, because inline arms are generated from the declaration and
  rebuilt in the same `day build`. The exception is the file form (`src = "…"`), which is
  hand-written and does not regenerate: the generator validates each file arm's signature against
  the declaration and fails the build on a mismatch. Without a version tag or compatibility
  shim, a stale arm fails the build instead of silently misreading a field.
- **`Option<T>` does not cross.** Model absence in the value (`level: i32` with `-1` for unknown)
  or return `Result`. Four languages spell null four ways, and negotiating that is not worth v1.
- **A function is synchronous unless it carries a `Done<T>`.** A plain call blocks the caller
  until the arm returns and can report nothing afterwards; see
  [Synchronous means dispatched](#synchronous-means-dispatched) for what an `Ok` then promises.
  A function whose last argument is `day_bridge::Done<T>` answers later, through
  [the callback tier](#callbacks). Streams (repeated delivery) are still deferred
  ([After v1](#after-v1)).

## Ownership

One rule, everywhere: **arguments are borrowed for the duration of the call; a returned value is
allocated by the callee and released by the generated free function on the side that allocated it.**

- Rust → foreign: the adapter receives pointers valid until it returns. It must copy anything it
  keeps. Nothing is freed by the foreign side.
- foreign → Rust: a returned `String` or `Vec<u8>` is copied into Rust ownership immediately, then
  the generated `day_bridge_free_*` is called on the allocating side.
- JavaScript and ArkTS are the exception: their runtimes own their memory, so the shim copies into
  and out of wasm memory and no free ever crosses.

## Threads

**A completion runs where the platform delivers it, and the generated future is the door
home.** An Android binder thread, a Swift completion on a background queue, an OkHttp dispatcher
thread, the browser's sole thread: the closure a `Done<T>` resolves runs right there, which is
why it is `Send`. Nothing in a bridged crate calls `day_reactive::on_main` — a part must work in
a plain `main` and under `cargo test`, where no main loop exists ([docs/async.md](async.md)
rule 3). What brings an answer to the UI thread is the awaitable form: `<fn>_future(…).await`
under `day::task` resumes on the UI thread, so the code after the `.await` writes signals
directly. A callback user captures a `Setter` instead, the same idiom every part documents.

This revises the rule dayffi left behind (every re-entry posts to the main loop): posting from
generated code would make the same arm behave differently in a test and in an app, and Day's
`!Send` signals are protected by the type system either way — a completion cannot capture one.

A bridged call itself runs on the caller's thread. On Android that means the generated adapter
attaches the JVM to the current thread and promotes any `jobject` it keeps to a `GlobalRef` before
Rust sees it.

## Synchronous means dispatched

A bridged call in v1 is synchronous: the caller blocks until the arm returns, and there is no way
for an arm to report anything after that. Several platform APIs are internally asynchronous
anyway (Android's `TextToSpeech` is unusable until its `OnInitListener` fires, and HarmonyOS's
`textToSpeech.createEngine` returns a promise), so the contract has to say what a returned `Ok`
claims.

**`Ok` means the platform accepted the request, not that the work finished.** An arm that starts
work it cannot wait for returns `Ok` once the request is queued, and a failure discovered later is
invisible to the caller. Three consequences follow:

- An arm whose implementation is a promise (JavaScript, ArkTS) must handle its own rejection
  (usually by logging), because a rejected promise cannot travel back through a synchronous return.
  Only a *thrown* error is convertible, which is why the type table lists "thrown" rather than
  "rejected promise" for those two languages.
- An API whose result the caller needs later (a permission prompt, a share sheet, an utterance
  ending) declares a `Done<T>` and answers through [the callback tier](#callbacks).
- Blocking the caller means blocking Day's UI thread if the call comes from an action. Keep
  bridged work short; anything long enough to be felt belongs behind a `Done<T>`.

`day-part-speech` uses both shapes: `stop` is fire-and-forget, and `speak` carries a `Done<i32>`
that the engine completes when the utterance ends.

## Callbacks

A function whose **last** argument is `day_bridge::Done<T>` is asynchronous. It returns once the
platform has ACCEPTED the request — its own return is `Result<(), day_bridge::Error>` and never
a value — and it answers exactly once, later, through the handle:

```rust
day_bridge::bridge! {
    #[day_bridge::declare]
    extern "day" {
        /// `done` completes with `1` finished, `0` stopped.
        fn speak_native(text: &str, done: day_bridge::Done<i32>) -> Result<(), day_bridge::Error>;
    }

    #[day_bridge::impl(swift, platforms = [ios, macos])]
    swift!(r#"
        func speak_native(text: String, done: UInt64) throws {
            let utterance = AVSpeechUtterance(string: text)
            pending[ObjectIdentifier(utterance)] = done   // answer from the delegate later:
            synthesizer.speak(utterance)                  // speak_native_complete(done, 1)
        }
    "#);

    #[day_bridge::impl(rust, platforms = [other])]
    fn speak_native(_text: &str, _done: day_bridge::Done<i32>) -> Result<(), day_bridge::Error> {
        Err(day_bridge::Error::Unsupported)   // an Err completes the caller with it
    }
}
```

`T` is `()`, a scalar, `String`, or `Vec<u8>`. The generator emits, on every target:

| Generated | Shape |
|---|---|
| `<fn>_async(args…, on_done)` | `on_done: impl FnOnce(Result<T, Error>) + Send + 'static`; returns `Result<(), Error>` |
| `<fn>_future(args…)` | `day_bridge::Completion<T>`, a `Future<Output = Result<T, Error>>` |
| `<fn>(args…, done: Done<T>)` | the arm itself — a Rust arm's own signature, or the generated marshalling for a foreign one |
| a `static` registry | the closures waiting on this function, keyed by token |

**The token.** A foreign arm receives the handle as a `u64` trailing argument (`uint64_t`,
`UInt64`, `long`, a `BigInt`) and answers with two generated helpers it can call from any thread,
at any later time: `<fn>_complete(done, value)` and `<fn>_fail(done)` (`<fn>_fail(done, message)`
on the JVM and the web, where a message can cross). Underneath is one exported symbol per
function, `day_bridge_complete_<crate>_<fn>`, which the generated Rust owns: a `@_silgen_name`
declaration in Swift, an `extern` in C and C++, a `private static native` on the JVM class, a
wasm export the module reaches through the runtime the shim hands `register(rt)`. Tokens come
from one process-wide counter and are never reused.

**At most once, and never never.** The closure is registered before the arm is called, so an arm
that completes synchronously, or fails before returning, is found either way. A completion for a
token that was already answered, cancelled, or never issued is a no-op. An arm that returns `Err`
completes the caller with that error — `<fn>_async` fires the callback and returns it, so either
channel alone is a complete answer. An arm that returns `Ok` and never completes leaves the
future pending; that is the arm's bug, and the same one a foreign API has when its own callback
never fires.

**Cancellation.** Dropping a `Completion` removes the slot: a late completion then finds nothing.
It does not reach into the platform — a dropped `speak_future` stops listening, not the voice.
An arm that can cancel exposes it as its own declared function (`stop_native`), which is what a
caller pairs with the drop.

**Promise-returning JavaScript arms complete themselves.** When a `js` arm returns a promise, the
generated wrapper completes the token with the resolved value, or fails it with the rejection's
message; an arm that answers from an event (`utterance.onend`) calls the helper instead and
returns nothing. A thrown error is "failed to start", as for a synchronous arm.

**An ArkTS arm over a Huawei kit says so.** Core Speech, Push, Map, Scan and the other HMS kits
are not in the public OpenHarmony SDK Day builds against, and the host compiles every staged
ArkTS module, so one such import would fail the whole build. An arm that imports one declares
`sdk = "hms"`; `day build` stages it only when hvigor's SDK carries an `hms` tree
(`OHOS_BASE_SDK_HOME/<api>/hms`, DevEco's `default/hms`, or `DAY_OHOS_HMS=1`), and leaves it
out otherwise — the crate's Rust half then finds no registered function and reports
`Unsupported`, as day-part-speech does on an OpenHarmony-only build.

**ArkTS arms run on the JS thread.** The generated Rust half reaches an arm through the ArkUI
shim's dispatcher (`day_bridge::arkts::invoke`, found by `dlsym` at run time like the
permissions prompter): from the JS thread — Day's UI thread on HarmonyOS — the call runs
inline; from any other thread it is posted over the shim's `uv_async` rail and the caller
parks until the loop has run it, so `Ok` still means the arm accepted the call. The host's
`EntryAbility` hands the generated `DayBridges.ets` record to the native module at startup
(`registerDayBridges`), and an arm completes through `dayBridgeComplete` — the generated
`<fn>_complete`/`<fn>_fail` helpers — or by returning a promise, which the shim settles into the
same completion. The completion export for ArkTS has one uniform shape for every value type
(`day_bridge_complete_arkts_<crate>_<fn>`), and the shim marshals the JS value into it. A
synchronous ArkTS arm may return a scalar (a boolean or a number), which the dispatcher hands
back the same way; strings and bytes come back only through a `Done<T>`.

**Kotlin `suspend` arms and streams** are not built.

## Errors

`day_bridge::Error` is the single error type crossing the boundary:

| Variant | Raised when |
|---|---|
| `Unsupported` | no arm for this target; what the `rust`/`other` fallback returns |
| `Foreign(String)` | the arm threw: a Swift `throws`, a JVM exception, a rejected promise, a nonzero C status |
| `Encoding` | an argument or result was not valid UTF-8 |
| `Runtime` | the platform runtime was unavailable (no JVM, no `Context`, COM init failed) |

An arm may not panic across the boundary. The generated adapter catches the platform's failure
mode (`catch`, `try`, a status code) and converts it, so an exception never unwinds into Rust.

## Platform selection, and what the target advertises

`platforms = [...]` accepts `ios`, `macos`, `android`, `ohos`, `web`, `linux`, `windows`, and
`other`. Exactly one arm may claim a given target; `other` catches the rest, and a crate with no
`other` arm fails its own build; day-build refuses to generate a module that would not compile
under day-mock.

Each bridged function reports a `Support` per target, and a generated `docs/bridge-matrix.md` (planned) will follow from
the declarations and CI-gated for drift, the way [`docs/duty-matrix.md`](duty-matrix.md), [`docs/coverage-matrix.md`](coverage-matrix.md),
and [`docs/recorder-matrix.md`](recorder-matrix.md) already are.

```rust
// day-part-speech/src/lib.rs — the generator emits one `<fn>_support()` per declaration.
pub fn available() -> Support {
    speak_native_support()
}
```

An arm may report `Emulated` rather than `Native` when the platform implementation is a partial
answer (HarmonyOS Core Speech Kit's zh-CN-only voices, for instance).

## What the build does

Two generators, each with exactly one artifact to own. **day-build**, in the crate's `build.rs` on
any host with no foreign toolchain, emits the Rust side (the arm bodies, the externs, the safe
wrappers, `<fn>_support()`) plus the C/C++ translation units cargo itself compiles through `cc`.
**`day build`** emits the foreign side into the project that target already builds from.

The CLI renders those adapters **from the crate's own sources**, using the same parser day-build
uses, rather than reading anything the build script produced, because staging has to finish
before the platform build compiles and a build script's output only exists once cargo has
already run. Reading sources makes staging independent of build order, as the macOS Swift
staging already treats `[package.metadata.day.macos]` shims.

An arm whose foreign half the CLI stages (Swift, Kotlin, Java, ArkTS, JavaScript) is compiled under a
`day_bridge_staged` cfg the CLI sets. Without it (a plain `cargo build`, or a target this arm does
not claim) the crate falls back to its `other` arm and reports `Unsupported`, so **a bridged crate
always compiles under bare cargo** instead of failing to link a symbol nobody produced. C and C++
need no such gate: cargo compiles them itself.

| Arm | Adapter lands in | Existing mechanism it rides |
|---|---|---|
| `swift` | the generated `DayPieces` SwiftPM module | the Xcode host projects' SwiftPM reference ([docs/swiftui.md](swiftui.md)) |
| `kotlin`, `java` | a Gradle `srcDirs` entry | the checked-in Gradle host project, JNI, `day-pieces.json` |
| `arkts` | a module with a generated `Index.ets` | the ohos host project, hvigor ([docs/harmonyos.md](harmonyos.md)) |
| `js` | `build/day/web/bridge/<crate>.js` | the day-dom shim, which imports it ([docs/web.md](web.md)) |
| `c`, `cpp` | `OUT_DIR/daybridge/<crate>.c` \| `.cpp` | `cc` from the crate's `build.rs` |
| `rust` | `OUT_DIR/daybridge/mod.rs` | nothing; it is ordinary Rust |

Bridge contributions are carried in the existing `day-pieces.json` aggregation rather than a second
manifest file, so Gradle keeps reading one contract.

### Android: `java` needs nothing, `kotlin` needs the plugin

Both languages produce the same `Day<CrateCamel>Bridge` class and the same JNI registration; they
differ in what the app's Gradle build can compile. AGP compiles `.java` from the staged source
directory in any project. It compiles `.kt` only from the *Kotlin* source set, which exists when the
project applies a Kotlin plugin, so a `.kt` arm in a project without one is silently skipped, and
the app dies at the first call with `ClassNotFoundException` for a class the build never produced.

That failure is caught before it can happen. `day build` fails, and `day lint` reports
`day::lint::bridge-kotlin-plugin`, when a dependency has a `kotlin` arm and the app's
`platform/android/app/build.gradle.kts` applies no Kotlin plugin. The message names the crates and
gives the three ways out: write the arm in Java, apply `org.jetbrains.kotlin.android`, or wire the
staged root into the Kotlin source set the project already has and mark the file `day: kotlin-ok`.

**A part published for other people's apps should use `java`.** It cannot know whether its
consumers have Kotlin, and the JVM half of a bridge is a handful of static methods where the two
languages differ little. `day-part-battery` and `day-part-speech` both do this. An app's own crate,
which controls its Gradle build, can use either.

## Linking

`link = ["ole32", "sapi"]` becomes `-lole32 -lsapi` on the crate that owns the arm. That is a
**hard dependency in two places**: the library's development package must exist on every machine
that builds the app, and the library itself must exist on every machine that runs it. A linked
library is a `DT_NEEDED` entry, so the dynamic loader refuses to start the process without it,
before `main`, whatever the user was trying to do.

That is correct for a system component the platform guarantees (`ole32` and `sapi` ship with
Windows). It is wrong for anything a user might not have installed. Desktop Linux is the usual
case: speech-dispatcher is a separate package, and linking it would mean an app that will not
launch at all on a machine with no speech engine, to protect a feature the user may never press.

**Load an optional service at first use instead.** `day-part-speech`'s Linux arm declares no
`link`, `dlopen`s `libspeechd.so.2`, and answers "no engine here" when it is absent:

```c
static int day_speech_load(void) {
    if (day_speech_looked) {
        return day_spd_open != NULL;
    }
    day_speech_looked = 1;
    void* lib = dlopen("libspeechd.so.2", RTLD_LAZY);
    if (lib == NULL) {
        lib = dlopen("libspeechd.so", RTLD_LAZY);
    }
    if (lib == NULL) {
        return 0;
    }
    day_spd_open = (day_spd_open_fn) dlsym(lib, "spd_open");
    …
}
```

The build then needs no development package at all, and the app launches everywhere.

**Report it in `Support` as well as in errors.** A crate whose arm may find nothing at runtime
should ask before answering: `day-part-speech`'s declaration carries an `engine_ready_native`
beside `speak_native`, and `available()` demotes the arm's compile-time claim to `Unsupported`
when the engine is missing, so the UI can say so instead of swallowing every press.

`day lint` reports `day::lint::bridge-link-missing` when an arm for the host's own platform links
a library the host cannot resolve, naming the crate and the library.

## What stays in `Cargo.toml`

Source moves into the `.rs`; build-graph facts do not.

```toml
[package.metadata.day.android]
gradle-dependencies = ["androidx.core:core-ktx:1.13.1"]
permissions = ["android.permission.VIBRATE"]
[package.metadata.day.ios]
frameworks = ["AVFoundation"]
platform = "13.0"
[package.metadata.day.linux]
pkg-config = ["speech-dispatcher"]
```

What disappears is the `java = [...]` / `swift = [...]` / `ets = [...]` directory lists, when the
code that lived in those directories is inline.

**What the `Foreign` payload holds depends on the boundary.** Swift and JavaScript pass the
thrown value's message through. A JVM arm's exception is *described to logcat* and cleared by the
generated wrapper (cleared because a pending exception makes the next thread-attach fatal, which
would turn a reportable error into a contained panic), so the Rust side carries the function name
and the detail is in `adb logcat`. A C or C++ arm returns a status code and has no message at all.

## What fails the build

One list, so an implementer and a reviewer see the same set. Each of these is a hard error, not a
warning; every one of them is a disagreement that would otherwise surface as a runtime crash or a
silently misread value.

| Failure | Raised by |
|---|---|
| A declared type outside the [type table](#types) | day-build, when the crate compiles |
| No `other` arm in the crate | day-build; it could not compile under day-mock |
| Two arms claiming the same target | day-build |
| An unknown arm option, or an out-of-range `encoding`/`support`/`sdk` value (`sdk` on a non-ArkTS arm too) | day-build |
| A symbol prefix shared by two crates | `day build`, naming both manifests |
| A file-form arm whose signature disagrees with the declaration | day-cli, per language |
| A `package`, `namespace`, or `module` line inside a prelude | day-build |
| An argument to a language macro other than `prelude` / `body` | day-build |
| Two crates exporting the same name into the web shim's import table | `day build` |
| A `kotlin` arm in an Android project with no Kotlin plugin | `day build`, `day lint` ([above](#android-java-needs-nothing-kotlin-needs-the-plugin)) |

A linked library the host cannot resolve is a **warning**, not a failure: `day lint` reports
`day::lint::bridge-link-missing` ([Linking](#linking)) and the build still runs, because the
linker's own error is the authority on whether it can be satisfied.

## Diagnostics

A compile error in an inline arm reports a line in a generated file. Where the language has a
mechanism for pointing back at the original source, the generator uses it; where it doesn't, v1
does not build one.

| Arm | Line mapping in v1 |
|---|---|
| `swift` | yes: `#sourceLocation(file:line:)` above the arm |
| `c`, `cpp` | yes: `#line` |
| `js`, `arkts` | yes: a source map beside the generated module |
| `kotlin`, `java` | **no**; the language has no line directive, and a diagnostic rewriter is deferred |

Every generated file, mapped or not, opens with a header naming the crate, the `.rs` path, and the
line the arm starts on, so an unmapped error is a subtraction away from the real source.

**This is why the file form exists.** On a language with no mapping, a long inline arm is a bad
trade: put it in `src = "platform/Speech.kt"`, where `kotlinc` line numbers are already correct and
an IDE, a formatter, and a linter all work. The ~25-line threshold is a suggestion everywhere
except Kotlin and Java, where it is the recommendation.

## Determinism and mtimes

These are two separate requirements, and meeting the first does not meet the second.

**Byte-stable output**, because generated sources are inputs to reproducible builds
([docs/reproducible-builds](https://daybrite.dev/docs/reproducible-builds)): arms are emitted in declaration order, symbol lists are sorted, and
no timestamp, absolute path, or hostname appears in generated output. The repro CI leg covers
bridge output like any other artifact.

**Stable mtimes**, because the native build behind each arm is incremental. Every generated file is
written through a touch-only-when-changed helper (read the current bytes, return early when they
match), and stale files are pruned rather than left behind, which is the rule DESIGN §17.5 already
states for conveyance ("touch only when changed — keeps native incremental builds warm") and the
macOS `DayPieces` generator already implements. Identical bytes rewritten unconditionally still
restamp the file, and `swift build` and hvigor key their incremental checks on **mtime and size**,
so an unconditional write recompiles the whole module on every `day build`. Gradle is the
exception: it content-hashes its inputs, so an identical rewrite costs nothing there.

Neither `std::fs::write` nor `std::fs::copy` is acceptable in a bridge generator: the first
restamps, and the second restamps *and* drops the source mtime.

## Testing

- The `other` arm is what makes `cargo test` and day-mock work on a development host, so it is
  mandatory.
- Each bridged crate carries a test that calls every declared function and asserts only that it
  does not panic, the shape `day-part-battery` already uses. A crate with a `Done<T>` also
  checks that the callback and the future both answer on the fallback (`day-part-speech`'s
  `the_completing_forms_always_answer`), with the ~20-line park/unpark `block_on` from
  [docs/async.md](async.md).
- day-build's `bridge_c.rs` compiles a C arm that completes a token, and `bridge_js.rs` parses
  a module whose arm returns a promise, so the generated helpers stay valid in both languages.
- Real behavior is checked where the arm runs: a dayscript step in the showcase walkthrough, on
  every target's CI leg.

## Worked example

[day-part-speech](https://github.com/daybrite/day-part-speech) is the reference crate: one file, six foreign arms (Swift, Java, ArkTS,
JavaScript, C++, C), a Rust fallback, a demo app whose walkthrough speaks a line on macOS, iOS,
and Android, and a Showcase page that does the same on every target. The
C++ arm drove the `encoding = "utf16"` option: SAPI speaks `WCHAR`, so the same `&str`
declaration is converted for Windows and left as UTF-8 everywhere else.
Read it before writing a bridge of your own.

## Non-goals

- **Scope stays at functions over simple values.** Rich structured values, cross-language object
  graphs, and out-of-process hosts were designed once as dayffi and dropped
  ([§15.3](../DESIGN.md)); the evidence there was that one string and one number covered every real
  case. This design starts smaller.
- **A bridge implements a function, never a view.** Native views enter the tree through
  `Toolkit::adopt` and the piece mechanisms ([docs/extending.md](extending.md)), which are unaffected.
- **Rust comes first.** Where Rust reaches the API (`objc2`, `#[link]`), use Rust.
- **Foreign state is limited to process statics.** An arm may hold a synthesizer or a connection in
  a file-scope static; it may not hold anything Rust is expected to own or free.

## Implementation phases

The order is chosen so each phase can be checked on its own and the expensive ones sit where they
can be cut without stranding the phases before them.

| # | Phase | Proves it works when |
|---|---|---|
| 0 | **Contract** — this file, DESIGN §15.6 | reviewed |
| 1 | **Skeleton** — `day-bridge` (discarding macros, `Error`, `Support`), `day-build/src/bridge.rs` scanner + Rust generator, `rust`/`other` arm only | a crate with only a fallback arm passes `cargo check` and `cargo test` on the host, under day-mock, with no foreign toolchain installed |
| 2 | **C / C++** — `cc` from the crate's `build.rs`, `#line` mapping, `link`/`pkg_config`/`encoding` attributes | a C arm builds on macOS, Linux, and Windows in CI, and an injected syntax error names the `.rs` line |
| 3 | **Swift** — adapters into the generated `DayPieces` module | an inline Swift arm calls AVFoundation on `macos-appkit` and `ios-uikit`; a Swift type error names the `.rs` line |
| 4 | **Kotlin / JNI** — generated object into a Gradle `srcDir`, `RegisterNatives` in `JNI_OnLoad`, thread attach, `GlobalRef` promotion | `day-part-battery` migrates: its `DayBattery.java`, its `java = [...]` table, and its packed-`i64` protocol all deleted, emulator walkthrough green |
| 5 | **JavaScript** — per-crate ES module under `build/day/web/bridge/`, imported by the day-dom shim, merged into the wasm import table | headless WebKit walkthrough green; two bridge crates linked together produce no duplicate or missing import |
| 6 | **ArkTS** — generated module with its `Index.ets`, wired through hvigor | the arm runs on the ohos leg in CI; that leg is the verification, with no local emulator run in the acceptance criteria (harmony-arkui is Tier 3, and its emulator is the least reliable part of the local loop) |
| 7 | **`day-part-speech`** — the reference crate, six arms, plus a Day-Showcase Platform-services demo and dayscript step (cross-repo) | `speak`/`stop` work on every target that claims support, the walkthrough drives the demo on each target's CI leg, and `available()` matches the generated matrix |
| 8 | **Migrate the synchronous parts** — battery (in 4), then haptics, deviceinfo, clipboard, prefs, network, http | every `platform/android/java` directory in those seven crates is gone, walkthroughs green |
| 9 | **Gates** — `docs/bridge-matrix.md` + drift check, the `day lint` rules from [What fails the build](#what-fails-the-build), determinism in the repro leg, `day doctor` probes per arm | CI fails on a hand-edited matrix, a missing `other` arm, and a symbol collision |

**Review checkpoints** fall after phase 3 (skeleton plus the two backends that share the least machinery), after phase 4 (Kotlin and the battery migration, the payoff and the riskiest single phase), and after phase 9.

Deferred with the callback tier, not scheduled here: `day-part-location`, `day-part-sensors`,
`day-part-permissions`, and `day-part-local-notify`, whose Android shims push events back to Rust
(6, 6, 1, and 1 listener respectively) and therefore need [callbacks and streams](#after-v1).

## After v1

Deferred, each with its shape sketched so v1 doesn't foreclose it:

- **Callbacks and futures** shipped 2026-09 as [the callback tier](#callbacks), with one
  divergence from the sketch: the trampoline does not post to the main loop (see
  [Threads](#threads)).
- **Streams.** Sensors and location want repeated delivery rather than a completion. This is a
  separate declaration (`#[day_bridge::stream]`) rather than a relaxation of the at-most-once
  callback rule: an `Emit<T>` fires many times and a paired stop arm ends it. The token
  registry and the per-language completion symbols are its building blocks.
- **Kotlin `suspend` arms**, launched by the generated wrapper and completing the token from the
  coroutine; opt-in per arm, since it pulls kotlinx-coroutines into the app's Gradle graph.
- **The ArkTS Rust half** shipped 2026-09 (see [Callbacks](#callbacks)); a synchronous
  `String`/`Vec<u8>` return still has no spelling on that arm.
- **Kotlin/Java diagnostic remapping**, if inline arms in those languages turn out to be common
  enough to justify a `kotlinc` output rewriter.

## Resolved decisions

Settled 2026-08-10, recorded so the reasoning outlives the discussion:

| Question | Decision | Why |
|---|---|---|
| Symbol collisions | **Fail the build**, naming both manifest paths | A path hash would make the failure silent and the symbol unreadable in a crash log; predictable names are what the derivation table is for |
| Win32 string width | **Opt-in per arm** (`encoding = "utf16"`), UTF-8 by default | A UTF-8 C library on Windows stays natural; an arm calling a wide API asks for what it needs |
| Kotlin `suspend` | **Deferred** past the callback tier | Bridgeable now that tokens exist; waits for an arm that needs it |
| Where a completion runs | **The platform's thread**; the generated future is the door to the UI thread (2026-09) | Posting from generated code would make an arm behave differently under `cargo test` and in an app, and a part may never call `on_main` ([docs/async.md](async.md) rule 3) |
| Multi-shot callbacks | **Deferred**; a separate `#[day_bridge::stream]` when it lands | Sensors and location need repeated delivery, which is a different shape from at-most-once completion; half-building either would foreclose the other |
| Struct evolution | **No versioning.** A change breaks every arm at once, and file-form arms are signature-validated | Everything regenerates in one build, so the only drift risk is the hand-written file arm, which the validator catches |

The one open question of v1 — whether to reserve argument space for the callback tier's `u64`
tokens — was answered by the tier itself: no space was reserved, and the token became a trailing
argument the generator adds, exactly as the default predicted. Nothing here is a published ABI,
so the regeneration cost nothing.
