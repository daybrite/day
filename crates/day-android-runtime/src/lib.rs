// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Shared Android JVM attachment, class loading, and checked Java bindings.
//! Hosts initialize this runtime before calling services. Widget rendering belongs to day-mdc.

extern crate self as day_android_runtime;

#[cfg(target_os = "android")]
pub use day_macros::java_class;
#[cfg(target_os = "android")]
pub use imp::*;

/// The part↔Java payload convention (docs/extending.md, "The Android bridging contract"):
/// One `byte[]` crosses JNI per call, laid out as
/// `[0..4)` status `i32` BE · `[4..8)` meta-block length `i32` BE · meta `"k\nv\n…"` UTF-8 ·
/// payload bytes. A negative status is a transport-error sentinel and the meta block carries
/// the error message instead of pairs (each part defines its own sentinel values; day-part-http
/// uses −1 timeout … −6 bad-url). Pure bytes, no JNI, so this compiles and tests on every
/// host; `DayEnvelope.java` is the Java twin and the two encode identically.
pub mod envelope {
    /// A decoded (or to-be-encoded) bridge envelope.
    #[derive(Clone, Debug, PartialEq)]
    pub struct Envelope {
        /// Non-negative: the call's status (HTTP status, a handle, …). Negative: an error
        /// sentinel; `meta` is empty and [`Envelope::error_message`] holds the text.
        pub status: i32,
        /// Key/value pairs (response headers, attributes); empty for error envelopes.
        pub meta: Vec<(String, String)>,
        /// The body / result bytes (for errors, the raw message bytes).
        pub payload: Vec<u8>,
    }

    impl Envelope {
        /// Serialize to the wire layout `DayEnvelope.java` produces.
        pub fn encode(&self) -> Vec<u8> {
            let mut meta = String::new();
            for (k, v) in &self.meta {
                meta.push_str(k);
                meta.push('\n');
                meta.push_str(v);
                meta.push('\n');
            }
            let meta = meta.into_bytes();
            let mut out = Vec::with_capacity(8 + meta.len() + self.payload.len());
            out.extend_from_slice(&self.status.to_be_bytes());
            out.extend_from_slice(&(meta.len() as i32).to_be_bytes());
            out.extend_from_slice(&meta);
            out.extend_from_slice(&self.payload);
            out
        }

        /// Parse the wire layout. `Err` is a malformed envelope (truncated), not a sentinel:
        /// sentinel statuses parse fine and are the caller's to interpret.
        pub fn decode(bytes: &[u8]) -> Result<Envelope, &'static str> {
            if bytes.len() < 8 {
                return Err("short envelope");
            }
            let status = i32::from_be_bytes(bytes[0..4].try_into().map_err(|_| "short")?);
            let meta_len =
                i32::from_be_bytes(bytes[4..8].try_into().map_err(|_| "short")?).max(0) as usize;
            let rest = &bytes[8..];
            if rest.len() < meta_len {
                return Err("truncated envelope");
            }
            let (meta_bytes, payload) = rest.split_at(meta_len);
            let mut meta = Vec::new();
            if status >= 0 {
                let mut lines = std::str::from_utf8(meta_bytes).unwrap_or("").split('\n');
                while let (Some(k), Some(v)) = (lines.next(), lines.next()) {
                    if !k.is_empty() {
                        meta.push((k.to_string(), v.to_string()));
                    }
                }
            }
            Ok(Envelope {
                status,
                meta,
                payload: if status < 0 {
                    meta_bytes.to_vec() // the error message rides the meta block
                } else {
                    payload.to_vec()
                },
            })
        }

        /// For a sentinel (negative-status) envelope: the error message text.
        pub fn error_message(&self) -> String {
            String::from_utf8_lossy(&self.payload).into_owned()
        }
    }

    #[cfg(test)]
    mod tests {
        use super::Envelope;

        #[test]
        fn round_trip_with_meta_and_payload() {
            let e = Envelope {
                status: 200,
                meta: vec![
                    ("Content-Type".into(), "text/plain".into()),
                    ("X-Two".into(), "b".into()),
                ],
                payload: b"hello".to_vec(),
            };
            assert_eq!(Envelope::decode(&e.encode()).unwrap(), e);
        }

        #[test]
        fn sentinel_carries_the_message() {
            let raw = Envelope {
                status: -3,
                meta: vec![("boom: handshake".into(), "".into())],
                payload: Vec::new(),
            };
            // Encode as Java's error() does: message in the meta block, no payload.
            let mut bytes = (-3i32).to_be_bytes().to_vec();
            let msg = b"boom: handshake";
            bytes.extend_from_slice(&(msg.len() as i32).to_be_bytes());
            bytes.extend_from_slice(msg);
            let d = Envelope::decode(&bytes).unwrap();
            assert_eq!(d.status, -3);
            assert_eq!(d.error_message(), "boom: handshake");
            let _ = raw;
        }

        #[test]
        fn truncation_is_malformed_not_sentinel() {
            assert!(Envelope::decode(&[0, 0]).is_err());
            let mut bytes = 200i32.to_be_bytes().to_vec();
            bytes.extend_from_slice(&99i32.to_be_bytes()); // claims 99 meta bytes, has none
            assert!(Envelope::decode(&bytes).is_err());
        }
    }
}

#[cfg(target_os = "android")]
mod imp {
    pub use jni;

    crate::java_class! {
        pub DayRuntime => dev.daybrite.day.bridge.DayRuntime,
        methods {
            static fn filesDirPath() -> JString,
            static fn cacheDirPath() -> JString,
        },
        fields {
            static ctx: android.content.Context,
        },
    }

    use jni::objects::{Global, JClass, JObject, JString};
    use jni::{Env, JavaVM};
    use linkme::distributed_slice;
    use std::os::raw::{c_char, c_int, c_void};
    use std::sync::OnceLock;

    /// Cache the JVM and the host's class loader, then validate linked Java bindings.
    /// Call on a Java-owned thread before using services, after setting `DayRuntime.ctx`.
    /// Repeated calls retain the process-wide JVM and class loader.
    pub fn init(env: &mut Env, host_class: &JClass) -> jni::errors::Result<()> {
        let vm = env.get_java_vm()?;
        let loader = host_class.get_class_loader(env)?;
        let loader = env.new_global_ref(loader)?;
        let _ = JAVA_VM.set(vm);
        let _ = APP_CLASS_LOADER.set(loader);
        check_java_classes(env);
        Ok(())
    }

    // liblog is always present in the Android NDK sysroot.
    #[link(name = "log")]
    unsafe extern "C" {
        fn __android_log_write(prio: c_int, tag: *const c_char, text: *const c_char) -> c_int;
    }
    unsafe extern "C" {
        fn pipe(fds: *mut c_int) -> c_int;
        fn dup2(oldfd: c_int, newfd: c_int) -> c_int;
        fn read(fd: c_int, buf: *mut c_void, count: usize) -> isize;
    }

    const ANDROID_LOG_INFO: c_int = 4;
    const ANDROID_LOG_ERROR: c_int = 6;

    /// Route the process's stdout (fd 1) and stderr (fd 2) into logcat under the tag
    /// `Day`; Android sends both to /dev/null otherwise, so `println!`/`eprintln!`
    /// (and Rust panics) would be invisible. stdout logs at `INFO`, stderr at `ERROR`, so
    /// the `Day` CLI can color them apart. Idempotent; safe to call once at startup.
    pub fn redirect_stdio_to_logcat() {
        static DONE: OnceLock<()> = OnceLock::new();
        if DONE.set(()).is_err() {
            return;
        }
        for (target_fd, prio) in [(1, ANDROID_LOG_INFO), (2, ANDROID_LOG_ERROR)] {
            let mut fds = [0 as c_int; 2];
            // SAFETY: standard self-pipe + dup2 redirect; fds live for the process.
            unsafe {
                if pipe(fds.as_mut_ptr()) != 0 || dup2(fds[1], target_fd) < 0 {
                    continue;
                }
            }
            let read_fd = fds[0];
            std::thread::spawn(move || {
                let tag = c"Day";
                let mut buf = [0u8; 2048];
                let mut line: Vec<u8> = Vec::new();
                loop {
                    let n = unsafe { read(read_fd, buf.as_mut_ptr() as *mut c_void, buf.len()) };
                    if n <= 0 {
                        break;
                    }
                    for &b in &buf[..n as usize] {
                        if b == b'\n' {
                            line.push(0);
                            unsafe {
                                __android_log_write(
                                    prio,
                                    tag.as_ptr(),
                                    line.as_ptr() as *const c_char,
                                );
                            }
                            line.clear();
                        } else {
                            line.push(b);
                        }
                    }
                }
            });
        }
    }

    static JAVA_VM: OnceLock<JavaVM> = OnceLock::new();
    /// GlobalRef to the app's ClassLoader (supplied by the host at init), so a
    /// `java_class!` binding resolves app classes from Rust-spawned threads too: their
    /// `FindClass` sees only the system loader, and parts like day-part-http call Java from the
    /// caller's worker thread.
    static APP_CLASS_LOADER: OnceLock<Global<jni::objects::JClassLoader<'static>>> =
        OnceLock::new();

    /// Run `f` with this thread attached to the JVM.
    ///
    /// A Java exception still pending when `f` returns is described to logcat and cleared: a
    /// binding call whose error was dropped must not leave one behind, where it would fail every
    /// JNI call after it and, back in Java, be thrown from whatever called into Rust. Calls that
    /// care about the failure go through [`java`], which clears at the call.
    pub fn with_env<R>(f: impl FnOnce(&mut Env) -> R) -> R {
        let vm = JAVA_VM
            .get()
            .expect("day-android-runtime: init() not called");
        vm.attach_current_thread(|env| {
            let out = f(env);
            if env.exception_check() {
                env.exception_describe();
                env.exception_clear();
            }
            Ok::<R, jni::errors::Error>(out)
        })
        .expect("attach_current_thread")
    }

    /// Run `f` with an attached `Env`, where `?` works on every JNI call inside.
    ///
    /// The scope for calls through a [`java_class!`](crate::java_class) binding. Those return
    /// `jni::errors::Result` and leave a thrown Java exception pending, and almost any JNI call
    /// made with one pending is undefined behavior. An `Err` here is logged, the exception is
    /// described to logcat and cleared, and the caller gets `None`, so a failed call can be
    /// ignored or substituted without the call site remembering the clear.
    pub fn java<R>(f: impl FnOnce(&mut Env) -> jni::errors::Result<R>) -> Option<R> {
        with_env(|env| java_in(env, f))
    }

    /// [`java`] for code that already holds an `Env`: a `with_native` tweak, a native method's
    /// body, a binding's `realize`.
    pub fn java_in<R>(
        env: &mut Env,
        f: impl FnOnce(&mut Env) -> jni::errors::Result<R>,
    ) -> Option<R> {
        match f(env) {
            Ok(v) => Some(v),
            Err(e) => {
                log::warn!("day-android-runtime: Java call failed: {e}");
                if env.exception_check() {
                    env.exception_describe();
                    env.exception_clear();
                }
                None
            }
        }
    }

    /// `obj` as the bound class `T`, for calling `T`'s instance methods on a View Day holds (a
    /// tweak's `with_native` handle, say). The cast is checked (`IsInstanceOf`), and the result
    /// is a local reference of this call's frame, so it never outlives `env`.
    pub fn downcast<'l, T: jni::refs::Reference>(
        env: &mut Env<'l>,
        obj: &JObject,
    ) -> jni::errors::Result<T::Kind<'l>> {
        let local = env.new_local_ref(obj)?;
        env.cast_local::<T>(local)
    }

    pub use linkme;

    /// One Java class bound by [`java_class!`](crate::java_class): its name, and the lookup that
    /// resolves the class, every bound method and field, and registers its native methods.
    pub struct JavaClassCheck {
        pub name: &'static str,
        /// The API level the binding needs (`since = 26`); 0 for every level. Below it the check
        /// skips the binding, and its callers ask [`sdk_int`] before calling.
        pub since: i32,
        pub resolve: fn(&mut Env) -> jni::errors::Result<()>,
    }

    /// The device's API level (`Build.VERSION.SDK_INT`), read once from the system property
    /// behind it, so it needs no JNI and works before `init`. 0 if the property cannot be read.
    pub fn sdk_int() -> i32 {
        static SDK: OnceLock<i32> = OnceLock::new();
        *SDK.get_or_init(|| {
            unsafe extern "C" {
                fn __system_property_get(name: *const c_char, value: *mut c_char) -> c_int;
            }
            // PROP_VALUE_MAX, from <sys/system_properties.h>.
            let mut value = [0u8; 92];
            // SAFETY: the name is a NUL-terminated literal and `value` is PROP_VALUE_MAX bytes,
            // the most the call writes, terminator included.
            let len = unsafe {
                __system_property_get(
                    c"ro.build.version.sdk".as_ptr(),
                    value.as_mut_ptr().cast::<c_char>(),
                )
            };
            let len = usize::try_from(len).unwrap_or(0).min(value.len());
            std::str::from_utf8(&value[..len])
                .ok()
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(0)
        })
    }

    /// Every [`java_class!`](crate::java_class) binding linked into the app.
    #[distributed_slice]
    pub static JAVA_CLASSES: [JavaClassCheck];

    /// Resolve every binding once, at start, so a Java class that is missing from the build or a
    /// method whose signature no longer matches is reported by name when the app launches, rather
    /// than on the page that first calls it.
    ///
    /// In a debug build, and in any scripted run (`DAYSCRIPT_PORT` set), a failure ends the
    /// process after naming every mismatch: a binding that fails takes its whole class with it,
    /// and a walkthrough drives pieces through Day rather than the native widget, so a piece
    /// rendering as a placeholder would otherwise pass. A release build outside a script carries
    /// on, and the calls through that binding fail softly through [`java`].
    fn check_java_classes(env: &mut Env) {
        let started = std::time::Instant::now();
        let mut failed = 0;
        let sdk = sdk_int();
        for class in JAVA_CLASSES {
            if sdk < class.since {
                continue;
            }
            if let Err(e) = (class.resolve)(env) {
                failed += 1;
                if env.exception_check() {
                    env.exception_describe();
                    env.exception_clear();
                }
                // stderr, not `log::`: this runs from `init`, before `crate::start` installs the
                // logger, and stdio already reaches logcat.
                eprintln!(
                    "day-android-runtime: Java binding {} does not match the app's classes: {e}",
                    class.name.replace(' ', "")
                );
            }
        }
        eprintln!(
            "day-android-runtime: resolved {} Java binding(s) in {:.2} ms ({failed} failed)",
            JAVA_CLASSES.len(),
            started.elapsed().as_secs_f64() * 1000.0
        );
        let strict = cfg!(debug_assertions) || std::env::var_os("DAYSCRIPT_PORT").is_some();
        if failed > 0 && strict {
            eprintln!(
                "day-android-runtime: stopping: {failed} Java binding(s) do not match the classes in this \
                 build (above). Rebuild after changing Java, or fix the binding's signature."
            );
            std::process::abort();
        }
    }

    /// The `load_class` hook every [`java_class!`](crate::java_class) binding uses: the app's
    /// class loader, cached at `init`, so a binding first used on a thread Rust started still
    /// finds app classes. That thread's own loader is the system one, which cannot (see `dfind`).
    pub fn load_app_class<'l, T: jni::refs::Reference>(
        env: &mut Env<'l>,
        context: &jni::refs::LoaderContext,
        initialize: bool,
    ) -> jni::errors::Result<JClass<'l>> {
        let Some(loader) = APP_CLASS_LOADER.get() else {
            return context.load_class_for_type::<T>(env, initialize);
        };
        jni::refs::LoaderContext::Loader(loader).load_class_for_type::<T>(env, initialize)
    }

    /// Whether the JVM has been cached, i.e. whether [`with_env`] can run at all.
    ///
    /// Public because a bridged part (docs/bridge.md) may be called before, or entirely outside, a
    /// Day app's `init`: a headless `day-part-*` crate is ordinary Rust that anyone can depend on.
    /// Asking first turns "no runtime" from a panic into `day_bridge::Error::Runtime`.
    pub fn vm_ready() -> bool {
        JAVA_VM.get().is_some()
    }

    /// Read a Java `String` local ref into a Rust `String` (`None` when the ref is null). Public:
    /// the `day` crate's JNI native methods use it to decode incoming string args.
    pub fn read_jstring(env: &Env, s: &JString) -> Option<String> {
        if s.is_null() {
            None
        } else {
            s.mutf8_chars(env).ok().map(|c| c.to_string())
        }
    }

    /// The Java `String` a call returns, read into Rust inside [`java_in`]: `None` when the call
    /// throws or returns null.
    pub fn java_string<F>(env: &mut Env, call: F) -> Option<String>
    where
        F: for<'a, 'l> FnOnce(&'a mut Env<'l>) -> jni::errors::Result<JString<'l>>,
    {
        java_in(env, |env| {
            let s = call(env)?;
            if s.is_null() {
                return Ok(None);
            }
            string_of(env, &s).map(Some)
        })
        .flatten()
    }

    /// A Java `String` as a Rust `String` (MUTF-8 decoded).
    pub fn string_of(env: &Env, s: &JString) -> jni::errors::Result<String> {
        Ok(s.mutf8_chars(env)?.to_string())
    }
}
