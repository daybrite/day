#!/usr/bin/env bash
# validate-apk.sh <dir> — install the shipped .apk on a running emulator and prove it starts (§20.3).
#
# Stage 1 of the android-mdc shipped-artifact validation: an APK cannot be installed without a
# device, so CI boots an emulator through reactivecircus/android-emulator-runner and runs this.
#
# It lives in a file rather than inline in the workflow because that action executes each LINE of
# its `script` input as a separate `sh -c`: shell state (a variable holding the APK path) would not
# survive from one line to the next, and that `sh` is dash, where `set -o pipefail` is an error
# rather than an option. One line invoking one script sidesteps both.
set -euo pipefail

dir="${1:?usage: validate-apk.sh <dir-holding-the-apk>}"

# -print -quit rather than `| head -1`: under `set -o pipefail` the early close would surface as a
# SIGPIPE failure of `find` itself.
apk="$(find "$dir" -name '*.apk' -print -quit)"
[ -n "$apk" ] || { echo "::error::no .apk under $dir"; exit 1; }
echo "validating $apk"

adb shell pm list packages | sort > /tmp/pkgs-before
adb install -r "$apk"
adb shell pm list packages | sort > /tmp/pkgs-after

# Whatever appeared is the package just installed — no aapt needed, and aapt is not reliably on
# PATH inside the emulator action.
pkg="$(comm -13 /tmp/pkgs-before /tmp/pkgs-after | sed -n '1s/^package://p' | tr -d '\r')"
[ -n "$pkg" ] || { echo "::error::the .apk installed no new package"; exit 1; }

adb shell monkey -p "$pkg" -c android.intent.category.LAUNCHER 1
sleep 10

# `pidof` exits non-zero with no output when the process is gone, which is what a crash on startup
# looks like from here.
pid="$(adb shell pidof "$pkg" | tr -d '\r' || true)"
if [ -z "$pid" ]; then
  echo "::error::$pkg is not running 10s after launch"
  adb logcat -d -t 200
  exit 1
fi
echo "installed and running: $pkg (pid $pid)"

adb uninstall "$pkg" > /dev/null
