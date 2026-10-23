//! Typed, reactive preferences independent of a settings UI.

use day_reactive::{Signal, watch};
use std::{any::Any, cell::RefCell, collections::HashMap, rc::Rc};

/// A value a [`Pref`] can hold: encoded to one string in the store and back. Implemented for
/// the plain scalars and strings; enums get it from [`pref_enum!`](crate::pref_enum).
pub trait PrefValue: Clone + PartialEq + 'static {
    fn encode(&self) -> String;
    fn decode(s: &str) -> Option<Self>;
}

impl PrefValue for bool {
    fn encode(&self) -> String {
        if *self { "true" } else { "false" }.into()
    }
    fn decode(s: &str) -> Option<Self> {
        match s {
            "true" | "1" => Some(true),
            "false" | "0" => Some(false),
            _ => None,
        }
    }
}

macro_rules! prefvalue_via_str {
    ($($t:ty),* $(,)?) => {
        $(impl PrefValue for $t {
            fn encode(&self) -> String { self.to_string() }
            fn decode(s: &str) -> Option<Self> { s.parse().ok() }
        })*
    };
}
prefvalue_via_str!(i64, i32, u32, u64, f64, String);

/// Declare an enum that round-trips through the store under short stable tags:
///
/// ```ignore
/// pref_enum! {
///     /// Temperature unit.
///     pub enum Unit { Celsius = "c", Fahrenheit = "f" }
/// }
/// ```
///
/// The enum derives `Clone, Copy, PartialEq, Eq, Debug`, implements [`PrefValue`], and gets
/// a `const ALL: &[Self]` in declaration order (what a a choice row iterates).
#[macro_export]
macro_rules! pref_enum {
    ($(#[$meta:meta])* $vis:vis enum $name:ident { $($(#[$vmeta:meta])* $variant:ident = $tag:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        $vis enum $name { $($(#[$vmeta])* $variant),+ }
        impl $name {
            /// Every variant, in declaration order.
            pub const ALL: &'static [$name] = &[$($name::$variant),+];
            /// The tag this variant is stored under.
            pub const fn tag(self) -> &'static str {
                match self { $($name::$variant => $tag),+ }
            }
        }
        impl $crate::PrefValue for $name {
            fn encode(&self) -> String { self.tag().to_string() }
            fn decode(s: &str) -> Option<Self> {
                match s { $($tag => Some($name::$variant),)+ _ => None }
            }
        }
    };
}

/// Where values live (docs/settings.md). The default, [`NativeStore`], is day-part-prefs: the
/// platform's own preferences facility. An app installs another with [`install_store`] before
/// building the settings UI; tests use [`MemoryStore`].
pub trait Store: 'static {
    fn get(&self, key: &str) -> Option<String>;
    fn set(&self, key: &str, value: &str) -> bool;
    fn remove(&self, key: &str) -> bool;
}

/// The platform store through day-part-prefs (docs/prefs.md): NSUserDefaults, SharedPreferences,
/// a per-app config file, localStorage.
pub struct NativeStore;

impl Store for NativeStore {
    fn get(&self, key: &str) -> Option<String> {
        crate::get(key)
    }
    fn set(&self, key: &str, value: &str) -> bool {
        crate::set(key, value)
    }
    fn remove(&self, key: &str) -> bool {
        crate::remove(key)
    }
}

/// An in-process store that forgets everything at exit: tests, and deterministic script runs.
#[derive(Default)]
pub struct MemoryStore {
    map: RefCell<HashMap<String, String>>,
}

impl Store for MemoryStore {
    fn get(&self, key: &str) -> Option<String> {
        self.map.borrow().get(key).cloned()
    }
    fn set(&self, key: &str, value: &str) -> bool {
        self.map.borrow_mut().insert(key.into(), value.into());
        true
    }
    fn remove(&self, key: &str) -> bool {
        self.map.borrow_mut().remove(key).is_some()
    }
}

day_core::tls_group! {
    /// The installed store (`None` = [`NativeStore`]).
    static STORE: RefCell<Option<Rc<dyn Store>>> = const { RefCell::new(None) };
    /// One signal per pref key, created on first use and seeded from the store.
    static SIGNALS: RefCell<HashMap<&'static str, Box<dyn Any>>> = RefCell::new(HashMap::new());
    /// The keys whose last store write failed ([`persistence_failures`]).
    static PERSIST_FAILURES: std::cell::OnceCell<Signal<Vec<String>>> = const { std::cell::OnceCell::new() };
}

/// The keys whose last write the store refused (`Store::set`/`remove` returned false), in
/// key order; empty while every write has landed. A failed write is also logged. The
/// in-memory value stays what the user chose for this run; it will not survive a relaunch,
/// which is what an app watching this signal can say. A key leaves the list when its own next
/// write lands; another key's success never clears it.
pub fn persistence_failures() -> Signal<Vec<String>> {
    PERSIST_FAILURES.with(|c| {
        *c.get_or_init(|| day_reactive::Scope::detached().enter(|| Signal::new(Vec::new())))
    })
}

fn note_persistence(key: &'static str, ok: bool) {
    let sig = persistence_failures();
    let listed = sig.get_untracked().iter().any(|k| k == key);
    if ok {
        if listed {
            sig.update(|v| v.retain(|k| k != key));
        }
    } else {
        log::warn!(
            "day-part-prefs: the store refused to write `{key}`; the value will not survive a relaunch"
        );
        if !listed {
            sig.update(|v| {
                v.push(key.to_string());
                v.sort();
            });
        }
    }
}

/// Remove `key` from the store through the checked path: an absent key afterwards is success
/// (removing what was never stored is not a failure), a key still present is reported.
fn remove_checked(key: &'static str) {
    let ok = with_store(|s| {
        s.remove(key);
        s.get(key).is_none()
    });
    note_persistence(key, ok);
}

/// Replace the store every [`Pref`] reads and writes. Call before any pref is read (before
/// building the settings UI); signals already seeded keep their values.
pub fn install_store(store: impl Store) {
    STORE.with(|s| *s.borrow_mut() = Some(Rc::new(store)));
}

fn with_store<R>(f: impl FnOnce(&dyn Store) -> R) -> R {
    let store = STORE.with(|s| s.borrow().clone());
    match store {
        Some(s) => f(&*s),
        None => f(&NativeStore),
    }
}

/// A typed, persisted, reactive setting (docs/settings.md), declared once as a static:
///
/// ```ignore
/// pub static SNAP: Pref<bool> = Pref::new("sketch.snap", || true);
/// ```
///
/// The signal behind it is created on the UI thread on first use, seeded from the store (or
/// the default when the key is absent or unparsable), and lives for the process. `Pref` is
/// `Copy`, so a row or a closure takes it by value.
pub struct Pref<T: PrefValue> {
    key: &'static str,
    default: fn() -> T,
    /// An environment variable that, when set at launch, makes the stored value yield to the
    /// default for this run (`DAY_THEME`, `DAY_LOCALE`: the env-wins rule, docs/settings.md).
    env: Option<&'static str>,
}

impl<T: PrefValue> Clone for Pref<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T: PrefValue> Copy for Pref<T> {}

impl<T: PrefValue> Pref<T> {
    /// Declare a pref under `key` with a default. `default` is a function so the static holds
    /// no value that needs `Sync` (`|| String::new()`, `|| Unit::Celsius`).
    pub const fn new(key: &'static str, default: fn() -> T) -> Self {
        Pref {
            key,
            default,
            env: None,
        }
    }
    /// The env-wins rule: while `var` is set in the process environment the stored value is
    /// ignored for this run (the signal seeds from the default), so a `day launch --env` run
    /// and every CI variant loop stay deterministic whatever an earlier run persisted. Writes
    /// still persist.
    pub const fn env(mut self, var: &'static str) -> Self {
        self.env = Some(var);
        self
    }
    /// The store key.
    pub fn key(&self) -> &'static str {
        self.key
    }
    /// The default value.
    pub fn default(&self) -> T {
        (self.default)()
    }
    /// Whether the env-wins variable is set for this run.
    pub fn env_forced(&self) -> bool {
        self.env
            .is_some_and(|v| std::env::var_os(v).is_some_and(|s| !s.is_empty()))
    }
    /// The signal behind the pref: hand it to a control or a binding. Created and seeded on
    /// first call.
    pub fn signal(&self) -> Signal<T> {
        let key = self.key;
        if let Some(sig) = SIGNALS.with(|m| {
            m.borrow()
                .get(key)
                .and_then(|b| b.downcast_ref::<Signal<T>>().copied())
        }) {
            return sig;
        }
        let stored = if self.env_forced() {
            None
        } else {
            with_store(|s| s.get(key)).and_then(|s| T::decode(&s))
        };
        let sig = Signal::global(stored.unwrap_or_else(|| (self.default)()));
        SIGNALS.with(|m| {
            m.borrow_mut().insert(key, Box::new(sig));
        });
        // Every write persists, whichever path wrote it: `set`, a control bound to the
        // signal, an app writing `signal()` itself. The store holds only non-default values
        // (a value equal to the default removes the key), so `reset` and "set to the
        // default" agree and a changed default on upgrade applies to every user who never
        // chose otherwise.
        let default = self.default;
        day_reactive::Scope::root().enter(|| {
            watch(
                move || sig.get(),
                move |v, _| {
                    // The signal keeps the new value for this run whatever the store says;
                    // a failed write is logged and reported through `persistence_failures`,
                    // so an app can tell the user the choice will not survive a relaunch.
                    if *v == default() {
                        remove_checked(key);
                    } else {
                        note_persistence(key, with_store(|s| s.set(key, &v.encode())));
                    }
                },
            );
        });
        sig
    }
    /// Tracked read: subscribes the running computation.
    pub fn get(&self) -> T {
        self.signal().get()
    }
    /// Untracked read.
    pub fn get_untracked(&self) -> T {
        self.signal().get_untracked()
    }
    /// Write: the signal, which persists (see [`Self::signal`]). An unchanged value is a no-op.
    pub fn set(&self, value: T) {
        self.signal().set_if_changed(value);
    }
    /// Back to the default: the key leaves the store.
    pub fn reset(&self) {
        // The signal write persists through the watcher when the value changes; a value
        // already at its default writes nothing there, so the removal runs on its own,
        // through the same checked path (a stale stored value, a refused removal).
        self.set((self.default)());
        remove_checked(self.key);
    }
    /// Whether the current value equals the default.
    pub fn is_default(&self) -> bool {
        self.get_untracked() == (self.default)()
    }
    /// Run `f` on every later change (not on subscription), in the current reactive scope.
    pub fn watch(&self, f: impl Fn(&T) + 'static) {
        let sig = self.signal();
        watch(move || sig.get(), move |v, _| f(v));
    }
}

#[cfg(test)]
mod persistence_tests {
    use super::*;

    /// A store that refuses every write and keeps every removal: what a read-only or full
    /// disk looks like to a pref.
    struct RefusingStore {
        map: RefCell<HashMap<String, String>>,
    }

    impl Store for RefusingStore {
        fn get(&self, key: &str) -> Option<String> {
            self.map.borrow().get(key).cloned()
        }
        fn set(&self, _key: &str, _value: &str) -> bool {
            false
        }
        fn remove(&self, _key: &str) -> bool {
            false
        }
    }

    static STUCK: Pref<i64> = Pref::new("test.stuck", || 1);
    static FINE: Pref<i64> = Pref::new("test.fine", || 1);

    #[test]
    fn refused_writes_are_reported_per_key_and_reset_is_checked() {
        let map: HashMap<String, String> = [("test.stuck".to_string(), "7".to_string())]
            .into_iter()
            .collect();
        install_store(RefusingStore {
            map: RefCell::new(map),
        });
        // Seeded from the stale stored value, so `reset` has something to remove.
        assert_eq!(STUCK.get_untracked(), 7);
        assert!(persistence_failures().get_untracked().is_empty());
        // Resetting to the default: the removal is refused, and that is reported even though
        // the value is now the default (nothing for the signal watcher to write).
        STUCK.reset();
        day_reactive::flush_sync();
        assert_eq!(STUCK.get_untracked(), 1);
        assert_eq!(
            persistence_failures().get_untracked(),
            vec!["test.stuck".to_string()]
        );
        // Already at the default: `reset` still goes through the checked removal.
        STUCK.reset();
        day_reactive::flush_sync();
        assert_eq!(
            persistence_failures().get_untracked(),
            vec!["test.stuck".to_string()]
        );
        // Another key's write (here refused too) joins the list; it never clears the first.
        FINE.set(5);
        day_reactive::flush_sync();
        assert_eq!(
            persistence_failures().get_untracked(),
            vec!["test.fine".to_string(), "test.stuck".to_string()]
        );
        // A store that accepts again: each key leaves the list on its own next write only.
        install_store(MemoryStore::default());
        FINE.set(6);
        day_reactive::flush_sync();
        assert_eq!(
            persistence_failures().get_untracked(),
            vec!["test.stuck".to_string()]
        );
        STUCK.set(2);
        day_reactive::flush_sync();
        assert!(persistence_failures().get_untracked().is_empty());
    }
}
