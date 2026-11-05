# Native Android context-menu tests

These instrumentation tests open real Android popups inside an installed debug Day app.
They cover lazy providers, repeated summons, local touch coordinates, accessibility
summons, empty results, replacement, reentrant removal, view release, row rebinding,
and coexistence with row clicks and declared tap gestures. The runner compiles a temporary
test APK and removes it after execution. It stops the target app and preserves its data.
Use an isolated test app, then relaunch it after the run.

Build and install the app against this checkout first. Set `JAVA_HOME` to a JDK and
`ANDROID_HOME` to an SDK containing platform 36 and build tools. Pass the app’s compiled
Java classes and the debug keystore that signed its APK (the default is
`~/.android/debug.keystore` with the standard Android debug password).

```sh
python3 crates/day-toolkit-material/tests/android/run-native.py \
  --serial emulator-5556 \
  --package dev.daybrite.news.automerge.android \
  --classes ../Day-News/build/android/app/build/intermediates/javac/debug/compileDebugJavaWithJavac/classes
```

The instrumentation invokes native `View.performLongClick` and checks the resulting
`PopupMenu`; it does not use DayScript’s direct model dispatch. Also exercise a physical
long press and scroll in the app: the fixture cannot verify an app’s own row bindings,
menu actions, or touch interception by its complete view hierarchy.
