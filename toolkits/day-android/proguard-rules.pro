# day-android — R8/ProGuard keep rules for Day's Android toolkit.
#
# `day build` adds this file to every Day app's release proguard configuration (resolved from the
# day-android crate, exactly like the Java shim under java/), beside the generated
# `day-jni-keep.pro` and any rules a part or piece declares in
# `[package.metadata.day.android].proguard` (docs/android-jni.md).
#
# What R8 must keep in Day's own Java is decided by the Rust that reaches it, not by a namespace
# rule. Every class and member Rust calls by name is bound with `java_class!`, and `day build`
# collects the binding's keep rule from the app's native library into `day-jni-keep.pro`, so a
# release build keeps exactly those and shrinks, renames and optimizes the rest. Activities,
# services, providers and receivers are kept by the rules AGP generates from the merged manifest.
# Nothing in Day's Java reaches another Day class by reflection; a component that does ships its
# own rules (`[package.metadata.day.android].proguard`).
#
# R8's optimizations (inlining, class merging) are off unless the app sets `[android] optimize`:
# they act on every library in the APK, and some reflect on their own classes in ways their
# consumer rules do not cover (WorkManager's Room database did). `proguard-dontoptimize.pro`
# carries that one line, and `day build` adds it for every app that has not opted in.

# Generic JNI safety net: a class that declares a native method is bound to the C symbol
# `Java_<class>_<method>`, so its class name and native-method names must survive minification.
# This covers app- and piece-supplied JNI shims (an install bridge, a background worker, …) without
# each one having to restate the rule. Their other methods Rust calls are kept by their bindings.
-keepclasseswithmembers,includedescriptorclasses class * {
    native <methods>;
}

# JSR-305 annotations (`javax.annotation.Nullable`, `Nonnull`, …) are compile-time only and never
# on the runtime classpath, but common transitive deps (okio, okhttp, guava) reference them — which
# R8 reports as "missing classes" and AGP escalates to a hard build error. Suppressing the warning
# for this always-absent annotation package is safe and standard boilerplate.
-dontwarn javax.annotation.**
