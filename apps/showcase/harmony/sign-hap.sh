#!/usr/bin/env bash
# Sign a HarmonyOS .hap with the OpenHarmony public RELEASE signing material bundled in the SDK — no
# Huawei developer account. This mirrors what hvigor / DevEco and the Eclipse Oniro CI
# (eclipse-oniro4openharmony/oniro-app-builder) do for a NORMAL app: sign a release provision profile
# for this bundle, then sign the .hap (INCLUDING its code signature) with the SDK's **shipped**
# release cert + key ("openharmony application profile release" / OpenHarmonyProfileRelease.pem).
#
# Why the shipped cert (and NOT a freshly `generate-app-cert`'d leaf): the emulator verifies the .hap
# CODE signature (the `-signCode` block) against its trusted code-sign store. A generated leaf — even a
# chain-valid one — is not in that store, so `bm install` fails with code 9568393 "verify code
# signature failed". The shipped release cert IS trusted, so the code signature verifies.
#
#   usage: sign-hap.sh <unsigned.hap> <signed.hap> <bundle-name> [toolchains-lib-dir]
set -euo pipefail

UNSIGNED="$1"; SIGNED="$2"; BUNDLE="$3"

# The signing material lives in a `toolchains/lib` dir. Prefer an explicit 4th arg; else find it next
# to the SDK NDK (OHOS_NDK_HOME/../toolchains/lib — where setup-ohos-sdk puts the RELEASE material) or
# under the command-line-tools SDK (DEVECO_SDK_HOME) — whichever actually has the release cert.
LIB="${4:-}"
if [ -z "$LIB" ]; then
  for cand in \
    "$(dirname "${OHOS_NDK_HOME:-/nonexistent}")/toolchains/lib" \
    "${DEVECO_SDK_HOME:-/nonexistent}/default/openharmony/toolchains/lib"; do
    if [ -f "$cand/OpenHarmonyProfileRelease.pem" ]; then LIB="$cand"; break; fi
  done
fi
: "${LIB:?could not locate OpenHarmony signing material — set OHOS_NDK_HOME/DEVECO_SDK_HOME or pass the toolchains/lib dir}"

JAR="$LIB/hap-sign-tool.jar"
P12="$LIB/OpenHarmony.p12"
CERT="$LIB/OpenHarmonyProfileRelease.pem"        # 3-cert chain; leaf = "…Application Profile Release"
TMPL="$LIB/UnsgnedReleasedProfileTemplate.json"
ALIAS="openharmony application profile release"  # the p12 key that matches the leaf of $CERT
PW=123456
W="$(mktemp -d)"; trap 'rm -rf "$W"' EXIT

# 1) Fill the release provision-profile template with this app's bundle name AND set its
#    distribution-certificate to the leaf of $CERT — the same cert the .hap is signed with (step 3),
#    so the profile authorizes the signing identity.
python3 - "$TMPL" "$BUNDLE" "$CERT" "$W/profile.json" <<'PY'
import json, re, sys
tmpl, bundle, certfile, out = sys.argv[1:5]
d = json.load(open(tmpl))
bi = d.setdefault("bundle-info", {})
bi["bundle-name"] = bundle
# The chain is [root, sub-CA, leaf]; the leaf (last) is the "…Application Profile Release" cert.
certs = re.findall(r"-----BEGIN CERTIFICATE-----.*?-----END CERTIFICATE-----",
                   open(certfile).read(), re.S)
bi["distribution-certificate"] = certs[-1] + "\n"
json.dump(d, open(out, "w"))
PY

# 2) Sign the provision profile -> .p7b (release identity).
java -jar "$JAR" sign-profile -mode localSign \
  -keyAlias "$ALIAS" -keyPwd "$PW" \
  -profileCertFile "$CERT" \
  -inFile "$W/profile.json" -signAlg SHA256withECDSA \
  -keystoreFile "$P12" -keystorePwd "$PW" -outFile "$W/profile.p7b"

# 3) Sign the .hap (+ its code signature) with the same shipped, trusted release cert/key.
java -jar "$JAR" sign-app -mode localSign \
  -keyAlias "$ALIAS" -keyPwd "$PW" \
  -appCertFile "$CERT" -profileFile "$W/profile.p7b" \
  -inFile "$UNSIGNED" -signAlg SHA256withECDSA \
  -keystoreFile "$P12" -keystorePwd "$PW" -outFile "$SIGNED" -signCode 1

echo "signed → $SIGNED"
