//! day-android — the android-widget backend (DESIGN.md §9). jni + the DayBridge Java shim
//! (java/dev/daybrite/day/bridge/ — the Java analogue of the Qt C++ shim; controls are Material 3
//! components from com.google.android.material, M3 Expressive themed). `Handle = AHandle(GlobalRef)`. Coordinates: Day works in dp; `set_frame` scales
//! by density to px and `measure` scales back. The JVM owns the main loop: `Platform::run`
//! hands the pre-registered root straight to `ready` (the Activity already called `init`).

#![allow(clippy::missing_safety_doc)]

#[cfg(target_os = "android")]
pub use imp::*;

/// Parity test for the event-kind wire table: the Java shim's `K_*` constants block in
/// DayBridge.java must mirror `day_spec::bridge::BridgeKind` exactly. Host-runnable — pure
/// text against the enum, no JNI — so a drifted or colliding kind fails `cargo test`
/// anywhere, not just on a device.
#[cfg(test)]
mod bridge_kinds_parity {
    #[test]
    fn java_constants_match_the_rust_enum() {
        use day_spec::bridge::BridgeKind;
        let java = include_str!("../java/dev/daybrite/day/bridge/DayBridge.java");
        let mut found = std::collections::BTreeMap::new();
        for line in java.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("public static final int K_")
                && let Some((name, value)) = rest.split_once(" = ")
            {
                let value: i32 = value
                    .trim_end_matches(';')
                    .parse()
                    .unwrap_or_else(|_| panic!("unparsable K_{name} line: {line}"));
                assert!(
                    found.insert(format!("K_{name}"), value).is_none(),
                    "duplicate Java constant K_{name}"
                );
            }
        }
        let expect = [
            ("K_PRESSED", BridgeKind::Pressed),
            ("K_TEXT_CHANGED", BridgeKind::TextChanged),
            ("K_TOGGLE_CHANGED", BridgeKind::ToggleChanged),
            ("K_VALUE_CHANGED", BridgeKind::ValueChanged),
            ("K_SELECTION_CHANGED", BridgeKind::SelectionChanged),
            ("K_NAV_BACK", BridgeKind::NavBack),
            ("K_FRAME_CHANGED", BridgeKind::FrameChanged),
            ("K_DEEPLINK", BridgeKind::Deeplink),
            ("K_PRESENT_BUTTON", BridgeKind::PresentButton),
            ("K_PRESENT_TEXT", BridgeKind::PresentText),
            ("K_PRESENT_DISMISSED", BridgeKind::PresentDismissed),
            ("K_GESTURE", BridgeKind::Gesture),
            ("K_CUSTOM", BridgeKind::Custom),
            ("K_MENU_ACTION", BridgeKind::MenuAction),
            ("K_LIFECYCLE", BridgeKind::Lifecycle),
            ("K_PRESENT_FILE", BridgeKind::PresentFile),
            ("K_FOCUS_CHANGED", BridgeKind::FocusChanged),
            ("K_SUBMITTED", BridgeKind::Submitted),
            ("K_WINDOW_RESIZED", BridgeKind::WindowResized),
        ];
        assert_eq!(
            found.len(),
            expect.len(),
            "Java K_* count differs from the enum: {found:?}"
        );
        for (name, kind) in expect {
            assert_eq!(
                found.get(name).copied(),
                Some(kind as i32),
                "{name} drifted from BridgeKind::{kind:?}"
            );
        }
    }
}

/// The part↔Java payload convention (docs/extending.md, "The Android bridging contract"):
/// ONE `byte[]` crosses JNI per call, laid out as
/// `[0..4)` status `i32` BE · `[4..8)` meta-block length `i32` BE · meta `"k\nv\n…"` UTF-8 ·
/// payload bytes. A NEGATIVE status is a transport-error sentinel and the meta block carries
/// the error message instead of pairs (each part defines its own sentinel values; day-part-http
/// uses −1 timeout … −6 bad-url). Pure bytes — no JNI — so this compiles and tests on every
/// host; `DayEnvelope.java` is the Java twin and the two encode identically.
pub mod envelope {
    /// A decoded (or to-be-encoded) bridge envelope.
    #[derive(Clone, Debug, PartialEq)]
    pub struct Envelope {
        /// Non-negative: the call's status (HTTP status, a handle, …). Negative: an error
        /// sentinel; `meta` is empty and [`Envelope::error_message`] holds the text.
        pub status: i32,
        /// Key/value pairs (response headers, attributes) — empty for error envelopes.
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

        /// Parse the wire layout. `Err` is a MALFORMED envelope (truncated), not a sentinel —
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
mod picker;
#[cfg(target_os = "android")]
mod textarea;

#[cfg(target_os = "android")]
pub mod ext;
#[cfg(target_os = "android")]
pub use ext::*;

#[cfg(target_os = "android")]
mod imp {
    pub use jni;

    use std::any::Any;
    use std::cell::{Cell, RefCell};
    use std::os::raw::{c_char, c_int, c_void};
    use std::rc::Rc;
    use std::sync::OnceLock;

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
    /// `Day` — Android sends both to /dev/null otherwise, so `println!`/`eprintln!`
    /// (and Rust panics) would be invisible. stdout logs at INFO, stderr at ERROR, so
    /// the `Day` CLI can colour them apart. Idempotent; safe to call once at startup.
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

    use jni::objects::{Global, JClass, JObject, JString, JValue, JValueOwned};
    use jni::signature::{
        FieldSignature, MethodSignature, RuntimeFieldSignature, RuntimeMethodSignature,
    };
    use jni::strings::JNIString;
    use jni::{Env, JavaVM};
    use linkme::distributed_slice;

    /// A shared global reference to a native View. jni 0.22's `Global` is a bare `'static` ref that
    /// is NOT `Clone` (cloning a global ref is a JNI call), so we wrap it in `Arc` — restoring the
    /// `Arc`-backed sharing `GlobalRef` had in 0.21, which `AHandle: Clone` (a day-core `Handle`)
    /// requires. The underlying JNI global ref is released when the last `Arc` owner drops.
    type Gref = std::sync::Arc<Global<JObject<'static>>>;

    /// jni 0.22 compat: `&str`-ergonomic wrappers over the typed name/signature API. In 0.22
    /// `call_*`/`find_class`/`get_static_field` take `AsRef<JNIStr>` names and pre-parsed
    /// `MethodSignature`/`FieldSignature` rather than `&str`; these adapt at runtime so the many
    /// call sites keep passing plain string literals. Public so piece/part crates with their own
    /// Android JNI code share one adapter: `use day_android::DayEnv;`.
    pub trait DayEnv<'l> {
        fn dcall_static(
            &mut self,
            class: &str,
            name: &str,
            sig: &str,
            args: &[JValue],
        ) -> jni::errors::Result<JValueOwned<'l>>;
        fn dcall(
            &mut self,
            obj: &JObject,
            name: &str,
            sig: &str,
            args: &[JValue],
        ) -> jni::errors::Result<JValueOwned<'l>>;
        fn dfield(
            &mut self,
            class: &str,
            name: &str,
            sig: &str,
        ) -> jni::errors::Result<JValueOwned<'l>>;
        fn dfind(&mut self, name: &str) -> jni::errors::Result<JClass<'l>>;
        fn dstr(&self, s: &JString) -> jni::errors::Result<String>;
    }
    impl<'l> DayEnv<'l> for Env<'l> {
        fn dcall_static(
            &mut self,
            class: &str,
            name: &str,
            sig: &str,
            args: &[JValue],
        ) -> jni::errors::Result<JValueOwned<'l>> {
            let sig = sig.parse::<RuntimeMethodSignature>()?;
            // Resolve through dfind, whose app-ClassLoader fallback makes app classes reachable
            // from Rust-spawned threads (a plain name lookup here would use the system loader).
            let cls = self.dfind(class)?;
            self.call_static_method(
                &cls,
                JNIString::from(name),
                MethodSignature::from(&sig),
                args,
            )
        }
        fn dcall(
            &mut self,
            obj: &JObject,
            name: &str,
            sig: &str,
            args: &[JValue],
        ) -> jni::errors::Result<JValueOwned<'l>> {
            let sig = sig.parse::<RuntimeMethodSignature>()?;
            self.call_method(
                obj,
                JNIString::from(name),
                MethodSignature::from(&sig),
                args,
            )
        }
        fn dfield(
            &mut self,
            class: &str,
            name: &str,
            sig: &str,
        ) -> jni::errors::Result<JValueOwned<'l>> {
            let sig = sig.parse::<RuntimeFieldSignature>()?;
            // Same app-ClassLoader routing as dcall_static — see dfind.
            let cls = self.dfind(class)?;
            self.get_static_field(&cls, JNIString::from(name), FieldSignature::from(&sig))
        }
        fn dfind(&mut self, name: &str) -> jni::errors::Result<JClass<'l>> {
            match self.find_class(JNIString::from(name)) {
                Ok(c) => Ok(c),
                Err(e) => {
                    // A Rust-spawned thread attaches with only the SYSTEM class loader, which
                    // cannot see app classes — clear the pending ClassNotFoundException and
                    // retry through the app loader cached at init.
                    self.exception_clear();
                    let Some(loader) = APP_CLASS_LOADER.get() else {
                        return Err(e);
                    };
                    let dotted = name.replace('/', ".");
                    let jname = self.new_string(&dotted)?;
                    let obj = self
                        .dcall(
                            loader,
                            "loadClass",
                            "(Ljava/lang/String;)Ljava/lang/Class;",
                            &[JValue::Object(&jname)],
                        )?
                        .l()?;
                    self.cast_local::<JClass>(obj)
                }
            }
        }
        fn dstr(&self, s: &JString) -> jni::errors::Result<String> {
            Ok(s.mutf8_chars(self)?.to_string())
        }
    }

    use day_spec::bridge;
    use day_spec::props::*;
    use day_spec::{
        A11yProps, AnimSpec, Cap, Curve, DrawOp, Event, EventSink, Font, ListSource, NodeId,
        PieceKind, Platform, Point, Proposal, RawHandle, Rect, Registry, Renderer, Size, Support,
        Toolkit, Transform, WindowOptions, kinds,
    };

    thread_local! {
        /// Recycling list (docs/list.md): row-pull sources keyed by LIST node id (Java passes it
        /// back in nativeListBind), and a stable GlobalRef per physical cell (by identityHashCode)
        /// so day-core's cell map keys consistently across ListView recycling.
        static LIST_SOURCES: std::cell::RefCell<std::collections::HashMap<i64, ListSource>> =
            std::cell::RefCell::new(std::collections::HashMap::new());
        static LIST_NODE: std::cell::RefCell<std::collections::HashMap<usize, i64>> =
            std::cell::RefCell::new(std::collections::HashMap::new());
        static LIST_CELLS: std::cell::RefCell<std::collections::HashMap<i32, Gref>> =
            std::cell::RefCell::new(std::collections::HashMap::new());
    }

    /// Row count, pulled by the Java adapter's getCount (reads the snapshot only; no tree).
    pub fn list_len(host_id: i64) -> usize {
        LIST_SOURCES.with(|m| m.borrow().get(&host_id).map(|s| (s.len)()).unwrap_or(0))
    }

    /// Fill a recycled cell — the Java adapter's getView calls this. A stable GlobalRef per
    /// physical cell (keyed by identityHashCode) gives day-core a consistent cell key.
    pub fn list_bind(env: &mut Env, host_id: i64, position: i32, cell: JObject) {
        let hash = env
            .dcall(&cell, "hashCode", "()I", &[])
            .and_then(|v| v.i())
            .unwrap_or(0);
        let gref = LIST_CELLS.with(|m| {
            m.borrow_mut()
                .entry(hash)
                .or_insert_with(|| {
                    std::sync::Arc::new(env.new_global_ref(&cell).expect("global ref"))
                })
                .clone()
        });
        let raw = gref.as_obj().as_raw() as RawHandle;
        let source = LIST_SOURCES.with(|m| m.borrow().get(&host_id).cloned());
        if let Some(source) = source {
            (source.bind_row)(position as usize, raw);
        }
    }

    pub const BRIDGE: &str = "dev/daybrite/day/bridge/DayBridge";

    #[derive(Clone)]
    pub struct AHandle(pub Gref);

    static JAVA_VM: OnceLock<JavaVM> = OnceLock::new();
    /// GlobalRef to the DayBridge class: FindClass from spawned native threads uses the SYSTEM
    /// class loader and cannot see app classes — cache the class on the main thread at init.
    static BRIDGE_CLASS: OnceLock<Global<JClass<'static>>> = OnceLock::new();
    /// GlobalRef to the app's ClassLoader (taken from the DayBridge class at init), so `dfind`
    /// can resolve app classes from Rust-spawned threads too: their `FindClass` sees only the
    /// system loader, and parts like day-part-http call Java from the caller's worker thread.
    static APP_CLASS_LOADER: OnceLock<Global<JObject<'static>>> = OnceLock::new();

    // --- Bundled data resources via the NDK AAssetManager (§18.3) --------------------------------
    // `resource("name")` reads the APK asset `name` with a zero-copy pointer into the (uncompressed)
    // asset via AAsset_getBuffer — the native AssetManager path the user asked for.
    #[allow(non_camel_case_types)]
    mod aasset {
        use std::os::raw::{c_char, c_int, c_void};
        pub enum AAssetManager {}
        pub enum AAsset {}
        pub const AASSET_MODE_BUFFER: c_int = 3;
        #[link(name = "android")]
        unsafe extern "C" {
            pub fn AAssetManager_fromJava(
                env: *mut jni::sys::JNIEnv,
                mgr: jni::sys::jobject,
            ) -> *mut AAssetManager;
            pub fn AAssetManager_open(
                mgr: *mut AAssetManager,
                filename: *const c_char,
                mode: c_int,
            ) -> *mut AAsset;
            pub fn AAsset_getBuffer(asset: *mut AAsset) -> *const c_void;
            pub fn AAsset_getLength64(asset: *mut AAsset) -> i64;
            pub fn AAsset_close(asset: *mut AAsset);
        }
    }

    /// The app's `AAssetManager` plus a GlobalRef to the Java `AssetManager` that keeps it alive.
    struct AssetMgr {
        aam: *mut aasset::AAssetManager,
        _keepalive: Global<JObject<'static>>,
    }
    // The AAssetManager pointer is valid for the app lifetime; resource() runs on the main thread.
    unsafe impl Send for AssetMgr {}
    unsafe impl Sync for AssetMgr {}
    static ASSET_MGR: OnceLock<AssetMgr> = OnceLock::new();

    /// Capture the `AAssetManager` from `DayBridge.ctx.getAssets()` and register the opener (init).
    fn register_resource_opener(env: &mut Env) {
        let Ok(ctx) = env
            .dfield(BRIDGE, "ctx", "Landroid/content/Context;")
            .and_then(|f| f.l())
        else {
            return;
        };
        let Ok(am) = env
            .dcall(
                &ctx,
                "getAssets",
                "()Landroid/content/res/AssetManager;",
                &[],
            )
            .and_then(|r| r.l())
        else {
            return;
        };
        let Ok(keepalive) = env.new_global_ref(&am) else {
            return;
        };
        let aam = unsafe { aasset::AAssetManager_fromJava(env.get_raw(), am.as_raw()) };
        if aam.is_null() {
            return;
        }
        let _ = ASSET_MGR.set(AssetMgr {
            aam,
            _keepalive: keepalive,
        });
        day_spec::resource::set_resource_opener(open_resource);
    }

    /// Opener: `resource("name")` -> the APK asset `name`, zero-copy from `AAsset_getBuffer`.
    fn open_resource(name: &str) -> Option<day_spec::resource::Resource> {
        let mgr = ASSET_MGR.get()?.aam;
        let cname = std::ffi::CString::new(name).ok()?;
        let asset =
            unsafe { aasset::AAssetManager_open(mgr, cname.as_ptr(), aasset::AASSET_MODE_BUFFER) };
        if asset.is_null() {
            return None;
        }
        let len = unsafe { aasset::AAsset_getLength64(asset) };
        let ptr = unsafe { aasset::AAsset_getBuffer(asset) } as *const u8;
        if ptr.is_null() || len < 0 {
            unsafe { aasset::AAsset_close(asset) };
            return None;
        }
        struct AssetGuard(*mut aasset::AAsset);
        impl Drop for AssetGuard {
            fn drop(&mut self) {
                unsafe { aasset::AAsset_close(self.0) };
            }
        }
        // Safety: `ptr`/`len` are the asset's buffer, valid until AAsset_close (held by the guard).
        Some(unsafe {
            day_spec::resource::Resource::from_raw(ptr, len as usize, Box::new(AssetGuard(asset)))
        })
    }

    /// The day-core event sink (node-id keyed).
    type Sink = Rc<dyn Fn(NodeId, Event)>;

    thread_local! {
        static SINK: RefCell<Option<Sink>> = const { RefCell::new(None) };
        static DENSITY: Cell<f64> = const { Cell::new(1.0) };
        static ROOT: RefCell<Option<(AHandle, Size)>> = const { RefCell::new(None) };
    }

    pub fn emit(id: NodeId, ev: Event) {
        let sink = SINK.with(|s| s.borrow().clone());
        if let Some(sink) = sink {
            sink(id, ev);
        }
    }

    fn density() -> f64 {
        DENSITY.with(|d| d.get())
    }

    /// Run with an attached `Env` (public: external renderers use this too). jni 0.22's
    /// `attach_current_thread` is callback-scoped; the callback returns `Ok` so the outer
    /// `Result` just unwraps.
    pub fn with_env<R>(f: impl FnOnce(&mut Env) -> R) -> R {
        let vm = JAVA_VM.get().expect("day-android: init() not called");
        vm.attach_current_thread(|env| Ok::<R, jni::errors::Error>(f(env)))
            .expect("attach_current_thread")
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

    /// View a `java.lang.String` object as a `JString`. String return values arrive as a
    /// `JObject` from `JValueOwned::l()`; casting is safe — `JString` is a transparent wrapper over
    /// the same `jobject`. Public: piece/part crates reading Java strings use it.
    pub fn as_jstring<'a>(obj: JObject<'a>) -> JString<'a> {
        // Safety: same repr (a jobject); caller guarantees the object is a java.lang.String.
        unsafe { std::mem::transmute(obj) }
    }

    /// Call a DayBridge static returning a View, as a shared global ref (public helper).
    pub fn make_view(env: &mut Env, method: &str, sig: &str, args: &[JValue]) -> Gref {
        let obj = env
            .dcall_static(BRIDGE, method, sig, args)
            .expect("DayBridge call")
            .l()
            .expect("View");
        std::sync::Arc::new(env.new_global_ref(obj).expect("global ref"))
    }

    fn call_void(method: &str, sig: &str, args: &[JValue]) {
        with_env(|env| {
            let _ = env.dcall_static(BRIDGE, method, sig, args);
        });
    }

    /// Lower an `AnimSpec` to `(duration_ms, curve_code)` for `DayBridge` (§8.4). `None` ⇒ `(0, 0)`,
    /// which `DayBridge` applies instantly. Curve codes: 0 linear, 1 easeIn, 2 easeOut, 3 easeInOut,
    /// 4 spring (Android runs it via `ViewPropertyAnimator` with a matching interpolator).
    fn anim_args(anim: Option<&AnimSpec>) -> (i32, i32) {
        match anim {
            None => (0, 0),
            Some(a) => (
                a.duration_ms as i32,
                match a.curve {
                    Curve::Linear => 0,
                    Curve::EaseIn => 1,
                    Curve::EaseOut => 2,
                    Curve::EaseInOut => 3,
                    Curve::Spring { .. } => 4,
                },
            ),
        }
    }

    /// Apply a `background`/`corner_radius` surface: a rounded `GradientDrawable` background +
    /// `clipToOutline`. The radius is density-scaled here (Java takes px). Idempotent — used at
    /// realize and on a reactive background patch.
    fn apply_surface(h: &AHandle, bg: Option<day_spec::Color>, corner_radius: f64, clips: bool) {
        let d = DENSITY.with(|x| x.get());
        call_void(
            "setSurface",
            "(Landroid/view/View;IZFZ)V",
            &[
                JValue::Object(h.0.as_obj()),
                JValue::Int(bg.map(argb_i32).unwrap_or(0)),
                JValue::Bool(bg.is_some()),
                JValue::Float((corner_radius * d) as f32),
                JValue::Bool(clips),
            ],
        );
    }

    fn measure_call(h: &AHandle, method: &str) -> f64 {
        with_env(|env| {
            env.dcall_static(
                BRIDGE,
                method,
                "(Landroid/view/View;)I",
                &[JValue::Object(h.0.as_obj())],
            )
            .expect("measure")
            .i()
            .unwrap_or(0) as f64
        })
    }

    /// Initialize globals from the Activity's nativeStart (called by `day::android_start`).
    pub fn init(env: &mut Env, root: JObject, density_: f32, w: i32, h: i32) {
        if let Ok(vm) = env.get_java_vm() {
            let _ = JAVA_VM.set(vm);
        }
        if let Ok(cls) = env.dfind(BRIDGE) {
            // Any app class's getClassLoader() yields the loader that can see ALL app classes;
            // cache it here on the main thread, where FindClass still resolves app classes.
            if let Ok(loader) = env
                .dcall(&cls, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])
                .and_then(|v| v.l())
                && let Ok(g) = env.new_global_ref(loader)
            {
                let _ = APP_CLASS_LOADER.set(g);
            }
            if let Ok(global) = env.new_global_ref(cls) {
                let _ = BRIDGE_CLASS.set(global);
            }
        }
        register_resource_opener(env);
        let d = density_ as f64;
        DENSITY.with(|x| x.set(d));
        let handle = AHandle(std::sync::Arc::new(
            env.new_global_ref(root).expect("root global ref"),
        ));
        let size = Size::new(w as f64 / d, h as f64 / d);
        ROOT.with(|r| *r.borrow_mut() = Some((handle, size)));
        // Android's OS temp dir isn't app-writable; use the app cache dir for the file-save staging
        // area (docs/files.md) so `save_file(..)` can write its temp before handing off to SAF.
        if let Ok(dir) = env
            .dcall_static(BRIDGE, "cacheDirPath", "()Ljava/lang/String;", &[])
            .and_then(|v| v.l())
        {
            // cacheDirPath returns a java.lang.String; view the object as a JString to read it.
            let jstr: JString = unsafe { std::mem::transmute(dir) };
            if let Ok(path) = env.dstr(&jstr)
                && !path.is_empty()
            {
                day_spec::present::set_app_temp_dir(path);
            }
        }
    }

    // The wire table (day_spec::bridge) as const match patterns — the Java side's K_* constants
    // mirror these, and day-android's parity test holds the two files together.
    const K_PRESSED: i32 = bridge::BridgeKind::Pressed as i32;
    const K_TEXT_CHANGED: i32 = bridge::BridgeKind::TextChanged as i32;
    const K_TOGGLE_CHANGED: i32 = bridge::BridgeKind::ToggleChanged as i32;
    const K_VALUE_CHANGED: i32 = bridge::BridgeKind::ValueChanged as i32;
    const K_SELECTION_CHANGED: i32 = bridge::BridgeKind::SelectionChanged as i32;
    const K_NAV_BACK: i32 = bridge::BridgeKind::NavBack as i32;
    const K_FRAME_CHANGED: i32 = bridge::BridgeKind::FrameChanged as i32;
    const K_DEEPLINK: i32 = bridge::BridgeKind::Deeplink as i32;
    const K_PRESENT_BUTTON: i32 = bridge::BridgeKind::PresentButton as i32;
    const K_PRESENT_TEXT: i32 = bridge::BridgeKind::PresentText as i32;
    const K_PRESENT_DISMISSED: i32 = bridge::BridgeKind::PresentDismissed as i32;
    const K_GESTURE: i32 = bridge::BridgeKind::Gesture as i32;
    const K_CUSTOM: i32 = bridge::BridgeKind::Custom as i32;
    const K_MENU_ACTION: i32 = bridge::BridgeKind::MenuAction as i32;
    const K_LIFECYCLE: i32 = bridge::BridgeKind::Lifecycle as i32;
    const K_PRESENT_FILE: i32 = bridge::BridgeKind::PresentFile as i32;
    const K_FOCUS_CHANGED: i32 = bridge::BridgeKind::FocusChanged as i32;
    const K_SUBMITTED: i32 = bridge::BridgeKind::Submitted as i32;
    const K_WINDOW_RESIZED: i32 = bridge::BridgeKind::WindowResized as i32;

    /// The single native trampoline (the app's `nativeOnEvent` forwards here). The kind
    /// numbers are `day_spec::bridge::BridgeKind` — the shared wire table.
    pub fn dispatch_event(env: &mut Env, id: i64, kind: i32, num: f64, jstr: &JString) {
        let ev = match kind {
            K_PRESSED => Event::Pressed,
            K_TEXT_CHANGED => {
                let text = env.dstr(jstr).ok().unwrap_or_default();
                Event::TextChanged(text)
            }
            K_TOGGLE_CHANGED => Event::ToggleChanged(num != 0.0),
            K_VALUE_CHANGED => Event::ValueChanged(num),
            K_SELECTION_CHANGED => Event::SelectionChanged(num as i64),
            // Navigation (docs/navigation.md): system back / gesture / toolbar up. num == 1.0
            // means the native FragmentManager already popped (predictive back commit, back
            // button, up arrow) — Rust updates the path without re-issuing the pop.
            K_NAV_BACK => Event::NavBack {
                already_popped: num != 0.0,
            },
            // Nav page size report, "w,h" in px.
            K_FRAME_CHANGED => {
                let text: String = env.dstr(jstr).ok().unwrap_or_default();
                let Some((w, h)) = text.split_once(',') else {
                    return;
                };
                let d = DENSITY.with(|x| x.get());
                let (Ok(w), Ok(h)) = (w.parse::<f64>(), h.parse::<f64>()) else {
                    return;
                };
                Event::FrameChanged(Size::new(w / d, h / d))
            }
            // Warm deep link: the nav piece handles Custom("deeplink").
            K_DEEPLINK => {
                let route: String = env.dstr(jstr).ok().unwrap_or_default();
                Event::custom("deeplink", route)
            }
            // Presentation answers (docs/dialogs.md): id == request id.
            K_PRESENT_BUTTON => Event::PresentResult {
                req: id as u64,
                result: day_spec::present::PresentResult::Button(num as i64),
            },
            K_PRESENT_TEXT => {
                let text: String = env.dstr(jstr).ok().unwrap_or_default();
                Event::PresentResult {
                    req: id as u64,
                    result: day_spec::present::PresentResult::Text(text),
                }
            }
            K_PRESENT_DISMISSED => Event::PresentResult {
                req: id as u64,
                result: day_spec::present::PresentResult::Dismissed,
            },
            // File-picker answer (docs/files.md): string = chosen locators (a cache path for open,
            // a content:// URI for save), joined by the unit separator. Reuse the `decode` tag 3.
            K_PRESENT_FILE => {
                let text: String = env.dstr(jstr).ok().unwrap_or_default();
                Event::PresentResult {
                    req: id as u64,
                    result: day_spec::present::PresentResult::decode(3, 0, text),
                }
            }
            // Gestures (docs/shapes.md): num = phase (0=tap 1=began 2=changed 3=ended),
            // string = "x,y,tx,ty" in px. Convert to dp like FrameChanged does.
            K_GESTURE => {
                let text: String = env.dstr(jstr).ok().unwrap_or_default();
                let p: Vec<f64> = text.split(',').filter_map(|s| s.parse().ok()).collect();
                if p.len() < 4 {
                    return;
                }
                let d = DENSITY.with(|x| x.get());
                let at = Point::new(p[0] / d, p[1] / d);
                let tr = Point::new(p[2] / d, p[3] / d);
                match num as i32 {
                    0 => Event::Tap(at),
                    1 => Event::Drag {
                        phase: day_spec::DragPhase::Began,
                        location: at,
                        translation: Point::ZERO,
                    },
                    3 => Event::Drag {
                        phase: day_spec::DragPhase::Ended,
                        location: at,
                        translation: tr,
                    },
                    _ => Event::Drag {
                        phase: day_spec::DragPhase::Changed,
                        location: at,
                        translation: tr,
                    },
                }
            }
            // Piece-defined custom event (§8.2's open event channel): a `&'static str` tag can't cross
            // JNI, so the tag is empty and the piece reads the primitive `num`/`text` payload. A piece
            // (e.g. day-piece-webview) calls `DayBridge.nativeOnEvent(id, 12, num, text)`.
            K_CUSTOM => {
                let text: String = env.dstr(jstr).ok().unwrap_or_default();
                Event::Custom { tag: "", num, text }
            }
            // Menu selection (docs/menus.md): `id` == the chosen action's dispatch id (0 for a
            // role/standard item, which dispatches to nothing). Routed by the pump to the closure.
            K_MENU_ACTION => Event::MenuAction(id as u64),
            // Activity lifecycle (docs/lifecycle.md): `num` is the phase code (day_spec::Lifecycle
            // order). DayActivity forwards onResume/onPause/onStart/onStop/onTrimMemory/onDestroy.
            K_LIFECYCLE => match android_lifecycle(num as i32) {
                Some(phase) => Event::Lifecycle(phase),
                None => return,
            },
            // Root size change (px as "w,h" text): the safe-area root grew or shrank — a late
            // inset pass, the soft keyboard, rotation, or a system-bar change. Routed to the
            // root as a window resize so Day relayouts; same rail as appkit's windowDidResize.
            // (18: the first free kind — 15 already carries file-picker answers.)
            K_WINDOW_RESIZED => {
                let text: String = env.dstr(jstr).ok().unwrap_or_default();
                let p: Vec<f64> = text.split(',').filter_map(|s| s.parse().ok()).collect();
                if p.len() < 2 {
                    return;
                }
                let d = DENSITY.with(|x| x.get());
                emit(
                    day_spec::WINDOW_NODE,
                    Event::WindowResized(Size::new(p[0] / d, p[1] / d)),
                );
                return;
            }
            // Focus pair + IME submit action (docs/focus.md).
            K_FOCUS_CHANGED => Event::FocusChanged(num != 0.0),
            K_SUBMITTED => Event::Submitted,
            unknown => {
                // A silently dropped kind is how the kind-15 collision hid for weeks: say so
                // once per kind in debug builds (release stays quiet — this is a dev signal).
                #[cfg(debug_assertions)]
                {
                    use std::sync::{Mutex, OnceLock};
                    static SEEN: OnceLock<Mutex<std::collections::BTreeSet<i32>>> = OnceLock::new();
                    let seen = SEEN.get_or_init(|| Mutex::new(std::collections::BTreeSet::new()));
                    if let Ok(mut g) = seen.lock()
                        && g.insert(unknown)
                    {
                        eprintln!("day-android: dropping unknown event kind {unknown}");
                    }
                }
                let _ = unknown;
                return;
            }
        };
        emit(NodeId(id as u64), ev);
    }

    /// Posted-closure trampoline (the app's `nativeRunPosted` forwards here).
    pub fn run_posted(token: i64) {
        let f: Box<Box<dyn FnOnce() + Send>> =
            unsafe { Box::from_raw(token as *mut Box<dyn FnOnce() + Send>) };
        f();
    }

    /// Frame-callback trampoline (the app's `nativeDoFrame` forwards here). `frame_nanos` is
    /// `Choreographer`'s frame time in nanoseconds; day-core wants seconds. Runs on the UI thread.
    pub fn run_frame(token: i64, frame_nanos: i64) {
        let f: Box<Box<dyn FnOnce(f64)>> =
            unsafe { Box::from_raw(token as *mut Box<dyn FnOnce(f64)>) };
        f(frame_nanos as f64 / 1_000_000_000.0);
    }

    #[distributed_slice]
    pub static RENDERERS: [fn() -> Renderer<Android>];

    pub struct Android {
        registry: Registry<Android>,
    }

    impl Android {
        pub fn new() -> Self {
            let mut registry = Registry::default();
            for f in RENDERERS {
                registry.register(f());
            }
            Android { registry }
        }
    }

    impl Default for Android {
        fn default() -> Self {
            Self::new()
        }
    }

    fn jstr(env: &mut Env, s: &str) -> jni::objects::JString<'static> {
        // SAFETY: local ref used immediately within the same JNI frame.
        unsafe { std::mem::transmute(env.new_string(s).expect("new_string")) }
    }

    /// Map an Android lifecycle phase code (day_spec::Lifecycle order) to the enum (docs/lifecycle.md).
    fn android_lifecycle(code: i32) -> Option<day_spec::Lifecycle> {
        use day_spec::Lifecycle::*;
        Some(match code {
            2 => DidBecomeActive,
            3 => WillResignActive,
            4 => WillEnterForeground,
            5 => DidEnterBackground,
            6 => DidReceiveMemoryWarning,
            7 => WillTerminate,
            _ => return None,
        })
    }

    /// Mobile backends deliver the FULL lifecycle (docs/lifecycle.md). `const` for
    /// `day::require_lifecycle!` compile-time guards.
    pub const fn lifecycle_supported(_phase: day_spec::Lifecycle) -> bool {
        true
    }

    /// Default label for a standard role left unlabeled by the app. (Android's own text-selection
    /// toolbar handles the actual Cut/Copy/Paste on editable views; a role in a day menu is shown
    /// for parity and dispatches nothing — see docs/menus.md.)
    fn android_role_label(role: day_spec::MenuRole) -> &'static str {
        use day_spec::MenuRole::*;
        match role {
            Cut => "Cut",
            Copy => "Copy",
            Paste => "Paste",
            SelectAll => "Select All",
            Undo => "Undo",
            Redo => "Redo",
            Delete => "Delete",
            About => "About",
            Quit => "Quit",
            Preferences => "Settings",
            Minimize => "Minimize",
            CloseWindow => "Close",
            Fullscreen => "Full Screen",
        }
    }

    /// Flatten the day-neutral menu tree to the line format `DayBridge.buildMenu` parses:
    /// `kind \t id \t enabled \t label` per line, where kind ∈ {A action, S submenu-open,
    /// E submenu-close, `-` separator}. Roles become plain actions with id 0.
    fn serialize_menu(items: &[day_spec::MenuItem], out: &mut String) {
        fn clean(s: &str) -> String {
            s.replace(['\t', '\n'], " ")
        }
        for item in items {
            match item {
                day_spec::MenuItem::Separator => out.push_str("-\t0\t1\t\n"),
                day_spec::MenuItem::Submenu { label, items } => {
                    out.push_str(&format!("S\t0\t1\t{}\n", clean(label)));
                    serialize_menu(items, out);
                    out.push_str("E\t0\t1\t\n");
                }
                day_spec::MenuItem::Action {
                    id,
                    label,
                    shortcut: _,
                    enabled,
                    role,
                } => {
                    let text = match role {
                        Some(r) if label.is_empty() => android_role_label(*r).to_string(),
                        _ => label.clone(),
                    };
                    out.push_str(&format!(
                        "A\t{}\t{}\t{}\n",
                        id,
                        *enabled as i32,
                        clean(&text)
                    ));
                }
            }
        }
    }

    /// Size (in **sp** — scales with Settings ▸ Display ▸ Font size, the Android accessibility text
    /// scale) + the style's inherent weight for a logical [`Font`]. Mobile scale, aligned with iOS.
    fn font_style(f: Font) -> (f32, day_spec::FontWeight) {
        use day_spec::FontWeight::*;
        match f {
            Font::LargeTitle => (34.0, Regular),
            Font::Title => (28.0, Regular),
            Font::Title2 => (22.0, Regular),
            Font::Title3 => (20.0, Regular),
            Font::Headline => (17.0, Semibold),
            Font::Subheadline => (15.0, Regular),
            Font::Body => (17.0, Regular),
            Font::Callout => (16.0, Regular),
            Font::Footnote => (13.0, Regular),
            Font::Caption => (12.0, Regular),
            Font::Caption2 => (11.0, Regular),
            Font::System(pt) => (pt as f32, Regular),
            Font::Custom(_, pt) => (pt as f32, Regular),
        }
    }

    /// The bundled family name when the spec is `Font::Custom` (§18.4) — passed to Java as the
    /// nullable `family` argument of `DayBridge.setLabelFont`, which resolves it to the
    /// `res/font/` resource `day build` staged from the project's `fonts/` directory.
    fn custom_family(spec: day_spec::FontSpec) -> Option<&'static str> {
        match spec.style {
            Font::Custom(name, _) => Some(name),
            _ => None,
        }
    }

    /// Day weight → Android font weight (Thin=100 … Black=900, for `Typeface.create(_, weight, _)`).
    fn android_weight(w: day_spec::FontWeight) -> i32 {
        use day_spec::FontWeight as W;
        match w {
            W::Thin => 100,
            W::UltraLight => 200,
            W::Light => 300,
            W::Regular => 400,
            W::Medium => 500,
            W::Semibold => 600,
            W::Bold => 700,
            W::Heavy => 800,
            W::Black => 900,
        }
    }

    /// (sp size, Android weight, italic) for `DayBridge.setLabelFont`.
    fn font_params(spec: day_spec::FontSpec) -> (f32, i32, bool) {
        let (sp, inherent) = font_style(spec.style);
        let weight = android_weight(spec.weight.unwrap_or(inherent));
        (sp, weight, spec.italic)
    }

    /// Day `Color` (0–1 floats) → a packed `0xAARRGGBB` int for `android.graphics.Color`.
    fn argb_i32(c: day_spec::Color) -> i32 {
        let ch = |x: f64| (x.clamp(0.0, 1.0) * 255.0).round() as u32;
        ((ch(c.a) << 24) | (ch(c.r) << 16) | (ch(c.g) << 8) | ch(c.b)) as i32
    }

    /// Warn ONCE per kind that this backend has no registered renderer for `kind`, before falling
    /// back to a visible placeholder. A missing renderer usually means the piece's `widget` feature
    /// wasn't enabled (Tier A.2 derives it automatically under `day build`). The message goes to both
    /// stderr (which `redirect_stdio_to_logcat` routes to logcat) and directly to logcat at ERROR, so
    /// it surfaces even before the redirect installs. Deduped per kind so it doesn't spam the log.
    fn warn_missing_renderer(kind: PieceKind) {
        static SEEN: std::sync::Mutex<Option<std::collections::HashSet<&'static str>>> =
            std::sync::Mutex::new(None);
        let Ok(mut guard) = SEEN.lock() else { return };
        if guard
            .get_or_insert_with(std::collections::HashSet::new)
            .insert(kind)
        {
            let msg = format!(
                "day: no renderer for piece kind \"{kind}\" on widget (android) \
                 — is the piece's widget feature enabled? (rendering a placeholder)"
            );
            eprintln!("{msg}");
            if let Ok(c) = std::ffi::CString::new(msg) {
                // SAFETY: liblog is linked (see the extern block above); `Day` + the message are
                // valid NUL-terminated C strings for the duration of the call.
                unsafe { __android_log_write(ANDROID_LOG_ERROR, c"Day".as_ptr(), c.as_ptr()) };
            }
        }
    }

    impl Toolkit for Android {
        type Handle = AHandle;

        fn capability(&self, cap: Cap) -> Support {
            match cap {
                Cap::Dialogs | Cap::FileDialogs | Cap::Animation | Cap::Cover => Support::Native,
                _ => Support::Unsupported,
            }
        }

        fn defer_system_gestures(&mut self, edges: day_spec::Edges) {
            // Any non-empty request enters swipe-to-reveal immersive mode (docs/cover.md).
            call_void(
                "setDeferSystemGestures",
                "(Z)V",
                &[JValue::Bool(!edges.is_empty())],
            );
        }

        fn present(&mut self, req: u64, spec: &day_spec::present::PresentSpec) {
            use day_spec::present::PresentSpec;
            let reqj = req as i64;
            match spec {
                PresentSpec::Dialog { sheet, .. } => with_env(|env| {
                    let title = jstr(env, spec.title());
                    let message = jstr(env, spec.message().unwrap_or(""));
                    let buttons = jstr(env, &spec.buttons_joined());
                    let roles = jstr(env, &spec.roles_joined());
                    let _ = env.dcall_static(
                        BRIDGE,
                        "present",
                        "(JZLjava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)V",
                        &[
                            JValue::Long(reqj),
                            JValue::Bool(*sheet),
                            JValue::Object(&title),
                            JValue::Object(&message),
                            JValue::Object(&buttons),
                            JValue::Object(&roles),
                        ],
                    );
                }),
                PresentSpec::Prompt {
                    placeholder,
                    initial,
                    ok,
                    cancel,
                    ..
                } => with_env(|env| {
                    let title = jstr(env, spec.title());
                    let message = jstr(env, spec.message().unwrap_or(""));
                    let ph = jstr(env, placeholder);
                    let init = jstr(env, initial);
                    let okj = jstr(env, ok);
                    let cancelj = jstr(env, cancel);
                    let _ = env.dcall_static(
                        BRIDGE,
                        "presentPrompt",
                        "(JLjava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)V",
                        &[
                            JValue::Long(reqj),
                            JValue::Object(&title),
                            JValue::Object(&message),
                            JValue::Object(&ph),
                            JValue::Object(&init),
                            JValue::Object(&okj),
                            JValue::Object(&cancelj),
                        ],
                    );
                }),
                // Storage Access Framework (docs/files.md). Java launches ACTION_OPEN_DOCUMENT /
                // ACTION_CREATE_DOCUMENT and, on result, copies through the ContentResolver: open →
                // an app cache file (readable path); save → the chosen content:// URI.
                PresentSpec::OpenFile { .. } => with_env(|env| {
                    let title = jstr(env, spec.title());
                    let filters = jstr(env, &spec.filters_joined());
                    let _ = env.dcall_static(
                        BRIDGE,
                        "presentFileOpen",
                        "(JLjava/lang/String;Ljava/lang/String;)V",
                        &[
                            JValue::Long(reqj),
                            JValue::Object(&title),
                            JValue::Object(&filters),
                        ],
                    );
                }),
                PresentSpec::SaveFile {
                    suggested_name,
                    src_path,
                    ..
                } => with_env(|env| {
                    let title = jstr(env, spec.title());
                    let name = jstr(env, suggested_name);
                    let src = jstr(env, src_path);
                    let filters = jstr(env, &spec.filters_joined());
                    let _ = env.dcall_static(
                        BRIDGE,
                        "presentFileSave",
                        "(JLjava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)V",
                        &[
                            JValue::Long(reqj),
                            JValue::Object(&title),
                            JValue::Object(&name),
                            JValue::Object(&src),
                            JValue::Object(&filters),
                        ],
                    );
                }),
            }
        }

        fn dismiss(&mut self, req: u64) {
            call_void("dismissPresent", "(J)V", &[JValue::Long(req as i64)]);
        }

        fn open_url(&mut self, url: &str) {
            with_env(|env| {
                let u = jstr(env, url);
                let _ = env.dcall_static(
                    BRIDGE,
                    "openUrl",
                    "(Ljava/lang/String;)V",
                    &[JValue::Object(&u)],
                );
            });
        }

        fn realize(&mut self, kind: PieceKind, props: &dyn Any, id: NodeId) -> AHandle {
            let idj = id.0 as i64;
            match kind {
                kinds::CONTAINER => {
                    let h = with_env(|env| {
                        AHandle(make_view(
                            env,
                            "makeContainer",
                            "()Landroid/view/View;",
                            &[],
                        ))
                    });
                    if let Some(p) = props.downcast_ref::<ContainerProps>() {
                        if p.role == Some(day_spec::SurfaceRole::SectionCard) {
                            let d = DENSITY.with(|x| x.get());
                            call_void(
                                "setSectionCard",
                                "(Landroid/view/View;F)V",
                                &[
                                    JValue::Object(h.0.as_obj()),
                                    JValue::Float((p.corner_radius * d) as f32),
                                ],
                            );
                        } else if p.background.is_some() || p.corner_radius > 0.0 || p.clips {
                            apply_surface(&h, p.background, p.corner_radius, p.clips);
                        }
                    }
                    h
                }
                kinds::SCROLL => {
                    let horizontal = props
                        .downcast_ref::<day_spec::props::ScrollProps>()
                        .map(|p| p.horizontal)
                        .unwrap_or(false);
                    with_env(|env| {
                        AHandle(make_view(
                            env,
                            "makeScroll",
                            "(Z)Landroid/view/View;",
                            &[JValue::Bool(horizontal)],
                        ))
                    })
                }
                kinds::LIST => {
                    let p = props.downcast_ref::<ListProps>().unwrap();
                    let d = DENSITY.with(|x| x.get());
                    let rowh = match p.row_height {
                        RowHeight::Uniform(h) => h,
                        RowHeight::Automatic => 44.0,
                    };
                    let handle = with_env(|env| {
                        AHandle(make_view(
                            env,
                            "makeList",
                            "(JIZ)Landroid/view/View;",
                            &[
                                JValue::Long(id.0 as i64),
                                JValue::Int((rowh * d).round() as i32),
                                JValue::Bool(p.selectable),
                            ],
                        ))
                    });
                    LIST_NODE.with(|m| {
                        m.borrow_mut()
                            .insert(handle.0.as_obj().as_raw() as usize, id.0 as i64)
                    });
                    handle
                }
                kinds::NAV => {
                    let p = props.downcast_ref::<NavProps>().unwrap();
                    with_env(|env| {
                        let s = jstr(env, &p.title);
                        AHandle(make_view(
                            env,
                            "makeNavHost",
                            "(JLjava/lang/String;)Landroid/view/View;",
                            &[JValue::Long(idj), JValue::Object(&s)],
                        ))
                    })
                }
                kinds::NAV_PAGE => with_env(|env| {
                    AHandle(make_view(
                        env,
                        "makeNavPage",
                        "(J)Landroid/view/View;",
                        &[JValue::Long(idj)],
                    ))
                }),
                // Fullscreen cover (docs/cover.md): a DayCover shell whose content pane is the
                // Day mount point; CoverPatch::Present re-homes it over the activity content.
                kinds::COVER => with_env(|env| {
                    AHandle(make_view(
                        env,
                        "makeCover",
                        "(J)Landroid/view/View;",
                        &[JValue::Long(idj)],
                    ))
                }),
                kinds::TABS => {
                    let p = props.downcast_ref::<TabsProps>().unwrap();
                    with_env(|env| {
                        AHandle(make_view(
                            env,
                            "makeTabs",
                            "(JI)Landroid/view/View;",
                            &[JValue::Long(idj), JValue::Int(p.selected as i32)],
                        ))
                    })
                }
                kinds::TABS_PAGE => {
                    let p = props.downcast_ref::<TabsPageProps>().unwrap();
                    with_env(|env| {
                        let title = jstr(env, &p.title);
                        // The tab's bundled-image NAME (empty = none); Java looks it up in res/drawable.
                        let icon = jstr(env, p.icon.as_deref().unwrap_or(""));
                        AHandle(make_view(
                            env,
                            "makeTabPage",
                            "(JLjava/lang/String;Ljava/lang/String;)Landroid/view/View;",
                            &[
                                JValue::Long(idj),
                                JValue::Object(&title),
                                JValue::Object(&icon),
                            ],
                        ))
                    })
                }
                kinds::NAV_MENU => {
                    let p = props.downcast_ref::<NavMenuProps>().unwrap();
                    let joined = p.items.join("\u{1f}");
                    // Parallel, index-aligned icon NAMES ("" = no icon for that row).
                    let joined_icons = p
                        .icons
                        .iter()
                        .map(|o| o.clone().unwrap_or_default())
                        .collect::<Vec<_>>()
                        .join("\u{1f}");
                    with_env(|env| {
                        let s = jstr(env, &joined);
                        let si = jstr(env, &joined_icons);
                        AHandle(make_view(
                            env,
                            "makeNavMenu",
                            "(JLjava/lang/String;Ljava/lang/String;)Landroid/view/View;",
                            &[JValue::Long(idj), JValue::Object(&s), JValue::Object(&si)],
                        ))
                    })
                }
                kinds::LABEL => {
                    let p = props.downcast_ref::<LabelProps>().unwrap();
                    let (sp, weight, italic) = font_params(p.font);
                    with_env(|env| {
                        let s = jstr(env, &p.text);
                        let view = make_view(
                            env,
                            "makeLabel",
                            "(Ljava/lang/String;)Landroid/view/View;",
                            &[JValue::Object(&s)],
                        );
                        let fam = match custom_family(p.font) {
                            Some(f) => JObject::from(jstr(env, f)),
                            None => JObject::null(),
                        };
                        let _ = env.dcall_static(
                            BRIDGE,
                            "setLabelFont",
                            "(Landroid/view/View;FIZLjava/lang/String;)V",
                            &[
                                JValue::Object(view.as_obj()),
                                JValue::Float(sp),
                                JValue::Int(weight),
                                JValue::Bool(italic),
                                JValue::Object(&fam),
                            ],
                        );
                        if let Some(col) = p.color {
                            let _ = env.dcall_static(
                                BRIDGE,
                                "setLabelColor",
                                "(Landroid/view/View;IZ)V",
                                &[
                                    JValue::Object(view.as_obj()),
                                    JValue::Int(argb_i32(col)),
                                    JValue::Bool(true),
                                ],
                            );
                        }
                        AHandle(view)
                    })
                }
                kinds::BUTTON => {
                    let p = props.downcast_ref::<ButtonProps>().unwrap();
                    with_env(|env| {
                        let s = jstr(env, &p.title);
                        AHandle(make_view(
                            env,
                            "makeButton",
                            "(JLjava/lang/String;)Landroid/view/View;",
                            &[JValue::Long(idj), JValue::Object(&s)],
                        ))
                    })
                }
                kinds::TOGGLE => {
                    let p = props.downcast_ref::<ToggleProps>().unwrap();
                    with_env(|env| {
                        AHandle(make_view(
                            env,
                            "makeToggle",
                            "(JZ)Landroid/view/View;",
                            &[JValue::Long(idj), JValue::Bool(p.on)],
                        ))
                    })
                }
                kinds::SLIDER => {
                    let p = props.downcast_ref::<SliderProps>().unwrap();
                    with_env(|env| {
                        AHandle(make_view(
                            env,
                            "makeSlider",
                            "(JDDD)Landroid/view/View;",
                            &[
                                JValue::Long(idj),
                                JValue::Double(p.value),
                                JValue::Double(p.min),
                                JValue::Double(p.max),
                            ],
                        ))
                    })
                }
                kinds::PICKER => crate::picker::realize_any(self, props, id),
                kinds::TEXT_AREA => crate::textarea::realize_any(self, props, id),
                kinds::TEXT_FIELD => {
                    let p = props.downcast_ref::<TextFieldProps>().unwrap();
                    with_env(|env| {
                        let v = jstr(env, &p.text);
                        let ph = jstr(env, &p.placeholder);
                        AHandle(make_view(
                            env,
                            "makeTextField",
                            "(JLjava/lang/String;Ljava/lang/String;)Landroid/view/View;",
                            &[JValue::Long(idj), JValue::Object(&v), JValue::Object(&ph)],
                        ))
                    })
                }
                kinds::DIVIDER => with_env(|env| {
                    AHandle(make_view(env, "makeDivider", "()Landroid/view/View;", &[]))
                }),
                kinds::PROGRESS => {
                    let p = props.downcast_ref::<ProgressProps>().unwrap();
                    with_env(|env| {
                        AHandle(make_view(
                            env,
                            "makeProgress",
                            "(ZD)Landroid/view/View;",
                            &[
                                JValue::Bool(p.value.is_some()),
                                JValue::Double(p.value.unwrap_or(0.0)),
                            ],
                        ))
                    })
                }
                kinds::CANVAS => with_env(|env| {
                    AHandle(make_view(env, "makeCanvas", "()Landroid/view/View;", &[]))
                }),
                kinds::IMAGE => {
                    let p = props.downcast_ref::<ImageProps>().unwrap();
                    // Scaling (§18.3): 0=fit (FIT_CENTER), 1=fill (CENTER_CROP), 2=stretch (FIT_XY).
                    let mode = match p.content_mode {
                        ContentMode::Fit => 0,
                        ContentMode::Fill => 1,
                        ContentMode::Stretch => 2,
                    };
                    with_env(|env| {
                        let s = jstr(env, &p.source);
                        AHandle(make_view(
                            env,
                            "makeImage",
                            "(Ljava/lang/String;I)Landroid/view/View;",
                            &[JValue::Object(&s), JValue::Int(mode)],
                        ))
                    })
                }
                _ => {
                    if let Some(make) = self.registry.get(kind).map(|r| r.make) {
                        return make(self, props, id);
                    }
                    warn_missing_renderer(kind);
                    with_env(|env| {
                        let s = jstr(env, &format!("⟨{kind}⟩"));
                        AHandle(make_view(
                            env,
                            "makeLabel",
                            "(Ljava/lang/String;)Landroid/view/View;",
                            &[JValue::Object(&s)],
                        ))
                    })
                }
            }
        }

        fn update(
            &mut self,
            h: &AHandle,
            kind: PieceKind,
            patch: &dyn Any,
            _anim: Option<&AnimSpec>,
        ) {
            match kind {
                kinds::CONTAINER => {
                    if let Some(ContainerPatch::Background(c)) =
                        patch.downcast_ref::<ContainerPatch>()
                    {
                        apply_surface(h, *c, 0.0, false);
                    }
                }
                // Mobile selection is transient (rows ripple, then push) — nothing to sync.
                kinds::NAV_MENU => {}
                kinds::TABS => {
                    if let Some(TabsPatch::Selected(i)) = patch.downcast_ref::<TabsPatch>() {
                        call_void(
                            "setTabsSelected",
                            "(Landroid/view/View;I)V",
                            &[JValue::Object(h.0.as_obj()), JValue::Int(*i as i32)],
                        );
                    }
                }
                kinds::NAV => {
                    if let Some(p) = patch.downcast_ref::<NavPatch>() {
                        match p {
                            NavPatch::Pushed { title } => with_env(|env| {
                                let s = jstr(env, title);
                                let _ = env.dcall_static(
                                    BRIDGE,
                                    "navPush",
                                    "(Landroid/view/View;Ljava/lang/String;)V",
                                    &[JValue::Object(h.0.as_obj()), JValue::Object(&s)],
                                );
                            }),
                            NavPatch::Popped => call_void(
                                "navPop",
                                "(Landroid/view/View;)V",
                                &[JValue::Object(h.0.as_obj())],
                            ),
                            NavPatch::Title(_) => {}
                        }
                    }
                }
                kinds::COVER => {
                    if let Some(p) = patch.downcast_ref::<CoverPatch>() {
                        match p {
                            CoverPatch::Present {
                                background,
                                dismiss_disabled,
                            } => call_void(
                                "coverPresent",
                                "(Landroid/view/View;IZZ)V",
                                &[
                                    JValue::Object(h.0.as_obj()),
                                    JValue::Int(background.map(argb_i32).unwrap_or(0)),
                                    JValue::Bool(background.is_some()),
                                    JValue::Bool(*dismiss_disabled),
                                ],
                            ),
                            CoverPatch::DismissDisabled(d) => call_void(
                                "coverSetDismissDisabled",
                                "(Landroid/view/View;Z)V",
                                &[JValue::Object(h.0.as_obj()), JValue::Bool(*d)],
                            ),
                            CoverPatch::Dismiss => call_void(
                                "coverDismiss",
                                "(Landroid/view/View;)V",
                                &[JValue::Object(h.0.as_obj())],
                            ),
                        }
                    }
                }
                kinds::LABEL => {
                    if let Some(p) = patch.downcast_ref::<LabelPatch>() {
                        match p {
                            LabelPatch::Text(t) => with_env(|env| {
                                let s = jstr(env, t);
                                let _ = env.dcall_static(
                                    BRIDGE,
                                    "setLabel",
                                    "(Landroid/view/View;Ljava/lang/String;)V",
                                    &[JValue::Object(h.0.as_obj()), JValue::Object(&s)],
                                );
                            }),
                            LabelPatch::Font(f) => {
                                let (sp, weight, italic) = font_params(*f);
                                let family = custom_family(*f);
                                with_env(|env| {
                                    let fam = match family {
                                        Some(name) => JObject::from(jstr(env, name)),
                                        None => JObject::null(),
                                    };
                                    let _ = env.dcall_static(
                                        BRIDGE,
                                        "setLabelFont",
                                        "(Landroid/view/View;FIZLjava/lang/String;)V",
                                        &[
                                            JValue::Object(h.0.as_obj()),
                                            JValue::Float(sp),
                                            JValue::Int(weight),
                                            JValue::Bool(italic),
                                            JValue::Object(&fam),
                                        ],
                                    );
                                });
                            }
                            LabelPatch::Color(c) => {
                                call_void(
                                    "setLabelColor",
                                    "(Landroid/view/View;IZ)V",
                                    &[
                                        JValue::Object(h.0.as_obj()),
                                        JValue::Int(c.map(argb_i32).unwrap_or(0)),
                                        JValue::Bool(c.is_some()),
                                    ],
                                );
                            }
                        }
                    }
                }
                kinds::BUTTON => {
                    if let Some(p) = patch.downcast_ref::<ButtonPatch>() {
                        match p {
                            ButtonPatch::Title(t) => with_env(|env| {
                                let s = jstr(env, t);
                                let _ = env.dcall_static(
                                    BRIDGE,
                                    "setLabel",
                                    "(Landroid/view/View;Ljava/lang/String;)V",
                                    &[JValue::Object(h.0.as_obj()), JValue::Object(&s)],
                                );
                            }),
                            ButtonPatch::Enabled(e) => call_void(
                                "setEnabled",
                                "(Landroid/view/View;Z)V",
                                &[JValue::Object(h.0.as_obj()), JValue::Bool(*e)],
                            ),
                        }
                    }
                }
                kinds::TOGGLE => {
                    if let Some(p) = patch.downcast_ref::<TogglePatch>() {
                        match p {
                            TogglePatch::On(on) => call_void(
                                "setToggle",
                                "(Landroid/view/View;Z)V",
                                &[JValue::Object(h.0.as_obj()), JValue::Bool(*on)],
                            ),
                            TogglePatch::Enabled(e) => call_void(
                                "setEnabled",
                                "(Landroid/view/View;Z)V",
                                &[JValue::Object(h.0.as_obj()), JValue::Bool(*e)],
                            ),
                        }
                    }
                }
                kinds::SLIDER => {
                    if let Some(p) = patch.downcast_ref::<SliderPatch>() {
                        match p {
                            SliderPatch::Value(v) => call_void(
                                "setSlider",
                                "(Landroid/view/View;DD)V",
                                &[
                                    JValue::Object(h.0.as_obj()),
                                    JValue::Double(*v),
                                    JValue::Double(0.0), // min recovered from the widget tag
                                ],
                            ),
                            SliderPatch::Enabled(e) => call_void(
                                "setEnabled",
                                "(Landroid/view/View;Z)V",
                                &[JValue::Object(h.0.as_obj()), JValue::Bool(*e)],
                            ),
                        }
                    }
                }
                kinds::PROGRESS => {
                    if let Some(ProgressPatch::Value(Some(v))) =
                        patch.downcast_ref::<ProgressPatch>()
                    {
                        call_void(
                            "setProgress",
                            "(Landroid/view/View;D)V",
                            &[JValue::Object(h.0.as_obj()), JValue::Double(*v)],
                        );
                    }
                }
                kinds::PICKER => crate::picker::update_any(self, h, patch),
                kinds::TEXT_AREA => crate::textarea::update_any(self, h, patch),
                kinds::TEXT_FIELD => {
                    if let Some(p) = patch.downcast_ref::<TextFieldPatch>() {
                        match p {
                            TextFieldPatch::Text { text, from_native } => {
                                if !*from_native {
                                    with_env(|env| {
                                        let s = jstr(env, text);
                                        let _ = env.dcall_static(
                                            BRIDGE,
                                            "setTextField",
                                            "(Landroid/view/View;Ljava/lang/String;)V",
                                            &[JValue::Object(h.0.as_obj()), JValue::Object(&s)],
                                        );
                                    });
                                }
                            }
                            TextFieldPatch::Placeholder(t) => with_env(|env| {
                                let s = jstr(env, t);
                                let _ = env.dcall_static(
                                    BRIDGE,
                                    "setPlaceholder",
                                    "(Landroid/view/View;Ljava/lang/String;)V",
                                    &[JValue::Object(h.0.as_obj()), JValue::Object(&s)],
                                );
                            }),
                            TextFieldPatch::Enabled(e) => call_void(
                                "setEnabled",
                                "(Landroid/view/View;Z)V",
                                &[JValue::Object(h.0.as_obj()), JValue::Bool(*e)],
                            ),
                        }
                    }
                }
                kinds::LIST => match patch.downcast_ref::<ListPatch>() {
                    Some(ListPatch::Reload) => {
                        // notifyDataSetChanged: getCount reads the snapshot, getView is deferred to
                        // the next layout — safe inside a with_tree borrow.
                        call_void(
                            "listReload",
                            "(Landroid/view/View;)V",
                            &[JValue::Object(h.0.as_obj())],
                        );
                    }
                    Some(ListPatch::ScrollToEnd) => {
                        // Posts smoothScrollToPosition(count-1) on the ListView (no-op if empty).
                        call_void(
                            "listScrollToEnd",
                            "(Landroid/view/View;)V",
                            &[JValue::Object(h.0.as_obj())],
                        );
                    }
                    _ => {}
                },
                _ => {
                    if let Some(update) = self.registry.get(kind).map(|r| r.update) {
                        update(self, h, patch);
                    }
                }
            }
        }

        fn release(&mut self, h: AHandle) {
            let key = h.0.as_obj().as_raw() as usize;
            if let Some(nid) = LIST_NODE.with(|m| m.borrow_mut().remove(&key)) {
                LIST_SOURCES.with(|m| {
                    m.borrow_mut().remove(&nid);
                });
            }
            call_void(
                "removeChild",
                "(Landroid/view/View;)V",
                &[JValue::Object(h.0.as_obj())],
            );
        }

        fn insert(&mut self, parent: &AHandle, child: &AHandle, _index: usize) {
            call_void(
                "addChild",
                "(Landroid/view/View;Landroid/view/View;)V",
                &[
                    JValue::Object(parent.0.as_obj()),
                    JValue::Object(child.0.as_obj()),
                ],
            );
        }

        fn remove(&mut self, _parent: &AHandle, child: &AHandle) {
            call_void(
                "removeChild",
                "(Landroid/view/View;)V",
                &[JValue::Object(child.0.as_obj())],
            );
        }

        fn move_child(&mut self, parent: &AHandle, child: &AHandle, _to: usize) {
            self.remove(parent, child);
            self.insert(parent, child, 0);
        }

        fn measure(&mut self, h: &AHandle, kind: PieceKind, p: Proposal) -> Size {
            let d = density();
            match kind {
                kinds::LABEL => {
                    let natural_w = measure_call(h, "measureWidth") / d;
                    match p.width {
                        Some(pw) if natural_w > pw => {
                            let wpx = (pw * d).round() as i32;
                            let hh = with_env(|env| {
                                env.dcall_static(
                                    BRIDGE,
                                    "measureHeightForWidth",
                                    "(Landroid/view/View;I)I",
                                    &[JValue::Object(h.0.as_obj()), JValue::Int(wpx)],
                                )
                                .expect("hfw")
                                .i()
                                .unwrap_or(0) as f64
                            });
                            Size::new(pw, hh / d)
                        }
                        _ => Size::new(natural_w, measure_call(h, "measureHeight") / d),
                    }
                }
                kinds::NAV_MENU => Size::new(
                    p.width.unwrap_or(320.0),
                    p.height
                        .unwrap_or_else(|| measure_call(h, "measureHeight") / d),
                ),
                kinds::SLIDER => Size::new(
                    p.width.unwrap_or(180.0),
                    (measure_call(h, "measureHeight") / d).max(24.0),
                ),
                // PICKER falls to the native measureWidth/measureHeight default below.
                kinds::TEXT_AREA => crate::textarea::measure_any(self, h, p),
                kinds::TEXT_FIELD => Size::new(
                    p.width.unwrap_or(180.0),
                    (measure_call(h, "measureHeight") / d).max(40.0),
                ),
                kinds::DIVIDER => Size::new(p.width.unwrap_or(0.0), 1.0),
                kinds::LIST => Size::new(p.width.unwrap_or(0.0), p.height.unwrap_or(0.0)),
                // A tabs host fills its container (like LIST). Its natural UNSPECIFIED probe is
                // useless: the M3 BottomNavigationView reports its expansive preferred width (every
                // item at full item width), which would lay the host out wider than the screen.
                kinds::TABS => Size::new(
                    p.width
                        .unwrap_or_else(|| measure_call(h, "measureWidth") / d),
                    p.height
                        .unwrap_or_else(|| measure_call(h, "measureHeight") / d),
                ),
                kinds::PROGRESS => {
                    // Determinate bar fills the proposed width (grow_w); the circular spinner
                    // keeps its natural square size (grow_w is false, so the engine uses it).
                    let nh = (measure_call(h, "measureHeight") / d).max(4.0);
                    let nw = (measure_call(h, "measureWidth") / d).max(20.0);
                    Size::new(p.width.unwrap_or(nw), nh)
                }
                _ => {
                    if let Some(measure) = self.registry.get(kind).and_then(|r| r.measure) {
                        return measure(self, h, p);
                    }
                    Size::new(
                        measure_call(h, "measureWidth") / d,
                        measure_call(h, "measureHeight") / d,
                    )
                }
            }
        }

        fn set_frame(&mut self, h: &AHandle, frame: Rect, _anim: Option<&AnimSpec>) {
            // Frame (DayFixed absolute placement) applies instantly; animated motion/scale uses the
            // transform channel below (translationX/Y + scale/rotation), which composes on top of
            // the laid-out position without relayout (§8.4).
            let d = density();
            call_void(
                "setFrame",
                "(Landroid/view/View;IIII)V",
                &[
                    JValue::Object(h.0.as_obj()),
                    JValue::Int((frame.origin.x * d).round() as i32),
                    JValue::Int((frame.origin.y * d).round() as i32),
                    JValue::Int((frame.size.width * d).round() as i32),
                    JValue::Int((frame.size.height * d).round() as i32),
                ],
            );
        }

        fn set_opacity(&mut self, h: &AHandle, opacity: f64, anim: Option<&AnimSpec>) {
            let (dur, curve) = anim_args(anim);
            call_void(
                "setOpacity",
                "(Landroid/view/View;FII)V",
                &[
                    JValue::Object(h.0.as_obj()),
                    JValue::Float(opacity as f32),
                    JValue::Int(dur),
                    JValue::Int(curve),
                ],
            );
        }

        fn set_transform(
            &mut self,
            h: &AHandle,
            t: Transform,
            _size: Size,
            anim: Option<&AnimSpec>,
        ) {
            let d = density();
            let (dur, curve) = anim_args(anim);
            call_void(
                "setTransform",
                "(Landroid/view/View;FFFFFII)V",
                &[
                    JValue::Object(h.0.as_obj()),
                    JValue::Float((t.tx * d) as f32),
                    JValue::Float((t.ty * d) as f32),
                    JValue::Float(t.sx as f32),
                    JValue::Float(t.sy as f32),
                    JValue::Float(t.rotate_deg as f32),
                    JValue::Int(dur),
                    JValue::Int(curve),
                ],
            );
        }

        fn set_scroll_content(&mut self, h: &AHandle, content: Size) {
            let d = density();
            call_void(
                "setScrollContent",
                "(Landroid/view/View;II)V",
                &[
                    JValue::Object(h.0.as_obj()),
                    JValue::Int((content.width * d).round() as i32),
                    JValue::Int((content.height * d).round() as i32),
                ],
            );
        }

        fn scroll_to(&mut self, h: &AHandle, target: Rect, animated: bool) {
            let d = density();
            call_void(
                "scrollToRect",
                "(Landroid/view/View;IIIIZ)V",
                &[
                    JValue::Object(h.0.as_obj()),
                    JValue::Int((target.origin.x * d).round() as i32),
                    JValue::Int((target.origin.y * d).round() as i32),
                    JValue::Int((target.size.width * d).round() as i32),
                    JValue::Int((target.size.height * d).round() as i32),
                    JValue::Bool(animated),
                ],
            );
        }

        fn focus(&mut self, h: &AHandle, _node: NodeId, focused: bool) {
            // DayBridge pairs the request with the IME (show on gain, hide on resign) and
            // resigns to the focusable content root — Android focus is never "nowhere".
            call_void(
                "focusView",
                "(Landroid/view/View;Z)V",
                &[JValue::Object(h.0.as_obj()), JValue::Bool(focused)],
            );
        }

        fn set_event_sink(&mut self, sink: EventSink) {
            SINK.with(|s| *s.borrow_mut() = Some(Rc::from(sink)));
        }

        fn enable_gesture(&mut self, h: &AHandle, node: NodeId, kind: day_spec::GestureKind) {
            let is_drag = matches!(kind, day_spec::GestureKind::Drag);
            call_void(
                "enableGesture",
                "(Landroid/view/View;JZ)V",
                &[
                    JValue::Object(h.0.as_obj()),
                    JValue::Long(node.0 as i64),
                    JValue::Bool(is_drag),
                ],
            );
        }

        fn set_context_menu(&mut self, h: &AHandle, _node: NodeId, items: &[day_spec::MenuItem]) {
            let mut spec = String::new();
            serialize_menu(items, &mut spec);
            with_env(|env| {
                let jspec = jstr(env, &spec);
                let _ = env.dcall_static(
                    BRIDGE,
                    "setContextMenu",
                    "(Landroid/view/View;Ljava/lang/String;)V",
                    &[JValue::Object(h.0.as_obj()), JValue::Object(&jspec)],
                );
            });
        }

        fn set_app_menu(&mut self, items: &[day_spec::MenuItem]) {
            // Android has no persistent menu bar; the platform convention for a global app menu is
            // the app-bar overflow (⋮). DayActivity.onCreateOptionsMenu builds from this spec.
            let mut spec = String::new();
            serialize_menu(items, &mut spec);
            with_env(|env| {
                let jspec = jstr(env, &spec);
                let _ = env.dcall_static(
                    BRIDGE,
                    "setAppMenu",
                    "(Ljava/lang/String;)V",
                    &[JValue::Object(&jspec)],
                );
            });
        }

        fn supports_lifecycle(&self, phase: day_spec::Lifecycle) -> bool {
            lifecycle_supported(phase)
        }

        fn attach_list(&mut self, host: &AHandle, source: ListSource) {
            let key = host.0.as_obj().as_raw() as usize;
            if let Some(nid) = LIST_NODE.with(|m| m.borrow().get(&key).copied()) {
                LIST_SOURCES.with(|m| {
                    m.borrow_mut().insert(nid, source);
                });
            }
            call_void(
                "listReload",
                "(Landroid/view/View;)V",
                &[JValue::Object(host.0.as_obj())],
            );
        }

        fn adopt(&mut self, raw: RawHandle) -> AHandle {
            // A recycling ListView cell (a DayFixed) — Day fills/rebinds its row content in place.
            with_env(|env| {
                let obj = unsafe { JObject::from_raw(env, raw as jni::sys::jobject) };
                AHandle(std::sync::Arc::new(
                    env.new_global_ref(&obj).expect("adopt: global ref"),
                ))
            })
        }

        fn set_a11y(&mut self, h: &AHandle, a11y: &A11yProps) {
            with_env(|env| {
                let label = jstr(env, a11y.label.as_deref().unwrap_or(""));
                let value = jstr(env, a11y.value.as_deref().unwrap_or(""));
                let _ = env.dcall_static(
                    BRIDGE,
                    "setA11y",
                    "(Landroid/view/View;Ljava/lang/String;Ljava/lang/String;Z)V",
                    &[
                        JValue::Object(h.0.as_obj()),
                        JValue::Object(&label),
                        JValue::Object(&value),
                        JValue::Bool(a11y.hidden),
                    ],
                );
            });
        }

        fn replay(&mut self, h: &AHandle, ops: &[DrawOp], _size: Size) {
            let (nums, texts) = day_spec::encode_ops(ops);
            with_env(|env| {
                let arr = env.new_double_array(nums.len()).expect("double array");
                arr.set_region(env, 0, &nums).expect("fill array");
                let joined = jstr(env, &texts.join("\u{1f}"));
                let _ = env.dcall_static(
                    BRIDGE,
                    "setCanvasOps",
                    "(Landroid/view/View;[DLjava/lang/String;)V",
                    &[
                        JValue::Object(h.0.as_obj()),
                        JValue::Object(&arr),
                        JValue::Object(&joined),
                    ],
                );
            });
        }

        fn snapshot_window(&mut self) -> Result<Vec<u8>, String> {
            Err("use `adb exec-out screencap -p` (device-level capture) on android-widget".into())
        }
    }

    impl Platform for Android {
        const TARGET: &'static str = "android-widget";
        const TOOLKIT: &'static str = "widget";

        fn run(self, _options: WindowOptions, ready: Box<dyn FnOnce(Self, AHandle, Size)>) {
            // The ActivityThread owns the loop; init() already registered the root.
            let (root, size) = ROOT
                .with(|r| r.borrow_mut().take())
                .expect("day-android: init() not called");
            ready(self, root, size);
        }

        fn post(f: Box<dyn FnOnce() + Send>) {
            let token = Box::into_raw(Box::new(f)) as i64;
            with_env(|env| {
                // Native-spawned threads see only the system class loader, so call through the
                // JClass cached on the main thread at init rather than a name lookup.
                let cls = BRIDGE_CLASS
                    .get()
                    .expect("day-android: bridge class not cached");
                let sig = "(J)V".parse::<RuntimeMethodSignature>().expect("sig");
                let res = env.call_static_method(
                    &**cls,
                    JNIString::from("postMain"),
                    MethodSignature::from(&sig),
                    &[JValue::Long(token)],
                );
                if res.is_err() {
                    env.exception_describe();
                    env.exception_clear();
                }
            });
        }

        /// Frame clock (§8.4): hand the pending callback to `Choreographer.postFrameCallback` (a
        /// one-shot; day-core re-arms while a frame consumer is live). `DayBridge.nativeDoFrame`
        /// trampolines back to `run_frame` on the UI thread with the frame time.
        fn request_frame(cb: Box<dyn FnOnce(f64) + 'static>) {
            let token = Box::into_raw(Box::new(cb)) as i64;
            call_void("requestFrame", "(J)V", &[JValue::Long(token)]);
        }
    }
}
