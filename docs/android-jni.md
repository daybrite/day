---
title: "Android JNI bindings"
description: "How Day's Rust calls Java and Java calls Rust on Android: java_class! bindings, the startup check, and the R8 keep rules they generate."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Android JNI bindings (`java_class!`)

> [!NOTE]
> **Status: shipped 2026-10.** Every Rust-to-Java call in day-android, the first-party pieces,
> parts and tweaks, and the code daybridge generates goes through a `java_class!` binding. The
> hand-typed `DayEnv` helpers (`dcall_static`, `dcall`, `dfield`, `dfind`, `try_make_view_on`) are
> gone.

A **binding** declares one Java class to Rust once: the methods Rust calls, the fields it reads,
and the native methods Java calls back. The declaration gives Rust typed calls with cached method
IDs, a check at launch that the Java still matches, and an R8 keep rule for exactly what is bound.

```rust
day_android::java_class! {
    DayCombo => dev.daybrite.day.piece.combobox.DayCombo,
    methods {
        static fn makeCombo(id: jlong, items: JString, text: JString, placeholder: JString)
            -> android.view.View,
        static fn setComboText(view: android.view.View, text: JString),
    },
}

fn update(_backend: &mut Android, h: &AHandle, text: &str) {
    day_android::java(|env| {
        let text = JString::from_str(env, text)?;
        DayCombo::setComboText(env, h.0.as_obj(), &text)
    });
}
```

`java_class!` is jni's `bind_java_type!` (jni 0.22) with Day's policies around it. Its grammar is
that macro's: `methods`, `fields`, `constructors`, `native_methods`, `type_map`, and the shorthand
and block member forms.

## Names are Java's

Classes, methods and fields keep the names Java gives them: `makeCombo` binds `makeCombo`, and
`speak_native` binds `speak_native`. A search for a Java name finds the Java method, the binding,
and every Rust call. The macro passes each name to jni explicitly, so jni's snake-to-camel-case
conversion never applies, and it allows `non_snake_case` on the items it generates. Parameter
names never reach Java and stay in Rust style.

## Types

Signatures use jni's type syntax:

| Java | In a binding | Rust value |
|---|---|---|
| `boolean`, `int`, `long`, `float`, `double` | `jboolean`, `jint`, `jlong`, `jfloat`, `jdouble` | `bool`, `i32`, `i64`, `f32`, `f64` |
| `String` | `JString` | `JString` (`JString::from_str(env, s)?` to make one) |
| `int[]`, `byte[]`, `double[]` | `jint[]`, `jbyte[]`, `jdouble[]` | `JIntArray`, `JByteArray`, `JDoubleArray` |
| `String[]` | `JString[]` | `JObjectArray<JString>` |
| any other class | its dotted name, `android.view.View` | `JObject` |

Object arguments take `impl AsRef<…>`, so a View Day holds passes as `h.0.as_obj()` and a
string as `&text`. A null argument is the type's `default()`: `JString::default()`,
`JByteArray::default()`.

## Calling

Bound calls return `jni::errors::Result`, and a thrown Java exception stays pending until it is
cleared. Calling with one pending fails every later JNI call, and returning to Java with one
pending rethrows it there. So calls go through one of these scopes, which describe the exception
to logcat and clear it:

| Helper | Use |
|---|---|
| `java(|env| …)` | Attach, run, and get `Option<R>`: `None` after a failure. `?` works inside. |
| `java_in(env, |env| …)` | The same with an `Env` already in hand (a `with_native` tweak, a native method). |
| `java_string(env, |env| …)` | A call returning a Java `String`, read into Rust; `None` for a throw or null. |
| `realize("what", |env| …)` | A piece's View factory: the View as an `AHandle`, or the visible `⟨what⟩` placeholder. |
| `downcast::<T>(env, obj)` | A View Day holds, as bound class `T`, to call `T`'s instance methods (checked cast). |
| `sdk_int()` | The device's API level, read without JNI. |

`with_env` also clears an exception still pending when its closure returns, as a backstop for a
call whose error was dropped.

## Java calling Rust

A `native_methods` block declares what Java calls. The binding generates a trait,
`<Name>NativeInterface`, implemented on `<Name>API`:

```rust
day_android::java_class! {
    DayLocation => dev.daybrite.day.location.DayLocation,
    native_methods {
        static fn nativeError(code: jint),
    },
}

impl DayLocationNativeInterface for DayLocationAPI {
    type Error = day_android::jni::errors::Error;
    fn nativeError<'l>(_env: &mut Env<'l>, _class: JClass<'l>, code: i32) -> Result<(), Self::Error> {
        crate::deliver(Err(match code {
            1 => LocationError::PermissionDenied,
            2 => LocationError::Disabled,
            _ => LocationError::Unavailable,
        }));
        Ok(())
    }
}
```

Each native is registered with the JVM when the startup check resolves the binding, and exported
under its JNI symbol name as well. jni's wrapper catches a panic and logs a returned error
(`LogErrorAndDefault`), so neither can unwind into the JVM. A native that should throw into Java
instead sets `error_policy = day_android::jni::errors::ThrowRuntimeExAndDefault` in block form.

Natives that start the app (`DayBridge.nativeStart`, `nativeStartWindow`) stay in the app's
`day_start_android!`, because they name the app's root piece.

## Platform and library classes

A binding can name any class: Day's own, the Android platform's, or a library's. Two rules apply.

- **Bind where a method is declared.** An R8 keep rule names one class, and keeps only members that
  class declares. Material's `Slider` inherits `getValueFrom` from `BaseSlider`, so a binding of
  `Slider` would leave that method free to be renamed. day-tweak-slider-tickmarks binds both.
- **Core types are jni's.** `java.lang.Object`, `Class`, `String` and `Throwable` cannot be bound;
  jni's `JObject`, `JClass` (with `get_class_loader`), `JString` and `JThrowable` cover them.

A binding needing a newer API level than Day's minimum (24) says so with `since = 26`. Below that
level the startup check skips it, and its callers ask `sdk_int()` before calling, as
day-tweak-tooltip does for `View.setTooltipText`.

## The startup check

Every binding registers itself in `day_android::JAVA_CLASSES`. `day_android::init` resolves each
one: it loads the class, looks up every bound method and field, and registers the natives. A Java
class missing from the build, or a member renamed or retyped, is named in logcat:

```text
day-android: Java binding dev.daybrite.day.piece.combobox.DayCombo does not match the app's classes: Method not found: setComboText (Landroid/view/View;Ljava/lang/String;)V
```

A binding resolves as a whole, so one missing member fails every call through that class. In a
debug build, and in any scripted run (`DAYSCRIPT_PORT` set), a failed check ends the process after
naming every mismatch: a walkthrough drives pieces through Day rather than the native widget, so a
piece rendering as its placeholder would otherwise pass. A release build outside a script carries
on, and the calls through that binding fail softly. The check costs about 0.05 ms per binding in
release.

Classes load through the app's class loader, cached at `init`, so a binding works the same from a
thread Rust started, whose own loader cannot see app classes.

## R8 keep rules

Each binding records a keep rule for its class and the members it binds, as text in the `day_keep`
section of the app's native library:

```proguard
-keep,includedescriptorclasses class dev.daybrite.day.piece.combobox.DayCombo {
    *** makeCombo(...);
    *** setComboText(...);
}
```

After cargo links the library, `day build` reads the section back into
`build/day/android/day-jni-keep.pro` and hands it to R8 with day-android's `proguard-rules.pro`.
Rules come from the library rather than the sources, so they cover exactly the bindings linked into
this app, including any another macro generates. `includedescriptorclasses` keeps the classes in a
kept member's signature, whose renaming would change the descriptor jni looks the method up by.

Nothing else in Day's Java is kept by name. Activities, services, providers and receivers are kept
by the rules AGP generates from the merged manifest; classes with natives keep their native methods
by day-android's rule; everything else R8 may shrink and rename. A component that reaches Java by
reflection ships its own rules in `[package.metadata.day.android].proguard`
([extending](extending.md)).

R8's optimizations (inlining, class merging) are off by default, because they act on every library
in the APK and some libraries reflect on their own classes in ways their consumer rules miss. An app
whose release walkthrough passes with them on opts in:

```toml
[android]
optimize = true
```

## Generated code and lints

The macro puts the generated items in a module of their own and allows there what jni's generated
code needs: `non_snake_case` for Java names, and, for jni-macros 0.22.4, `deprecated`
(`AtomicBool::fetch_update`) and `clippy::too_many_arguments`. No allow is needed at the call site.
