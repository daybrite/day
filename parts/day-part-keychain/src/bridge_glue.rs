// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The daybridge declaration the Android arm implements (docs/bridge.md), and the Java itself.
//! `android.rs` reads the answers; this file is the wire. Compiled on Android alone.

use std::sync::{Arc, Mutex};

// Status-only answers are an `i32`; `get_native` and the presence read answer `status ‖ bytes`.
day_bridge::bridge! {
    #[day_bridge::declare]
    extern "day" {
        /// Whether a protection level (its byte code) is realized here: 0 yes, else no.
        fn protection_support_native(level: i32) -> Result<i32, day_bridge::Error>;
        /// Store, replacing any item under `key`.
        fn set_native(key: &str, secret: &[u8], level: i32) -> Result<i32, day_bridge::Error>;
        /// Read without a prompt: `status ‖ secret`, completed before the call returns (the Java
        /// generator carries bytes through a completion only); a user-presence item answers
        /// NEEDS_PRESENCE.
        fn get_native(key: &str, done: day_bridge::Done<Vec<u8>>) -> Result<(), day_bridge::Error>;
        /// Read, prompting with `reason` when the item needs the person: `status ‖ secret`.
        fn get_presence_native(key: &str, reason: &str, done: day_bridge::Done<Vec<u8>>) -> Result<(), day_bridge::Error>;
        /// Remove the item under `key`.
        fn remove_native(key: &str) -> Result<i32, day_bridge::Error>;
        /// 1 when an item exists under `key`, 0 when none, else a status.
        fn contains_native(key: &str) -> Result<i32, day_bridge::Error>;
        /// Every item as `<level digit><key>`, joined by U+001F.
        fn entries_native() -> Result<String, day_bridge::Error>;
    }

    #[day_bridge::impl(java, platforms = [android])]
    java!(
        prelude = r#"
            import android.app.KeyguardManager;
            import android.content.Context;
            import android.content.DialogInterface;
            import android.hardware.biometrics.BiometricManager;
            import android.hardware.biometrics.BiometricPrompt;
            import android.os.Build;
            import android.os.CancellationSignal;
            import android.security.keystore.KeyGenParameterSpec;
            import android.security.keystore.KeyPermanentlyInvalidatedException;
            import android.security.keystore.KeyProperties;
            import android.security.keystore.UserNotAuthenticatedException;
            import dev.daybrite.day.bridge.DayBridge;
            import java.io.ByteArrayOutputStream;
            import java.io.File;
            import java.io.FileInputStream;
            import java.io.FileOutputStream;
            import java.io.InputStream;
            import java.nio.charset.StandardCharsets;
            import java.security.KeyPairGenerator;
            import java.security.KeyStore;
            import java.security.MessageDigest;
            import java.security.PrivateKey;
            import java.security.PublicKey;
            import java.security.SecureRandom;
            import java.security.spec.MGF1ParameterSpec;
            import java.util.ArrayList;
            import java.util.List;
            import java.util.concurrent.Executor;
            import javax.crypto.Cipher;
            import javax.crypto.KeyGenerator;
            import javax.crypto.SecretKey;
            import javax.crypto.spec.GCMParameterSpec;
            import javax.crypto.spec.OAEPParameterSpec;
            import javax.crypto.spec.PSource;
            import javax.crypto.spec.SecretKeySpec;
        "#,
        body = r#"
            // Status codes (android.rs `status`).
            private static final int OK = 0;
            private static final int NOT_FOUND = 1;
            private static final int NEEDS_PRESENCE = 2;
            private static final int UNREADABLE = 3;
            private static final int UNSUPPORTED = 4;
            private static final int CANCELLED = 5;
            private static final int AUTH_FAILED = 6;
            private static final int PASSCODE_NOT_SET = 7;
            private static final int LOCKOUT = 8;
            private static final int UNAVAILABLE = 10;
            private static final int TOO_LARGE = 11;
            private static final int PLATFORM = 20;

            // Protection levels (Protection::code).
            private static final int WHEN_UNLOCKED = 0;
            private static final int AFTER_FIRST_UNLOCK = 1;
            private static final int USER_PRESENCE = 2;

            private static final String KEYSTORE = "AndroidKeyStore";
            private static final String AES_ALIAS = "dev.daybrite.day.keychain.aes.v1";
            private static final String RSA_ALIAS = "dev.daybrite.day.keychain.rsa.v1";
            private static final String AES_GCM = "AES/GCM/NoPadding";
            private static final String RSA_OAEP = "RSA/ECB/OAEPWithSHA-256AndMGF1Padding";
            // The Keystore's OAEP uses SHA-1 for MGF1 whatever the transformation says; naming it
            // on both sides keeps the wrap and the unwrap in agreement.
            private static final OAEPParameterSpec OAEP = new OAEPParameterSpec(
                "SHA-256", "MGF1", MGF1ParameterSpec.SHA1, PSource.PSpecified.DEFAULT);
            private static final int GCM_IV_LEN = 12;
            private static final int GCM_TAG_BITS = 128;
            private static final int MAX_SECRET = 64 * 1024;
            private static final byte[] MAGIC = "DAYKEYCH".getBytes(StandardCharsets.US_ASCII);
            private static final byte VERSION = 1;
            private static final String EXTENSION = ".dayk";
            private static final String SEP = "\u001f";

            /** Serializes the file and keystore work within this process. */
            private static final Object LOCK = new Object();
            private static CancellationSignal daySignal = null;

            // --- files -----------------------------------------------------------------------

            private static File dir() {
                Context ctx = DayBridge.ctx;
                if (ctx == null) {
                    return null;
                }
                return new File(ctx.getNoBackupFilesDir(), "day-keychain");
            }

            private static String fileName(String key) throws Exception {
                byte[] digest = MessageDigest.getInstance("SHA-256").digest(key.getBytes(StandardCharsets.UTF_8));
                StringBuilder hex = new StringBuilder(64);
                for (byte b : digest) {
                    hex.append(Character.forDigit((b >> 4) & 15, 16)).append(Character.forDigit(b & 15, 16));
                }
                return hex.append(EXTENSION).toString();
            }

            private static byte[] encodeRecord(String key, int level, byte[] payload) throws Exception {
                byte[] name = key.getBytes(StandardCharsets.UTF_8);
                ByteArrayOutputStream out = new ByteArrayOutputStream(12 + name.length + payload.length);
                out.write(MAGIC);
                out.write(VERSION);
                out.write(level);
                out.write(name.length & 0xff);
                out.write((name.length >> 8) & 0xff);
                out.write(name);
                out.write(payload);
                return out.toByteArray();
            }

            /** A decoded record: level, key, payload. Null when the bytes are not one. */
            private static final class Record {
                int level;
                String key;
                byte[] payload;
            }

            private static Record decodeRecord(byte[] bytes) {
                if (bytes.length < 12 || bytes[8] != VERSION) {
                    return null;
                }
                for (int i = 0; i < 8; i++) {
                    if (bytes[i] != MAGIC[i]) {
                        return null;
                    }
                }
                int level = bytes[9];
                if (level < WHEN_UNLOCKED || level > USER_PRESENCE) {
                    return null;
                }
                int nameLen = (bytes[10] & 0xff) | ((bytes[11] & 0xff) << 8);
                if (bytes.length < 12 + nameLen) {
                    return null;
                }
                Record r = new Record();
                r.level = level;
                r.key = new String(bytes, 12, nameLen, StandardCharsets.UTF_8);
                r.payload = new byte[bytes.length - 12 - nameLen];
                System.arraycopy(bytes, 12 + nameLen, r.payload, 0, r.payload.length);
                return r;
            }

            private static byte[] readAll(File file) throws Exception {
                try (InputStream in = new FileInputStream(file)) {
                    ByteArrayOutputStream out = new ByteArrayOutputStream();
                    byte[] buf = new byte[8192];
                    int n;
                    while ((n = in.read(buf)) > 0) {
                        out.write(buf, 0, n);
                    }
                    return out.toByteArray();
                }
            }

            private static void writeAtomic(File dir, String name, byte[] bytes) throws Exception {
                if (!dir.isDirectory() && !dir.mkdirs()) {
                    throw new Exception("cannot create " + dir);
                }
                File tmp = new File(dir, name + "." + android.os.Process.myPid() + ".tmp");
                File dst = new File(dir, name);
                try (FileOutputStream out = new FileOutputStream(tmp)) {
                    out.write(bytes);
                    out.getFD().sync();
                }
                if (!tmp.renameTo(dst)) {
                    tmp.delete();
                    throw new Exception("cannot replace " + dst);
                }
            }

            private static Record readRecord(String key) throws Exception {
                File dir = dir();
                if (dir == null) {
                    return null;
                }
                File file = new File(dir, fileName(key));
                if (!file.isFile()) {
                    return null;
                }
                Record r = decodeRecord(readAll(file));
                if (r == null || !r.key.equals(key)) {
                    throw new CorruptItem();
                }
                return r;
            }

            private static final class CorruptItem extends Exception {}

            // --- keys ------------------------------------------------------------------------

            private static KeyStore keystore() throws Exception {
                KeyStore ks = KeyStore.getInstance(KEYSTORE);
                ks.load(null);
                return ks;
            }

            private static SecretKey aesKey(boolean create) throws Exception {
                KeyStore ks = keystore();
                KeyStore.Entry entry = ks.getEntry(AES_ALIAS, null);
                if (entry instanceof KeyStore.SecretKeyEntry) {
                    return ((KeyStore.SecretKeyEntry) entry).getSecretKey();
                }
                if (!create) {
                    return null;
                }
                KeyGenerator gen = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, KEYSTORE);
                gen.init(new KeyGenParameterSpec.Builder(AES_ALIAS,
                        KeyProperties.PURPOSE_ENCRYPT | KeyProperties.PURPOSE_DECRYPT)
                    .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                    .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                    .setKeySize(256)
                    .build());
                return gen.generateKey();
            }

            private static void ensureRsaKey() throws Exception {
                KeyStore ks = keystore();
                if (ks.containsAlias(RSA_ALIAS)) {
                    return;
                }
                KeyGenParameterSpec.Builder spec = new KeyGenParameterSpec.Builder(RSA_ALIAS,
                        KeyProperties.PURPOSE_ENCRYPT | KeyProperties.PURPOSE_DECRYPT)
                    .setKeySize(2048)
                    .setDigests(KeyProperties.DIGEST_SHA256, KeyProperties.DIGEST_SHA1)
                    .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_RSA_OAEP)
                    .setUserAuthenticationRequired(true)
                    .setInvalidatedByBiometricEnrollment(false);
                if (Build.VERSION.SDK_INT >= 30) {
                    spec.setUserAuthenticationParameters(0,
                        KeyProperties.AUTH_BIOMETRIC_STRONG | KeyProperties.AUTH_DEVICE_CREDENTIAL);
                } else {
                    spec.setUserAuthenticationValidityDurationSeconds(-1);
                }
                KeyPairGenerator gen = KeyPairGenerator.getInstance(KeyProperties.KEY_ALGORITHM_RSA, KEYSTORE);
                gen.initialize(spec.build());
                gen.generateKeyPair();
            }

            private static boolean deviceSecure() {
                Context ctx = DayBridge.ctx;
                if (ctx == null) {
                    return false;
                }
                KeyguardManager keyguard = (KeyguardManager) ctx.getSystemService(Context.KEYGUARD_SERVICE);
                return keyguard != null && keyguard.isDeviceSecure();
            }

            // --- the cipher work -------------------------------------------------------------

            private static byte[] gcmSeal(SecretKey key, byte[] aad, byte[] plain) throws Exception {
                Cipher cipher = Cipher.getInstance(AES_GCM);
                cipher.init(Cipher.ENCRYPT_MODE, key);
                cipher.updateAAD(aad);
                byte[] iv = cipher.getIV();
                byte[] sealed = cipher.doFinal(plain);
                byte[] out = new byte[iv.length + sealed.length];
                System.arraycopy(iv, 0, out, 0, iv.length);
                System.arraycopy(sealed, 0, out, iv.length, sealed.length);
                return out;
            }

            private static byte[] gcmOpen(SecretKey key, byte[] aad, byte[] payload) throws Exception {
                if (payload.length < GCM_IV_LEN + 16) {
                    throw new CorruptItem();
                }
                Cipher cipher = Cipher.getInstance(AES_GCM);
                cipher.init(Cipher.DECRYPT_MODE, key, new GCMParameterSpec(GCM_TAG_BITS, payload, 0, GCM_IV_LEN));
                cipher.updateAAD(aad);
                return cipher.doFinal(payload, GCM_IV_LEN, payload.length - GCM_IV_LEN);
            }

            /** A presence item's payload: wrapped-key length (2 bytes), the wrapped key, then GCM. */
            private static byte[] sealPresence(String key, byte[] plain) throws Exception {
                ensureRsaKey();
                byte[] raw = new byte[32];
                new SecureRandom().nextBytes(raw);
                SecretKey itemKey = new SecretKeySpec(raw, "AES");
                PublicKey pub = keystore().getCertificate(RSA_ALIAS).getPublicKey();
                Cipher wrap = Cipher.getInstance(RSA_OAEP);
                wrap.init(Cipher.ENCRYPT_MODE, pub, OAEP);
                byte[] wrapped = wrap.doFinal(raw);
                java.util.Arrays.fill(raw, (byte) 0);
                byte[] sealed = gcmSeal(itemKey, key.getBytes(StandardCharsets.UTF_8), plain);
                byte[] out = new byte[2 + wrapped.length + sealed.length];
                out[0] = (byte) (wrapped.length & 0xff);
                out[1] = (byte) ((wrapped.length >> 8) & 0xff);
                System.arraycopy(wrapped, 0, out, 2, wrapped.length);
                System.arraycopy(sealed, 0, out, 2 + wrapped.length, sealed.length);
                return out;
            }

            private static int wrappedLength(byte[] payload) throws Exception {
                if (payload.length < 2) {
                    throw new CorruptItem();
                }
                int len = (payload[0] & 0xff) | ((payload[1] & 0xff) << 8);
                if (payload.length < 2 + len) {
                    throw new CorruptItem();
                }
                return len;
            }

            /** The RSA decryption cipher for a presence item, initialized but not yet authorized. */
            private static Cipher unwrapCipher() throws Exception {
                KeyStore ks = keystore();
                KeyStore.Entry entry = ks.getEntry(RSA_ALIAS, null);
                if (!(entry instanceof KeyStore.PrivateKeyEntry)) {
                    throw new CorruptItem();
                }
                PrivateKey priv = ((KeyStore.PrivateKeyEntry) entry).getPrivateKey();
                Cipher unwrap = Cipher.getInstance(RSA_OAEP);
                unwrap.init(Cipher.DECRYPT_MODE, priv, OAEP);
                return unwrap;
            }

            /** Finish a presence read with a cipher the prompt authorized. */
            private static byte[] openPresence(Cipher unwrap, String key, byte[] payload) throws Exception {
                int len = wrappedLength(payload);
                byte[] raw = unwrap.doFinal(payload, 2, len);
                SecretKey itemKey = new SecretKeySpec(raw, "AES");
                java.util.Arrays.fill(raw, (byte) 0);
                byte[] sealed = new byte[payload.length - 2 - len];
                System.arraycopy(payload, 2 + len, sealed, 0, sealed.length);
                return gcmOpen(itemKey, key.getBytes(StandardCharsets.UTF_8), sealed);
            }

            private static byte[] withStatus(int status, byte[] payload) {
                byte[] out = new byte[1 + payload.length];
                out[0] = (byte) status;
                System.arraycopy(payload, 0, out, 1, payload.length);
                return out;
            }

            private static byte[] failure(Throwable t) {
                if (t instanceof CorruptItem
                    || t instanceof KeyPermanentlyInvalidatedException
                    || t instanceof UserNotAuthenticatedException
                    || t instanceof javax.crypto.AEADBadTagException
                    || t instanceof java.security.GeneralSecurityException) {
                    return new byte[] { (byte) UNREADABLE };
                }
                return withStatus(PLATFORM, String.valueOf(t).getBytes(StandardCharsets.UTF_8));
            }

            // --- the declared functions -------------------------------------------------------

            public static int protection_support_native(int level) {
                if (level == USER_PRESENCE) {
                    return Build.VERSION.SDK_INT >= 28 ? 0 : 1;
                }
                return 0;
            }

            public static int set_native(String key, byte[] secret, int level) {
                synchronized (LOCK) {
                    File dir = dir();
                    if (dir == null) {
                        return UNAVAILABLE;
                    }
                    if (secret.length > MAX_SECRET) {
                        return TOO_LARGE;
                    }
                    try {
                        byte[] payload;
                        if (level == USER_PRESENCE) {
                            if (Build.VERSION.SDK_INT < 28) {
                                return UNSUPPORTED;
                            }
                            if (!deviceSecure()) {
                                return PASSCODE_NOT_SET;
                            }
                            payload = sealPresence(key, secret);
                        } else {
                            payload = gcmSeal(aesKey(true), key.getBytes(StandardCharsets.UTF_8), secret);
                        }
                        writeAtomic(dir, fileName(key), encodeRecord(key, level, payload));
                        return OK;
                    } catch (Throwable t) {
                        return failure(t)[0];
                    }
                }
            }

            public static void get_native(String key, long done) {
                get_native_complete(done, dayRead(key));
            }

            private static byte[] dayRead(String key) {
                synchronized (LOCK) {
                    try {
                        if (dir() == null) {
                            return new byte[] { (byte) UNAVAILABLE };
                        }
                        Record r = readRecord(key);
                        if (r == null) {
                            return new byte[] { (byte) NOT_FOUND };
                        }
                        if (r.level == USER_PRESENCE) {
                            return new byte[] { (byte) NEEDS_PRESENCE };
                        }
                        SecretKey aes = aesKey(false);
                        if (aes == null) {
                            return new byte[] { (byte) UNREADABLE };
                        }
                        return withStatus(OK, gcmOpen(aes, key.getBytes(StandardCharsets.UTF_8), r.payload));
                    } catch (Throwable t) {
                        return failure(t);
                    }
                }
            }

            public static void get_presence_native(final String key, final String reason, final long done) {
                final Record r;
                final Cipher unwrap;
                synchronized (LOCK) {
                    try {
                        if (dir() == null) {
                            get_presence_native_complete(done, new byte[] { (byte) UNAVAILABLE });
                            return;
                        }
                        Record found = readRecord(key);
                        if (found == null) {
                            get_presence_native_complete(done, new byte[] { (byte) NOT_FOUND });
                            return;
                        }
                        if (found.level != USER_PRESENCE) {
                            get_presence_native_complete(done, dayRead(key));
                            return;
                        }
                        if (Build.VERSION.SDK_INT < 28) {
                            get_presence_native_complete(done, new byte[] { (byte) UNSUPPORTED });
                            return;
                        }
                        wrappedLength(found.payload);
                        r = found;
                        unwrap = unwrapCipher();
                    } catch (Throwable t) {
                        get_presence_native_complete(done, failure(t));
                        return;
                    }
                }
                final Context ctx = DayBridge.ctx;
                DayBridge.main.post(new Runnable() {
                    @Override
                    public void run() {
                        dayPrompt(ctx, r, unwrap, reason, done);
                    }
                });
            }

            private static void dayPrompt(Context ctx, final Record r, Cipher unwrap, String reason, final long done) {
                try {
                    Executor executor = ctx.getMainExecutor();
                    BiometricPrompt.Builder builder = new BiometricPrompt.Builder(ctx).setTitle(reason);
                    if (Build.VERSION.SDK_INT >= 30) {
                        builder.setAllowedAuthenticators(BiometricManager.Authenticators.BIOMETRIC_STRONG
                            | BiometricManager.Authenticators.DEVICE_CREDENTIAL);
                    } else {
                        builder.setNegativeButton(ctx.getString(android.R.string.cancel), executor,
                            new DialogInterface.OnClickListener() {
                                @Override
                                public void onClick(DialogInterface dialog, int which) {
                                    daySettle(done, new byte[] { (byte) CANCELLED });
                                }
                            });
                    }
                    CancellationSignal signal = new CancellationSignal();
                    daySignal = signal;
                    builder.build().authenticate(new BiometricPrompt.CryptoObject(unwrap), signal, executor,
                        new BiometricPrompt.AuthenticationCallback() {
                            @Override
                            public void onAuthenticationError(int code, CharSequence message) {
                                int status;
                                switch (code) {
                                    case BiometricPrompt.BIOMETRIC_ERROR_USER_CANCELED:
                                    case BiometricPrompt.BIOMETRIC_ERROR_CANCELED:
                                        status = CANCELLED; break;
                                    case BiometricPrompt.BIOMETRIC_ERROR_LOCKOUT:
                                    case BiometricPrompt.BIOMETRIC_ERROR_LOCKOUT_PERMANENT:
                                        status = LOCKOUT; break;
                                    case BiometricPrompt.BIOMETRIC_ERROR_NO_BIOMETRICS:
                                    case 14: // BIOMETRIC_ERROR_NO_DEVICE_CREDENTIAL (API 29)
                                        status = PASSCODE_NOT_SET; break;
                                    case BiometricPrompt.BIOMETRIC_ERROR_TIMEOUT:
                                    case BiometricPrompt.BIOMETRIC_ERROR_UNABLE_TO_PROCESS:
                                        status = AUTH_FAILED; break;
                                    default:
                                        daySettle(done, withStatus(PLATFORM,
                                            (code + ": " + message).getBytes(StandardCharsets.UTF_8)));
                                        return;
                                }
                                daySettle(done, new byte[] { (byte) status });
                            }

                            @Override
                            public void onAuthenticationSucceeded(BiometricPrompt.AuthenticationResult result) {
                                try {
                                    Cipher authorized = result.getCryptoObject().getCipher();
                                    byte[] secret;
                                    synchronized (LOCK) {
                                        secret = openPresence(authorized, r.key, r.payload);
                                    }
                                    daySettle(done, withStatus(OK, secret));
                                } catch (Throwable t) {
                                    daySettle(done, failure(t));
                                }
                            }
                        });
                } catch (Throwable t) {
                    daySettle(done, failure(t));
                }
            }

            private static void daySettle(long done, byte[] answer) {
                daySignal = null;
                get_presence_native_complete(done, answer);
            }

            public static int remove_native(String key) {
                synchronized (LOCK) {
                    File dir = dir();
                    if (dir == null) {
                        return UNAVAILABLE;
                    }
                    try {
                        File file = new File(dir, fileName(key));
                        if (!file.isFile()) {
                            return NOT_FOUND;
                        }
                        return file.delete() ? OK : PLATFORM;
                    } catch (Throwable t) {
                        return PLATFORM;
                    }
                }
            }

            public static int contains_native(String key) {
                File dir = dir();
                if (dir == null) {
                    return UNAVAILABLE;
                }
                try {
                    return new File(dir, fileName(key)).isFile() ? 1 : 0;
                } catch (Throwable t) {
                    return PLATFORM;
                }
            }

            public static String entries_native() {
                File dir = dir();
                if (dir == null) {
                    return "";
                }
                File[] files = dir.listFiles();
                if (files == null) {
                    return "";
                }
                List<String> out = new ArrayList<>();
                for (File file : files) {
                    if (!file.getName().endsWith(EXTENSION)) {
                        continue;
                    }
                    try {
                        Record r = decodeRecord(readAll(file));
                        if (r != null) {
                            out.add(r.level + r.key);
                        }
                    } catch (Throwable t) {
                        // A damaged file is reported by the read that asks for it.
                    }
                }
                StringBuilder joined = new StringBuilder();
                for (String item : out) {
                    if (joined.length() > 0) {
                        joined.append(SEP);
                    }
                    joined.append(item);
                }
                return joined.toString();
            }
        "#,
    );

    // The fallback every bridge declares. This module is compiled only on Android, so it never
    // runs; it satisfies the rule that a bridge always has an answer for an unclaimed target.
    #[day_bridge::impl(rust, platforms = [other])]
    fn protection_support_native(_level: i32) -> Result<i32, day_bridge::Error> {
        Err(day_bridge::Error::Unsupported)
    }

    #[day_bridge::impl(rust, platforms = [other])]
    fn set_native(_key: &str, _secret: &[u8], _level: i32) -> Result<i32, day_bridge::Error> {
        Err(day_bridge::Error::Unsupported)
    }

    #[day_bridge::impl(rust, platforms = [other])]
    fn get_native(_key: &str, _done: day_bridge::Done<Vec<u8>>) -> Result<(), day_bridge::Error> {
        Err(day_bridge::Error::Unsupported)
    }

    #[day_bridge::impl(rust, platforms = [other])]
    fn get_presence_native(_key: &str, _reason: &str, _done: day_bridge::Done<Vec<u8>>) -> Result<(), day_bridge::Error> {
        Err(day_bridge::Error::Unsupported)
    }

    #[day_bridge::impl(rust, platforms = [other])]
    fn remove_native(_key: &str) -> Result<i32, day_bridge::Error> {
        Err(day_bridge::Error::Unsupported)
    }

    #[day_bridge::impl(rust, platforms = [other])]
    fn contains_native(_key: &str) -> Result<i32, day_bridge::Error> {
        Err(day_bridge::Error::Unsupported)
    }

    #[day_bridge::impl(rust, platforms = [other])]
    fn entries_native() -> Result<String, day_bridge::Error> {
        Err(day_bridge::Error::Unsupported)
    }
}

// The generated synchronous entry points are private to this module; `android.rs` reaches them
// through these.

pub(crate) fn support_level(level: i32) -> Result<i32, day_bridge::Error> {
    protection_support_native(level)
}

pub(crate) fn store(key: &str, secret: &[u8], level: i32) -> Result<i32, day_bridge::Error> {
    set_native(key, secret, level)
}

/// What a read completes with.
type Answer = Result<Vec<u8>, day_bridge::Error>;

/// The plain read. The arm completes before it returns, so the answer is in the slot by the
/// time the call comes back; an arm that did not (its bug) reads as a runtime failure.
pub(crate) fn read(key: &str) -> Result<Vec<u8>, day_bridge::Error> {
    let slot: Arc<Mutex<Option<Answer>>> = Arc::new(Mutex::new(None));
    let sink = Arc::clone(&slot);
    get_native_async(key, move |answer| {
        *lock(&sink) = Some(answer);
    })?;
    let taken = lock(&slot).take();
    taken.unwrap_or(Err(day_bridge::Error::Runtime))
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match m.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    }
}

pub(crate) fn remove_item(key: &str) -> Result<i32, day_bridge::Error> {
    remove_native(key)
}

pub(crate) fn has_item(key: &str) -> Result<i32, day_bridge::Error> {
    contains_native(key)
}

pub(crate) fn list_items() -> Result<String, day_bridge::Error> {
    entries_native()
}
