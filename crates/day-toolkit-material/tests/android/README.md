# Native Android instrumentation tests

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

## Drag handlers keep their touch stream

`--test DragHold` runs `DragHoldInstrumentation.java`: a piece with a drag handler inside
Day's scroll view inside its pull-to-refresh, driven by touches injected through the input
pipeline. It checks that the drag receives every move and no cancel, that the page neither
scrolls nor refreshes under it, and that the same swipes on a piece with only a tap handler
still scroll the page and pull to refresh. It then checks drags declared along one axis
(`DayBridge.setDragAxis`, what `.on_drag_along` sets): a row that drags sideways inside a
vertical scroll view keeps a horizontal swipe, uncancelled and with the page still, and gives a
vertical swipe to the scroll view, which scrolls; a column that drags vertically inside a
horizontal scroll view does the same the other way round. The app must include the
pull-to-refresh piece.

```sh
python3 crates/day-toolkit-material/tests/android/run-native.py --test DragHold \
  --serial emulator-5554 --package <app id> --classes <app classes>
```
