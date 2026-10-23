// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The one daybridge declaration both foreign arms implement (docs/bridge.md): Java on Android,
//! ArkTS on HarmonyOS. day-build generates one module per crate from every `bridge!` block, so
//! the declaration lives here once and `android.rs` / `ohos.rs` read the answers in their own
//! vocabularies.
//!
//! # The wire
//!
//! An enum cannot cross the bridge, so the arms answer with small integers and bytes:
//!
//! - `availability_native(kind)`: HarmonyOS probes one `UserAuthType` (`kind`) and answers 0 or
//!   the kit's error code; Android ignores `kind` and answers its own availability code
//!   (0 available, 1 not enrolled, 2 no hardware, 3 unsupported, 4 busy, 5 disabled by policy,
//!   6 no screen lock).
//! - `biometry_native()`: a bitmask of the sensors the device lists (1 fingerprint, 2 face,
//!   4 iris).
//! - `authenticate_native(...)`: completes with one byte naming how the person passed followed by
//!   the platform's token (HarmonyOS: the `UserAuthType` that passed and the kit's token; Android:
//!   the prompt's authentication type, 0 unknown, 1 device credential, 2 biometric, and no
//!   token), or fails with the platform's result code as decimal text (`1000` is Android's
//!   negative button, this arm's own number).
//!
//! Every arm runs on its platform's main or JS thread and completes there.

day_bridge::bridge! {
    #[day_bridge::declare]
    extern "day" {
        /// Whether a prompt can succeed now; see the module note for each platform's answer.
        fn availability_native(kind: i32) -> Result<i32, day_bridge::Error>;
        /// The sensors the device lists, as a bitmask: 1 fingerprint, 2 face, 4 iris.
        fn biometry_native() -> Result<i32, day_bridge::Error>;
        /// Show the prompt. `done` completes with a method byte followed by the platform token,
        /// or fails with the platform's result code as decimal text.
        fn authenticate_native(challenge: &[u8], reason: &str, fallback: &str, allow_credential: bool, done: day_bridge::Done<Vec<u8>>) -> Result<(), day_bridge::Error>;
        /// Dismiss the prompt on screen, if any.
        fn cancel_native() -> Result<(), day_bridge::Error>;
    }

    // Android: the platform prompt in `android.hardware.biometrics` (API 28 and later), which
    // takes an application `Context` and needs no Gradle dependency. Plain Java so it compiles in
    // any Android project. `USE_BIOMETRIC` comes from the portable `biometrics` permission.
    #[day_bridge::impl(java, platforms = [android])]
    java!(
        prelude = r#"
            import android.app.KeyguardManager;
            import android.content.Context;
            import android.content.DialogInterface;
            import android.content.pm.PackageManager;
            import android.hardware.biometrics.BiometricManager;
            import android.hardware.biometrics.BiometricPrompt;
            import android.os.Build;
            import android.os.CancellationSignal;
            import dev.daybrite.day.bridge.DayRuntime;
            import java.util.concurrent.Executor;
        "#,
        body = r#"
            private static CancellationSignal daySignal = null;

            public static int availability_native(int kind) {
                Context ctx = DayRuntime.ctx;
                if (ctx == null || Build.VERSION.SDK_INT < 28) {
                    return 3;
                }
                KeyguardManager keyguard = (KeyguardManager) ctx.getSystemService(Context.KEYGUARD_SERVICE);
                if (keyguard != null && !keyguard.isDeviceSecure()) {
                    return 6;
                }
                if (Build.VERSION.SDK_INT >= 29) {
                    BiometricManager manager = (BiometricManager) ctx.getSystemService(Context.BIOMETRIC_SERVICE);
                    if (manager == null) {
                        return 2;
                    }
                    int status = Build.VERSION.SDK_INT >= 30
                        ? manager.canAuthenticate(BiometricManager.Authenticators.BIOMETRIC_STRONG)
                        : manager.canAuthenticate();
                    switch (status) {
                        case BiometricManager.BIOMETRIC_SUCCESS: return 0;
                        case BiometricManager.BIOMETRIC_ERROR_NONE_ENROLLED: return 1;
                        case BiometricManager.BIOMETRIC_ERROR_NO_HARDWARE: return 2;
                        case BiometricManager.BIOMETRIC_ERROR_HW_UNAVAILABLE: return 4;
                        case 15: return 5; // BIOMETRIC_ERROR_SECURITY_UPDATE_REQUIRED (API 30)
                        case -2: return 3; // BIOMETRIC_ERROR_UNSUPPORTED (API 30)
                        default: return 0; // BIOMETRIC_STATUS_UNKNOWN: the prompt decides
                    }
                }
                // API 28 has no manager: the hardware features are the only probe, and
                // enrollment is reported by the prompt itself.
                return biometry_native() == 0 ? 2 : 0;
            }

            public static int biometry_native() {
                Context ctx = DayRuntime.ctx;
                if (ctx == null) {
                    return 0;
                }
                PackageManager pm = ctx.getPackageManager();
                int mask = 0;
                if (pm.hasSystemFeature("android.hardware.fingerprint")) mask |= 1;
                if (pm.hasSystemFeature("android.hardware.biometrics.face")) mask |= 2;
                if (pm.hasSystemFeature("android.hardware.biometrics.iris")) mask |= 4;
                return mask;
            }

            public static void authenticate_native(byte[] challenge, final String reason, final String fallback,
                                                   final boolean allowCredential, final long done) {
                final Context ctx = DayRuntime.ctx;
                if (ctx == null) {
                    authenticate_native_fail(done, "no application context");
                    return;
                }
                if (Build.VERSION.SDK_INT < 28) {
                    authenticate_native_fail(done, "3000");
                    return;
                }
                // The prompt is built and shown on the main thread; its callbacks arrive there.
                DayRuntime.main.post(new Runnable() {
                    @Override
                    public void run() {
                        dayShow(ctx, reason, fallback, allowCredential, done);
                    }
                });
            }

            private static void dayShow(Context ctx, String reason, String fallback,
                                        boolean allowCredential, final long done) {
                try {
                    Executor executor = ctx.getMainExecutor();
                    BiometricPrompt.Builder builder = new BiometricPrompt.Builder(ctx).setTitle(reason);
                    boolean credential = allowCredential;
                    if (Build.VERSION.SDK_INT >= 30) {
                        int kinds = BiometricManager.Authenticators.BIOMETRIC_STRONG;
                        if (credential) {
                            kinds |= BiometricManager.Authenticators.DEVICE_CREDENTIAL;
                        }
                        builder.setAllowedAuthenticators(kinds);
                    } else if (Build.VERSION.SDK_INT >= 29 && credential) {
                        builder.setDeviceCredentialAllowed(true);
                    } else {
                        credential = false;
                    }
                    if (!credential) {
                        // A biometric-only prompt needs a negative button; pressing it is the
                        // fallback the caller handles.
                        String label = fallback.isEmpty() ? ctx.getString(android.R.string.cancel) : fallback;
                        builder.setNegativeButton(label, executor, new DialogInterface.OnClickListener() {
                            @Override
                            public void onClick(DialogInterface dialog, int which) {
                                dayFail(done, 1000);
                            }
                        });
                    }
                    BiometricPrompt prompt = builder.build();
                    CancellationSignal signal = new CancellationSignal();
                    daySignal = signal;
                    prompt.authenticate(signal, executor, new BiometricPrompt.AuthenticationCallback() {
                        @Override
                        public void onAuthenticationError(int code, CharSequence message) {
                            dayFail(done, code);
                        }

                        @Override
                        public void onAuthenticationSucceeded(BiometricPrompt.AuthenticationResult result) {
                            int kind = 0;
                            if (Build.VERSION.SDK_INT >= 30) {
                                kind = result.getAuthenticationType();
                            }
                            daySignal = null;
                            authenticate_native_complete(done, new byte[] { (byte) kind });
                        }
                    });
                } catch (Throwable t) {
                    daySignal = null;
                    authenticate_native_fail(done, t.toString());
                }
            }

            private static void dayFail(long done, int code) {
                daySignal = null;
                authenticate_native_fail(done, String.valueOf(code));
            }

            public static void cancel_native() {
                final CancellationSignal signal = daySignal;
                if (signal != null) {
                    DayRuntime.main.post(new Runnable() {
                        @Override
                        public void run() {
                            signal.cancel();
                        }
                    });
                }
            }
        "#,
    );

    // HarmonyOS: the User Authentication Kit, ArkTS only (no C API in the public SDK). The dialog
    // lists the modalities it is given and refuses one the device lacks, so the arm probes each
    // with `getAvailableStatus` and lists the ones with an executor, enrolled or not; a device with
    // nothing enrolled then fails at `start()` with NOT_ENROLLED. `ACCESS_BIOMETRIC` comes from the
    // portable `biometrics` permission.
    #[day_bridge::impl(arkts, platforms = [ohos])]
    arkts!(
        prelude = r#"
            import { userAuth } from '@kit.UserAuthenticationKit';
            import { BusinessError } from '@kit.BasicServicesKit';
        "#,
        body = r#"
            let dayInstance: userAuth.UserAuthInstance | undefined = undefined;

            export function availability_native(kind: number): number {
              try {
                userAuth.getAvailableStatus(kind as userAuth.UserAuthType, userAuth.AuthTrustLevel.ATL1);
                return 0;
              } catch (e) {
                return (e as BusinessError).code;
              }
            }

            // Whether the device has an executor for the modality, enrolled or not: anything but
            // "type not supported" / "trust level not supported" / "no permission" says it does.
            function dayHasExecutor(kind: userAuth.UserAuthType): boolean {
              const code = availability_native(kind);
              return code !== 12500005 && code !== 12500006 && code !== 201;
            }

            export function biometry_native(): number {
              let mask = 0;
              if (dayHasExecutor(userAuth.UserAuthType.FINGERPRINT)) { mask |= 1; }
              if (dayHasExecutor(userAuth.UserAuthType.FACE)) { mask |= 2; }
              return mask;
            }

            export function authenticate_native(challenge: Uint8Array, reason: string, fallback: string,
                                                allowCredential: boolean, done: number): void {
              const types: userAuth.UserAuthType[] = [];
              if (dayHasExecutor(userAuth.UserAuthType.FACE)) { types.push(userAuth.UserAuthType.FACE); }
              if (dayHasExecutor(userAuth.UserAuthType.FINGERPRINT)) { types.push(userAuth.UserAuthType.FINGERPRINT); }
              if (allowCredential && dayHasExecutor(userAuth.UserAuthType.PIN)) { types.push(userAuth.UserAuthType.PIN); }
              if (types.length === 0) {
                authenticate_native_fail(done, '12500005');
                return;
              }
              const authParam: userAuth.AuthParam = {
                challenge: challenge,
                authType: types,
                authTrustLevel: userAuth.AuthTrustLevel.ATL1,
              };
              const widgetParam: userAuth.WidgetParam = { title: reason };
              let instance: userAuth.UserAuthInstance;
              try {
                instance = userAuth.getUserAuthInstance(authParam, widgetParam);
              } catch (e) {
                authenticate_native_fail(done, `${(e as BusinessError).code}`);
                return;
              }
              instance.on('result', {
                onResult(result: userAuth.UserAuthResult): void {
                  dayInstance = undefined;
                  if (result.result === userAuth.UserAuthResultCode.SUCCESS) {
                    const token: Uint8Array = result.token ?? new Uint8Array(0);
                    const out = new Uint8Array(1 + token.length);
                    out[0] = result.authType ?? 0;
                    out.set(token, 1);
                    authenticate_native_complete(done, out);
                  } else {
                    authenticate_native_fail(done, `${result.result}`);
                  }
                }
              });
              dayInstance = instance;
              try {
                instance.start();
              } catch (e) {
                dayInstance = undefined;
                authenticate_native_fail(done, `${(e as BusinessError).code}`);
              }
            }

            export function cancel_native(): void {
              const instance = dayInstance;
              if (instance !== undefined) {
                try {
                  instance.cancel();
                } catch (e) {
                  console.info(`day-part-biometrics: cancel: ${(e as BusinessError).code}`);
                }
              }
            }
        "#,
    );

    // The fallback every bridge declares. This module is compiled only on the two targets above,
    // so it never runs; it satisfies the rule that a bridge always has an answer for an unclaimed
    // target.
    #[day_bridge::impl(rust, platforms = [other])]
    fn availability_native(_kind: i32) -> Result<i32, day_bridge::Error> {
        Err(day_bridge::Error::Unsupported)
    }

    #[day_bridge::impl(rust, platforms = [other])]
    fn biometry_native() -> Result<i32, day_bridge::Error> {
        Err(day_bridge::Error::Unsupported)
    }

    #[day_bridge::impl(rust, platforms = [other])]
    fn authenticate_native(_challenge: &[u8], _reason: &str, _fallback: &str, _allow_credential: bool, _done: day_bridge::Done<Vec<u8>>) -> Result<(), day_bridge::Error> {
        Err(day_bridge::Error::Unsupported)
    }

    #[day_bridge::impl(rust, platforms = [other])]
    fn cancel_native() -> Result<(), day_bridge::Error> {
        Err(day_bridge::Error::Unsupported)
    }
}

// The generated synchronous entry points are private to this module; the per-OS files reach
// them through these.

/// `availability_native`, see the module note.
pub(crate) fn probe_availability(kind: i32) -> Result<i32, day_bridge::Error> {
    availability_native(kind)
}

/// `biometry_native`: the sensor bitmask.
pub(crate) fn probe_biometry() -> Result<i32, day_bridge::Error> {
    biometry_native()
}

/// `cancel_native`.
pub(crate) fn cancel_prompt() -> Result<(), day_bridge::Error> {
    cancel_native()
}
