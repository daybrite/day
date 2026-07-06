#!/usr/bin/env bash
# Cross-compile the Day app (Rust → libentry.so) for HarmonyOS and drop it into the entry module's
# libs/ so hvigor packages it into the .hap. Then build/sign/install with DevEco Studio or hvigorw.
#
# Prereqs:
#   - OpenHarmony SDK `native` dir           → export OHOS_NDK_HOME=/path/to/ohos-sdk/native
#   - rustup OHOS std targets                → rustup target add aarch64-unknown-linux-ohos x86_64-unknown-linux-ohos
#   - a rustup (not Homebrew) toolchain with those targets' std
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../../.." && pwd)"          # the day workspace root
: "${OHOS_NDK_HOME:?set OHOS_NDK_HOME to the OpenHarmony SDK 'native' directory}"

# Which arch: "emulator" (x86_64) or "device" (arm64). Default builds both.
ARCHES=("${1:-both}")
[ "${ARCHES[0]}" = "both" ] && ARCHES=(emulator device)

# Prefer a rustup toolchain (Homebrew rustc ships no OHOS std).
CARGO="${CARGO:-cargo}"
PROFILE="${PROFILE:-debug}"
FLAG=""; [ "$PROFILE" = "release" ] && FLAG="--release"

build_one() {
  local target="$1" abi="$2"
  local linker="$OHOS_NDK_HOME/llvm/bin/${target}-clang"
  echo ">>> building libentry.so for $target ($abi)"
  local up="$(echo "$target" | tr 'a-z-' 'A-Z_')"
  env "CARGO_TARGET_${up}_LINKER=$linker" \
    "$CARGO" build -p day-arkui-demo --target "$target" $FLAG --manifest-path "$REPO/Cargo.toml"
  mkdir -p "$HERE/entry/libs/$abi"
  cp "$REPO/target/$target/$PROFILE/libentry.so" "$HERE/entry/libs/$abi/libentry.so"
  echo "    -> entry/libs/$abi/libentry.so"
}

for a in "${ARCHES[@]}"; do
  case "$a" in
    emulator) build_one x86_64-unknown-linux-ohos x86_64 ;;
    device)   build_one aarch64-unknown-linux-ohos arm64-v8a ;;
    *) echo "unknown arch '$a' (use emulator|device|both)"; exit 1 ;;
  esac
done

echo
echo "Native module built. Next:"
echo "  1) Open this 'harmony/' project in DevEco Studio (it fills in default resources), OR run"
echo "     hvigorw assembleHap  (needs the OpenHarmony command-line-tools)."
echo "  2) Sign the .hap (DevEco auto-sign, or a debug signing profile)."
echo "  3) Launch the HarmonyOS emulator, then:  hdc install entry/build/.../entry-default-signed.hap"
echo "     and start the ability, or just Run ▶ from DevEco."
