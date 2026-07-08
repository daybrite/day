#!/usr/bin/env node
// Prepare hvigor signing for this HarmonyOS app using the OpenHarmony public RELEASE material bundled
// in the SDK — no Huawei developer account. Run (cwd = this harmony/ project) BEFORE `hvigorw
// assembleHap`; hvigor's SignHap task then signs the .hap — including its CODE signature — during the
// build, which is what makes it installable on the Oniro emulator.
//
// Why hvigor (not an external `hap-sign-tool sign-app`): OpenHarmony's runtime verifies the code
// signature (fs-verity-style page hashes of the packed .abc/.so) at install. hvigor packages AND code
// signs in one coordinated pass; signing an already-packaged .hap out-of-band yields a signature the
// device rejects with `9568393 verify code signature failed`. This mirrors eclipse-oniro4openharmony/
// oniro-app-builder, which writes signingConfigs into build-profile.json5 and lets hvigor sign.
//
// This regenerates the (git-ignored) `signatures/` dir each build: the keystore + profile cert copied
// from the SDK, a bundle-specific provision profile signed here, and the fixed password-cipher
// `material/` that pairs with the encrypted store/key passwords committed in build-profile.json5.
import * as fs from 'node:fs';
import * as path from 'node:path';
import * as crypto from 'node:crypto';
import { execFileSync } from 'node:child_process';

const projectDir = process.cwd();

// The OpenHarmony password cipher for the public keystore password "123456", as fixed blobs. These
// pair with the encrypted `storePassword`/`keyPassword` in build-profile.json5's signingConfig — hvigor
// re-derives the work key from this material to decrypt them. (Fixed, not random, so the committed
// build-profile.json5 stays valid; regenerate both together via scripts/ohos/gen-material.mjs.)
const MATERIAL = {
  'fd/0': 'eff3e69cb2d262acdcff5c72ee43891a',
  'fd/1': 'b8c565413f5f4d8c99f652946356e68f',
  'fd/2': '194b8955bdbda46e42a75f3267752ce9',
  ac: '39eed4ae3990002ad5d3ae36e4026e13',
  ce: '0000002020525c002b4d91c6ab0b881b7e3e4b37317fc7f27b36aa720497f2a474c3ee2c1628ac25e55dc85201c86a97',
};

// Find the SDK's `toolchains/lib` (release signing material). setup-ohos-sdk puts it next to the NDK
// (OHOS_NDK_HOME/../toolchains/lib); a full SDK has it under OHOS_BASE_SDK_HOME / DEVECO_SDK_HOME.
function findLib() {
  const cands = [];
  if (process.env.OHOS_NDK_HOME) cands.push(path.join(path.dirname(process.env.OHOS_NDK_HOME), 'toolchains', 'lib'));
  if (process.env.OHOS_BASE_SDK_HOME) cands.push(path.join(process.env.OHOS_BASE_SDK_HOME, 'toolchains', 'lib'));
  if (process.env.DEVECO_SDK_HOME) cands.push(path.join(process.env.DEVECO_SDK_HOME, 'default', 'openharmony', 'toolchains', 'lib'));
  for (const c of cands) if (fs.existsSync(path.join(c, 'OpenHarmonyProfileRelease.pem'))) return c;
  throw new Error(
    'setup-signing: could not locate OpenHarmony signing material (OpenHarmonyProfileRelease.pem). ' +
      'Set OHOS_NDK_HOME, OHOS_BASE_SDK_HOME, or DEVECO_SDK_HOME.',
  );
}

// Minimal JSON5 read (strip // and /* */ comments + trailing commas) — enough for AppScope/app.json5.
function readJson5(p) {
  const s = fs
    .readFileSync(p, 'utf8')
    .replace(/\/\*[\s\S]*?\*\//g, '')
    .replace(/(^|[^:])\/\/.*$/gm, '$1')
    .replace(/,(\s*[}\]])/g, '$1');
  return JSON.parse(s);
}

const lib = findLib();
const JAR = path.join(lib, 'hap-sign-tool.jar');
const P12 = path.join(lib, 'OpenHarmony.p12');
const PEM = path.join(lib, 'OpenHarmonyProfileRelease.pem');
const TMPL = path.join(lib, 'UnsgnedReleasedProfileTemplate.json');
const ALIAS = 'openharmony application profile release';
const PW = '123456';

const sig = path.join(projectDir, 'signatures');
fs.rmSync(sig, { recursive: true, force: true });
fs.mkdirSync(sig, { recursive: true });

// 1) Password-cipher material (fixed) — pairs with build-profile.json5's encrypted passwords. hvigor's
//    reader takes the single file in each leaf dir, so the sha256-named filenames are cosmetic.
for (const [rel, hex] of Object.entries(MATERIAL)) {
  const dir = path.join(sig, 'material', rel);
  fs.mkdirSync(dir, { recursive: true });
  const buf = Buffer.from(hex, 'hex');
  fs.writeFileSync(path.join(dir, crypto.createHash('sha256').update(buf).digest('hex')), buf);
}

// 2) Keystore + profile-signing cert, copied from the SDK.
fs.copyFileSync(P12, path.join(sig, 'OpenHarmony.p12'));
fs.copyFileSync(PEM, path.join(sig, 'OpenHarmonyProfileRelease.pem'));

// 3) Fill the release provision-profile template: this app's bundle name, normal-app apl, and a
//    distribution-certificate = the pem's leaf ("…Application Profile Release") — the cert hvigor
//    signs the HAP with, so the profile authorizes it.
const bundle = readJson5(path.join(projectDir, 'AppScope', 'app.json5')).app.bundleName;
const tmpl = JSON.parse(fs.readFileSync(TMPL, 'utf8'));
tmpl['bundle-info']['bundle-name'] = bundle;
tmpl['bundle-info']['apl'] = 'normal';
tmpl['bundle-info']['app-feature'] = 'hos_normal_app';
const certs = fs.readFileSync(PEM, 'utf8').match(/-----BEGIN CERTIFICATE-----[\s\S]*?-----END CERTIFICATE-----/g);
tmpl['bundle-info']['distribution-certificate'] = certs[certs.length - 1] + '\n';
const tmplOut = path.join(sig, 'UnsgnedReleasedProfileTemplate.json');
fs.writeFileSync(tmplOut, JSON.stringify(tmpl, null, 2));

// 4) Sign the provision profile -> app1-profile.p7b (release identity).
execFileSync(
  'java',
  ['-jar', JAR, 'sign-profile', '-keyAlias', ALIAS, '-signAlg', 'SHA256withECDSA', '-mode', 'localSign',
    '-profileCertFile', PEM, '-inFile', tmplOut, '-keystoreFile', P12,
    '-outFile', path.join(sig, 'app1-profile.p7b'), '-keyPwd', PW, '-keystorePwd', PW],
  { stdio: 'inherit' },
);

console.log(`setup-signing: prepared ${path.relative(projectDir, sig)}/ for bundle ${bundle}`);
