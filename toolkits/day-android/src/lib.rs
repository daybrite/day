// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! day-android — the android-mdc backend (DESIGN.md §9). jni + the DayBridge Java shim
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
            ("K_VALUE_COMMITTED", BridgeKind::ValueCommitted),
            ("K_SEARCH_CHANGED", BridgeKind::SearchChanged),
            ("K_NAV_PRESENTATION", BridgeKind::NavPresentation),
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
            ("K_SAFE_AREA", BridgeKind::SafeArea),
            // These two were absent from this table while present in DayBridge.java, so the
            // parity assertion below was failing before the search work touched it.
            ("K_WINDOW_CLOSED", BridgeKind::WindowClosed),
            ("K_WINDOW_FOCUSED", BridgeKind::WindowFocused),
            // Same story again: DayBridge.java gained K_APPEARANCE_CHANGED without this row.
            ("K_APPEARANCE_CHANGED", BridgeKind::AppearanceChanged),
            ("K_COVER_HIDDEN", BridgeKind::CoverHidden),
            ("K_LINK_ACTIVATED", BridgeKind::LinkActivated),
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
    /// After a failed JNI call, a Java exception may be PENDING on the env — and calling
    /// almost any further JNI function in that state is undefined behavior per the JNI
    /// spec. Every `DayEnv` wrapper funnels its error path through here (describe = stack
    /// trace to logcat, then clear), so an `Err` from `dcall`/`dcall_static`/`dfield` is
    /// always safe to ignore or handle without the call site remembering the clear.
    fn clear_pending(env: &Env, class: &str, name: &str) {
        if env.exception_check() {
            eprintln!("day-android: JNI call {class}.{name} threw (cleared; trace in logcat):");
            env.exception_describe();
            env.exception_clear();
        }
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
            let r = self.call_static_method(
                &cls,
                JNIString::from(name),
                MethodSignature::from(&sig),
                args,
            );
            if r.is_err() {
                clear_pending(self, class, name);
            }
            r
        }
        fn dcall(
            &mut self,
            obj: &JObject,
            name: &str,
            sig: &str,
            args: &[JValue],
        ) -> jni::errors::Result<JValueOwned<'l>> {
            let sig = sig.parse::<RuntimeMethodSignature>()?;
            let r = self.call_method(
                obj,
                JNIString::from(name),
                MethodSignature::from(&sig),
                args,
            );
            if r.is_err() {
                clear_pending(self, "<obj>", name);
            }
            r
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
            let r = self.get_static_field(&cls, JNIString::from(name), FieldSignature::from(&sig));
            if r.is_err() {
                clear_pending(self, class, name);
            }
            r
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
        A11yProps, AnimSpec, Builtin, Cap, Curve, DrawOp, Event, EventSink, Font, ListSource,
        NodeId, PieceKind, Platform, Point, Proposal, RawHandle, Rect, Registry, Renderer, Size,
        Support, Toolkit, Transform, WindowOptions, kinds,
    };

    thread_local! {
        /// Recycling list (docs/list.md): row-pull sources keyed by LIST node id (Java passes it
        /// back in nativeListBind), and a stable GlobalRef per physical cell so day-core's cell
        /// map keys consistently across ListView recycling. Cells are grouped BY LIST so that
        /// releasing a list frees its cells' JNI global refs with it — a long session would
        /// otherwise leak one global ref per physical cell toward the JNI table limit. Within a
        /// list, cells key by `identityHashCode`, the only stable identity the wire carries
        /// (nativeListBind sends `(hostId, position, cell)` and nothing else). Collisions are
        /// possible in principle; one would conflate two physical cells of the SAME list, and
        /// carrying a bridge-assigned cell id would widen the wire for a case never observed.
        static LIST_SOURCES: std::cell::RefCell<std::collections::HashMap<i64, ListSource>> =
            std::cell::RefCell::new(std::collections::HashMap::new());
        static LIST_NODE: std::cell::RefCell<std::collections::HashMap<usize, i64>> =
            std::cell::RefCell::new(std::collections::HashMap::new());
        /// Label view ptr → node id, so a `LabelPatch::Runs` (which carries no id) can still tell
        /// a link's ClickableSpan which node to report against. Entries drop in `release`.
        static LABEL_NODE: std::cell::RefCell<std::collections::HashMap<usize, i64>> =
            std::cell::RefCell::new(std::collections::HashMap::new());
        #[allow(clippy::type_complexity)] // host id → (cell identityHashCode → its GlobalRef)
        static LIST_CELLS: std::cell::RefCell<
            std::collections::HashMap<i64, std::collections::HashMap<i32, Gref>>,
        > = std::cell::RefCell::new(std::collections::HashMap::new());
    }

    /// Row count, pulled by the Java adapter's getCount (reads the snapshot only; no tree).
    /// A JNI up-call entry: the body is contained — a panic unwinding the frame would abort.
    pub fn list_len(host_id: i64) -> usize {
        day_spec::ffi_guard::contain(0, || {
            LIST_SOURCES.with(|m| m.borrow().get(&host_id).map(|s| (s.len)()).unwrap_or(0))
        })
    }

    /// Fill a recycled cell — the Java adapter's getView calls this. A stable GlobalRef per
    /// physical cell (keyed by identityHashCode under its list) gives day-core a consistent
    /// cell key. A JNI up-call entry running app row builders: contained like `list_len`.
    pub fn list_bind(env: &mut Env, host_id: i64, position: i32, cell: JObject) {
        day_spec::ffi_guard::contain((), || {
            let hash = env
                .dcall(&cell, "hashCode", "()I", &[])
                .and_then(|v| v.i())
                .unwrap_or(0);
            let cached = LIST_CELLS.with(|m| {
                m.borrow()
                    .get(&host_id)
                    .and_then(|cells| cells.get(&hash).cloned())
            });
            let gref = match cached {
                Some(g) => g,
                None => {
                    // A failed global ref (JNI table exhausted) skips this bind — the next
                    // bind retries — rather than panicking out of the up-call.
                    let Ok(g) = env.new_global_ref(&cell) else {
                        return;
                    };
                    let g = std::sync::Arc::new(g);
                    LIST_CELLS.with(|m| {
                        m.borrow_mut()
                            .entry(host_id)
                            .or_default()
                            .insert(hash, g.clone())
                    });
                    g
                }
            };
            let raw = gref.as_obj().as_raw() as RawHandle;
            let source = LIST_SOURCES.with(|m| m.borrow().get(&host_id).cloned());
            if let Some(source) = source {
                (source.bind_row)(position as usize, raw);
            }
        });
    }

    /// The reorder guard's verdict for a hovered drop (docs/list.md), pulled synchronously by
    /// ItemTouchHelper's canDropOver. ItemTouchHelper cannot relocate the gap, so a Retarget
    /// verdict (accepted != proposed) reads as a deny for that hover. The source is cloned out
    /// before the guard runs — no thread-local borrow held.
    pub fn list_can_drop(host_id: i64, from: i32, to: i32) -> bool {
        // JNI up-call entry (ItemTouchHelper's canDropOver): contain the app's guard.
        day_spec::ffi_guard::contain(false, || {
            let source = LIST_SOURCES.with(|m| m.borrow().get(&host_id).cloned());
            let Some(source) = source else { return false };
            let Some(r) = source.reorder.as_ref() else {
                return false;
            };
            let len = (source.len)();
            let (from, to) = (from.max(0) as usize, to.max(0) as usize);
            if from >= len || to >= len {
                return false;
            }
            (r.can_move)(from, to) == to as i64
        })
    }

    /// Commit one incremental ItemTouchHelper swap through the sync seam (rotates day's
    /// snapshot, defers the app callback). Returns whether the swap was accepted.
    pub fn list_move(host_id: i64, from: i32, to: i32) -> bool {
        // JNI up-call entry: contain the commit through the sync seam.
        day_spec::ffi_guard::contain(false, || {
            if !list_can_drop(host_id, from, to) {
                return false;
            }
            let source = LIST_SOURCES.with(|m| m.borrow().get(&host_id).cloned());
            let Some(r) = source.and_then(|s| s.reorder) else {
                return false;
            };
            let (from, to) = (from as usize, to as usize);
            if from != to {
                (r.move_row)(from, to);
            }
            true
        })
    }

    /// May this row be swiped away? Consulted from `getMovementFlags`, so a protected row
    /// simply reports no swipe direction and never moves under the finger (docs/list.md).
    pub fn list_can_delete(host_id: i64, index: i32) -> bool {
        // JNI up-call entry (getMovementFlags): contain the app's guard.
        day_spec::ffi_guard::contain(false, || {
            let source = LIST_SOURCES.with(|m| m.borrow().get(&host_id).cloned());
            let Some(source) = source else { return false };
            let Some(d) = source.delete.as_ref() else {
                return false;
            };
            let index = index.max(0) as usize;
            index < (source.len)() && (d.can_delete)(index)
        })
    }

    /// Commit a swipe-to-delete through the sync seam (shortens day's snapshot, defers the app
    /// callback). Returns whether the delete was accepted.
    pub fn list_delete(host_id: i64, index: i32) -> bool {
        // JNI up-call entry: contain the commit through the sync seam.
        day_spec::ffi_guard::contain(false, || {
            if !list_can_delete(host_id, index) {
                return false;
            }
            let source = LIST_SOURCES.with(|m| m.borrow().get(&host_id).cloned());
            let Some(d) = source.and_then(|s| s.delete) else {
                return false;
            };
            (d.delete_row)(index as usize);
            true
        })
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

    /// Whether the JVM has been cached — i.e. whether [`with_env`] can run at all.
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

    /// View a `java.lang.String` object as a `JString`. String return values arrive as a
    /// `JObject` from `JValueOwned::l()`; casting is safe — `JString` is a transparent wrapper over
    /// the same `jobject`. Public: piece/part crates reading Java strings use it.
    pub fn as_jstring<'a>(obj: JObject<'a>) -> JString<'a> {
        // Safety: same repr (a jobject); caller guarantees the object is a java.lang.String.
        unsafe { std::mem::transmute(obj) }
    }

    /// Call a DayBridge static returning a View, as a shared global ref (public helper).
    ///
    /// Never panics on a Java throw: this is the funnel for every builtin realize, and it
    /// runs inside a JNI up-call — a panic unwinding out of that frame aborts the process,
    /// which shipped as the "splash-only blank app" failure. On error the pending exception
    /// is described-and-cleared (stack trace in logcat, via the `DayEnv` wrappers) and a
    /// visible `⟨method⟩` placeholder label stands in, so one broken view cannot take down
    /// the whole tree build. [`try_make_view`] is the fallible form for callers that want
    /// to handle the failure themselves.
    /// Pack a color the way `android.graphics.Color` wants it (`0xAARRGGBB`, as a signed int).
    pub fn argb(c: day_spec::Color) -> i32 {
        let f = |v: f64| (v.clamp(0.0, 1.0) * 255.0) as u32;
        ((f(c.a) << 24) | (f(c.r) << 16) | (f(c.g) << 8) | f(c.b)) as i32
    }

    /// Style a button in place through the bridge. The view stays a `MaterialButton`, so its
    /// ripple, state overlays, focus and accessibility role are Material's, not ours.
    pub fn apply_button_style(env: &mut Env, v: &Gref, style: day_spec::props::ButtonStyleSpec) {
        use day_spec::props::ButtonStyleSpec as S;
        let (kind, fill) = match style {
            S::Automatic => (0, day_spec::Color::CLEAR),
            S::Bordered => (1, day_spec::Color::CLEAR),
            S::Prominent => (2, day_spec::Color::CLEAR),
            S::Tinted(c) => (3, c),
        };
        let _ = env.dcall_static(
            BRIDGE,
            "setButtonStyle",
            "(Landroid/view/View;III)V",
            &[
                JValue::Object(v.as_obj()),
                JValue::Int(kind),
                JValue::Int(argb(fill)),
                JValue::Int(argb(S::on_tint(fill))),
            ],
        );
    }

    /// Send a label's text + runs across in one call (docs/text-runs.md).
    ///
    /// Offsets convert from Rust BYTES to Java's UTF-16 indices here, per run: any emoji or CJK
    /// in the string makes the two disagree, and an off-by-N range styles the wrong words.
    pub fn set_label_runs(
        env: &mut Env,
        node: i64,
        v: &Gref,
        text: &str,
        runs: &[day_spec::TextRun],
    ) {
        // Flat parallel arrays: one JNI call rather than one per run, on a path that runs for
        // every label patch.
        let mut starts = Vec::with_capacity(runs.len());
        let mut ends = Vec::with_capacity(runs.len());
        let mut flags = Vec::with_capacity(runs.len());
        let mut colors = Vec::with_capacity(runs.len());
        let mut links: Vec<Option<String>> = Vec::with_capacity(runs.len());
        for r in runs {
            let Some(slice) = text.get(r.range.clone()) else {
                continue;
            };
            let start = text[..r.range.start].encode_utf16().count() as i32;
            starts.push(start);
            ends.push(start + slice.encode_utf16().count() as i32);
            let mut f = 0i32;
            if r.font
                .weight
                .is_some_and(|w| w >= day_spec::FontWeight::Semibold)
            {
                f |= 1;
            }
            if r.font.italic {
                f |= 2;
            }
            if r.font.monospace {
                f |= 4;
            }
            if r.strikethrough {
                f |= 8;
            }
            if r.color.is_some() {
                f |= 16;
            }
            flags.push(f);
            colors.push(r.color.map(argb).unwrap_or(0));
            links.push(r.link.clone());
        }
        let (Ok(sa), Ok(se), Ok(sf), Ok(sc)) = (
            env.new_int_array(starts.len()),
            env.new_int_array(ends.len()),
            env.new_int_array(flags.len()),
            env.new_int_array(colors.len()),
        ) else {
            return;
        };
        if sa.set_region(env, 0, &starts).is_err()
            || se.set_region(env, 0, &ends).is_err()
            || sf.set_region(env, 0, &flags).is_err()
            || sc.set_region(env, 0, &colors).is_err()
        {
            return;
        }
        // Link targets ride ONE joined string, as the canvas op stream already does — a Java
        // object array through JNI costs a class lookup and a per-element store for a payload
        // that is empty in almost every label.
        let joined = links
            .iter()
            .map(|l| l.clone().unwrap_or_default())
            .collect::<Vec<_>>()
            .join("\u{1f}");
        let s = jstr(env, text);
        let jl = jstr(env, &joined);
        let _ = env.dcall_static(
            BRIDGE,
            "setLabelRuns",
            "(Landroid/view/View;JLjava/lang/String;[I[I[I[ILjava/lang/String;)V",
            &[
                JValue::Object(v.as_obj()),
                JValue::Long(node),
                JValue::Object(&s),
                JValue::Object(&sa),
                JValue::Object(&se),
                JValue::Object(&sf),
                JValue::Object(&sc),
                JValue::Object(&jl),
            ],
        );
    }

    pub fn make_view(env: &mut Env, method: &str, sig: &str, args: &[JValue]) -> Gref {
        match try_make_view(env, method, sig, args) {
            Ok(g) => g,
            Err(e) => {
                eprintln!(
                    "day-android: DayBridge.{method} failed ({e}); substituting a placeholder view"
                );
                placeholder_view(env, method)
            }
        }
    }

    /// [`make_view`] without the placeholder fallback: `Err` on a Java throw (already
    /// cleared), a wrong return type, or a null View.
    pub fn try_make_view(
        env: &mut Env,
        method: &str,
        sig: &str,
        args: &[JValue],
    ) -> Result<Gref, jni::errors::Error> {
        try_make_view_on(env, BRIDGE, method, sig, args)
    }

    /// [`try_make_view`] against an arbitrary staged class: piece crates calling their OWN
    /// Java factory (docs/bridge.md) share the same non-panicking path.
    pub fn try_make_view_on(
        env: &mut Env,
        class: &str,
        method: &str,
        sig: &str,
        args: &[JValue],
    ) -> Result<Gref, jni::errors::Error> {
        let obj = env.dcall_static(class, method, sig, args)?.l()?;
        if obj.is_null() {
            return Err(jni::errors::Error::NullPtr("factory returned a null View"));
        }
        Ok(std::sync::Arc::new(env.new_global_ref(obj)?))
    }

    /// The visible stand-in for a native make that failed: the same `⟨name⟩` label the
    /// missing-renderer path shows (degrade loudly, keep the tree building).
    pub fn placeholder_view(env: &mut Env, what: &str) -> Gref {
        let text = jstr(env, &format!("⟨{what}⟩"));
        try_make_view(
            env,
            "makeLabel",
            "(Ljava/lang/String;)Landroid/view/View;",
            &[JValue::Object(&text)],
        )
        // The bridge itself cannot build a plain label: nothing can render. Aborting
        // here is honest — and it is the only remaining panic on this path.
        .expect("day-android: DayBridge.makeLabel unavailable — bridge not staged?")
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
            // Runs inside the layout pass (a JNI up-call): a Java throw must degrade to a
            // zero measurement, not a panic-abort. The wrapper already cleared the exception.
            env.dcall_static(
                BRIDGE,
                method,
                "(Landroid/view/View;)I",
                &[JValue::Object(h.0.as_obj())],
            )
            .and_then(|v| v.i())
            .unwrap_or(0) as f64
        })
    }

    /// Initialize globals from the Activity's nativeStart (called by `day::android_start`).
    /// A JNI up-call entry: contained, so a startup failure logs instead of aborting.
    pub fn init(env: &mut Env, root: JObject, density_: f32, w: i32, h: i32) {
        day_spec::ffi_guard::contain((), || {
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
            // Only fails when the JNI global-ref table is already exhausted; without a root
            // there is nothing to run, so degrade loudly rather than panic-abort the up-call.
            let Ok(root_ref) = env.new_global_ref(root) else {
                eprintln!("day-android: init could not take a global ref on the root view");
                return;
            };
            let handle = AHandle(std::sync::Arc::new(root_ref));
            let size = Size::new(w as f64 / d, h as f64 / d);
            ROOT.with(|r| *r.borrow_mut() = Some((handle, size)));
            // Android's OS temp dir isn't app-writable; use the app cache dir for the file-save
            // staging area (docs/files.md) so `save_file(..)` can write its temp before handing
            // off to SAF.
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
        });
    }

    thread_local! {
        /// Secondary DayWindowActivity roots (docs/windows.md): (day node, the root's
        /// global ref — kept alive alongside the tree's own adopted ref).
        static SECONDARY: RefCell<Vec<(u64, Gref)>> = const { RefCell::new(Vec::new()) };
    }

    /// A secondary DayWindowActivity's first laid-out root (the `nativeStartWindow` JNI
    /// export lands here): adopt it as the parked day window's content
    /// (docs/windows.md). `false` ⇒ the window was closed before the activity finished
    /// connecting — the activity finishes itself.
    pub fn window_started(env: &mut Env, root: JObject, node: i64, w: i32, h: i32) -> bool {
        // JNI up-call entry (nativeStartWindow): contain the adoption + day-core completion.
        day_spec::ffi_guard::contain(false, || {
            let d = DENSITY.with(|x| x.get());
            let Ok(gref) = env.new_global_ref(&root) else {
                return false;
            };
            let gref = std::sync::Arc::new(gref);
            let raw = gref.as_obj().as_raw() as day_spec::RawHandle;
            SECONDARY.with(|s| s.borrow_mut().push((node as u64, gref)));
            let ok = day_core::finish_window_open(
                day_spec::NodeId(node as u64),
                raw,
                Size::new(w as f64 / d, h as f64 / d),
            );
            if !ok {
                SECONDARY.with(|s| s.borrow_mut().retain(|(n, _)| *n != node as u64));
            }
            ok
        })
    }

    /// The day node of the secondary window whose adopted content is `host`, if any.
    fn secondary_node_of(env: &mut Env, host: &AHandle) -> Option<u64> {
        SECONDARY.with(|s| {
            s.borrow()
                .iter()
                .find(|(_, gref)| {
                    env.is_same_object(host.0.as_obj(), gref.as_obj())
                        .unwrap_or(false)
                })
                .map(|(n, _)| *n)
        })
    }

    // The wire table (day_spec::bridge) as const match patterns — the Java side's K_* constants
    // mirror these, and day-android's parity test holds the two files together.
    const K_PRESSED: i32 = bridge::BridgeKind::Pressed as i32;
    const K_TEXT_CHANGED: i32 = bridge::BridgeKind::TextChanged as i32;
    const K_TOGGLE_CHANGED: i32 = bridge::BridgeKind::ToggleChanged as i32;
    const K_VALUE_CHANGED: i32 = bridge::BridgeKind::ValueChanged as i32;
    const K_VALUE_COMMITTED: i32 = bridge::BridgeKind::ValueCommitted as i32;
    const K_SEARCH_CHANGED: i32 = bridge::BridgeKind::SearchChanged as i32;
    const K_NAV_PRESENTATION: i32 = bridge::BridgeKind::NavPresentation as i32;
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
    const K_SAFE_AREA: i32 = bridge::BridgeKind::SafeArea as i32;
    const K_WINDOW_CLOSED: i32 = bridge::BridgeKind::WindowClosed as i32;
    const K_WINDOW_FOCUSED: i32 = bridge::BridgeKind::WindowFocused as i32;
    const K_APPEARANCE_CHANGED: i32 = bridge::BridgeKind::AppearanceChanged as i32;
    const K_COVER_HIDDEN: i32 = bridge::BridgeKind::CoverHidden as i32;
    const K_LINK_ACTIVATED: i32 = bridge::BridgeKind::LinkActivated as i32;

    /// The single native trampoline (the app's `nativeOnEvent` forwards here). The kind
    /// numbers are `day_spec::bridge::BridgeKind` — the shared wire table. A JNI up-call
    /// entry: the decode + dispatch is contained (`day_spec::ffi_guard`) — a panic
    /// unwinding this frame would abort the process.
    pub fn dispatch_event(env: &mut Env, id: i64, kind: i32, num: f64, jstr: &JString) {
        day_spec::ffi_guard::contain((), || {
            dispatch_event_inner(env, id, kind, num, jstr);
        });
    }

    fn dispatch_event_inner(env: &mut Env, id: i64, kind: i32, num: f64, jstr: &JString) {
        let ev = match kind {
            K_PRESSED => Event::Pressed,
            K_TEXT_CHANGED => {
                let text = env.dstr(jstr).ok().unwrap_or_default();
                Event::TextChanged(text)
            }
            K_TOGGLE_CHANGED => Event::ToggleChanged(num != 0.0),
            K_VALUE_CHANGED => Event::ValueChanged(num),
            K_VALUE_COMMITTED => Event::ValueCommitted(num),
            // Inline search on the nav list (docs/search.md): the field's new text.
            K_SEARCH_CHANGED => Event::SearchChanged(env.dstr(jstr).ok().unwrap_or_default()),
            // SlidingPaneLayout settled on a presentation; Day reconciles rather than drives.
            K_NAV_PRESENTATION => Event::NavPresentationChanged(if num >= 0.5 {
                day_spec::props::NavPresentation::Split
            } else {
                day_spec::props::NavPresentation::Stack
            }),
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
            // Warm deep link: the nav piece handles RouteRequested.
            K_DEEPLINK => {
                let route: String = env.dstr(jstr).ok().unwrap_or_default();
                Event::RouteRequested(route)
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
            // A cover's hide slide finished (docs/cover.md): DayCover reports so Rust can
            // dispose the content.
            K_COVER_HIDDEN => Event::CoverHidden,
            // A styled run's link was tapped (docs/text-runs.md): the ClickableSpan reports its
            // target, and day-core routes it to the label's `.on_link()`.
            K_LINK_ACTIVATED => Event::LinkActivated(env.dstr(jstr).ok().unwrap_or_default()),
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
            // Safe-area report (edge-to-edge mode, docs/layout.md): update the global insets
            // signal; apps read `day::safe_area()`. Not an Event — nothing to route to a node.
            K_SAFE_AREA => {
                let text: String = env.dstr(jstr).ok().unwrap_or_default();
                let p: Vec<f64> = text.split(',').filter_map(|s| s.parse().ok()).collect();
                if p.len() < 4 {
                    return;
                }
                let d = DENSITY.with(|x| x.get());
                day_core::set_safe_area(day_spec::Insets {
                    top: p[0] / d,
                    bottom: p[1] / d,
                    leading: p[2] / d,
                    trailing: p[3] / d,
                });
                return;
            }
            // Light/dark switch (DayActivity.onConfigurationChanged). Not an Event — nothing to
            // route to a node; day-core restyles what it owns and rebuilds app-painted surfaces.
            K_APPEARANCE_CHANGED => {
                day_core::note_appearance_changed();
                return;
            }
            K_WINDOW_RESIZED => {
                let text: String = env.dstr(jstr).ok().unwrap_or_default();
                let p: Vec<f64> = text.split(',').filter_map(|s| s.parse().ok()).collect();
                if p.len() < 2 {
                    return;
                }
                let d = DENSITY.with(|x| x.get());
                // id 0 = the primary activity (WINDOW_NODE); a secondary DayWindowActivity
                // reports against its own day root (docs/windows.md).
                let target = if id == 0 {
                    day_spec::WINDOW_NODE
                } else {
                    day_spec::NodeId(id as u64)
                };
                emit(target, Event::WindowResized(Size::new(p[0] / d, p[1] / d)));
                return;
            }
            // Secondary-window lifecycle (docs/windows.md): the id is the window's root.
            K_WINDOW_CLOSED => Event::WindowClosed,
            K_WINDOW_FOCUSED => Event::WindowFocused(num != 0.0),
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

    /// Posted-closure trampoline (the app's `nativeRunPosted` forwards here). The closure is
    /// arbitrary app/day-core code inside a JNI up-call, so it runs contained.
    pub fn run_posted(token: i64) {
        // SAFETY: `token` is the Box::into_raw pointer `Platform::post` minted; Java hands it
        // back exactly once.
        let f: Box<Box<dyn FnOnce() + Send>> =
            unsafe { Box::from_raw(token as *mut Box<dyn FnOnce() + Send>) };
        day_spec::ffi_guard::contain((), f);
    }

    /// Frame-callback trampoline (the app's `nativeDoFrame` forwards here). `frame_nanos` is
    /// `Choreographer`'s frame time in nanoseconds; day-core wants seconds. Runs on the UI
    /// thread, contained like `run_posted`.
    pub fn run_frame(token: i64, frame_nanos: i64) {
        // SAFETY: `token` is the Box::into_raw pointer `request_frame` minted; Java hands it
        // back exactly once.
        let f: Box<Box<dyn FnOnce(f64)>> =
            unsafe { Box::from_raw(token as *mut Box<dyn FnOnce(f64)>) };
        day_spec::ffi_guard::contain((), move || f(frame_nanos as f64 / 1_000_000_000.0));
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
            NewWindow => "New Window",
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
                day_spec::MenuItem::Submenu { label, items, .. } => {
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
    fn font_params(spec: day_spec::FontSpec) -> (f32, i32, bool, bool) {
        let (sp, inherent) = font_style(spec.style);
        let weight = android_weight(spec.weight.unwrap_or(inherent));
        (sp, weight, spec.italic, spec.tabular)
    }

    /// Day `Color` (0–1 floats) → a packed `0xAARRGGBB` int for `android.graphics.Color`.
    /// Per-row nav icon tints as an index-aligned joined string ("0" = untinted) — the
    /// best-effort `setNavMenuTints` wire format (docs/vectors.md).
    fn nav_tints_joined(tints: &[Option<day_spec::Color>]) -> String {
        tints
            .iter()
            .map(|t| t.map(argb_i32).unwrap_or(0).to_string())
            .collect::<Vec<_>>()
            .join("\u{1f}")
    }

    /// Per-row nav context menus (docs/menus.md) as one string: each row's
    /// [`serialize_menu`] spec (empty = no menu), joined by U+001E — a separator the line
    /// format itself never contains. Ridden best-effort AFTER makeNavMenu/updateNavMenu,
    /// like the tints.
    fn nav_menus_joined(menus: &[Vec<day_spec::MenuItem>]) -> String {
        menus
            .iter()
            .map(|items| {
                let mut spec = String::new();
                serialize_menu(items, &mut spec);
                spec
            })
            .collect::<Vec<_>>()
            .join("\u{1e}")
    }

    fn argb_i32(c: day_spec::Color) -> i32 {
        let ch = |x: f64| (x.clamp(0.0, 1.0) * 255.0).round() as u32;
        ((ch(c.a) << 24) | (ch(c.r) << 16) | (ch(c.g) << 8) | ch(c.b)) as i32
    }

    /// Warn ONCE per kind that this backend has no registered renderer for `kind`, before falling
    /// back to a visible placeholder. A missing renderer usually means the piece's `mdc` feature
    /// wasn't enabled (Tier A.2 derives it automatically under `day build`). The message goes to both
    /// stderr (which `redirect_stdio_to_logcat` routes to logcat) and directly to logcat at ERROR, so
    /// it surfaces even before the redirect installs. Deduped per kind so it doesn't spam the log.
    fn warn_missing_renderer(kind: PieceKind) {
        day_spec::placeholder::report(kind, "android");
    }

    /// Realize fallback for a builtin arm whose props failed to downcast
    /// ([`day_spec::props_of`] already reported it): the same visible `⟨kind⟩` label a
    /// missing renderer shows, so one mismatched piece cannot abort the tree build.
    fn realize_placeholder(kind: PieceKind) -> AHandle {
        with_env(|env| AHandle(placeholder_view(env, kind)))
    }

    /// Ask the bridge for a PNG of this app's window (docs/window-image.md).
    ///
    /// The bytes come back as a Java `byte[]` and are copied out with `convert_byte_array` — no
    /// base64 round trip for what is already binary.
    fn android_window_image(chrome: bool) -> Result<Vec<u8>, String> {
        with_env(|env| {
            let obj = env
                .dcall_static(BRIDGE, "windowImage", "(Z)[B", &[JValue::Bool(chrome)])
                .map_err(|e| format!("windowImage: {e}"))?
                .l()
                .map_err(|e| format!("windowImage: {e}"))?;
            if obj.is_null() {
                return Err("no window to capture".to_string());
            }
            // SAFETY: the call above returns a byte[] or null; null is handled just above.
            let arr: jni::objects::JByteArray = unsafe { std::mem::transmute(obj) };
            env.convert_byte_array(&arr)
                .map_err(|e| format!("windowImage: {e}"))
        })
    }

    impl Toolkit for Android {
        type Handle = AHandle;

        fn capability(&self, cap: Cap) -> Support {
            match cap {
                // `View.draw(Canvas)` renders this app's own window into a bitmap
                // (docs/window-image.md); surface-backed content is the documented gap.
                Cap::Snapshot => Support::Native,
                // SpannableString spans, drawn by the one TextView (docs/text-runs.md).
                // A link run is a ClickableSpan; the TextView takes LinkMovementMethod when one
                // is present, which is what makes the tap land.
                Cap::TextRuns | Cap::TextLinks => Support::Native,
                // EditText honors editable / selectable / spell-check (DayTextArea shim).
                Cap::Dialogs
                | Cap::FileDialogs
                | Cap::Animation
                | Cap::Cover
                // The MaterialToolbar names the destination on every page (DayNavHost
                // syncChrome) — content needn't repeat the title (docs/navigation.md).
                | Cap::NavHeader
                // A SlidingPaneLayout hosts every `selector(Sidebar)`, so two panes are
                // available wherever they fit — a tablet, a foldable open, a phone in landscape
                // if the widths allow (docs/size-classes.md).
                | Cap::NavSplit
                | Cap::TextEditable
                | Cap::TextSelectable
                | Cap::TextSpellCheck
                // ItemTouchHelper on the RecyclerView list: long-press lift, elevation,
                // incremental swaps — the platform's own reorder (docs/list.md).
                | Cap::ListReorder
                // ItemTouchHelper's swipe half, with the Material red field revealing behind
                // the row (docs/list.md).
                | Cap::ListDelete
                // Document-style DayWindowActivity instances (docs/windows.md): separate
                // recents entries; side-by-side in split-screen/freeform/desktop windowing.
                | Cap::MultiWindow
                // View.getBaseline() — the platform's own answer (docs/baseline.md).
                | Cap::BaselineAlignment => Support::Native,
                // EMULATED: SlidingPaneLayout decides at MEASURE time whether both panes fit, so
                // the platform owns the presentation and Day observes it through
                // `Event::NavPresentationChanged` rather than pushing one in
                // (docs/size-classes.md).
                Cap::NavRepresent => Support::Emulated,
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
            match Builtin::from_key(kind) {
                Some(Builtin::Container) => {
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
                Some(Builtin::Scroll) => {
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
                Some(Builtin::List) => {
                    let Some(p) = day_spec::props_of::<ListProps>(kind, "android", props) else {
                        return realize_placeholder(kind);
                    };
                    let d = DENSITY.with(|x| x.get());
                    let rowh = match p.row_height {
                        RowHeight::Uniform(h) => h,
                        RowHeight::Automatic => 44.0,
                    };
                    let handle = with_env(|env| {
                        let del_label = jstr(env, &p.delete_label);
                        AHandle(make_view(
                            env,
                            "makeList",
                            "(JIZZZLjava/lang/String;)Landroid/view/View;",
                            &[
                                JValue::Long(id.0 as i64),
                                JValue::Int((rowh * d).round() as i32),
                                JValue::Bool(p.selectable),
                                JValue::Bool(p.reorderable),
                                JValue::Bool(p.deletable),
                                JValue::Object(&del_label),
                            ],
                        ))
                    });
                    LIST_NODE.with(|m| {
                        m.borrow_mut()
                            .insert(handle.0.as_obj().as_raw() as usize, id.0 as i64)
                    });
                    handle
                }
                Some(Builtin::Nav) => {
                    let Some(p) = day_spec::props_of::<NavProps>(kind, "android", props) else {
                        return realize_placeholder(kind);
                    };
                    // `Stack` in props is literal — a host that is a stack at EVERY size (a
                    // nested `stack()` under a split host, docs/size-classes.md) — so it gets a
                    // plain single-pane host. Only an adaptive host builds a SlidingPaneLayout;
                    // nesting one inside a pane re-runs the whole tiling decision at pane width.
                    let adaptive = p.presentation != day_spec::props::NavPresentation::Stack;
                    let host = with_env(|env| {
                        let s = jstr(env, &p.title);
                        AHandle(make_view(
                            env,
                            "makeNavHost",
                            "(JLjava/lang/String;ZF)Landroid/view/View;",
                            &[
                                JValue::Long(idj),
                                JValue::Object(&s),
                                JValue::Bool(adaptive),
                                // The tiling threshold comes from Day's breakpoint table, so
                                // the platform's measure-time answer agrees with it.
                                JValue::Float(day_spec::SizeClass::SPLIT_MIN_WIDTH as f32),
                            ],
                        ))
                    });
                    // Optional trailing bar action (docs/navigation.md): set on the host AFTER it
                    // exists, via a best-effort call that SWALLOWS any failure. A bar button is
                    // decoration; keeping it off `makeNavHost` (whose failure aborts the whole
                    // native tree build — `make_view` unwraps) means nothing here can blank the app.
                    // On tap the item re-enters as `MenuAction(id)`.
                    if let Some(a) = &p.bar_action {
                        let (icon, label, action) = (
                            a.icon.clone().unwrap_or_default(),
                            a.label.clone(),
                            a.action as i64,
                        );
                        with_env(|env| {
                            let ic = jstr(env, &icon);
                            let lb = jstr(env, &label);
                            let _ = env.dcall_static(
                                BRIDGE,
                                "setNavMenu",
                                "(Landroid/view/View;Ljava/lang/String;Ljava/lang/String;J)V",
                                &[
                                    JValue::Object(host.0.as_obj()),
                                    JValue::Object(&ic),
                                    JValue::Object(&lb),
                                    JValue::Long(action),
                                ],
                            );
                            // A throw (or an old bridge lacking the method) leaves a pending
                            // exception — clear it so it can't poison the next JNI call.
                            if env.exception_check() {
                                env.exception_clear();
                            }
                        });
                    }
                    // Inline search (docs/search.md), same best-effort discipline as the bar
                    // action above and for the same reason: a throw on the host's own build path
                    // blanks the app, so the field goes on afterwards or not at all.
                    if let Some(sp) = p
                        .search
                        .as_ref()
                        .filter(|sp| sp.placement == day_spec::props::SearchPlacement::Inline)
                    {
                        let (prompt, text) = (sp.prompt.clone(), sp.text.clone());
                        with_env(|env| {
                            let pr = jstr(env, &prompt);
                            let tx = jstr(env, &text);
                            let _ = env.dcall_static(
                                BRIDGE,
                                "setNavSearch",
                                "(Landroid/view/View;JLjava/lang/String;Ljava/lang/String;)V",
                                &[
                                    JValue::Object(host.0.as_obj()),
                                    JValue::Long(idj),
                                    JValue::Object(&pr),
                                    JValue::Object(&tx),
                                ],
                            );
                            if env.exception_check() {
                                env.exception_clear();
                            }
                        });
                    }
                    host
                }
                Some(Builtin::NavPage) => with_env(|env| {
                    AHandle(make_view(
                        env,
                        "makeNavPage",
                        "(J)Landroid/view/View;",
                        &[JValue::Long(idj)],
                    ))
                }),
                // Fullscreen cover (docs/cover.md): a DayCover shell whose content pane is the
                // Day mount point; CoverPatch::Present re-homes it over the activity content.
                Some(Builtin::Cover) => with_env(|env| {
                    AHandle(make_view(
                        env,
                        "makeCover",
                        "(J)Landroid/view/View;",
                        &[JValue::Long(idj)],
                    ))
                }),
                Some(Builtin::Tabs) => {
                    let Some(p) = day_spec::props_of::<TabsProps>(kind, "android", props) else {
                        return realize_placeholder(kind);
                    };
                    with_env(|env| {
                        AHandle(make_view(
                            env,
                            "makeTabs",
                            "(JI)Landroid/view/View;",
                            &[JValue::Long(idj), JValue::Int(p.selected as i32)],
                        ))
                    })
                }
                Some(Builtin::TabsPage) => {
                    let Some(p) = day_spec::props_of::<TabsPageProps>(kind, "android", props)
                    else {
                        return realize_placeholder(kind);
                    };
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
                Some(Builtin::NavMenu) => {
                    let Some(p) = day_spec::props_of::<NavMenuProps>(kind, "android", props) else {
                        return realize_placeholder(kind);
                    };
                    let joined = p.items.join("\u{1f}");
                    // Parallel, index-aligned icon NAMES ("" = no icon for that row).
                    let joined_icons = p
                        .icons
                        .iter()
                        .map(|o| o.clone().unwrap_or_default())
                        .collect::<Vec<_>>()
                        .join("\u{1f}");
                    let joined_tints = nav_tints_joined(&p.tints);
                    let joined_badge_icons = p
                        .badge_icons
                        .iter()
                        .map(|o| o.clone().unwrap_or_default())
                        .collect::<Vec<_>>()
                        .join("\u{1f}");
                    let joined_badge_tints = nav_tints_joined(&p.badge_tints);
                    let joined_menus = nav_menus_joined(&p.menus);
                    with_env(|env| {
                        let s = jstr(env, &joined);
                        let si = jstr(env, &joined_icons);
                        let handle = AHandle(make_view(
                            env,
                            "makeNavMenu",
                            "(JLjava/lang/String;Ljava/lang/String;)Landroid/view/View;",
                            &[JValue::Long(idj), JValue::Object(&s), JValue::Object(&si)],
                        ));
                        // Per-row icon tints ride a best-effort follow-up (docs/vectors.md) —
                        // decoration stays OFF makeNavMenu's critical path, so a tint problem
                        // can never abort the tree build (the navhost lesson).
                        let st = jstr(env, &joined_tints);
                        let _ = env.dcall_static(
                            BRIDGE,
                            "setNavMenuTints",
                            "(Landroid/view/View;Ljava/lang/String;)V",
                            &[JValue::Object(handle.0.as_obj()), JValue::Object(&st)],
                        );
                        // The trailing status glyph (docs/navigation.md), same best-effort rule.
                        let sb = jstr(env, &joined_badge_icons);
                        let sbt = jstr(env, &joined_badge_tints);
                        let _ = env.dcall_static(
                            BRIDGE,
                            "setNavMenuBadges",
                            "(Landroid/view/View;Ljava/lang/String;Ljava/lang/String;)V",
                            &[
                                JValue::Object(handle.0.as_obj()),
                                JValue::Object(&sb),
                                JValue::Object(&sbt),
                            ],
                        );
                        // Per-row context menus (docs/menus.md): same best-effort follow-up.
                        let sm = jstr(env, &joined_menus);
                        let _ = env.dcall_static(
                            BRIDGE,
                            "setNavRowMenus",
                            "(Landroid/view/View;Ljava/lang/String;)V",
                            &[JValue::Object(handle.0.as_obj()), JValue::Object(&sm)],
                        );
                        if env.exception_check() {
                            env.exception_clear();
                        }
                        handle
                    })
                }
                Some(Builtin::Label) => {
                    let Some(p) = day_spec::props_of::<LabelProps>(kind, "android", props) else {
                        return realize_placeholder(kind);
                    };
                    let (sp, weight, italic, tabular) = font_params(p.font);
                    with_env(|env| {
                        let s = jstr(env, &p.text);
                        let view = make_view(
                            env,
                            "makeLabel",
                            "(Ljava/lang/String;)Landroid/view/View;",
                            &[JValue::Object(&s)],
                        );
                        // A whole-label monospace ask rides the family parameter: "monospace" is
                        // Android's own alias for the system fixed-pitch face, so this is the same
                        // request `TypefaceSpan("monospace")` makes for a single run.
                        let fam = match (custom_family(p.font), p.font.monospace) {
                            (Some(f), _) => JObject::from(jstr(env, f)),
                            (None, true) => JObject::from(jstr(env, "monospace")),
                            (None, false) => JObject::null(),
                        };
                        let _ = env.dcall_static(
                            BRIDGE,
                            "setLabelFont",
                            "(Landroid/view/View;FIZLjava/lang/String;Z)V",
                            &[
                                JValue::Object(view.as_obj()),
                                JValue::Float(sp),
                                JValue::Int(weight),
                                JValue::Bool(italic),
                                JValue::Object(&fam),
                                JValue::Bool(tabular),
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
                        // A label patch carries no node id, and a link's ClickableSpan needs one
                        // to report through — so remember it here, where the id is in hand.
                        LABEL_NODE
                            .with(|m| m.borrow_mut().insert(view.as_obj().as_raw() as usize, idj));
                        if !p.runs.is_empty() {
                            set_label_runs(env, idj, &view, &p.text, &p.runs);
                        }
                        AHandle(view)
                    })
                }
                Some(Builtin::Button) => {
                    let Some(p) = day_spec::props_of::<ButtonProps>(kind, "android", props) else {
                        return realize_placeholder(kind);
                    };
                    with_env(|env| {
                        let s = jstr(env, &p.title);
                        let v = make_view(
                            env,
                            "makeButton",
                            "(JLjava/lang/String;)Landroid/view/View;",
                            &[JValue::Long(idj), JValue::Object(&s)],
                        );
                        apply_button_style(env, &v, p.style);
                        AHandle(v)
                    })
                }
                Some(Builtin::Toggle) => {
                    let Some(p) = day_spec::props_of::<ToggleProps>(kind, "android", props) else {
                        return realize_placeholder(kind);
                    };
                    with_env(|env| {
                        AHandle(make_view(
                            env,
                            "makeToggle",
                            "(JZZ)Landroid/view/View;",
                            &[
                                JValue::Long(idj),
                                JValue::Bool(p.on),
                                JValue::Bool(p.enabled),
                            ],
                        ))
                    })
                }
                Some(Builtin::Slider) => {
                    let Some(p) = day_spec::props_of::<SliderProps>(kind, "android", props) else {
                        return realize_placeholder(kind);
                    };
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
                Some(Builtin::Picker) => crate::picker::realize_any(self, props, id),
                Some(Builtin::TextArea) => crate::textarea::realize_any(self, props, id),
                Some(Builtin::TextField) => {
                    let Some(p) = day_spec::props_of::<TextFieldProps>(kind, "android", props)
                    else {
                        return realize_placeholder(kind);
                    };
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
                Some(Builtin::Divider) => with_env(|env| {
                    AHandle(make_view(env, "makeDivider", "()Landroid/view/View;", &[]))
                }),
                Some(Builtin::Progress) => {
                    let Some(p) = day_spec::props_of::<ProgressProps>(kind, "android", props)
                    else {
                        return realize_placeholder(kind);
                    };
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
                Some(Builtin::Canvas) => with_env(|env| {
                    AHandle(make_view(env, "makeCanvas", "()Landroid/view/View;", &[]))
                }),
                Some(Builtin::Image) => {
                    let Some(p) = day_spec::props_of::<ImageProps>(kind, "android", props) else {
                        return realize_placeholder(kind);
                    };
                    // Scaling (§18.3): 0=fit (FIT_CENTER), 1=fill (CENTER_CROP), 2=stretch (FIT_XY).
                    let mode = match p.content_mode {
                        ContentMode::Fit => 0,
                        ContentMode::Fill => 1,
                        ContentMode::Stretch => 2,
                    };
                    // Vector-glyph tint (docs/vectors.md) as ARGB; 0 = none (a real tint always
                    // has alpha 0xFF, so 0 is unambiguous).
                    let tint = p.tint.map(argb_i32).unwrap_or(0);
                    with_env(|env| {
                        let s = jstr(env, &p.source);
                        AHandle(make_view(
                            env,
                            "makeImage",
                            "(Ljava/lang/String;II)Landroid/view/View;",
                            &[JValue::Object(&s), JValue::Int(mode), JValue::Int(tint)],
                        ))
                    })
                }
                // A recycled list cell is ADOPTED from the native list, never realized
                // through this path; anything else is an extension piece.
                Some(Builtin::ListCell) | None => {
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
                kinds::IMAGE => {
                    if let Some(day_spec::props::ImagePatch::Tint(c)) =
                        patch.downcast_ref::<day_spec::props::ImagePatch>()
                    {
                        // Drawable tint, as at realize (docs/vectors.md); 0 = authored colors.
                        let tint = c.map(argb_i32).unwrap_or(0);
                        with_env(|env| {
                            let _ = env.dcall_static(
                                BRIDGE,
                                "setImageTint",
                                "(Landroid/view/View;I)V",
                                &[JValue::Object(&h.0), JValue::Int(tint)],
                            );
                        });
                    }
                }
                kinds::CONTAINER => {
                    if let Some(ContainerPatch::Background(c)) =
                        patch.downcast_ref::<ContainerPatch>()
                    {
                        apply_surface(h, *c, 0.0, false);
                    }
                }
                kinds::NAV_MENU => {
                    match patch.downcast_ref::<NavMenuPatch>() {
                        // A data-driven `.items(signal, …)` block re-derived: rebuild the native
                        // rows so each click listener reports its CURRENT index — stale rows
                        // shift every selection after a removed item by one and drop the last
                        // row's selection entirely.
                        Some(NavMenuPatch::Items {
                            items,
                            icons,
                            tints,
                            menus,
                            badge_icons,
                            badge_tints,
                            ..
                        }) => {
                            let joined = items.join("\u{1f}");
                            let joined_icons = icons
                                .iter()
                                .map(|o| o.clone().unwrap_or_default())
                                .collect::<Vec<_>>()
                                .join("\u{1f}");
                            let joined_tints = nav_tints_joined(tints);
                            let joined_badge_icons = badge_icons
                                .iter()
                                .map(|o| o.clone().unwrap_or_default())
                                .collect::<Vec<_>>()
                                .join("\u{1f}");
                            let joined_badge_tints = nav_tints_joined(badge_tints);
                            let joined_menus = nav_menus_joined(menus);
                            with_env(|env| {
                                let s = jstr(env, &joined);
                                let si = jstr(env, &joined_icons);
                                let _ = env.dcall_static(
                                    BRIDGE,
                                    "updateNavMenu",
                                    "(Landroid/view/View;Ljava/lang/String;Ljava/lang/String;)V",
                                    &[
                                        JValue::Object(h.0.as_obj()),
                                        JValue::Object(&s),
                                        JValue::Object(&si),
                                    ],
                                );
                                let st = jstr(env, &joined_tints);
                                let _ = env.dcall_static(
                                    BRIDGE,
                                    "setNavMenuTints",
                                    "(Landroid/view/View;Ljava/lang/String;)V",
                                    &[JValue::Object(h.0.as_obj()), JValue::Object(&st)],
                                );
                                let sb = jstr(env, &joined_badge_icons);
                                let sbt = jstr(env, &joined_badge_tints);
                                let _ = env.dcall_static(
                                    BRIDGE,
                                    "setNavMenuBadges",
                                    "(Landroid/view/View;Ljava/lang/String;Ljava/lang/String;)V",
                                    &[
                                        JValue::Object(h.0.as_obj()),
                                        JValue::Object(&sb),
                                        JValue::Object(&sbt),
                                    ],
                                );
                                let sm = jstr(env, &joined_menus);
                                let _ = env.dcall_static(
                                    BRIDGE,
                                    "setNavRowMenus",
                                    "(Landroid/view/View;Ljava/lang/String;)V",
                                    &[JValue::Object(h.0.as_obj()), JValue::Object(&sm)],
                                );
                                if env.exception_check() {
                                    env.exception_clear();
                                }
                            });
                        }
                        // Mobile selection is transient (rows ripple, then push) — no highlight
                        // to sync.
                        Some(NavMenuPatch::Selected(_)) | None => {}
                    }
                }
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
                    // Inline search (docs/search.md): the app writing its query patches the live
                    // field, so a sync never rebuilds it or takes the insertion point. The Java
                    // side guards the echo while it writes.
                    if let Some(day_spec::props::SearchPatch::Text(t)) =
                        patch.downcast_ref::<day_spec::props::SearchPatch>()
                    {
                        let text = t.clone();
                        with_env(|env| {
                            let tx = jstr(env, &text);
                            let _ = env.dcall_static(
                                BRIDGE,
                                "setNavSearchText",
                                "(Landroid/view/View;Ljava/lang/String;)V",
                                &[JValue::Object(h.0.as_obj()), JValue::Object(&tx)],
                            );
                            if env.exception_check() {
                                env.exception_clear();
                            }
                        });
                    }
                    if let Some(p) = patch.downcast_ref::<NavPatch>() {
                        match p {
                            NavPatch::Pushed { title, immersive } => with_env(|env| {
                                let s = jstr(env, title);
                                let _ = env.dcall_static(
                                    BRIDGE,
                                    "navPush",
                                    "(Landroid/view/View;Ljava/lang/String;Z)V",
                                    &[
                                        JValue::Object(h.0.as_obj()),
                                        JValue::Object(&s),
                                        JValue::Bool(*immersive),
                                    ],
                                );
                            }),
                            NavPatch::Popped => call_void(
                                "navPop",
                                "(Landroid/view/View;)V",
                                &[JValue::Object(h.0.as_obj())],
                            ),
                            NavPatch::Title(t) => with_env(|env| {
                                let s = jstr(env, t);
                                let _ = env.dcall_static(
                                    BRIDGE,
                                    "navSetTitle",
                                    "(Landroid/view/View;Ljava/lang/String;)V",
                                    &[JValue::Object(h.0.as_obj()), JValue::Object(&s)],
                                );
                            }),
                            NavPatch::GuardTop(on) => with_env(|env| {
                                let _ = env.dcall_static(
                                    BRIDGE,
                                    "navSetGuard",
                                    "(Landroid/view/View;Z)V",
                                    &[JValue::Object(h.0.as_obj()), JValue::Bool(*on)],
                                );
                            }),
                            // Unreachable: this backend answers `Cap::NavRepresent =
                            // Unsupported`, so the pieces layer never sends it. The plan for
                            // Android is `SlidingPaneLayout`, which decides at measure time and
                            // is OBSERVED rather than told (docs/size-classes.md).
                            NavPatch::Presentation(_) => {}
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
                                let (sp, weight, italic, tabular) = font_params(*f);
                                let family = custom_family(*f);
                                with_env(|env| {
                                    let fam = match family {
                                        Some(name) => JObject::from(jstr(env, name)),
                                        None => JObject::null(),
                                    };
                                    let _ = env.dcall_static(
                                        BRIDGE,
                                        "setLabelFont",
                                        "(Landroid/view/View;FIZLjava/lang/String;Z)V",
                                        &[
                                            JValue::Object(h.0.as_obj()),
                                            JValue::Float(sp),
                                            JValue::Int(weight),
                                            JValue::Bool(italic),
                                            JValue::Object(&fam),
                                            JValue::Bool(tabular),
                                        ],
                                    );
                                });
                            }
                            LabelPatch::Runs(text, runs) => {
                                let node = LABEL_NODE.with(|m| {
                                    m.borrow()
                                        .get(&(h.0.as_obj().as_raw() as usize))
                                        .copied()
                                        .unwrap_or(0)
                                });
                                with_env(|env| set_label_runs(env, node, &h.0, text, runs))
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
                            ButtonPatch::Style(s) => {
                                with_env(|env| apply_button_style(env, &h.0, *s))
                            }
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
                        // Posts smoothScrollToPosition(count-1) on the RecyclerView (no-op if empty).
                        call_void(
                            "listScrollToEnd",
                            "(Landroid/view/View;)V",
                            &[JValue::Object(h.0.as_obj())],
                        );
                    }
                    Some(ListPatch::ScrollToRow(row)) => {
                        call_void(
                            "listScrollToRow",
                            "(Landroid/view/View;I)V",
                            &[JValue::Object(h.0.as_obj()), JValue::Int(*row as i32)],
                        );
                    }
                    // Not implemented: RowSizeInvalidated (the adapter re-measures on the next
                    // notifyDataSetChanged) and Selected (no programmatic selection sync yet).
                    Some(ListPatch::RowSizeInvalidated(_))
                    | Some(ListPatch::Selected(_))
                    | None => {}
                },
                _ => {
                    if let Some(update) = self.registry.get(kind).map(|r| r.update) {
                        update(self, h, patch);
                    }
                }
            }
        }

        /// Offer a satellite piece its teardown hook before `release` frees the handle (§15.2).
        fn release_piece(&mut self, kind: day_spec::PieceKind, h: &Self::Handle) {
            // Copy the fn pointer out first: the registry lookup borrows `self` immutably and
            // the hook needs it mutably.
            let f = self.registry.get(kind).and_then(|r| r.release);
            if let Some(f) = f {
                f(self, h);
            }
        }
        fn release(&mut self, h: AHandle) {
            // A released window root drops its SECONDARY record (docs/windows.md teardown;
            // the activity itself already finished or is finishing).
            with_env(|env| {
                SECONDARY.with(|s| {
                    s.borrow_mut().retain(|(_, gref)| {
                        !env.is_same_object(h.0.as_obj(), gref.as_obj())
                            .unwrap_or(false)
                    })
                });
            });
            let key = h.0.as_obj().as_raw() as usize;
            // One sweep drops this view's entry from every registered `SideTable` — present
            // and future — so ptr-keyed side state cannot outlive the handle
            // (day_spec::sidetable; the maps below predate it and stay manual).
            day_spec::sidetable::sweep(key);
            LABEL_NODE.with(|m| m.borrow_mut().remove(&key));
            if let Some(nid) = LIST_NODE.with(|m| m.borrow_mut().remove(&key)) {
                LIST_SOURCES.with(|m| {
                    m.borrow_mut().remove(&nid);
                });
                // Drop the list's per-cell GlobalRefs with it (see LIST_CELLS): each Arc drop
                // releases its JNI global ref, keeping long sessions off the JNI table limit.
                LIST_CELLS.with(|m| {
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
                                .and_then(|v| v.i())
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

        /// `View.getBaseline()` — the same answer `LinearLayout`'s `baselineAligned` uses
        /// (docs/baseline.md). TextView and everything built on it override it; the base View
        /// returns -1, which is "no baseline".
        fn first_baseline(&mut self, h: &AHandle, kind: PieceKind, size: Size) -> Option<f64> {
            if !day_spec::kind_has_baseline(kind) {
                return None;
            }
            let d = density();
            let px = with_env(|env| {
                env.dcall_static(
                    BRIDGE,
                    "baselineAt",
                    "(Landroid/view/View;II)I",
                    &[
                        JValue::Object(h.0.as_obj()),
                        JValue::Int((size.width * d).round() as i32),
                        JValue::Int((size.height * d).round() as i32),
                    ],
                )
                .ok()?
                .i()
                .ok()
            })?;
            (px >= 0).then(|| px as f64 / d)
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

        fn set_selectable(&mut self, h: &AHandle, selectable: bool) -> Option<AHandle> {
            // A plain label is an android.widget.TextView; make its text selectable (long-press →
            // copy, docs/text.md). A direct instance call — no DayBridge method needed.
            with_env(|env| {
                let _ = env.dcall(
                    h.0.as_obj(),
                    "setTextIsSelectable",
                    "(Z)V",
                    &[JValue::Bool(selectable)],
                );
            });
            None
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
                    // Only fails when the JNI global-ref table is exhausted (a process-fatal
                    // leak elsewhere); there is no cell to hand back without the ref.
                    env.new_global_ref(&obj).expect("adopt: global ref"),
                ))
            })
        }

        fn open_window(
            &mut self,
            id: NodeId,
            options: &day_spec::WindowOptions,
            kind: day_spec::WindowKind,
        ) -> day_spec::WindowOpenReply<AHandle> {
            // Preferences stay modal on mobile (docs/windows.md) — the cover fallback is
            // the platform settings idiom; Normal windows become document activities.
            if kind == day_spec::WindowKind::Preferences {
                return day_spec::WindowOpenReply::Unsupported;
            }
            let ok = with_env(|env| {
                let Ok(title) = env.new_string(&options.title) else {
                    return false;
                };
                env.dcall_static(
                    BRIDGE,
                    "openWindow",
                    "(JLjava/lang/String;)V",
                    &[
                        JValue::Long(id.0 as i64),
                        JValue::Object(&JObject::from(title)),
                    ],
                )
                .is_ok()
            });
            if ok {
                day_spec::WindowOpenReply::Pending
            } else {
                day_spec::WindowOpenReply::Unsupported
            }
        }

        fn close_window(&mut self, host: &AHandle) {
            with_env(|env| {
                if let Some(node) = secondary_node_of(env, host) {
                    let _ = env.dcall_static(
                        BRIDGE,
                        "closeWindow",
                        "(J)V",
                        &[JValue::Long(node as i64)],
                    );
                }
            });
        }

        fn focus_window(&mut self, host: &AHandle) {
            with_env(|env| {
                if let Some(node) = secondary_node_of(env, host) {
                    let _ = env.dcall_static(
                        BRIDGE,
                        "focusWindow",
                        "(J)V",
                        &[JValue::Long(node as i64)],
                    );
                }
            });
        }

        fn set_window_title(&mut self, host: &AHandle, title: &str) {
            with_env(|env| {
                if let Some(node) = secondary_node_of(env, host) {
                    let Ok(jtitle) = env.new_string(title) else {
                        return;
                    };
                    let _ = env.dcall_static(
                        BRIDGE,
                        "setWindowTitle",
                        "(JLjava/lang/String;)V",
                        &[
                            JValue::Long(node as i64),
                            JValue::Object(&JObject::from(jtitle)),
                        ],
                    );
                }
            });
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
                // Allocation failure (OOM-class) skips the frame instead of panicking out
                // of a JNI up-call (which aborts); the next replay redraws in full.
                let Ok(arr) = env.new_double_array(nums.len()) else {
                    return;
                };
                if arr.set_region(env, 0, &nums).is_err() {
                    return;
                }
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
            android_window_image(false)
        }

        /// The decor view rather than the content view — the window with its action bar and
        /// system-bar backgrounds (docs/window-image.md).
        fn snapshot_window_chrome(&mut self) -> Result<Vec<u8>, String> {
            android_window_image(true)
        }

        /// The system color mode, DAY_THEME override first (themed capture runs).
        fn dark_mode(&mut self) -> bool {
            match std::env::var("DAY_THEME").ok().as_deref() {
                Some("dark") => return true,
                Some("light") => return false,
                _ => {}
            }
            with_env(|env| {
                env.dcall_static(
                    "dev/daybrite/day/bridge/DayBridge",
                    "isDarkMode",
                    "()Z",
                    &[],
                )
                .and_then(|v| v.z())
                .unwrap_or(false)
            })
        }

        /// Whether native transitions have settled (dayscript screenshots wait on this):
        /// currently the cover slide — a capture mid-present/mid-dismiss shows a half-slid
        /// surface (DayBridge.uiIdle / DayCover.slidesInFlight).
        fn ui_idle(&mut self) -> bool {
            with_env(|env| {
                env.dcall_static("dev/daybrite/day/bridge/DayBridge", "uiIdle", "()Z", &[])
                    .and_then(|v| v.z())
                    .unwrap_or(true)
            })
        }
    }

    /// Navigation persistence backed by the Activity's saved instance state (docs/navigation.md).
    /// The map lives on the Java side (`DayBridge.navState`) because that is where the platform
    /// hands it out and takes it back — DayActivity restores it in `onCreate` before native
    /// starts, and writes it into the outgoing Bundle in `onSaveInstanceState`. Its lifetime is
    /// the TASK: a process the system reclaims comes back on the page the user left, while a
    /// cold launch, or a task the user swiped off Recents, starts clean.
    struct InstanceNavStore;

    impl day_core::NavStore for InstanceNavStore {
        fn load(&self, key: &str) -> Option<String> {
            if !vm_ready() {
                return None;
            }
            with_env(|env| {
                let k = jstr(env, key);
                let obj = env
                    .dcall_static(
                        BRIDGE,
                        "navLoad",
                        "(Ljava/lang/String;)Ljava/lang/String;",
                        &[JValue::Object(&k)],
                    )
                    .ok()?
                    .l()
                    .ok()?;
                if obj.is_null() {
                    return None;
                }
                env.dstr(&as_jstring(obj)).ok()
            })
        }

        fn save(&self, key: &str, value: &str) {
            if !vm_ready() {
                return;
            }
            with_env(|env| {
                let k = jstr(env, key);
                let v = jstr(env, value);
                let _ = env.dcall_static(
                    BRIDGE,
                    "navSave",
                    "(Ljava/lang/String;Ljava/lang/String;)V",
                    &[JValue::Object(&k), JValue::Object(&v)],
                );
            });
        }
    }

    impl Platform for Android {
        const TARGET: &'static str = "android-mdc";
        const TOOLKIT: &'static str = "mdc";

        fn run(self, _options: WindowOptions, ready: Box<dyn FnOnce(Self, AHandle, Size)>) {
            // The ActivityThread owns the loop; init() already registered the root.
            let (root, size) = ROOT
                .with(|r| r.borrow_mut().take())
                .expect("day-android: init() not called");
            // Navigation restore is the platform's own instance-state contract here, not an app
            // opt-in (docs/navigation.md): every Android app is expected to come back where the
            // user left it after the system reclaims its process. Installed before `ready` builds
            // the tree, so the first build of a `.restore(key)` surface reads it; an app that
            // installs its own store later still replaces this one.
            day_core::set_nav_store(std::rc::Rc::new(InstanceNavStore));
            ready(self, root, size);
        }

        fn locale_hints(&self) -> Vec<String> {
            // The device's ordered language preference, which is the ambient locale Day
            // negotiates its catalogs against (§12.2, docs/localization.md). Comma-joined on the
            // Java side because a `String[]` return would need array marshaling for a list that
            // is never more than a handful of tags.
            if !vm_ready() {
                return Vec::new();
            }
            let joined = with_env(|env| {
                let obj = env
                    .dcall_static(BRIDGE, "localeTags", "()Ljava/lang/String;", &[])
                    .ok()?
                    .l()
                    .ok()?;
                if obj.is_null() {
                    return None;
                }
                env.dstr(&as_jstring(obj)).ok()
            });
            joined
                .unwrap_or_default()
                .split(',')
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(str::to_string)
                .collect()
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
