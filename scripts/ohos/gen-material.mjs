// Port of OpenHarmony's signing-material cipher (matches hap-sign-tool / hvigor decryption, as
// reverse-engineered by eclipse-oniro4openharmony/oniro-app-builder). Generates a material dir, then
// prints the 5 material file blobs (hex) + the encrypted "123456" store/key passwords (hex) so they
// can be baked, deterministically, into setup-signing.mjs + build-profile.json5.
import * as fs from 'node:fs';
import * as path from 'node:path';
import * as os from 'node:os';
import * as crypto from 'node:crypto';

const COMPONENT = Buffer.from([49,243,9,115,214,175,91,184,211,190,177,88,101,131,192,119]);

function encrypt(key, data) {
  const iv = crypto.randomBytes(12);
  const cipher = crypto.createCipheriv('aes-128-gcm', key, iv);
  const ct = Buffer.concat([cipher.update(data), cipher.final()]);
  const tag = cipher.getAuthTag();
  const out = Buffer.alloc(4 + iv.length + ct.length + tag.length);
  out.writeUInt32BE(ct.length + tag.length, 0);
  iv.copy(out, 4); ct.copy(out, 16); tag.copy(out, 16 + ct.length);
  return out;
}
function decrypt(key, data) {
  const total = data.readUInt32BE(0);
  const iv = data.subarray(4, 16);
  const ctLen = total - 16;
  const ct = data.subarray(16, 16 + ctLen);
  const tag = data.subarray(16 + ctLen);
  const d = crypto.createDecipheriv('aes-128-gcm', key, iv);
  d.setAuthTag(tag);
  return Buffer.concat([d.update(ct), d.final()]);
}
function xorBuffers(bufs) {
  const r = Buffer.from(bufs[0]);
  for (let i = 1; i < bufs.length; i++) for (let j = 0; j < r.length; j++) r[j] ^= bufs[i][j];
  return r;
}
function getRootKey(fd, salt) {
  return crypto.pbkdf2Sync(xorBuffers(fd.concat([COMPONENT])).toString(), salt, 10000, 16, 'sha256');
}
function storeKey(dir, len) {
  const key = crypto.randomBytes(len);
  fs.writeFileSync(path.join(dir, crypto.createHash('sha256').update(key).digest('hex')), key);
  return key;
}
function createMaterial(mp) {
  fs.mkdirSync(mp, { recursive: true });
  const fdDir = path.join(mp, 'fd'), acDir = path.join(mp, 'ac'), ceDir = path.join(mp, 'ce');
  for (const d of [fdDir, acDir, ceDir]) fs.mkdirSync(d, { recursive: true });
  const fd = [];
  for (const s of ['0','1','2']) { const sd = path.join(fdDir, s); fs.mkdirSync(sd, {recursive:true}); fd.push(storeKey(sd, 16)); }
  const salt = storeKey(acDir, 16);
  const rootKey = getRootKey(fd, salt);
  const workKey = crypto.randomBytes(16);
  const enc = encrypt(rootKey, workKey);
  fs.writeFileSync(path.join(ceDir, crypto.createHash('sha256').update(enc).digest('hex')), enc);
}
function readOne(dir) {
  const f = fs.readdirSync(dir).filter((x) => x !== '.DS_Store');
  return fs.readFileSync(path.join(dir, f[0]));
}
function getKey(mp) {
  const fd = ['0','1','2'].map((s) => readOne(path.join(mp, 'fd', s)));
  const salt = readOne(path.join(mp, 'ac'));
  return decrypt(getRootKey(fd, salt), readOne(path.join(mp, 'ce')));
}
function encryptPwd(pw, mp) { return encrypt(getKey(mp), Buffer.from(pw, 'utf-8')).toString('hex'); }
function decryptPwd(hex, mp) { return decrypt(getKey(mp), Buffer.from(hex, 'hex')).toString('utf-8'); }

const mp = fs.mkdtempSync(path.join(os.tmpdir(), 'mat-'));
createMaterial(mp);
const fd0 = readOne(path.join(mp,'fd','0')), fd1 = readOne(path.join(mp,'fd','1')), fd2 = readOne(path.join(mp,'fd','2'));
const salt = readOne(path.join(mp,'ac')), ce = readOne(path.join(mp,'ce'));
const encStore = encryptPwd('123456', mp), encKey = encryptPwd('123456', mp);
// round-trip check
if (decryptPwd(encStore, mp) !== '123456' || decryptPwd(encKey, mp) !== '123456') throw new Error('round-trip failed');
console.log(JSON.stringify({
  fd0: fd0.toString('hex'), fd1: fd1.toString('hex'), fd2: fd2.toString('hex'),
  salt: salt.toString('hex'), ce: ce.toString('hex'),
  encStore, encKey,
}, null, 2));
