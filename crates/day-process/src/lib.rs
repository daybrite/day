// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! What a Day process has before, and without, a user interface: its identity and directories,
//! the services that start with it, native notification delivery, and the panic guard every
//! foreign entry point runs through.
//!
//! Nothing here depends on the widget tree, a toolkit or the reactive runtime, so a background
//! worker, a push handler, an app extension or a command-line tool links this crate alone.
//! `day-spec` and `day-core` build on it.
//!
//! Exactly one copy of this crate may be linked into a process: the registries below are
//! link-time slices, and a second copy would have its own, which the host never runs. The
//! manifest's `links` key makes Cargo refuse a dependency graph that resolves two.

pub use linkme;

/// How fully a platform provides something a caller asks about.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Support {
    Native,
    Emulated,
    /// The default: what an unimplemented capability answers everywhere.
    #[default]
    Unsupported,
}

/// Containment for FFI entry points: native callbacks, JNI up-calls, posted-closure
/// trampolines. A Rust panic unwinding out of an `extern "C"` frame is undefined
/// behavior (an abort), so every backend trampoline runs its body through
/// [`ffi_guard::contain`]: a caught panic is reported, the installed recovery hook
/// restores runtime coherence (day-core installs the reactive runtime's
/// `recover_from_panic` at boot), and the given default is returned.
pub mod ffi_guard {
    use std::sync::OnceLock;

    static RECOVERY: OnceLock<fn()> = OnceLock::new();

    /// Install the post-panic recovery hook (idempotent; first install wins). day-core
    /// registers `day_reactive::recover_from_panic` here so a contained panic cannot
    /// leave the reactive observer stack stranded.
    pub fn set_recovery(f: fn()) {
        let _ = RECOVERY.set(f);
    }

    /// Run `f`, containing any panic; on panic, report, run the recovery hook, and
    /// return `default`.
    pub fn contain<R>(default: R, f: impl FnOnce() -> R) -> R {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
            Ok(r) => r,
            Err(payload) => {
                let msg = payload
                    .downcast_ref::<&str>()
                    .map(std::string::ToString::to_string)
                    .or_else(|| payload.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "<non-string panic>".into());
                log::error!("panic contained at an FFI boundary: {msg}");
                if let Some(recover) = RECOVERY.get() {
                    recover();
                }
                default
            }
        }
    }
}

/// Process services that must initialize before a window or reactive tree exists.
///
/// Optional parts register initializers at link time. Initializers run on the native startup
/// thread once per process and must return promptly. They may register OS callbacks; they must
/// not access UI signals or mount widgets. Android services can initialize this registry after
/// attaching the JVM without starting an Activity.
pub mod services {
    use std::sync::Once;

    /// A process initializer supplied by an optional part.
    pub struct Service {
        pub name: &'static str,
        pub initialize: fn(),
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[linkme::distributed_slice]
    pub static SERVICES: [Service];

    /// Initialize linked process services. A native host calls this after its runtime is ready.
    pub fn initialize() {
        static ONCE: Once = Once::new();
        ONCE.call_once(|| {
            #[cfg(not(target_arch = "wasm32"))]
            for service in SERVICES {
                (service.initialize)();
            }
        });
    }

    /// Native notification delivery before or after a window exists. Handlers must return promptly
    /// and use a main-thread setter to update UI state. Payloads are JSON from the platform.
    #[derive(Clone)]
    pub enum NotificationEvent {
        Registered(Vec<u8>),
        RegistrationFailed(String),
        Received { payload: String, opened: bool },
    }

    impl std::fmt::Debug for NotificationEvent {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::Registered(_) => f.debug_tuple("Registered").finish_non_exhaustive(),
                Self::RegistrationFailed(error) => {
                    f.debug_tuple("RegistrationFailed").field(error).finish()
                }
                Self::Received { opened, .. } => f
                    .debug_struct("Received")
                    .field("opened", opened)
                    .finish_non_exhaustive(),
            }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[linkme::distributed_slice]
    pub static NOTIFICATION_HANDLERS: [fn(&NotificationEvent)];

    /// Forward a native delegate callback to linked optional parts.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "public signature; toolkits hand over the event they built"
    )]
    pub fn notification(event: NotificationEvent) {
        #[cfg(not(target_arch = "wasm32"))]
        for handler in NOTIFICATION_HANDLERS {
            handler(&event);
        }
        #[cfg(target_arch = "wasm32")]
        let _ = event;
    }
}

/// A route requested from outside the user interface, at any lifecycle stage, from any thread:
/// the rail a notification tap or a shortcut navigates through (docs/notify.md).
///
/// The request is buffered here, below the UI, so a part that receives a tap in a process with
/// no window yet (Android delivers one on a JNI thread, Apple on a delegate callback, and both can
/// arrive before the app has launched) records it without linking the navigation code. The UI
/// runtime takes the buffer once routes exist and installs a notifier to hear about later
/// requests. The newest request wins: a user who taps twice gets the second destination.
pub mod routes {
    use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};

    static REQUESTED: Mutex<Option<String>> = Mutex::new(None);
    static NOTIFIER: OnceLock<fn()> = OnceLock::new();

    fn slot() -> MutexGuard<'static, Option<String>> {
        // A panic while holding this lock would otherwise wedge every later navigation; the
        // buffer is one `Option<String>`, so recovering the value is always safe.
        REQUESTED.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Ask the app to navigate to `route`. An empty route is ignored.
    pub fn request(route: &str) {
        if route.is_empty() {
            return;
        }
        *slot() = Some(route.to_owned());
        if let Some(notify) = NOTIFIER.get() {
            notify();
        }
    }

    /// The buffered route, left in place.
    #[must_use]
    pub fn requested() -> Option<String> {
        slot().clone()
    }

    /// Take the buffered route, if any.
    #[must_use]
    pub fn take() -> Option<String> {
        slot().take()
    }

    /// Install the function called after each [`request`], on the requesting thread. The UI
    /// runtime installs one that applies the route on its own thread. The first install wins.
    pub fn set_notifier(notify: fn()) {
        let _ = NOTIFIER.set(notify);
    }
}

// ---------------------------------------------------------------------------
// App directories (docs/fs.md "Where files live")
// ---------------------------------------------------------------------------

/// Where an app keeps its own files: its data, cache and configuration directories, each a
/// folder named after the app id in the place its platform sets aside for that kind of file
/// (docs/fs.md "Where files live"). Every Day crate that stores files resolves them here, so an
/// app's database, documents, caches, settings file and remembered window frames sit together
/// and apart from every other app's.
///
/// These only resolve paths; nothing is created until something is written. Each is `None` where
/// the platform has no such directory: the web, or an Android or HarmonyOS process whose host
/// passed none.
pub mod storage {
    use std::path::PathBuf;

    /// The app's id, `[app] id` from Day.toml: `DAY_APP_ID` from `day launch`, else the value
    /// `day build` baked in, else the executable's name (a test binary or a bare `cargo run`).
    /// Made safe as a single folder name.
    #[must_use]
    pub fn app_id() -> String {
        std::env::var("DAY_APP_ID")
            .ok()
            .filter(|id| !id.is_empty())
            .or_else(|| {
                option_env!("DAY_APP_ID")
                    .filter(|id| !id.is_empty())
                    .map(str::to_owned)
            })
            .or_else(|| {
                let exe = std::env::current_exe().ok()?;
                exe.file_stem()?.to_str().map(str::to_owned)
            })
            .map_or_else(|| "day-app".to_owned(), |id| folder_name(&id))
    }

    /// The app's data directory, for what it cannot recreate: databases, documents, downloads
    /// the user kept. Application Support on Apple platforms (inside the container when
    /// sandboxed), `%LOCALAPPDATA%` on Windows, `$XDG_DATA_HOME` on Linux, the app's files
    /// directory on Android and HarmonyOS. `DAY_DATA_DIR` replaces it.
    #[must_use]
    pub fn data_dir() -> Option<PathBuf> {
        resolve(Dir::Data, Os::HOST, &app_id(), &env_path)
    }

    /// [`data_dir`] for the app `app_id` rather than this process's own: a host that keeps a
    /// folder per hosted app (a day-lite superapp, a crash reporter's store), or a tool finding
    /// where an app it launched writes. On Android and HarmonyOS, where the host's directory is
    /// already the one app's, the id changes nothing.
    #[must_use]
    pub fn data_dir_for(app_id: &str) -> Option<PathBuf> {
        resolve(Dir::Data, Os::HOST, &folder_name(app_id), &env_path)
    }

    /// The app's cache directory, for what it can fetch or compute again: the system may clear
    /// it, and backups skip it. Caches on Apple platforms, `%LOCALAPPDATA%\<id>\cache` on
    /// Windows, `$XDG_CACHE_HOME` on Linux, the app's cache directory on Android and HarmonyOS.
    /// `DAY_CACHE_DIR` replaces it; with only `DAY_DATA_DIR` set it is that directory's `cache`.
    #[must_use]
    pub fn cache_dir() -> Option<PathBuf> {
        resolve(Dir::Cache, Os::HOST, &app_id(), &env_path)
    }

    /// The app's configuration directory, for small settings files: Application Support on
    /// Apple platforms (whose preferences proper are the user defaults), the roaming
    /// `%APPDATA%` on Windows, `$XDG_CONFIG_HOME` on Linux. With `DAY_DATA_DIR` set (as the
    /// Android and HarmonyOS hosts do) it is that directory's `config`.
    #[must_use]
    pub fn config_dir() -> Option<PathBuf> {
        resolve(Dir::Config, Os::HOST, &app_id(), &env_path)
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Dir {
        Data,
        Cache,
        Config,
    }

    /// The platform families whose conventions differ.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Os {
        /// macOS and iOS: `~/Library`, which is the app's container when sandboxed.
        Apple,
        Windows,
        /// Android and HarmonyOS: the host passes the app's private directories.
        Hosted,
        /// Linux and the BSDs: the XDG base directories.
        Unix,
        Web,
    }

    impl Os {
        const HOST: Os = if cfg!(any(target_os = "macos", target_os = "ios")) {
            Os::Apple
        } else if cfg!(target_os = "windows") {
            Os::Windows
        } else if cfg!(any(target_os = "android", target_env = "ohos")) {
            Os::Hosted
        } else if cfg!(target_family = "wasm") {
            Os::Web
        } else {
            Os::Unix
        };
    }

    /// An environment variable naming a directory: set, non-empty and absolute, else ignored.
    fn env_path(name: &str) -> Option<PathBuf> {
        let path = PathBuf::from(std::env::var_os(name)?);
        path.is_absolute().then_some(path)
    }

    /// One folder name: no separators, no `.`/`..`, nothing a filesystem rejects.
    fn folder_name(id: &str) -> String {
        let name: String = id
            .chars()
            .map(|c| match c {
                '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
                c if c.is_control() => '_',
                c => c,
            })
            .collect();
        match name.trim() {
            "" | "." | ".." => "day-app".to_owned(),
            name => name.to_owned(),
        }
    }

    /// The pure resolution, over a platform and an environment, so every platform's rules are
    /// testable on any host.
    fn resolve(
        dir: Dir,
        os: Os,
        id: &str,
        var: &dyn Fn(&str) -> Option<PathBuf>,
    ) -> Option<PathBuf> {
        // The explicit overrides: a mobile host's own directories, or a test's scratch root.
        if dir == Dir::Cache
            && let Some(cache) = var("DAY_CACHE_DIR")
        {
            return Some(cache);
        }
        if let Some(data) = var("DAY_DATA_DIR") {
            return Some(match dir {
                Dir::Data => data,
                Dir::Cache => data.join("cache"),
                Dir::Config => data.join("config"),
            });
        }
        let home = || var("HOME");
        match os {
            // One expression serves sandboxed and unsandboxed apps: inside the App Sandbox,
            // and always on iOS, `HOME` is the container's Data folder, so the same Library
            // path lands inside it. Apple asks for a bundle-id folder within Application
            // Support and Caches either way. XDG variables are ignored here: a Finder launch
            // never sees a shell's, so honoring them would split one app's data across two
            // places depending on how it was started.
            Os::Apple => {
                let library = home()?.join("Library");
                let base = match dir {
                    Dir::Data | Dir::Config => library.join("Application Support"),
                    Dir::Cache => library.join("Caches"),
                };
                Some(base.join(id))
            }
            // Local for data and caches: a database or cache roams badly, and a DPAPI-sealed
            // file cannot be read on another machine. Roaming for the small settings files.
            Os::Windows => {
                let profile = |leaf: &str| Some(var("USERPROFILE")?.join("AppData").join(leaf));
                let local = || var("LOCALAPPDATA").or_else(|| profile("Local"));
                match dir {
                    Dir::Data => Some(local()?.join(id)),
                    Dir::Cache => Some(local()?.join(id).join("cache")),
                    Dir::Config => Some(var("APPDATA").or_else(|| profile("Roaming"))?.join(id)),
                }
            }
            Os::Unix => {
                let (xdg, fallback) = match dir {
                    Dir::Data => ("XDG_DATA_HOME", ".local/share"),
                    Dir::Cache => ("XDG_CACHE_HOME", ".cache"),
                    Dir::Config => ("XDG_CONFIG_HOME", ".config"),
                };
                Some(var(xdg).or_else(|| Some(home()?.join(fallback)))?.join(id))
            }
            Os::Hosted | Os::Web => None,
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::collections::HashMap;

        /// The resolved path as text. `Path::join` uses the host's separator, so on a Windows host
        /// another platform's path comes back with `\` between the joined parts; the expectations
        /// are that platform's own, so its separators are compared as `/`.
        fn at(os: Os, dir: Dir, vars: &[(&str, &str)]) -> Option<String> {
            let vars: HashMap<&str, &str> = vars.iter().copied().collect();
            let var = |name: &str| vars.get(name).map(PathBuf::from);
            resolve(dir, os, "dev.example.notes", &var).map(|p| {
                let text = p.display().to_string();
                if os == Os::Windows {
                    text
                } else {
                    text.replace('\\', "/")
                }
            })
        }

        #[test]
        fn apple_uses_library_folders_named_for_the_app() {
            let home = [("HOME", "/Users/ada"), ("XDG_DATA_HOME", "/Users/ada/.xdg")];
            let support = "/Users/ada/Library/Application Support/dev.example.notes";
            assert_eq!(at(Os::Apple, Dir::Data, &home).as_deref(), Some(support));
            assert_eq!(at(Os::Apple, Dir::Config, &home).as_deref(), Some(support));
            assert_eq!(
                at(Os::Apple, Dir::Cache, &home).as_deref(),
                Some("/Users/ada/Library/Caches/dev.example.notes")
            );
            // A sandboxed app's HOME is its container, so its folders land inside it.
            let container = [(
                "HOME",
                "/Users/ada/Library/Containers/dev.example.notes/Data",
            )];
            assert_eq!(
                at(Os::Apple, Dir::Data, &container).as_deref(),
                Some(
                    "/Users/ada/Library/Containers/dev.example.notes/Data/Library/Application Support/dev.example.notes"
                )
            );
        }

        #[test]
        fn windows_keeps_data_local_and_settings_roaming() {
            let vars = [
                ("LOCALAPPDATA", r"C:\Users\ada\AppData\Local"),
                ("APPDATA", r"C:\Users\ada\AppData\Roaming"),
            ];
            let data = at(Os::Windows, Dir::Data, &vars).unwrap_or_default();
            assert!(data.starts_with(r"C:\Users\ada\AppData\Local"), "{data}");
            assert!(data.ends_with("dev.example.notes"), "{data}");
            let cache = at(Os::Windows, Dir::Cache, &vars).unwrap_or_default();
            assert!(cache.ends_with("cache"), "{cache}");
            let config = at(Os::Windows, Dir::Config, &vars).unwrap_or_default();
            assert!(
                config.starts_with(r"C:\Users\ada\AppData\Roaming"),
                "{config}"
            );
        }

        #[test]
        fn unix_follows_xdg_then_home() {
            let xdg = [
                ("HOME", "/home/ada"),
                ("XDG_DATA_HOME", "/data"),
                ("XDG_CACHE_HOME", "/cache"),
                ("XDG_CONFIG_HOME", "/config"),
            ];
            assert_eq!(
                at(Os::Unix, Dir::Data, &xdg).as_deref(),
                Some("/data/dev.example.notes")
            );
            assert_eq!(
                at(Os::Unix, Dir::Cache, &xdg).as_deref(),
                Some("/cache/dev.example.notes")
            );
            assert_eq!(
                at(Os::Unix, Dir::Config, &xdg).as_deref(),
                Some("/config/dev.example.notes")
            );
            let home = [("HOME", "/home/ada")];
            assert_eq!(
                at(Os::Unix, Dir::Data, &home).as_deref(),
                Some("/home/ada/.local/share/dev.example.notes")
            );
            assert_eq!(
                at(Os::Unix, Dir::Cache, &home).as_deref(),
                Some("/home/ada/.cache/dev.example.notes")
            );
            assert_eq!(
                at(Os::Unix, Dir::Config, &home).as_deref(),
                Some("/home/ada/.config/dev.example.notes")
            );
        }

        #[test]
        fn hosts_and_tests_override_every_platform() {
            let host = [
                ("DAY_DATA_DIR", "/data/user/0/dev.example.notes/files"),
                ("DAY_CACHE_DIR", "/data/user/0/dev.example.notes/cache"),
            ];
            for os in [Os::Hosted, Os::Apple, Os::Windows, Os::Unix] {
                assert_eq!(
                    at(os, Dir::Data, &host).as_deref(),
                    Some("/data/user/0/dev.example.notes/files")
                );
                assert_eq!(
                    at(os, Dir::Cache, &host).as_deref(),
                    Some("/data/user/0/dev.example.notes/cache")
                );
            }
            let scratch = [("HOME", "/Users/ada"), ("DAY_DATA_DIR", "/tmp/run")];
            assert_eq!(
                at(Os::Apple, Dir::Cache, &scratch).as_deref(),
                Some("/tmp/run/cache")
            );
            assert_eq!(
                at(Os::Apple, Dir::Config, &scratch).as_deref(),
                Some("/tmp/run/config")
            );
            // Without a host directory there is nowhere to put anything.
            assert_eq!(at(Os::Hosted, Dir::Data, &[("HOME", "/")]), None);
            assert_eq!(at(Os::Web, Dir::Data, &[("HOME", "/")]), None);
        }

        #[test]
        fn app_ids_become_one_folder_name() {
            assert_eq!(folder_name("dev.daybrite.news"), "dev.daybrite.news");
            assert_eq!(folder_name("a/b\\c:d"), "a_b_c_d");
            assert_eq!(folder_name(".."), "day-app");
            assert_eq!(folder_name(""), "day-app");
        }
    }
}
