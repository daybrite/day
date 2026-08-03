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

COMBO="${1:?usage: repro-check.sh <combo> <dir-original> <dir-rebuilt> <report-dir>}"
DIR_A="${2:?missing dir-original}"
DIR_B="${3:?missing dir-rebuilt}"
REPORT="${4:?missing report-dir}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

mkdir -p "$REPORT"
SUMMARY="$REPORT/summary.md"
: > "$SUMMARY"

note() { echo "$*" | tee -a "$SUMMARY"; }

for d in "$DIR_A" "$DIR_B"; do
  [ -d "$d" ] || { echo "::error::not a directory: $d"; exit 2; }
done

# --- file lists must agree before anything else is meaningful --------------------------------
( cd "$DIR_A" && find . -type f | LC_ALL=C sort ) > "$REPORT/files-a.txt"
( cd "$DIR_B" && find . -type f | LC_ALL=C sort ) > "$REPORT/files-b.txt"
if ! diff -u "$REPORT/files-a.txt" "$REPORT/files-b.txt" > "$REPORT/filelist.diff"; then
  note "## $COMBO — FAIL (structural)"
  note ""
  note 'The rebuild produced a different set of files:'
  note '```'
  cat "$REPORT/filelist.diff" | tee -a "$SUMMARY"
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
    if ! diff -rq "$pa" "$pb" > "$REPORT/payload-$(basename "$rel").diff" 2>&1; then
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
    note "| \`${rel#./}\` | \`$(shasum -a 256 "$DIR_A/${rel#./}" | cut -d' ' -f1)\` |"
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
  note "- \`$f\` — original \`$(shasum -a 256 "$DIR_A/$f" | cut -c1-16)…\` vs rebuilt \`$(shasum -a 256 "$DIR_B/$f" | cut -c1-16)…\`"
done
if [ ${#no_extractor[@]} -gt 0 ]; then
  note ""
  note "No payload extractor for these formats, so they were checked at the container tier only:"
  for f in "${no_extractor[@]}"; do note "- \`$f\`"; done
fi
note ""
note "See the \`diffoscope-*.html\` files in this report for the byte-level explanation."

[ ${#payload_diffs[@]} -gt 0 ] && exit 1
exit 10
