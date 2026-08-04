#!/usr/bin/env bash
# Reproducibility check (DESIGN.md §20.3): compare an artifact built by a platform-toolkit job
# against the same artifact rebuilt from the same commit in a different directory.
#
#   usage: repro-check.sh <combo> <dir-original> <dir-rebuilt> <report-dir>
#
# Two tiers, because they carry different weight:
#
#   payload   — the compiled code (Mach-O / ELF / PE / .so) extracted from whatever container it
#               ships in. This is what reproducibility is actually about, and a mismatch here
#               FAILS: it means the same sources produced different machine code, or a build path
#               leaked into the binary.
#   container — the shipped file itself (.dmg/.ipa/.apk/.aab/.hap/.msix/.flatpak/-setup.exe). Every
#               one of these is produced by a tool that stamps mtimes (hdiutil, ditto -c -k, gradle,
#               flatpak-builder, makeappx, makensis), and none of them is wired to SOURCE_DATE_EPOCH
#               yet, so a mismatch here is REPORTED but does not fail.
#
# Exit codes: 0 = both tiers identical. 10 = payload identical, container differs (advisory).
#             1 = payload differs (hard failure). 2 = structural problem (file lists disagree).
set -uo pipefail

# Standard tools first. harmony-arkui puts the OpenHarmony SDK toolchains on PATH, and they ship a
# `diff` that rejects GNU options *and exits 0* — which silently turned every payload comparison
# into a pass. Nothing here decides a verdict with diff(1) any more (cmp only), but a shadowed
# coreutils is a trap worth closing outright.
export PATH="/usr/bin:/bin:$PATH"

COMBO="${1:?usage: repro-check.sh <combo> <dir-original> <dir-rebuilt> <report-dir>}"
DIR_A="${2:?missing dir-original}"
DIR_B="${3:?missing dir-rebuilt}"
REPORT="${4:?missing report-dir}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

mkdir -p "$REPORT"
SUMMARY="$REPORT/summary.md"
: > "$SUMMARY"

note() { echo "$*" | tee -a "$SUMMARY"; }

# sha256 of $1, as a bare hex digest. NOT `shasum`: that is a Perl script living in
# /usr/bin/core_perl on the Windows runners' Git Bash, which the forced PATH above does not
# include — so it resolved to "command not found" there and the report printed empty hashes.
# coreutils' sha256sum IS in /usr/bin on every runner; the rest are belt and braces.
sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | cut -d' ' -f1
  elif command -v openssl >/dev/null 2>&1; then
    openssl dgst -sha256 "$1" | awk '{print $NF}'
  else
    echo "sha256-unavailable"
  fi
}

for d in "$DIR_A" "$DIR_B"; do
  [ -d "$d" ] || { echo "::error::not a directory: $d"; exit 2; }
done

# --- file lists must agree before anything else is meaningful --------------------------------
( cd "$DIR_A" && find . -type f | LC_ALL=C sort ) > "$REPORT/files-a.txt"
( cd "$DIR_B" && find . -type f | LC_ALL=C sort ) > "$REPORT/files-b.txt"
if ! cmp -s "$REPORT/files-a.txt" "$REPORT/files-b.txt"; then
  note "## $COMBO — FAIL (structural)"
  note ""
  note 'The rebuild produced a different set of files.'
  note ''
  note 'Original:'
  note '```'
  cat "$REPORT/files-a.txt" | tee -a "$SUMMARY"
  note '```'
  note 'Rebuilt:'
  note '```'
  cat "$REPORT/files-b.txt" | tee -a "$SUMMARY"
  note '```'
  exit 2
fi
if [ ! -s "$REPORT/files-a.txt" ]; then
  echo "::error::no files to compare in $DIR_A"; exit 2
fi

# --- payload extraction ----------------------------------------------------------------------
# Pull the compiled code out of a container into $2. Returns 1 when the format has no payload
# extractor yet — the caller records that rather than pretending the tier passed.
extract_payload() {
  local file="$1" dest="$2" tmp
  mkdir -p "$dest"
  tmp="$(mktemp -d)"
  case "$file" in
    *.dmg)
      local mnt="$tmp/mnt"
      mkdir -p "$mnt"
      hdiutil attach -nobrowse -readonly -quiet -mountpoint "$mnt" "$file" || { rm -rf "$tmp"; return 1; }
      # Every Mach-O in the bundle: the main executable plus any bundled dylib/framework. Names are
      # relative to the mountpoint — the mountpoint itself is a temp path and differs per call, so
      # including it would make the two sides look different on filename alone.
      while IFS= read -r f; do
        [ -s "$f" ] || continue
        cp "$f" "$dest/$(echo "${f#"$mnt/"}" | tr '/' '_')"
      done < <(find "$mnt" \( -path '*/Contents/MacOS/*' -o -path '*/Contents/Frameworks/*' \) \
        -type f | LC_ALL=C sort)
      hdiutil detach -quiet "$mnt" || true
      ;;
    *.ipa|*.apk|*.aab|*.hap|*.msix|*.zip)
      unzip -q -o "$file" -d "$tmp/x" 2>/dev/null || { rm -rf "$tmp"; return 1; }
      # Mach-O executables (ipa), native libs (apk/aab/hap), PE images (msix). Resources carry no
      # code, so they belong to the container tier, not this one.
      while IFS= read -r f; do
        case "$(basename "$f")" in
          *.plist|*.png|*.jpg|*.json|*.xml|*.nib|*.car|*.strings|*.md|*.txt) continue ;;
        esac
        [ -s "$f" ] || continue
        cp "$f" "$dest/$(echo "${f#"$tmp/x/"}" | tr '/' '_')"
      done < <(find "$tmp/x" \
        \( -path '*/Payload/*.app/*' -o -name '*.so' -o -name '*.exe' -o -name '*.dll' \) \
        -type f | LC_ALL=C sort)
      ;;
    *setup.exe)
      # NSIS installer (windows-xaml). 7-Zip reads the NSIS format; it is preinstalled on the
      # Windows runners (7zip 26.x) but is not always on PATH inside Git Bash, so the Program Files
      # locations are probed too. No 7-Zip ⇒ return 1, i.e. the honest "unverified", not a pass.
      local sevenzip=""
      for cand in 7z 7za 7zz \
        "/c/Program Files/7-Zip/7z.exe" "/c/Program Files (x86)/7-Zip/7z.exe"; do
        if command -v "$cand" >/dev/null 2>&1; then sevenzip="$cand"; break; fi
      done
      [ -n "$sevenzip" ] || { rm -rf "$tmp"; return 1; }
      "$sevenzip" x -y -o"$tmp/x" "$file" >/dev/null 2>&1 || { rm -rf "$tmp"; return 1; }
      while IFS= read -r f; do
        [ -s "$f" ] || continue
        cp "$f" "$dest/$(echo "${f#"$tmp/x/"}" | tr '/' '_')"
      done < <(find "$tmp/x" \( -name '*.exe' -o -name '*.dll' \) -type f \
        `# NSIS's own furniture, not the app: the plugin DLLs it unpacks at run time, and the` \
        `# uninstaller makensis generates during the pack. Comparing those would test NSIS's` \
        `# determinism rather than day's, and the uninstaller is a fresh build each time.` \
        ! -path '*$PLUGINSDIR*' ! -iname 'uninst*.exe' ! -iname 'Uninstall*.exe' \
        | LC_ALL=C sort)
      ;;
    *)
      rm -rf "$tmp"
      return 1
      ;;
  esac
  rm -rf "$tmp"
  return 0
}

# Strip the metadata that is derived from the build path rather than from the sources, so the
# payload tier compares code. Today that is the Mach-O LC_UUID (Apple's linker hashes the
# object-file paths into it) plus the ad-hoc signature that covers it.
normalize() {
  local f="$1"
  case "$(file -b "$f" 2>/dev/null)" in
    *Mach-O*)
      command -v codesign >/dev/null 2>&1 && codesign --remove-signature "$f" 2>/dev/null
      python3 "$HERE/macho-normalize.py" "$f" 2>/dev/null || true
      ;;
  esac
}

# Compare two directories by content using cmp only — see the PATH note at the top. Returns 0 when
# every file matches, 1 on any difference (including a differing file list).
dirs_identical() {
  local x="$1" y="$2" f
  ( cd "$x" && find . -type f | LC_ALL=C sort ) > "$REPORT/.dx" 2>/dev/null
  ( cd "$y" && find . -type f | LC_ALL=C sort ) > "$REPORT/.dy" 2>/dev/null
  cmp -s "$REPORT/.dx" "$REPORT/.dy" || return 1
  while IFS= read -r f; do
    cmp -s "$x/$f" "$y/$f" || return 1
  done < "$REPORT/.dx"
  return 0
}

# --- compare ----------------------------------------------------------------------------------
container_diffs=(); payload_diffs=(); no_extractor=(); metadata_only=()

while IFS= read -r rel; do
  a="$DIR_A/${rel#./}"; b="$DIR_B/${rel#./}"
  if cmp -s "$a" "$b"; then
    continue
  fi
  container_diffs+=("${rel#./}")

  # The container differs — does the code inside differ too?
  pa="$REPORT/payload/a/${rel#./}"; pb="$REPORT/payload/b/${rel#./}"
  if extract_payload "$a" "$pa" && extract_payload "$b" "$pb" \
     && [ -n "$(ls -A "$pa" 2>/dev/null)" ]; then
    for f in "$pa"/* "$pb"/*; do [ -f "$f" ] && normalize "$f"; done
    if ! dirs_identical "$pa" "$pb"; then
      payload_diffs+=("${rel#./}")
    fi
  else
    # A bare binary (no container) still gets the payload tier directly.
    if [ "$(file -b "$a" 2>/dev/null | cut -c1-6)" = "Mach-O" ] || [ "$(file -b "$a" 2>/dev/null | cut -c1-3)" = "ELF" ]; then
      mkdir -p "$pa" "$pb"; cp "$a" "$pa/bin"; cp "$b" "$pb/bin"
      normalize "$pa/bin"; normalize "$pb/bin"
      if cmp -s "$pa/bin" "$pb/bin"; then
        # The code matches; what differed was metadata normalize() strips — on Apple platforms the
        # LC_UUID the linker derives from object-file paths, plus the signature covering it.
        metadata_only+=("${rel#./}")
      else
        payload_diffs+=("${rel#./}")
      fi
    else
      no_extractor+=("${rel#./}")
    fi
  fi
done < "$REPORT/files-a.txt"

# --- report -------------------------------------------------------------------------------------
if [ ${#container_diffs[@]} -eq 0 ]; then
  note "## $COMBO — reproducible"
  note ""
  note "Every file is byte-for-byte identical to the original build, rebuilt from a different directory."
  note ""
  note '| file | sha256 |'
  note '| --- | --- |'
  while IFS= read -r rel; do
    note "| \`${rel#./}\` | \`$(sha256 "$DIR_A/${rel#./}")\` |"
  done < "$REPORT/files-a.txt"
  exit 0
fi

# diffoscope explains the byte differences; it recurses into every container format used here.
if command -v diffoscope >/dev/null 2>&1; then
  for rel in "${container_diffs[@]}"; do
    safe="$(echo "$rel" | tr '/ ' '__')"
    diffoscope --max-report-size 2000000 \
      --text "$REPORT/diffoscope-$safe.txt" --html "$REPORT/diffoscope-$safe.html" \
      "$DIR_A/$rel" "$DIR_B/$rel" >/dev/null 2>&1 || true
  done
else
  note "_diffoscope is not installed; no structural diff was produced._"
  note ""
fi

if [ ${#payload_diffs[@]} -gt 0 ]; then
  note "## $COMBO — NOT reproducible (payload differs)"
  note ""
  note "The compiled code itself changed when the same commit was built in a different directory."
  note "That points at a build path leaking into the binary, or nondeterministic codegen."
  note ""
  note "Payload mismatches:"
  for f in "${payload_diffs[@]}"; do note "- \`$f\`"; done
elif [ ${#no_extractor[@]} -gt 0 ]; then
  # Never report a format as reproducible on the strength of a check that did not run. The bytes
  # differ and nothing here can see inside the container, so the honest verdict is "unverified",
  # and unverified fails — otherwise the job is decoration.
  note "## $COMBO — UNVERIFIED (no payload extractor)"
  note ""
  note "These artifacts differ byte-for-byte and this script cannot open them, so whether the"
  note "compiled code matches is unknown. Add an extractor for the format in"
  note "\`extract_payload\` (scripts/ci/repro-check.sh), or have the packing job upload the"
  note "pre-container payload the way the macos job uploads its \`.app\` (§20.3)."
  note ""
  note "Unopened formats:"
  for f in "${no_extractor[@]}"; do note "- \`$f\`"; done
else
  note "## $COMBO — code reproducible, bytes differ"
  note ""
  note "The compiled code is identical. What differs is metadata that the build path feeds into,"
  note "and packaging containers whose tools stamp mtimes without honouring \`SOURCE_DATE_EPOCH\`"
  note "(§20.3)."
fi
if [ ${#metadata_only[@]} -gt 0 ]; then
  note ""
  note "Identical after normalization — differ only in build-path-derived metadata"
  note "(Mach-O \`LC_UUID\` and the signature covering it):"
  for f in "${metadata_only[@]}"; do note "- \`$f\`"; done
fi
note ""
note "Files whose bytes differ:"
for f in "${container_diffs[@]}"; do
  note "- \`$f\` — original \`$(sha256 "$DIR_A/$f" | cut -c1-16)…\` vs rebuilt \`$(sha256 "$DIR_B/$f" | cut -c1-16)…\`"
done
note ""
note "See the \`diffoscope-*.html\` files in this report for the byte-level explanation."

[ ${#payload_diffs[@]} -gt 0 ] && exit 1
[ ${#no_extractor[@]} -gt 0 ] && exit 1
exit 10
