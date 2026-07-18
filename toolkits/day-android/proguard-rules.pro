# day-android — R8/ProGuard keep rules for Day's Android toolkit.
#
# `day build` aggregates this file into every Day app's release proguard configuration (resolved
# from the day-android crate, exactly like the Java shim under java/). It is the framework half of
# the convention documented in docs/extending.md: any Part or Piece that hands Java classes to
# native code by name ships its own `proguard-rules.pro` and declares it in
# `[package.metadata.day.android].proguard`, and day-build folds them all in.
#
# Why any of this is needed: Day renders through a Java bridge that native (Rust) code reaches by
# fully-qualified name — `DayActivity` is the launcher Activity, `DayBridge` hosts the JNI
# trampolines (the C symbols `Java_dev_daybrite_day_bridge_DayBridge_native*`), and the other views
# are constructed and looked up by name. R8 minification renames classes and methods by default, so
# without these keeps the release APK installs and then crashes at launch (ClassNotFoundException /
# UnsatisfiedLinkError / a renamed class breaking JNI FindClass).

# AGP requires the `proguard-android-optimize.txt` base file, but its aggressive method/class
# optimizations (inlining, merging) routinely break reflection-heavy libraries — WorkManager's Room
# `WorkDatabase` fails to instantiate, for one. R8 still SHRINKS unused code and RENAMES what no
# keep rule protects; it just skips those optimizations. A conservative, predictable default for a
# framework whose apps mix Rust, JNI, and arbitrary Android libraries. (An app that wants the extra
# optimization can re-enable it in its own proguard-rules.pro once it has verified its release build.)
-dontoptimize

# Day's first-party Java lives entirely under `dev.daybrite.day.**` — the render bridge
# (`…day.bridge.*`: DayActivity, DayBridge, the views) and every official Part/Piece's shim
# (`…day.piece.*` / `…day.part.*`, e.g. `dev.daybrite.day.piece.picker.DayPicker`). All of it is
# constructed, found (JNI FindClass), and called by name, so keep the whole namespace intact. This
# covers current and future first-party pieces without each restating the rule; an app's own classes
# and any THIRD-PARTY piece outside this namespace stay covered by the aggregation convention above
# (their own proguard-rules.pro, declared in `[package.metadata.day.android].proguard`).
-keep class dev.daybrite.day.** { *; }

# Generic JNI safety net: a class that declares a native method is bound to the C symbol
# `Java_<class>_<method>`, so its class name and native-method names must survive minification.
# This covers app- and piece-supplied JNI shims (an install bridge, a background worker, …) without
# each one having to restate the rule — though a shim whose *non-native* methods are also called by
# name (via `dcall_static`) still keeps those itself, in its own proguard-rules.pro.
-keepclasseswithmembers,includedescriptorclasses class * {
    native <methods>;
}

# JSR-305 annotations (`javax.annotation.Nullable`, `Nonnull`, …) are compile-time only and never
# on the runtime classpath, but common transitive deps (okio, okhttp, guava) reference them — which
# R8 reports as "missing classes" and AGP escalates to a hard build error. Suppressing the warning
# for this always-absent annotation package is safe and standard boilerplate.
-dontwarn javax.annotation.**
