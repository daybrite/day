#!/usr/bin/env bash
# Sign a HarmonyOS .hap with the OpenHarmony DEFAULT DEBUG signing material bundled in the SDK's
# command-line-tools — no Huawei developer account required. Mirrors what DevEco Studio's "auto-sign"
# does: export the CA chain from OpenHarmony.p12, generate an app-signing cert chain, sign a debug
# provision profile for this bundle, then sign the .hap.
#
#   usage: sign-hap.sh <unsigned.hap> <signed.hap> <bundle-name> [toolchains-lib-dir]
#
# The lib dir defaults to $DEVECO_SDK_HOME/default/openharmony/toolchains/lib (set by the CLT).
set -euo pipefail

UNSIGNED="$1"; SIGNED="$2"; BUNDLE="$3"
LIB="${4:-$DEVECO_SDK_HOME/default/openharmony/toolchains/lib}"
JAR="$LIB/hap-sign-tool.jar"; P12="$LIB/OpenHarmony.p12"; PW=123456
W="$(mktemp -d)"; trap 'rm -rf "$W"' EXIT

# 1) Export the root + sub CA certs from the default keystore.
keytool -exportcert -rfc -alias "openharmony application root ca" \
  -keystore "$P12" -storepass "$PW" -storetype PKCS12 -file "$W/root.cer" >/dev/null 2>&1
keytool -exportcert -rfc -alias "openharmony application ca" \
  -keystore "$P12" -storepass "$PW" -storetype PKCS12 -file "$W/subca.cer" >/dev/null 2>&1

# 2) Generate the app-signing cert chain (leaf + sub CA + root CA) for the default release key.
java -jar "$JAR" generate-app-cert \
  -keyAlias "OpenHarmony Application Release" \
  -issuer "C=CN,O=OpenHarmony,OU=OpenHarmony Team,CN=OpenHarmony Application CA" \
  -issuerKeyAlias "OpenHarmony Application CA" \
  -subject "C=CN,O=OpenHarmony,OU=OpenHarmony Team,CN=OpenHarmony Application Release" \
  -validity 3650 -signAlg SHA256withECDSA \
  -rootCaCertFile "$W/root.cer" -subCaCertFile "$W/subca.cer" \
  -keystoreFile "$P12" -keystorePwd "$PW" -keyPwd "$PW" -issuerKeyPwd "$PW" \
  -outForm certChain -outFile "$W/app.cer"

# 3) Fill the debug provision-profile template with this app's bundle name + the generated leaf cert.
python3 - "$LIB/UnsgnedDebugProfileTemplate.json" "$BUNDLE" "$W/app.cer" "$W/profile.json" <<'PY'
import json, sys, re
tmpl, bundle, appcer, out = sys.argv[1:5]
d = json.load(open(tmpl))
d.setdefault("bundle-info", {})["bundle-name"] = bundle
# Match the profile's development-certificate to the leaf we just generated.
leaf = re.search(r"-----BEGIN CERTIFICATE-----.*?-----END CERTIFICATE-----",
                 open(appcer).read(), re.S)
if leaf:
    d["bundle-info"]["development-certificate"] = leaf.group(0) + "\n"
json.dump(d, open(out, "w"))
PY

# 4) Sign the profile → .p7b.
java -jar "$JAR" sign-profile -mode localSign \
  -keyAlias "OpenHarmony Application Profile Debug" -keyPwd "$PW" \
  -profileCertFile "$LIB/OpenHarmonyProfileDebug.pem" \
  -inFile "$W/profile.json" -signAlg SHA256withECDSA \
  -keystoreFile "$P12" -keystorePwd "$PW" -outFile "$W/profile.p7b"

# 5) Sign the .hap.
java -jar "$JAR" sign-app -mode localSign \
  -keyAlias "OpenHarmony Application Release" -keyPwd "$PW" \
  -appCertFile "$W/app.cer" -profileFile "$W/profile.p7b" \
  -inFile "$UNSIGNED" -signAlg SHA256withECDSA \
  -keystoreFile "$P12" -keystorePwd "$PW" -outFile "$SIGNED" -signCode 1

echo "signed → $SIGNED"
