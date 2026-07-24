#!/usr/bin/env node
// Post-build: patch the hvigor-built .hap so it declares itself an OpenHarmony app, then sign it with
// the OpenHarmony public RELEASE material bundled in the SDK — no Huawei developer account.
//
// Why the patch: the openharmony-rs/emulator-action Oniro emulator enforces app CODE-signature
// verification but does not trust the public OpenHarmony release cert, so `bm install` fails with
// `code:9568393 verify code signature failed` — for hvigor's OWN signature as much as an out-of-band
// one; the signing method is not the problem. OpenHarmony's BMS, however, SKIPS install-time
// code-sign verification for apps that declare `compileSdkType: "OpenHarmony"` on devices that lack
// Huawei OH code signing (installd `PerformCodeSignatureCheck` returns early when
// `isCompileSdkOpenHarmony && !IsSupportOHCodeSign()` — the Oniro reference build has no
// SUPPORT_OH_CODE_SIGN). That is the standard path by which OpenHarmony apps install on OpenHarmony
// devices; the parser even defaults compileSdkType to "OpenHarmony". hvigor (HarmonyOS
// command-line-tools) writes "HarmonyOS" into module.json, forcing enforcement — so we rewrite it.
//
// The app/profile signature (hapVerify) still runs, so we sign with the SDK's release material (which
// the emulator DOES accept — installs got past hapVerify to the code-sign stage before).
//
//   usage: node sign-hap.mjs <unsigned.hap> <signed.hap>   (cwd = the platform/ohos/ project root)
import * as fs from 'node:fs';
import * as path from 'node:path';
import * as os from 'node:os';
import { execFileSync } from 'node:child_process';

const [unsigned, signed] = process.argv.slice(2);
if (!unsigned || !signed) {
  console.error('usage: node sign-hap.mjs <unsigned.hap> <signed.hap>');
  process.exit(2);
}

// Find the SDK's toolchains/lib (release signing material). setup-ohos-sdk puts it next to the NDK
// (OHOS_NDK_HOME/../toolchains/lib); a full SDK has it under OHOS_BASE_SDK_HOME / DEVECO_SDK_HOME.
function findLib() {
  const cands = [];
  if (process.env.OHOS_NDK_HOME) cands.push(path.join(path.dirname(process.env.OHOS_NDK_HOME), 'toolchains', 'lib'));
  if (process.env.OHOS_BASE_SDK_HOME) cands.push(path.join(process.env.OHOS_BASE_SDK_HOME, 'toolchains', 'lib'));
  if (process.env.DEVECO_SDK_HOME) cands.push(path.join(process.env.DEVECO_SDK_HOME, 'default', 'openharmony', 'toolchains', 'lib'));
  for (const c of cands) if (fs.existsSync(path.join(c, 'OpenHarmonyProfileRelease.pem'))) return c;
  throw new Error(
    'sign-hap: could not locate OpenHarmony signing material (OpenHarmonyProfileRelease.pem). ' +
      'Set OHOS_NDK_HOME, OHOS_BASE_SDK_HOME, or DEVECO_SDK_HOME.',
  );
}

const lib = findLib();
const JAR = path.join(lib, 'hap-sign-tool.jar');
const P12 = path.join(lib, 'OpenHarmony.p12');
const PEM = path.join(lib, 'OpenHarmonyProfileRelease.pem'); // 3-cert chain; leaf = "…Application Profile Release"
const TMPL = path.join(lib, 'UnsgnedReleasedProfileTemplate.json');
const ALIAS = 'openharmony application profile release';
const PW = '123456';

const work = fs.mkdtempSync(path.join(os.tmpdir(), 'hap-sign-'));
try {
  // 1) Patch module.json (compileSdkType -> "OpenHarmony") in a copy of the unsigned hap. Only the
  //    module.json entry is rewritten; every other entry (incl. the aligned native libs) stays byte
  //    identical, and the signature we add next covers the patched content.
  const patched = path.join(work, 'patched.hap');
  fs.copyFileSync(unsigned, patched);
  execFileSync('unzip', ['-o', '-j', patched, 'module.json', '-d', work], { stdio: 'pipe' });
  const mjPath = path.join(work, 'module.json');
  if (!fs.existsSync(mjPath)) throw new Error('sign-hap: module.json not found at the hap root');
  const mj = JSON.parse(fs.readFileSync(mjPath, 'utf8'));
  mj.app = mj.app || {};
  mj.app.compileSdkType = 'OpenHarmony';
  fs.writeFileSync(mjPath, JSON.stringify(mj));
  execFileSync('zip', [path.resolve(patched), 'module.json'], { cwd: work, stdio: 'pipe' });

  // 2) Fill the release provision-profile template (bundle name from AppScope/app.json5, normal-app
  //    apl, distribution-certificate = the pem's leaf — the cert we sign the hap with).
  const bundle = readJson5(path.join(process.cwd(), 'AppScope', 'app.json5')).app.bundleName;
  const tmpl = JSON.parse(fs.readFileSync(TMPL, 'utf8'));
  tmpl['bundle-info']['bundle-name'] = bundle;
  tmpl['bundle-info']['apl'] = 'normal';
  tmpl['bundle-info']['app-feature'] = 'hos_normal_app';
  const certs = fs.readFileSync(PEM, 'utf8').match(/-----BEGIN CERTIFICATE-----[\s\S]*?-----END CERTIFICATE-----/g);
  tmpl['bundle-info']['distribution-certificate'] = certs[certs.length - 1] + '\n';
  const tmplOut = path.join(work, 'profile.json');
  fs.writeFileSync(tmplOut, JSON.stringify(tmpl, null, 2));

  // 3) Sign the provision profile -> .p7b.
  const p7b = path.join(work, 'profile.p7b');
  java(['sign-profile', '-keyAlias', ALIAS, '-signAlg', 'SHA256withECDSA', '-mode', 'localSign',
    '-profileCertFile', PEM, '-inFile', tmplOut, '-keystoreFile', P12, '-outFile', p7b,
    '-keyPwd', PW, '-keystorePwd', PW]);

  // 4) Sign the patched hap (release identity). The code signature is present but never verified on
  //    the emulator (skipped for an OpenHarmony app); hapVerify checks this app/profile signature.
  java(['sign-app', '-keyAlias', ALIAS, '-signAlg', 'SHA256withECDSA', '-mode', 'localSign',
    '-appCertFile', PEM, '-profileFile', p7b, '-inFile', patched, '-keystoreFile', P12,
    '-outFile', signed, '-keyPwd', PW, '-keystorePwd', PW, '-signCode', '1']);

  console.log(`sign-hap: ${path.basename(signed)} (compileSdkType=OpenHarmony, bundle ${bundle})`);
} finally {
  fs.rmSync(work, { recursive: true, force: true });
}

function java(args) {
  execFileSync('java', ['-jar', JAR, ...args], { stdio: 'inherit' });
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
