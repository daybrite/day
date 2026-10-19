// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! day-piece-settings: the settings and About surface every Day app shares (docs/settings.md).
//!
//! Three layers, all toolkit-neutral:
//!
//! - **Values.** [`Pref<T>`]: a typed, persisted, reactive setting declared once as a static.
//!   Reads are signals the app can watch; writes land in the [`Store`] (the platform's own
//!   preferences store through day-part-prefs unless the app installs another).
//! - **Declaration.** [`Preferences`] holds an [`About`] and the [`Group`]s of [`Section`]s of
//!   rows ([`toggle`], [`choice`], [`slider`], [`stepper`], [`text`], [`action`], [`link`],
//!   [`navigate`], [`info`], [`note`], [`custom`]). Groups carry a localized title, an icon and
//!   a route id. Other crates contribute groups through a [`Provider`].
//! - **Presentation.** [`Preferences::install`] registers the Preferences window (desktop) or
//!   the navigation presentation (mobile: an About header, then the groups as a list that
//!   pushes one page per group), the About window, the Settings…/About/Check for Updates… menu
//!   items, and the `settings/<group>` string routes. The pieces are composed from ordinary Day
//!   pieces on every backend; a toolkit feature adds native rows and panels where the platform
//!   has a stronger idiom.
//!
//! ```ignore
//! // root(), after res::locales::install():
//! let prefs = day_piece_settings::Preferences::new()
//!     .about(About::from_manifest().link(res::str::about_website(), "https://daybrite.dev"))
//!     .group(day_piece_settings::general_group(res::locales::ALL))
//!     .install();
//! // … and in the nav, for the platforms with no menu bar:
//! nav(section)
//!     .items(move || prefs.nav_keys(Section::Settings), move |s| prefs.nav_item(*s))
//!     .destination(move |_: &Section| prefs.page())
//! ```

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use day_core::{AnyPiece, Piece, PieceVec};
use day_pieces::prelude::*;
use day_pieces::{Label, TextSource};
use day_reactive::{Signal, watch};
use day_spec::{Cap, Support};

day_pieces::glue_modules!(appkit, gtk, qt, uikit, mdc, arkui);

#[cfg(all(feature = "xaml", target_os = "windows"))]
#[path = "lib-xaml.rs"]
mod xaml_impl;

/// A `(tag, autonym)` locale table, the shape of a generated `res::locales::ALL`.
pub type LocaleTable = &'static [(&'static str, &'static str)];

/// What an app's `root()` and settings module import: the declaration types, the handle, the
/// row constructors as a module, and the General group.
pub mod prelude {
    pub use crate::{
        APPEARANCE, About, Appearance, Group, LANGUAGE, Pref, PrefValue, Preferences, Prefs,
        Presentation, Providers, Row, Section, appearance_row, general_group, language_row,
        pref_enum, rows,
    };
}

// ===========================================================================
// Values: Pref<T> over a Store
// ===========================================================================

/// A value a [`Pref`] can hold: encoded to one string in the store and back. Implemented for
/// the plain scalars and strings; enums get it from [`pref_enum!`].
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
/// a `const ALL: &[Self]` in declaration order (what a [`choice`] row iterates).
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
/// [`Preferences::install`]; tests use [`MemoryStore`].
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
        day_part_prefs::get(key)
    }
    fn set(&self, key: &str, value: &str) -> bool {
        day_part_prefs::set(key, value)
    }
    fn remove(&self, key: &str) -> bool {
        day_part_prefs::remove(key)
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
    /// The installed declaration, what the free functions ([`open`], [`open_at`]) reach.
    static INSTALLED: RefCell<Option<Rc<Installed>>> = const { RefCell::new(None) };
    /// Providers registered at run time ([`register_provider`]): the wasm path, and explicit
    /// registration anywhere.
    static RUNTIME_PROVIDERS: RefCell<Vec<&'static Provider>> = const { RefCell::new(Vec::new()) };
    /// Whether the General group's appearance/language watchers are installed (once per process).
    static GENERAL_WIRED: Cell<bool> = const { Cell::new(false) };
    /// The keys whose last store write failed ([`persistence_failures`]).
    static PERSIST_FAILURES: std::cell::OnceCell<Signal<Vec<String>>> = const { std::cell::OnceCell::new() };
    /// The locale the app started in (system or `--locale`), captured before any stored
    /// override applies: what the language picker's "System" entry restores.
    static SYSTEM_LOCALE: std::cell::OnceCell<String> = const { std::cell::OnceCell::new() };
    /// Sub-page builders by title, what a pushed `page:<title>` route builds.
    static SUBPAGES: RefCell<HashMap<String, Build>> = RefCell::new(HashMap::new());
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
            "day-piece-settings: the store refused to write `{key}`; the value will not survive a relaunch"
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
/// [`Preferences::install`]); signals already seeded keep their values.
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
/// pub static SNAP: Pref<bool> = Pref::new("sketch.snap", true);
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

// ===========================================================================
// Declaration: Preferences, Group, Section, rows
// ===========================================================================

/// A group's icon: a bundled vector/image (`res::vectors::prefs_weather`) or a standard glyph.
#[derive(Clone, Debug, PartialEq)]
pub enum GroupIcon {
    Image(day_spec::ImageName),
    Symbol(day_spec::Symbol),
}

impl From<day_spec::Symbol> for GroupIcon {
    fn from(s: day_spec::Symbol) -> Self {
        GroupIcon::Symbol(s)
    }
}
impl From<day_spec::ImageName> for GroupIcon {
    fn from(n: day_spec::ImageName) -> Self {
        GroupIcon::Image(n)
    }
}
impl From<day_spec::VectorName> for GroupIcon {
    fn from(n: day_spec::VectorName) -> Self {
        GroupIcon::Image(n.into())
    }
}

type Build = Rc<dyn Fn() -> AnyPiece>;
type Avail = Rc<dyn Fn() -> bool>;

/// One group of settings (docs/settings.md): a page on desktop (a toolbar tab, a dialog page,
/// a sidebar entry) and a row of the mobile root list that pushes the page.
#[derive(Clone)]
pub struct Group {
    id: &'static str,
    title: TextSource,
    subtitle: Option<TextSource>,
    icon: Option<GroupIcon>,
    sections: Vec<Section>,
    custom: Option<Build>,
    order: i32,
    available: Option<Avail>,
}

impl Group {
    /// A group with a route id (also its element id, `prefs-group-<id>`) and a localized title.
    pub fn new<M>(id: &'static str, title: impl IntoText<M>) -> Self {
        Group {
            id,
            title: title.into_text(),
            subtitle: None,
            icon: None,
            sections: Vec::new(),
            custom: None,
            order: 0,
            available: None,
        }
    }
    /// The glyph beside the title: a bundled vector (the same name a `nav` item takes) or a
    /// standard [`day_spec::Symbol`].
    pub fn icon(mut self, icon: impl Into<GroupIcon>) -> Self {
        self.icon = Some(icon.into());
        self
    }
    /// One line under the title in the mobile root list ("Units, data source").
    pub fn subtitle<M>(mut self, t: impl IntoText<M>) -> Self {
        self.subtitle = Some(t.into_text());
        self
    }
    /// Append a section.
    pub fn section(mut self, section: Section) -> Self {
        self.sections.push(section);
        self
    }
    /// Append a row to the group's trailing untitled section (created as needed).
    pub fn row(mut self, row: Row) -> Self {
        match self.sections.last_mut() {
            Some(s) if s.title.is_none() && s.footer.is_none() => s.rows.push(row),
            _ => self.sections.push(Section::new().row(row)),
        }
        self
    }
    /// A bespoke piece that is the whole group page (Skies' city editor). Sections, if any,
    /// render above it.
    pub fn custom<P: Piece>(mut self, build: impl Fn() -> P + 'static) -> Self {
        self.custom = Some(Rc::new(move || AnyPiece::new(build())));
        self
    }
    /// Sort key: app groups keep declaration order at 0, providers default to 500, About is
    /// always last.
    pub fn order(mut self, order: i32) -> Self {
        self.order = order;
        self
    }
    /// Hide the whole group when false (evaluated at install and at each build).
    pub fn available(mut self, f: impl Fn() -> bool + 'static) -> Self {
        self.available = Some(Rc::new(f));
        self
    }
    /// Hide the group unless the toolkit supports `cap`.
    pub fn requires(self, cap: Cap) -> Self {
        self.available(move || day_core::capability(cap) != Support::Unsupported)
    }
    /// The route id.
    pub fn id(&self) -> &'static str {
        self.id
    }
    fn is_available(&self) -> bool {
        self.available.as_ref().is_none_or(|f| f())
    }
}

/// A titled card of rows inside a group.
#[derive(Clone, Default)]
pub struct Section {
    title: Option<TextSource>,
    footer: Option<TextSource>,
    rows: Vec<Row>,
}

impl Section {
    pub fn new() -> Self {
        Section::default()
    }
    /// The header above the card.
    pub fn title<M>(mut self, t: impl IntoText<M>) -> Self {
        self.title = Some(t.into_text());
        self
    }
    /// The footnote under the card.
    pub fn footer<M>(mut self, t: impl IntoText<M>) -> Self {
        self.footer = Some(t.into_text());
        self
    }
    pub fn row(mut self, row: Row) -> Self {
        self.rows.push(row);
        self
    }
    pub fn rows(mut self, rows: impl IntoIterator<Item = Row>) -> Self {
        self.rows.extend(rows);
        self
    }
}

/// How a [`choice`] row presents its options.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ChoiceStyle {
    /// A menu/combo (the default): any number of options.
    #[default]
    Menu,
    /// A segmented control: for a few short labels (Light / Dark / System).
    Segmented,
}

#[derive(Clone)]
enum RowKind {
    Toggle(Signal<bool>),
    Choice {
        labels: Vec<TextSource>,
        index: Signal<usize>,
        style: ChoiceStyle,
    },
    Slider {
        value: Signal<f64>,
        range: std::ops::RangeInclusive<f64>,
        step: Option<f64>,
        value_text: Option<Rc<dyn Fn(f64) -> String>>,
    },
    Stepper {
        value: Signal<f64>,
        range: std::ops::RangeInclusive<f64>,
        step: f64,
    },
    Text {
        value: Signal<String>,
        placeholder: Option<TextSource>,
        secure: bool,
    },
    Action {
        on: Rc<dyn Fn()>,
        destructive: bool,
        confirm: Option<TextSource>,
        confirm_label: Option<TextSource>,
        busy: Option<Signal<bool>>,
    },
    Link(String),
    Navigate(Build),
    Info {
        value: TextSource,
        copyable: bool,
    },
    Note,
    Custom(Build),
}

/// One row of a section (docs/settings.md), built by the row constructors.
#[derive(Clone)]
pub struct Row {
    title: TextSource,
    caption: Option<TextSource>,
    id: Option<String>,
    enabled: Option<Signal<bool>>,
    available: Option<Avail>,
    kind: RowKind,
}

impl Row {
    fn new<M>(title: impl IntoText<M>, kind: RowKind) -> Self {
        Row {
            title: title.into_text(),
            caption: None,
            id: None,
            enabled: None,
            available: None,
            kind,
        }
    }
    /// The row's title (every row constructor takes one, this replaces it).
    pub fn title<M>(mut self, t: impl IntoText<M>) -> Self {
        self.title = t.into_text();
        self
    }
    /// A secondary line under the title.
    pub fn caption<M>(mut self, t: impl IntoText<M>) -> Self {
        self.caption = Some(t.into_text());
        self
    }
    /// The element id of the row's control (default `pref-<key>` for a pref row).
    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }
    /// Gray the control out while the signal is false.
    pub fn enabled(mut self, enabled: Signal<bool>) -> Self {
        self.enabled = Some(enabled);
        self
    }
    /// Gray the control out while `f` is false: a derived condition (`|| !INCOGNITO.get()`),
    /// tracked like any reactive read.
    pub fn enabled_when(self, f: impl Fn() -> bool + 'static) -> Self {
        let sig = Signal::global(f());
        day_reactive::Scope::root().enter(|| {
            watch(f, move |v, _| sig.set_if_changed(*v));
        });
        self.enabled(sig)
    }
    /// Omit the row when false.
    pub fn available(mut self, f: impl Fn() -> bool + 'static) -> Self {
        self.available = Some(Rc::new(f));
        self
    }
    /// Omit the row unless the toolkit supports `cap`.
    pub fn requires(self, cap: Cap) -> Self {
        self.available(move || day_core::capability(cap) != Support::Unsupported)
    }
    fn is_available(&self) -> bool {
        self.available.as_ref().is_none_or(|f| f())
    }
}

/// The row constructors (docs/settings.md). Use them qualified (`rows::toggle(..)`): several
/// share a name with a piece in the Day prelude.
pub mod rows {
    use super::*;

    fn pref_id<T: PrefValue>(p: &Pref<T>) -> String {
        format!("pref-{}", p.key)
    }

    /// An on/off row bound to a `Pref<bool>`.
    pub fn toggle<M>(pref: &'static Pref<bool>, title: impl IntoText<M>) -> Row {
        Row::new(title, RowKind::Toggle(pref.signal())).id(pref_id(pref))
    }

    /// A one-of-N row: `options` pairs each value with its label. Menu by default;
    /// [`Row::segmented`] for a few short labels.
    pub fn choice<T: PrefValue, M, L, LM>(
        pref: &'static Pref<T>,
        title: impl IntoText<M>,
        options: impl IntoIterator<Item = (T, L)>,
    ) -> Row
    where
        L: IntoText<LM>,
    {
        let (values, labels): (Vec<T>, Vec<TextSource>) =
            options.into_iter().map(|(v, l)| (v, l.into_text())).unzip();
        let sig = pref.signal();
        let current = sig.get_untracked();
        let index = Signal::global(values.iter().position(|v| *v == current).unwrap_or(0));
        // Two-way: the index drives the pref; an outside write to the pref (a menu command, a
        // reset) moves the index. `set_if_changed` on both ends breaks the echo.
        {
            let values = values.clone();
            watch(
                move || index.get(),
                move |ix, _| {
                    if let Some(v) = values.get(*ix) {
                        pref.set(v.clone());
                    }
                },
            );
        }
        watch(
            move || sig.get(),
            move |v, _| {
                if let Some(ix) = values.iter().position(|x| x == v) {
                    index.set_if_changed(ix);
                }
            },
        );
        Row::new(
            title,
            RowKind::Choice {
                labels,
                index,
                style: ChoiceStyle::Menu,
            },
        )
        .id(pref_id(pref))
    }

    impl Row {
        /// Present a [`choice`] as a segmented control.
        pub fn segmented(mut self) -> Self {
            if let RowKind::Choice { style, .. } = &mut self.kind {
                *style = ChoiceStyle::Segmented;
            }
            self
        }
        /// A [`slider`]'s or [`stepper`]'s step.
        pub fn step(mut self, s: f64) -> Self {
            match &mut self.kind {
                RowKind::Slider { step, .. } => *step = Some(s),
                RowKind::Stepper { step, .. } => *step = s,
                _ => {}
            }
            self
        }
        /// A [`slider`]'s value label, shown beside the control.
        pub fn value_text(mut self, f: impl Fn(f64) -> String + 'static) -> Self {
            if let RowKind::Slider { value_text, .. } = &mut self.kind {
                *value_text = Some(Rc::new(f));
            }
            self
        }
        /// A [`text`] row's placeholder.
        pub fn placeholder<M>(mut self, t: impl IntoText<M>) -> Self {
            if let RowKind::Text { placeholder, .. } = &mut self.kind {
                *placeholder = Some(t.into_text());
            }
            self
        }
        /// A [`text`] row that hides its characters.
        pub fn secure(mut self) -> Self {
            if let RowKind::Text { secure, .. } = &mut self.kind {
                *secure = true;
            }
            self
        }
        /// An [`action`] drawn as destructive (red where the platform has a red).
        pub fn destructive(mut self) -> Self {
            if let RowKind::Action { destructive, .. } = &mut self.kind {
                *destructive = true;
            }
            self
        }
        /// An [`action`] that asks first: a confirm dialog with `message`, destructive when the
        /// row is.
        pub fn confirm<M>(mut self, message: impl IntoText<M>) -> Self {
            if let RowKind::Action { confirm, .. } = &mut self.kind {
                *confirm = Some(message.into_text());
            }
            self
        }
        /// The confirm dialog's own button label ("Reset" rather than the generic OK).
        pub fn confirm_label<M>(mut self, label: impl IntoText<M>) -> Self {
            if let RowKind::Action { confirm_label, .. } = &mut self.kind {
                *confirm_label = Some(label.into_text());
            }
            self
        }
        /// An [`action`] that shows a spinner and disables while the signal is true.
        pub fn busy(mut self, busy: Signal<bool>) -> Self {
            if let RowKind::Action { busy: b, .. } = &mut self.kind {
                *b = Some(busy);
            }
            self
        }
        /// An [`info`] row with a copy affordance.
        pub fn copyable(mut self) -> Self {
            if let RowKind::Info { copyable, .. } = &mut self.kind {
                *copyable = true;
            }
            self
        }
    }

    /// A continuous value over `range`.
    pub fn slider<M>(
        pref: &'static Pref<f64>,
        title: impl IntoText<M>,
        range: std::ops::RangeInclusive<f64>,
    ) -> Row {
        let sig = pref.signal();
        Row::new(
            title,
            RowKind::Slider {
                value: sig,
                range,
                step: None,
                value_text: None,
            },
        )
        .id(pref_id(pref))
    }

    /// A small integer with increment/decrement controls.
    pub fn stepper<M>(
        pref: &'static Pref<i64>,
        title: impl IntoText<M>,
        range: std::ops::RangeInclusive<i64>,
    ) -> Row {
        let sig = pref.signal();
        let value = Signal::global(sig.get_untracked() as f64);
        watch(move || value.get(), move |v, _| pref.set(v.round() as i64));
        watch(
            move || sig.get(),
            move |v, _| value.set_if_changed(*v as f64),
        );
        Row::new(
            title,
            RowKind::Stepper {
                value,
                range: (*range.start() as f64)..=(*range.end() as f64),
                step: 1.0,
            },
        )
        .id(pref_id(pref))
    }

    /// A free-text value; every edit persists.
    pub fn text<M>(pref: &'static Pref<String>, title: impl IntoText<M>) -> Row {
        let sig = pref.signal();
        Row::new(
            title,
            RowKind::Text {
                value: sig,
                placeholder: None,
                secure: false,
            },
        )
        .id(pref_id(pref))
    }

    /// A command row: a button with the title. [`Row::destructive`], [`Row::confirm`],
    /// [`Row::busy`].
    pub fn action<M>(title: impl IntoText<M>, on: impl Fn() + 'static) -> Row {
        Row::new(
            title,
            RowKind::Action {
                on: Rc::new(on),
                destructive: false,
                confirm: None,
                confirm_label: None,
                busy: None,
            },
        )
    }

    /// A row that opens `url` in the platform's handler.
    pub fn link<M>(title: impl IntoText<M>, url: impl Into<String>) -> Row {
        Row::new(title, RowKind::Link(url.into()))
    }

    /// A row that pushes a sub-page built by `build` (a chevron row).
    pub fn navigate<M, P: Piece>(title: impl IntoText<M>, build: impl Fn() -> P + 'static) -> Row {
        Row::new(
            title,
            RowKind::Navigate(Rc::new(move || AnyPiece::new(build()))),
        )
    }

    /// A read-only title/value pair. [`Row::copyable`].
    pub fn info<M, VM>(title: impl IntoText<M>, value: impl IntoText<VM>) -> Row {
        Row::new(
            title,
            RowKind::Info {
                value: value.into_text(),
                copyable: false,
            },
        )
    }

    /// A footnote paragraph inside a section.
    pub fn note<M>(text: impl IntoText<M>) -> Row {
        Row::new(text, RowKind::Note)
    }

    /// Any piece as a row, inside the section's card.
    pub fn custom<P: Piece>(build: impl Fn() -> P + 'static) -> Row {
        Row::new("", RowKind::Custom(Rc::new(move || AnyPiece::new(build()))))
    }
}

// ===========================================================================
// About
// ===========================================================================

/// The app's About content (docs/settings.md): a window on the desktops, the toolkit's About
/// dialog on GTK, a page inside Settings on mobile and Windows.
#[derive(Clone, Default)]
pub struct About {
    name: Option<TextSource>,
    version: Option<String>,
    build: Option<String>,
    tagline: Option<TextSource>,
    copyright: Option<TextSource>,
    icon: Option<day_spec::ImageName>,
    links: Vec<(TextSource, String)>,
    privacy: Option<String>,
    credits: Option<TextSource>,
    licenses: Option<TextSource>,
    custom: Option<Build>,
}

impl About {
    /// Name, version and build from the app identity `day build` exports (`Day.toml`
    /// `[app]`), falling back to the window's app name and `WindowOptions::version` at run
    /// time.
    pub fn from_manifest() -> Self {
        About {
            version: option_env!("DAY_SETTINGS_APP_VERSION").map(str::to_owned),
            build: option_env!("DAY_SETTINGS_APP_BUILD").map(str::to_owned),
            ..Default::default()
        }
    }
    /// The display name (default: the app title from the manifest, else the window title).
    pub fn name<M>(mut self, t: impl IntoText<M>) -> Self {
        self.name = Some(t.into_text());
        self
    }
    pub fn version(mut self, v: impl Into<String>) -> Self {
        self.version = Some(v.into());
        self
    }
    pub fn build(mut self, b: impl Into<String>) -> Self {
        self.build = Some(b.into());
        self
    }
    /// One line under the name.
    pub fn tagline<M>(mut self, t: impl IntoText<M>) -> Self {
        self.tagline = Some(t.into_text());
        self
    }
    pub fn copyright<M>(mut self, t: impl IntoText<M>) -> Self {
        self.copyright = Some(t.into_text());
        self
    }
    /// The app icon as a bundled image (`res::images::icon`).
    pub fn icon(mut self, icon: impl Into<day_spec::ImageName>) -> Self {
        self.icon = Some(icon.into());
        self
    }
    /// A titled link (Website, Source Code, …).
    pub fn link<M>(mut self, title: impl IntoText<M>, url: impl Into<String>) -> Self {
        self.links.push((title.into_text(), url.into()));
        self
    }
    /// The privacy policy URL: a known slot (the stores ask for it, GTK has a field).
    pub fn privacy(mut self, url: impl Into<String>) -> Self {
        self.privacy = Some(url.into());
        self
    }
    /// Acknowledgements, as a text block (Markdown where the toolkit renders it).
    pub fn credits<M>(mut self, t: impl IntoText<M>) -> Self {
        self.credits = Some(t.into_text());
        self
    }
    /// Third-party license notices, as one text block.
    pub fn licenses<M>(mut self, t: impl IntoText<M>) -> Self {
        self.licenses = Some(t.into_text());
        self
    }
    /// A bespoke piece that replaces the composed About body.
    pub fn custom<P: Piece>(mut self, build: impl Fn() -> P + 'static) -> Self {
        self.custom = Some(Rc::new(move || AnyPiece::new(build())));
        self
    }

    /// The resolved display name.
    pub fn display_name(&self) -> String {
        match &self.name {
            Some(n) => n.initial(),
            None => option_env!("DAY_SETTINGS_APP_TITLE")
                .map(str::to_owned)
                .or_else(|| day_core::launch_options().map(|o| o.app_name.unwrap_or(o.title)))
                .unwrap_or_default(),
        }
    }
    /// The version line, localized ("Version 2.3.0 (14)").
    pub fn version_line(&self) -> Option<String> {
        let version = self
            .version
            .clone()
            .or_else(|| day_core::launch_options().and_then(|o| o.version.clone()))?;
        let mut line = day_fluent::tr("day-about-version")
            .arg("version", version)
            .format();
        if let Some(build) = &self.build
            && !build.is_empty()
        {
            line.push_str(&format!(" ({build})"));
        }
        Some(line)
    }
    pub fn version_value(&self) -> Option<&str> {
        self.version.as_deref()
    }
    pub fn build_value(&self) -> Option<&str> {
        self.build.as_deref()
    }
    pub fn links(&self) -> &[(TextSource, String)] {
        &self.links
    }
    pub fn privacy_url(&self) -> Option<&str> {
        self.privacy.as_deref()
    }
    pub fn icon_name(&self) -> Option<&day_spec::ImageName> {
        self.icon.as_ref()
    }
    pub fn tagline_text(&self) -> Option<String> {
        self.tagline.as_ref().map(TextSource::initial)
    }
    pub fn copyright_text(&self) -> Option<String> {
        self.copyright.as_ref().map(TextSource::initial)
    }
    pub fn credits_text(&self) -> Option<String> {
        self.credits.as_ref().map(TextSource::initial)
    }
    pub fn licenses_text(&self) -> Option<String> {
        self.licenses.as_ref().map(TextSource::initial)
    }
    pub fn custom_piece(&self) -> Option<AnyPiece> {
        self.custom.as_ref().map(|b| b())
    }
}

// ===========================================================================
// Providers
// ===========================================================================

/// A settings group contributed by another crate (docs/settings.md): the self-updater's
/// Updates group is the first. Declared as a static through [`provider!`], which registers it
/// in the link-time slice on native targets; on wasm (no linkme) and anywhere an explicit
/// order matters, [`register_provider`] / [`Preferences::provider`] name it directly.
pub struct Provider {
    /// The group's route id; also used by [`Preferences::without_provider`].
    pub id: &'static str,
    /// Sort key among the app's groups (apps default to 0, providers to 500).
    pub order: i32,
    /// Whether the group applies to this build/platform at all.
    pub available: fn() -> bool,
    /// The group.
    pub group: fn() -> Group,
    /// Runs once inside [`Preferences::install`], after the store is reachable: register menu
    /// actions, start a scheduler, apply a persisted value.
    pub startup: Option<fn()>,
}

/// Every statically registered [`Provider`] (native targets; empty on wasm).
#[cfg(not(target_arch = "wasm32"))]
#[linkme::distributed_slice]
pub static PROVIDERS: [&'static Provider];

/// Declare a provider static and register it (docs/settings.md):
///
/// ```ignore
/// day_piece_settings::provider! {
///     static UPDATES: Provider = Provider { id: "updates", order: 900, available: || true,
///         group: updates_group, startup: Some(schedule) };
/// }
/// ```
#[macro_export]
macro_rules! provider {
    ($(#[$meta:meta])* $vis:vis static $name:ident : Provider = $init:expr ;) => {
        $(#[$meta])*
        $vis static $name: $crate::Provider = $init;
        // The crate path keeps an out-of-tree provider free of its own `linkme` dependency.
        #[cfg(not(target_arch = "wasm32"))]
        #[$crate::linkme::distributed_slice($crate::PROVIDERS)]
        #[linkme(crate = $crate::linkme)]
        static __DAY_SETTINGS_PROVIDER: &'static $crate::Provider = &$name;
    };
}

#[doc(hidden)]
pub use linkme;

/// Register a provider at run time (before [`Preferences::install`]): the wasm path, and the
/// explicit alternative to the link-time slice anywhere.
pub fn register_provider(p: &'static Provider) {
    RUNTIME_PROVIDERS.with(|r| {
        let mut r = r.borrow_mut();
        if !r.iter().any(|q| std::ptr::eq(*q, p)) {
            r.push(p);
        }
    });
}

fn all_providers() -> Vec<&'static Provider> {
    let mut out: Vec<&'static Provider> = Vec::new();
    #[cfg(not(target_arch = "wasm32"))]
    out.extend(PROVIDERS.iter().copied());
    RUNTIME_PROVIDERS.with(|r| {
        for p in r.borrow().iter() {
            if !out.iter().any(|q| std::ptr::eq(*q, *p)) {
                out.push(p);
            }
        }
    });
    out
}

/// Which providers [`Preferences::install`] collects.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Providers {
    /// Every registered provider whose `available` says yes (the default).
    #[default]
    Registered,
    /// Only the ones named with [`Preferences::provider`].
    None,
}

// ===========================================================================
// Preferences: the declaration and install()
// ===========================================================================

/// How the surface is presented; chosen per toolkit by default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presentation {
    /// A desktop Preferences window with the groups as tabs/pages (the app menu opens it).
    Window,
    /// A navigation presentation: a root list (About header, groups) pushing one page per
    /// group. Mobile, and any app that keeps settings as an in-app route.
    Navigation,
}

/// The settings declaration (docs/settings.md). Build it in `root()` and call
/// [`Self::install`].
pub struct Preferences {
    about: Option<About>,
    groups: Vec<Group>,
    providers: Providers,
    explicit: Vec<&'static Provider>,
    excluded: Vec<&'static str>,
    presentation: Option<Presentation>,
    window_size: day_spec::Size,
    title: Option<TextSource>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self::new()
    }
}

impl Preferences {
    pub fn new() -> Self {
        Preferences {
            about: None,
            groups: Vec::new(),
            providers: Providers::Registered,
            explicit: Vec::new(),
            excluded: Vec::new(),
            presentation: None,
            window_size: day_spec::Size::new(560.0, 640.0),
            title: None,
        }
    }
    /// The About content; registers the About menu item and window.
    pub fn about(mut self, about: About) -> Self {
        self.about = Some(about);
        self
    }
    /// Append a group.
    pub fn group(mut self, group: Group) -> Self {
        self.groups.push(group);
        self
    }
    /// Which registered providers to collect (default: all available).
    pub fn providers(mut self, policy: Providers) -> Self {
        self.providers = policy;
        self
    }
    /// Name a provider explicitly (also what a wasm build does, where the link-time slice is
    /// empty).
    pub fn provider(mut self, p: &'static Provider) -> Self {
        self.explicit.push(p);
        self
    }
    /// Leave out a registered provider by id.
    pub fn without_provider(mut self, id: &'static str) -> Self {
        self.excluded.push(id);
        self
    }
    /// Pin the presentation instead of letting the toolkit decide.
    pub fn presentation(mut self, p: Presentation) -> Self {
        self.presentation = Some(p);
        self
    }
    /// The desktop window's size (height is a ceiling: the window fits its page).
    pub fn window_size(mut self, width: f64, height: f64) -> Self {
        self.window_size = day_spec::Size::new(width, height);
        self
    }
    /// The surface's title (default: the platform's word, Settings or Preferences).
    pub fn title<M>(mut self, t: impl IntoText<M>) -> Self {
        self.title = Some(t.into_text());
        self
    }

    /// Wire everything up (docs/settings.md) and return the handle the app's nav and pages
    /// use. Call once, in `root()`, after the locale catalog installs and before `app_menu`.
    pub fn install(self) -> Prefs {
        let Preferences {
            about,
            mut groups,
            providers,
            explicit,
            excluded,
            presentation,
            window_size,
            title,
        } = self;
        // Collect providers: the registered set (unless opted out), plus explicit ones.
        let mut chosen: Vec<&'static Provider> = Vec::new();
        if providers == Providers::Registered {
            chosen.extend(all_providers());
        }
        for p in explicit {
            if !chosen.iter().any(|q| std::ptr::eq(*q, p)) {
                chosen.push(p);
            }
        }
        chosen.retain(|p| !excluded.contains(&p.id) && (p.available)());
        for p in &chosen {
            if let Some(startup) = p.startup {
                startup();
            }
            groups.push((p.group)().order(p.order));
        }
        groups.sort_by_key(|g| g.order);
        let presentation = presentation.unwrap_or_else(default_presentation);
        let title = title.unwrap_or_else(|| TextSource::Dyn(Rc::new(platform_word)));
        let installed = Rc::new(Installed {
            about,
            groups,
            presentation,
            title,
            selected: Signal::global(String::new()),
            path: Signal::global(Vec::new()),
        });
        INSTALLED.with(|i| *i.borrow_mut() = Some(installed.clone()));

        // Desktop: the Preferences window and its menu item, and the About window. A
        // navigation-presentation toolkit with an app menu (the web's emulated menu button)
        // registers the cover as the surface instead, so the injected Settings… item exists
        // and opens it; without a menu nothing is registered and the app's nav item is the
        // entry.
        match presentation {
            Presentation::Window => {
                let inst = installed.clone();
                day_core::register_preferences_with(
                    day_spec::WindowOptions {
                        title: installed.title.initial(),
                        size: window_size,
                        size_to_fit: true,
                        ..day_spec::WindowOptions::default()
                    },
                    move || window_shell(inst.clone()),
                );
            }
            Presentation::Navigation
                if day_core::capability(Cap::AppMenu) != Support::Unsupported =>
            {
                let inst = installed.clone();
                day_core::register_preferences_with(
                    day_spec::WindowOptions {
                        title: installed.title.initial(),
                        ..day_spec::WindowOptions::default()
                    },
                    move || navigation_shell_with_done(inst.clone()),
                );
            }
            Presentation::Navigation => {}
        }
        if installed.about.is_some() {
            install_about(installed.clone());
        }
        register_route_controller(installed.clone());
        Prefs(installed)
    }
}

/// The installed declaration, shared by the handle and the free functions.
struct Installed {
    about: Option<About>,
    groups: Vec<Group>,
    presentation: Presentation,
    title: TextSource,
    /// The selected group id in the desktop window ("" = first).
    selected: Signal<String>,
    /// The pushed path in the navigation presentation.
    path: Signal<Vec<SettingsRoute>>,
}

impl Installed {
    fn visible_groups(&self) -> Vec<&Group> {
        self.groups.iter().filter(|g| g.is_available()).collect()
    }
    fn group(&self, id: &str) -> Option<&Group> {
        self.groups.iter().find(|g| g.id == id && g.is_available())
    }
}

/// The word the platform uses for the surface: "Settings" on macOS, Windows and the phones,
/// "Preferences" on GNOME and KDE.
fn platform_word() -> String {
    let gnome_or_kde = matches!(day_core::toolkit_key(), Some("gtk" | "qt"));
    if gnome_or_kde {
        day_l10n::t("day-preferences")
    } else {
        day_l10n::t("day-settings")
    }
}

fn default_presentation() -> Presentation {
    // A toolkit that can open a window and has a menu bar to reach it from gets the window;
    // everything else (the phones, HarmonyOS, the web) gets the navigation presentation.
    let windows = day_core::capability(Cap::MultiWindow) != Support::Unsupported;
    let menu = day_core::capability(Cap::AppMenu) != Support::Unsupported;
    if windows && menu {
        Presentation::Window
    } else {
        Presentation::Navigation
    }
}

/// The handle [`Preferences::install`] returns: `Copy`, so pages and the nav take it by value.
#[derive(Clone)]
pub struct Prefs(Rc<Installed>);

impl Prefs {
    /// Open-or-focus the surface: the window on desktop; on mobile, select the nav item when
    /// one is mounted, else present the navigation presentation as a cover.
    pub fn open(&self) -> bool {
        open()
    }
    /// Open and select a group (`"updates"`), optionally a row (`"updates/check"`).
    pub fn open_at(&self, route: &str) -> bool {
        open_at(route)
    }
    /// Whether the app should place a Settings entry in its own navigation: true where the
    /// platform gives no app menu to reach the surface from (the phones); false on the
    /// desktops and on the web, whose menu button carries Settings… and About.
    pub fn in_nav(&self) -> bool {
        self.0.presentation == Presentation::Navigation
            && day_core::capability(Cap::AppMenu) == Support::Unsupported
    }
    /// The keys for a `nav().items(..)` source: `[key]` where [`Self::in_nav`], else empty.
    pub fn nav_keys<K: Clone>(&self, key: K) -> Vec<K> {
        if self.in_nav() { vec![key] } else { Vec::new() }
    }
    /// A ready nav item titled with the platform's word.
    pub fn nav_item<K>(&self, key: K) -> NavItem<K> {
        let title = self.0.title.clone();
        item(key, move || title.initial())
    }
    /// The navigation presentation as a piece: the destination of the nav item, or any place
    /// the app embeds settings.
    pub fn page(&self) -> AnyPiece {
        navigation_shell(self.0.clone())
    }
    /// One group's page alone (an in-reader sheet, a sidebar pane).
    pub fn group_page(&self, id: &str) -> AnyPiece {
        match self.0.group(id) {
            Some(g) => group_page(self.0.clone(), g, false),
            None => column(()).any(),
        }
    }
    /// One group's sections without a scroll, for a sheet that stacks several groups and
    /// owns the scrolling itself: put the bodies in one `scroll`. The phones keep their native
    /// shape, sized to its rows.
    pub fn group_body(&self, id: &str) -> AnyPiece {
        match self.0.group(id) {
            Some(g) => group_body(self.0.clone(), g),
            None => column(()).any(),
        }
    }
    /// The composed About page.
    pub fn about_page(&self) -> AnyPiece {
        about_page(self.0.clone())
    }
    /// The surface's title.
    pub fn title(&self) -> String {
        self.0.title.initial()
    }
}

fn installed() -> Option<Rc<Installed>> {
    INSTALLED.with(|i| i.borrow().clone())
}

/// Open-or-focus the installed settings surface (see [`Prefs::open`]).
pub fn open() -> bool {
    let Some(inst) = installed() else {
        log::warn!("day-piece-settings: open() before Preferences::install()");
        return false;
    };
    match inst.presentation {
        Presentation::Window => day_core::open_preferences(),
        Presentation::Navigation => day_core::navigate(NAV_KEY) || open_cover(inst),
    }
}

/// Present the navigation presentation as a cover (a Done button closes it): the mobile path
/// when no nav item is mounted.
fn open_cover(inst: Rc<Installed>) -> bool {
    // A newly presented Settings cover starts at its group list. Keep the current
    // page when merely focusing a cover that is already open. `open_at` applies its
    // requested group after this call, so deep links still land on that group.
    if day_core::window_by_key(day_core::windows::PREFERENCES_KEY).is_none() {
        inst.path.set(Vec::new());
    }
    day_core::open_window(
        Some(day_core::windows::PREFERENCES_KEY),
        day_spec::WindowOptions {
            title: inst.title.initial(),
            ..day_spec::WindowOptions::default()
        },
        day_spec::WindowKind::Preferences,
        move || navigation_shell_with_done(inst.clone()),
    );
    true
}

/// The nav item key an app conventionally mounts the surface under; `open()` tries it first.
pub const NAV_KEY: &str = "settings";

/// The installed navigation presentation as a piece (see [`Prefs::page`]), for code that has
/// no [`Prefs`] handle at hand. An empty piece before [`Preferences::install`].
pub fn page() -> AnyPiece {
    match installed() {
        Some(inst) => navigation_shell(inst),
        None => column(()).any(),
    }
}

/// One installed group's page alone (see [`Prefs::group_page`]).
pub fn group_page_of(id: &str) -> AnyPiece {
    match installed() {
        Some(inst) => Prefs(inst).group_page(id),
        None => column(()).any(),
    }
}

/// Open the surface at a group route (`"updates"`, `"updates/check"`).
pub fn open_at(route: &str) -> bool {
    let Some(inst) = installed() else {
        return false;
    };
    let (group, _row) = route.split_once('/').unwrap_or((route, ""));
    if group == "about" {
        return day_core::open_about();
    }
    if inst.group(group).is_none() {
        return false;
    }
    match inst.presentation {
        Presentation::Window => {
            inst.selected.set_if_changed(group.to_string());
            day_core::open_preferences()
        }
        Presentation::Navigation => {
            // Already inside the settings section (a route's second segment arriving while
            // the first is current): just move the stack, without re-entering `navigate`.
            let inside = day_core::current_route()
                .is_some_and(|r| r == NAV_KEY || r.starts_with(&format!("{NAV_KEY}/")));
            let ok = inside || open();
            inst.path.set(vec![SettingsRoute::Group(group.to_string())]);
            ok
        }
    }
}

/// Register the desktop surface as a route root, so `navigate("settings/updates")` and a deep
/// link open the window on the group (the navigation presentation routes through its nav
/// item instead).
///
/// Two controllers: the root answers `settings` (and `about`, and a bare group id) by opening
/// the window; a second, registered right after it, answers the group segment that follows.
/// day-core feeds an absolute path's tail only to the surfaces nested after the one that
/// took its head, never to that surface itself, and the window's own nav is `.local()` (it
/// must not contribute to the app's route), so the tail needs a surface of its own to land on.
///
/// For the window presentation, and for a navigation presentation whose toolkit has an app
/// menu (the web). Where the platform has no menu the app's nav item owns the `settings`
/// route and the stack inside it owns the group segment; a controller there would be
/// consulted before the app's nav (it registers earlier) and would present the cover over a
/// section that never switched. An app with no nav host on such a platform reaches the
/// cover through [`open`] from its own affordance.
fn register_route_controller(inst: Rc<Installed>) {
    // The web is the navigation presentation with an emulated app menu: the cover is the
    // registered preferences surface and no nav item claims `settings`, so the routes land
    // here too (the starter's `navigate: settings` step runs on every target).
    let menu = day_core::capability(Cap::AppMenu) != Support::Unsupported;
    if inst.presentation != Presentation::Window && !menu {
        return;
    }
    let open_surface = day_core::open_preferences;
    let root = inst.clone();
    let open_root = open_surface;
    let _root_token = day_core::register_nav(day_core::NavController {
        push: Box::new(move |route| {
            if route == NAV_KEY {
                open_root()
            } else if route == "about" {
                day_core::open_about()
            } else if root.group(route).is_some() {
                open_at(route)
            } else {
                false
            }
        }),
        pop: Box::new(|_| false),
        current: Box::new(String::new),
        enter: Box::new(move |segment| segment == NAV_KEY && open_surface()),
        segments: Box::new(Vec::new),
        depth: 0,
        layer: Box::new(|| 0),
        active: Box::new(|| true),
    });
    let group = inst;
    let _group_token = day_core::register_nav(day_core::NavController {
        push: Box::new(|_| false),
        pop: Box::new(|_| false),
        current: Box::new(String::new),
        enter: Box::new(move |segment| group.group(segment).is_some() && open_at(segment)),
        segments: Box::new(Vec::new),
        // Depth 0, like the root: day-core offers an absolute path's tail to the surfaces
        // after the anchor in depth order, registration order within a depth. At depth 1
        // this controller came after the app's own nav (depth 0, registered later), which
        // took a tail naming one of its sections (`settings/practice` opened Lexicon's
        // Practice section). At depth 0 it is consulted first, and a segment naming no group
        // still falls through to the app.
        depth: 0,
        layer: Box::new(|| 0),
        active: Box::new(|| true),
    });
}

fn install_about(inst: Rc<Installed>) {
    #[cfg(feature = "gtk")]
    {
        if gtk_impl::install_about(inst.clone()) {
            return;
        }
    }
    let name = inst
        .about
        .as_ref()
        .map(About::display_name)
        .unwrap_or_default();
    let title = day_fluent::tr("day-about-app").arg("app", name).format();
    day_core::register_about_window(
        day_spec::WindowOptions {
            title,
            size: day_spec::Size::new(320.0, 480.0),
            size_to_fit: true,
            ..day_spec::WindowOptions::default()
        },
        move || about_page(inst.clone()),
    );
}

// ===========================================================================
// The General group: appearance + language
// ===========================================================================

pref_enum! {
    /// The app's appearance override: the platform's light or dark look, or whatever the system
    /// says (the default).
    pub enum Appearance { Light = "light", Dark = "dark", System = "system" }
}

/// The appearance pref every app shares, key `day.appearance`. `DAY_THEME` wins at launch.
pub static APPEARANCE: Pref<Appearance> =
    Pref::new("day.appearance", || Appearance::System).env("DAY_THEME");

/// The language pref every app shares, key `day.locale`: a locale tag, or empty for the
/// system language. `DAY_LOCALE` wins at launch.
pub static LANGUAGE: Pref<String> = Pref::new("day.locale", String::new).env("DAY_LOCALE");

fn system_locale() -> String {
    SYSTEM_LOCALE
        .with(|c| c.get().cloned())
        .unwrap_or_else(|| day_fluent::locale().get_untracked())
}

fn apply_appearance(a: Appearance) {
    if day_core::capability(Cap::Appearance) == Support::Unsupported {
        return;
    }
    day_core::set_appearance(match a {
        Appearance::Light => Some(false),
        Appearance::Dark => Some(true),
        Appearance::System => None,
    });
}

fn apply_language(tag: &str) {
    if tag.is_empty() {
        day_fluent::set_locale(&system_locale());
    } else {
        day_fluent::set_locale(tag);
    }
}

/// Apply the persisted appearance and language at boot and keep applying every later change
/// (docs/settings.md). Runs once per process; [`general_group`] calls it, and an app that
/// uses the prefs without the group calls it itself. Honors the env-wins rule: under
/// `DAY_THEME`/`DAY_LOCALE` the stored value is not applied at boot (the launch override
/// stays deterministic), while live changes always apply.
pub fn wire_general() {
    if GENERAL_WIRED.with(|w| w.replace(true)) {
        return;
    }
    SYSTEM_LOCALE.with(|c| {
        let _ = c.set(day_fluent::locale().get_untracked());
    });
    if !APPEARANCE.env_forced() {
        apply_appearance(APPEARANCE.get_untracked());
    }
    if !LANGUAGE.env_forced() {
        let tag = LANGUAGE.get_untracked();
        if !tag.is_empty() {
            apply_language(&tag);
        }
    }
    let root = day_reactive::Scope::root();
    root.enter(|| {
        APPEARANCE.watch(|a| apply_appearance(*a));
        LANGUAGE.watch(|tag| apply_language(tag));
    });
}

/// The "General" group every app starts with: Appearance (`theme-picker`, present where the
/// toolkit honors a runtime override) and Language (`language-picker`: System plus every
/// bundled locale under its own name). `locales` is the app's generated `res::locales::ALL`.
pub fn general_group(locales: LocaleTable) -> Group {
    wire_general();
    Group::new("general", day_fluent::tr("day-settings-general"))
        .icon(day_spec::Symbol::Settings)
        .row(appearance_row())
        .row(language_row(locales))
}

/// The Light / Dark / System row alone (id `theme-picker`), for apps composing their own group.
pub fn appearance_row() -> Row {
    wire_general();
    rows::choice(
        &APPEARANCE,
        day_fluent::tr("day-settings-theme"),
        [
            (Appearance::Light, day_fluent::tr("day-theme-light")),
            (Appearance::Dark, day_fluent::tr("day-theme-dark")),
            (Appearance::System, day_fluent::tr("day-theme-system")),
        ],
    )
    .segmented()
    .id("theme-picker")
    .requires(Cap::Appearance)
}

/// The language row alone (id `language-picker`).
pub fn language_row(locales: LocaleTable) -> Row {
    wire_general();
    let mut options: Vec<(String, TextSource)> = vec![(
        String::new(),
        day_fluent::tr("day-theme-system").into_text(),
    )];
    options.extend(
        locales
            .iter()
            .map(|(tag, name)| ((*tag).to_string(), TextSource::Static((*name).to_string()))),
    );
    choice_text(&LANGUAGE, day_fluent::tr("day-settings-language"), options).id("language-picker")
}

/// [`choice`] over already-resolved label sources (what [`language_row`] builds from a
/// locale table).
fn choice_text<T: PrefValue, M>(
    pref: &'static Pref<T>,
    title: impl IntoText<M>,
    options: Vec<(T, TextSource)>,
) -> Row {
    rows::choice(
        pref,
        title,
        options.into_iter().map(|(v, l)| (v, Resolved(l))),
    )
}

/// An `IntoText` adapter over a `TextSource` (the sources module offers no direct impl).
struct Resolved(TextSource);
struct ResolvedMark;
impl IntoText<ResolvedMark> for Resolved {
    fn into_text(self) -> TextSource {
        self.0
    }
}

// ===========================================================================
// Composed presentation
// ===========================================================================

/// The route type of the navigation presentation's stack: pushed pages are groups, About, a
/// row's sub-page or a choice list.
#[derive(Clone, PartialEq, Debug)]
pub enum SettingsRoute {
    About,
    Group(String),
}

impl Route for SettingsRoute {
    fn key(&self) -> String {
        match self {
            SettingsRoute::About => "about".into(),
            SettingsRoute::Group(g) => g.clone(),
        }
    }
    fn from_key(key: &str) -> Option<Self> {
        Some(match key {
            "about" => SettingsRoute::About,
            g => SettingsRoute::Group(g.to_string()),
        })
    }
    fn title(&self) -> String {
        match self {
            SettingsRoute::About => day_l10n::t("day-about"),
            SettingsRoute::Group(g) => installed()
                .and_then(|i| i.group(g).map(|g| g.title.initial()))
                .unwrap_or_else(|| g.clone()),
        }
    }
}

fn text_label(t: &TextSource) -> Label {
    match t {
        TextSource::Static(s) => label(s.clone()),
        TextSource::Dyn(f) => {
            let f = f.clone();
            label(move || f())
        }
    }
}

/// A text source as a closure a label or a control re-reads: tracked, so a localized source
/// (`tr(..)`, a generated accessor) relabels when the locale changes, and a signal-backed
/// one when the signal does.
fn text_source_fn(t: &TextSource) -> impl Fn() -> String + 'static {
    let t = t.clone();
    move || resolve_text(&t)
}

/// Resolve a text source, tracked (see [`text_source_fn`]).
pub(crate) fn resolve_text(t: &TextSource) -> String {
    match t {
        TextSource::Static(s) => s.clone(),
        TextSource::Dyn(f) => f(),
    }
}

/// A choice row's picker, the same on every presentation: the option labels re-resolve on
/// locale change (`options_reactive`), seeded from the same sources, and the row's enabled
/// gate reaches the control. Every arm builds its picker here, so the two behaviors cannot
/// drift between the composed rows and a native row's suffix control.
pub(crate) fn choice_picker(
    labels: &[TextSource],
    index: Signal<usize>,
    segmented: bool,
    enabled: impl Fn() -> bool + 'static,
) -> day_pieces::Picker<Signal<usize>> {
    let sources: Rc<Vec<TextSource>> = Rc::new(labels.to_vec());
    let initial: Vec<String> = sources.iter().map(resolve_text).collect();
    let srcs = sources.clone();
    let mut p = picker(initial, index)
        .options_reactive(move || srcs.iter().map(resolve_text).collect())
        .enabled(enabled);
    if segmented {
        p = p.segmented();
    }
    p
}

/// A stepper row's two buttons, the same on every presentation: each steps the value within
/// the bounds, and each is enabled only while the row's gate holds AND the bound leaves room.
/// A native row's disabled styling never reaches its child buttons, so the gate is applied to
/// the buttons themselves here.
pub(crate) fn stepper_buttons(
    v: Signal<f64>,
    range: &std::ops::RangeInclusive<f64>,
    step: f64,
    enabled: impl Fn() -> bool + Copy + 'static,
) -> (day_pieces::Button, day_pieces::Button) {
    let (lo, hi, st) = (*range.start(), *range.end(), step);
    let dec = button("−")
        .action(move || v.set((v.get_untracked() - st).max(lo)))
        .enabled(move || enabled() && v.get() > lo);
    let inc = button("+")
        .action(move || v.set((v.get_untracked() + st).min(hi)))
        .enabled(move || enabled() && v.get() < hi);
    (dec, inc)
}

/// The same script targets on every settings presentation: the id names the value label,
/// and its two buttons have stable suffixes. Keep these on the controls, not their row.
pub(crate) fn stepper_control(
    value: Signal<f64>,
    dec: day_pieces::Button,
    inc: day_pieces::Button,
    id: Option<&str>,
) -> AnyPiece {
    let value_label = label(move || format!("{}", value.get() as i64)).tabular();
    match id {
        Some(id) => row((
            dec.id(format!("{id}-decrement")),
            value_label.id(id),
            inc.id(format!("{id}-increment")),
        ))
        .spacing(8.0)
        .align(VAlign::Center)
        .any(),
        None => row((dec, value_label, inc))
            .spacing(8.0)
            .align(VAlign::Center)
            .any(),
    }
}

/// The desktop window: a sidebar of groups beside the selected page (one group: the page
/// alone). A toolkit feature replaces this with its native panel.
fn window_shell(inst: Rc<Installed>) -> AnyPiece {
    #[cfg(all(feature = "appkit", target_os = "macos"))]
    {
        if let Some(p) = appkit_impl::window_shell(inst.clone()) {
            return p;
        }
    }
    #[cfg(feature = "gtk")]
    {
        if let Some(p) = gtk_impl::window_shell(inst.clone()) {
            return p;
        }
    }
    #[cfg(feature = "qt")]
    {
        if let Some(p) = qt_impl::window_shell(inst.clone()) {
            return p;
        }
    }
    composed_window_shell(inst)
}

fn composed_window_shell(inst: Rc<Installed>) -> AnyPiece {
    let groups = inst.visible_groups();
    if groups.len() <= 1 {
        return match groups.first() {
            Some(g) => group_page(inst.clone(), g, true),
            None => column(()).any(),
        };
    }
    if inst.selected.get_untracked().is_empty() {
        inst.selected.set(groups[0].id.to_string());
    }
    let mut nav = nav(inst.selected)
        .style(NavStyle::Sidebar)
        .local()
        .id("prefs-nav");
    for g in groups {
        let id = g.id;
        let title = text_source_fn(&g.title);
        let inst2 = inst.clone();
        let build = move || {
            let inst = inst2.clone();
            match inst.group(id) {
                Some(g) => group_page(inst.clone(), g, true),
                None => column(()).any(),
            }
        };
        nav = match &g.icon {
            Some(GroupIcon::Image(name)) => nav.item_icon(id, title, name.clone(), build),
            _ => nav.item(id, title, build),
        };
    }
    let nav = nav.any();
    #[cfg(all(feature = "xaml", target_os = "windows"))]
    let nav = xaml_impl::sidebar_symbols(nav, &inst.visible_groups());
    nav
}

/// A group's page: its sections as a form, then its custom piece.
fn group_page(inst: Rc<Installed>, g: &Group, desktop: bool) -> AnyPiece {
    // The phones draw a group as their own grouped table (a `UITableView` in the inset
    // grouped style, a Material preference list) when the arm provides one.
    if !desktop {
        #[cfg(all(feature = "uikit", target_os = "ios"))]
        {
            if let Some(p) = uikit_impl::group_page(inst.clone(), g) {
                return p;
            }
        }
        #[cfg(all(feature = "mdc", target_os = "android"))]
        {
            if let Some(p) = android_impl::group_page(inst.clone(), g) {
                return p;
            }
        }
        #[cfg(all(feature = "arkui", target_env = "ohos"))]
        {
            if let Some(p) = arkui_impl::group_page(inst.clone(), g) {
                return p;
            }
        }
    }
    composed_group_page(inst, g, desktop)
}

/// One group's sections with no scroll of their own (see [`Prefs::group_body`]). The phones'
/// native pages size themselves to their rows inside a Day `scroll`.
fn group_body(inst: Rc<Installed>, g: &Group) -> AnyPiece {
    #[cfg(all(feature = "uikit", target_os = "ios"))]
    {
        if let Some(p) = uikit_impl::group_body(inst.clone(), g) {
            return p;
        }
    }
    #[cfg(all(feature = "mdc", target_os = "android"))]
    {
        if let Some(p) = android_impl::group_body(inst.clone(), g) {
            return p;
        }
    }
    #[cfg(all(feature = "arkui", target_env = "ohos"))]
    {
        if let Some(p) = arkui_impl::group_body(inst.clone(), g) {
            return p;
        }
    }
    column((composed_group_form(inst, g, false),))
        .align(HAlign::Leading)
        .padding(16.0)
        .grow_w()
        .any()
}

/// A group's sections as a form, then its custom piece.
fn composed_group_form(inst: Rc<Installed>, g: &Group, desktop: bool) -> AnyPiece {
    let mut parts: Vec<AnyPiece> = Vec::new();
    for s in &g.sections {
        if let Some(p) = section_piece(inst.clone(), s, desktop) {
            parts.push(p);
        }
    }
    if let Some(custom) = &g.custom {
        parts.push(custom());
    }
    form(PieceVec(parts))
        .id(format!("prefs-group-{}", g.id))
        .any()
}

/// A group's page from ordinary Day pieces: its sections as a form, then its custom piece.
fn composed_group_page(inst: Rc<Installed>, g: &Group, desktop: bool) -> AnyPiece {
    let body = composed_group_form(inst, g, desktop);
    // A desktop panel fits its page (`size_to_fit`), so the page measures its content; past
    // the height the window can give it (a long page, a large text size) it scrolls.
    if desktop {
        hugging_scroll(
            column((body,))
                .align(HAlign::Leading)
                .padding(20.0)
                .grow_w(),
        )
        .any()
    } else {
        scroll(
            column((body,))
                .align(HAlign::Leading)
                .padding(16.0)
                .grow_w(),
        )
        .any()
    }
}

fn section_piece(inst: Rc<Installed>, s: &Section, desktop: bool) -> Option<AnyPiece> {
    let rows: Vec<AnyPiece> = s
        .rows
        .iter()
        .filter(|r| r.is_available())
        .map(|r| row_piece(inst.clone(), r, desktop))
        .collect();
    if rows.is_empty() {
        return None;
    }
    let mut sec = section(PieceVec(rows));
    if let Some(t) = &s.title {
        sec = sec.title(text_source_fn(t));
    }
    if let Some(f) = &s.footer {
        sec = sec.footer(text_source_fn(f));
    }
    Some(sec.any())
}

/// One composed row: title (and caption) leading, the control trailing, the System Settings
/// / iOS Settings shape every platform reads. A toolkit feature replaces it with its native row.
fn row_piece(inst: Rc<Installed>, r: &Row, desktop: bool) -> AnyPiece {
    #[cfg(feature = "gtk")]
    {
        if let Some(p) = gtk_impl::row_piece(inst.clone(), r, desktop) {
            return p;
        }
    }
    #[cfg(all(feature = "uikit", target_os = "ios"))]
    {
        if let Some(p) = uikit_impl::row_piece(inst.clone(), r, desktop) {
            return p;
        }
    }
    #[cfg(all(feature = "mdc", target_os = "android"))]
    {
        if let Some(p) = android_impl::row_piece(inst.clone(), r, desktop) {
            return p;
        }
    }
    #[cfg(all(feature = "xaml", target_os = "windows"))]
    {
        if let Some(p) = xaml_impl::row_piece(inst.clone(), r, desktop) {
            return p;
        }
    }
    #[cfg(all(feature = "arkui", target_env = "ohos"))]
    {
        if let Some(p) = arkui_impl::row_piece(inst.clone(), r, desktop) {
            return p;
        }
    }
    composed_row(inst, r, desktop)
}

/// The title column of a row: the title, and the caption under it in the footnote style.
fn title_column(r: &Row) -> AnyPiece {
    let title = text_label(&r.title);
    match &r.caption {
        Some(c) => column((
            title,
            text_label(c).font(day_spec::Font::Footnote).secondary(),
        ))
        .spacing(2.0)
        .align(HAlign::Leading)
        .any(),
        None => title.any(),
    }
}

/// Title leading, control trailing, on one line; a stretching control (a text field) takes
/// the row's remaining width.
fn trailing_row(titles: AnyPiece, control: AnyPiece) -> AnyPiece {
    // The title column wraps past a readable width rather than widening into the control:
    // a long caption would otherwise push a popup off the panel's edge.
    row((titles.max_width(TITLE_COLUMN_MAX), spacer(), control))
        .spacing(12.0)
        .align(VAlign::Center)
        .grow_w()
        .any()
}

/// The widest a composed row's title column grows before its caption wraps.
const TITLE_COLUMN_MAX: f64 = 360.0;
/// A composed slider's width, the same on every row so a page of sliders aligns.
const SLIDER_WIDTH: f64 = 200.0;
/// The value column beside a composed slider: room for "1000%", trailing-aligned.
const VALUE_WIDTH: f64 = 52.0;

fn composed_row(inst: Rc<Installed>, r: &Row, _desktop: bool) -> AnyPiece {
    let id = r.id.clone();
    let enabled = r.enabled;
    let with_id = |p: AnyPiece| match &id {
        Some(id) => p.id(id.clone()).any(),
        None => p,
    };
    let enabled_fn = move || enabled.is_none_or(|e| e.get());
    match &r.kind {
        RowKind::Toggle(sig) => {
            let ctl = with_id(toggle(*sig).enabled(enabled_fn).any());
            trailing_row(title_column(r), ctl)
        }
        RowKind::Choice {
            labels,
            index,
            style,
        } => {
            let p = choice_picker(labels, *index, *style == ChoiceStyle::Segmented, enabled_fn);
            trailing_row(title_column(r), with_id(p.any()))
        }
        RowKind::Slider {
            value,
            range,
            step,
            value_text,
        } => {
            let mut s = slider(*value).range(range.clone()).enabled(enabled_fn);
            if let Some(st) = step {
                s = s.step(*st);
            }
            // The id goes on the control itself, before the layout modifiers wrap it. The
            // slider takes one fixed width on every row, so a page of sliders lines up at
            // both ends however long each row's title is (System Settings does the same);
            // only a row too narrow for it stacks the control under the title.
            let ctl = match &id {
                Some(i) => s
                    .id(i.clone())
                    .min_width(SLIDER_WIDTH)
                    .max_width(SLIDER_WIDTH)
                    .any(),
                None => s.min_width(SLIDER_WIDTH).max_width(SLIDER_WIDTH).any(),
            };
            let ctl = match value_text {
                Some(f) => {
                    let f = f.clone();
                    let v = *value;
                    // One line, trailing-aligned in a fixed column so the values line up
                    // whatever their length; id `<id>-value` for scripts.
                    let value_label = label(move || f(v.get()))
                        .tabular()
                        .secondary()
                        .single_line()
                        .align(day_spec::props::TextAlign::Trailing);
                    let value_label = match &id {
                        Some(i) => value_label
                            .id(format!("{i}-value"))
                            .min_width(VALUE_WIDTH)
                            .max_width(VALUE_WIDTH)
                            .any(),
                        None => value_label
                            .min_width(VALUE_WIDTH)
                            .max_width(VALUE_WIDTH)
                            .any(),
                    };
                    row((ctl, value_label))
                        .spacing(8.0)
                        .align(VAlign::Center)
                        .any()
                }
                None => ctl,
            };
            trailing_row(title_column(r), ctl)
        }
        RowKind::Stepper { value, range, step } => {
            let v = *value;
            let (dec, inc) = stepper_buttons(v, range, *step, enabled_fn);
            let ctl = stepper_control(v, dec, inc, id.as_deref());
            trailing_row(title_column(r), ctl)
        }
        RowKind::Text {
            value,
            placeholder,
            secure,
        } => {
            let mut f = if *secure {
                secure_field(*value)
            } else {
                text_field(*value)
            };
            if let Some(ph) = placeholder {
                f = f.placeholder(text_source_fn(ph));
            }
            let f = f.enabled(enabled_fn);
            let ctl = match &id {
                Some(i) => f.id(i.clone()).grow_w().min_width(160.0).any(),
                None => f.grow_w().min_width(160.0).any(),
            };
            trailing_row(title_column(r), ctl)
        }
        RowKind::Action {
            on,
            destructive: _,
            confirm,
            confirm_label,
            busy,
        } => {
            let on = on.clone();
            let title = text_source_fn(&r.title);
            let confirm = confirm.clone();
            let confirm_label = confirm_label.clone();
            let busy = *busy;
            let run = move || {
                let on = on.clone();
                match &confirm {
                    Some(message) => {
                        let message = message.initial();
                        let title = title();
                        let label = confirm_label.as_ref().map(TextSource::initial);
                        day_core::task(async move {
                            if crate::confirm_dialog(&title, &message, label).await {
                                on();
                            }
                        });
                    }
                    None => on(),
                }
            };
            let b = button(text_source_fn(&r.title))
                .action(run)
                .enabled(move || enabled_fn() && !busy.is_some_and(|b| b.get()));
            let ctl = with_id(b.any());
            let ctl = match busy {
                Some(b) => row((when(move || b.get(), spinner), ctl))
                    .spacing(8.0)
                    .align(VAlign::Center)
                    .any(),
                None => ctl,
            };
            // The button carries the title; a caption is the row's explanatory line, so it
            // stands alone in the title column rather than repeating the title above it.
            match &r.caption {
                Some(c) => trailing_row(caption_only(c), ctl),
                None => ctl,
            }
        }
        RowKind::Link(url) => {
            let url = url.clone();
            let l = link(text_source_fn(&r.title), url);
            match &r.caption {
                Some(c) => trailing_row(caption_only(c), with_id(l.any())),
                None => with_id(l.any()),
            }
        }
        RowKind::Navigate(build) => {
            let build = build.clone();
            let title = text_source_fn(&r.title);
            let open = move || {
                let build = build.clone();
                let title = title();
                push_subpage(inst.clone(), title, build);
            };
            with_id(
                row((title_column(r), spacer(), label("›").secondary()))
                    .spacing(12.0)
                    .align(VAlign::Center)
                    .grow_w()
                    .on_tap(open)
                    .any(),
            )
        }
        RowKind::Info { value, copyable } => {
            let v = text_label(value).secondary();
            let v = if *copyable {
                // A visible affordance: the value plus a plain copy button.
                let text = value.clone();
                row((
                    v,
                    button("")
                        .icon(day_spec::Symbol::Copy)
                        .icon_only()
                        .plain()
                        .action(move || {
                            day_part_clipboard::set_text(&text.initial());
                        }),
                ))
                .spacing(4.0)
                .align(VAlign::Center)
                .any()
            } else {
                v.any()
            };
            trailing_row(title_column(r), with_id(v))
        }
        RowKind::Note => with_id(
            text_label(&r.title)
                .font(day_spec::Font::Footnote)
                .secondary()
                .any(),
        ),
        RowKind::Custom(build) => with_id(build()),
    }
}

/// Push a row's sub-page: onto the navigation presentation's stack when inside one, else into
/// a cover.
fn push_subpage(inst: Rc<Installed>, title: String, build: Build) {
    SUBPAGES.with(|s| s.borrow_mut().insert(title.clone(), build.clone()));
    #[cfg(feature = "gtk")]
    {
        // Into the open preferences dialog, as one of its own sub-pages.
        if gtk_impl::push_subpage(inst.clone(), title.clone(), build.clone()) {
            return;
        }
    }
    if inst.presentation == Presentation::Navigation {
        inst.path
            .update(|p| p.push(SettingsRoute::Group(format!("page:{title}"))));
    } else {
        day_core::open_window(
            Some(&format!("day.settings.{title}")),
            day_spec::WindowOptions {
                title,
                size: day_spec::Size::new(480.0, 560.0),
                size_to_fit: true,
                ..day_spec::WindowOptions::default()
            },
            day_spec::WindowKind::Preferences,
            move || build(),
        );
    }
}

async fn confirm_dialog(title: &str, message: &str, label: Option<String>) -> bool {
    let dialog = confirm(title.to_string())
        .message(message.to_string())
        .destructive();
    match label {
        Some(l) => dialog.confirm_label(l).await,
        None => dialog.await,
    }
}

/// A row's caption alone, for rows whose control already shows the title (a button, a link).
fn caption_only(c: &TextSource) -> AnyPiece {
    text_label(c)
        .font(day_spec::Font::Footnote)
        .secondary()
        .any()
}

/// The navigation presentation: a stack whose root lists About and the groups.
fn navigation_shell(inst: Rc<Installed>) -> AnyPiece {
    navigation_stack(inst, false)
}

/// The same inside a cover: a Done button closes it.
fn navigation_shell_with_done(inst: Rc<Installed>) -> AnyPiece {
    navigation_stack(inst, true)
}

fn navigation_stack(inst: Rc<Installed>, done: bool) -> AnyPiece {
    let root_inst = inst.clone();
    let dest_inst = inst.clone();
    let mut stack = nav_stack(inst.path, root_page(root_inst))
        .title(text_source_fn(&inst.title))
        .destination(move |r: &SettingsRoute| destination(dest_inst.clone(), r));
    if done {
        stack = stack.toolbar(
            toolbar_button("settings-done", day_fluent::tr("day-settings-done")).action(|| {
                if let Some(w) = day_core::window_by_key(day_core::windows::PREFERENCES_KEY) {
                    w.close();
                }
            }),
        );
    }
    stack.id("settings-stack").any()
}

fn destination(inst: Rc<Installed>, r: &SettingsRoute) -> AnyPiece {
    match r {
        SettingsRoute::About => about_page(inst),
        SettingsRoute::Group(g) => {
            if let Some(title) = g.strip_prefix("page:") {
                return SUBPAGES
                    .with(|s| s.borrow().get(title).cloned())
                    .map(|b| b())
                    .unwrap_or_else(|| column(()).any());
            }
            match inst.group(g) {
                Some(group) => group_page(inst.clone(), group, false),
                None => column(()).any(),
            }
        }
    }
}

/// The mobile root page: the About header card, then the groups as chevron rows (one group:
/// its rows inline under the header).
/// One row of the mobile root list, for a toolkit arm that draws the list natively: a group
/// (or the About header) with what the row shows and what tapping it does.
// Consumed by the uikit/android/arkui arms, which only compile for their targets.
#[allow(dead_code)]
pub(crate) struct RootEntry {
    /// The group id (`"about"` for the About header), also the row's element id suffix.
    pub id: String,
    pub title: TextSource,
    pub subtitle: Option<TextSource>,
    pub icon: Option<GroupIcon>,
    /// Push the entry's page onto the settings stack.
    pub open: Rc<dyn Fn()>,
}

/// The root list's entries in order: the About header first (when an About is declared),
/// then every visible group.
#[allow(dead_code)]
pub(crate) fn root_entries(inst: &Rc<Installed>) -> (Option<RootEntry>, Vec<RootEntry>) {
    let path = inst.path;
    let about = inst.about.as_ref().map(|about| RootEntry {
        id: "about".into(),
        title: TextSource::Static(about.display_name()),
        subtitle: about.version_line().map(TextSource::Static),
        icon: about.icon_name().cloned().map(GroupIcon::Image),
        open: Rc::new(move || path.update(|p| p.push(SettingsRoute::About))),
    });
    let groups = inst
        .visible_groups()
        .into_iter()
        .map(|g| {
            let id = g.id;
            RootEntry {
                id: id.to_string(),
                title: g.title.clone(),
                subtitle: g.subtitle.clone(),
                icon: g.icon.clone(),
                open: Rc::new(move || {
                    path.update(|p| p.push(SettingsRoute::Group(id.to_string())))
                }),
            }
        })
        .collect();
    (about, groups)
}

/// The mobile root page: the arm's native grouped list where one exists, else composed.
fn root_page(inst: Rc<Installed>) -> AnyPiece {
    #[cfg(all(feature = "uikit", target_os = "ios"))]
    {
        if let Some(p) = uikit_impl::root_page(inst.clone()) {
            return p;
        }
    }
    #[cfg(all(feature = "mdc", target_os = "android"))]
    {
        if let Some(p) = android_impl::root_page(inst.clone()) {
            return p;
        }
    }
    #[cfg(all(feature = "arkui", target_env = "ohos"))]
    {
        if let Some(p) = arkui_impl::root_page(inst.clone()) {
            return p;
        }
    }
    composed_root_page(inst)
}

/// The mobile root page from ordinary Day pieces: the About header card, then the groups as
/// chevron rows (one group: its rows inline under the header).
fn composed_root_page(inst: Rc<Installed>) -> AnyPiece {
    let mut parts: Vec<AnyPiece> = Vec::new();
    if let Some(about) = &inst.about {
        parts.push(about_header(inst.clone(), about));
    }
    let groups = inst.visible_groups();
    if groups.len() == 1 {
        let g = groups[0];
        for s in &g.sections {
            if let Some(p) = section_piece(inst.clone(), s, false) {
                parts.push(p);
            }
        }
        if let Some(custom) = &g.custom {
            parts.push(custom());
        }
    } else {
        let rows: Vec<AnyPiece> = groups
            .iter()
            .map(|g| {
                let id = g.id;
                let path = inst.path;
                let titles = match &g.subtitle {
                    Some(sub) => column((
                        text_label(&g.title),
                        text_label(sub).font(day_spec::Font::Footnote).secondary(),
                    ))
                    .spacing(2.0)
                    .align(HAlign::Leading)
                    .any(),
                    None => text_label(&g.title).any(),
                };
                // A bundled vector draws as an image; a standard glyph has no image name, so
                // it draws as a plain icon-only button that opens the same page (the only piece
                // that renders a `Symbol` on every toolkit). No icon: an empty slot keeps the
                // titles of iconed and icon-less rows aligned.
                // A standard glyph draws from day-spec's shared outline as a template image
                // (an icon-only button would bring the toolkit's button chrome with it: on
                // the web a pill that squashed the glyph). No icon: an empty slot keeps the
                // titles of iconed and icon-less rows aligned.
                let leading = match &g.icon {
                    Some(GroupIcon::Image(name)) => {
                        image(name.clone()).template().frame(24.0, 24.0).any()
                    }
                    Some(GroupIcon::Symbol(sym)) => match sym.outline_svg() {
                        Some(svg) => image(svg.into_bytes()).template().frame(24.0, 24.0).any(),
                        None => spacer().frame(24.0, 24.0).any(),
                    },
                    None => spacer().frame(24.0, 24.0).any(),
                };
                row((leading, titles, spacer(), label("›").secondary()))
                    .spacing(12.0)
                    .align(VAlign::Center)
                    .grow_w()
                    .on_tap(move || path.update(|p| p.push(SettingsRoute::Group(id.to_string()))))
                    .id(format!("prefs-group-{id}"))
                    .any()
            })
            .collect();
        parts.push(section(PieceVec(rows)).any());
    }
    scroll(
        form(PieceVec(parts))
            .padding(16.0)
            .grow_w()
            .id("settings-root"),
    )
    .any()
}

/// The About header card on the mobile root: icon, name, version; tapping opens the About page.
fn about_header(inst: Rc<Installed>, about: &About) -> AnyPiece {
    let path = inst.path;
    let name = about.display_name();
    let version = about.version_line().unwrap_or_default();
    let icon = match about.icon_name() {
        Some(n) => image(n.clone()).frame(48.0, 48.0).corner_radius(10.0).any(),
        None => column(()).any(),
    };
    section((row((
        icon,
        column((
            label(name).bold(),
            label(version).font(day_spec::Font::Footnote).secondary(),
        ))
        .spacing(2.0)
        .align(HAlign::Leading),
        spacer(),
        label("›").secondary(),
    ))
    .spacing(12.0)
    .align(VAlign::Center)
    .grow_w()
    .on_tap(move || path.update(|p| p.push(SettingsRoute::About)))
    .id("settings-about"),))
    .any()
}

/// The composed About page: icon, name, version, tagline, links, credits, licenses, copyright.
fn about_page(inst: Rc<Installed>) -> AnyPiece {
    let Some(about) = &inst.about else {
        return column(()).any();
    };
    if let Some(custom) = about.custom_piece() {
        return custom;
    }
    if inst.presentation == Presentation::Navigation {
        #[cfg(all(feature = "uikit", target_os = "ios"))]
        {
            if let Some(p) = uikit_impl::about_page(inst.clone()) {
                return p;
            }
        }
        #[cfg(all(feature = "mdc", target_os = "android"))]
        {
            if let Some(p) = android_impl::about_page(inst.clone()) {
                return p;
            }
        }
    }
    let mut head: Vec<AnyPiece> = Vec::new();
    if let Some(n) = about.icon_name() {
        head.push(image(n.clone()).frame(96.0, 96.0).corner_radius(20.0).any());
    }
    head.push(
        label(about.display_name())
            .font(day_spec::Font::Title2)
            .bold()
            .id("about-name")
            .any(),
    );
    if let Some(v) = about.version_line() {
        head.push(label(v).secondary().id("about-version").any());
    }
    if let Some(t) = about.tagline_text() {
        head.push(label(t).id("about-tagline").any());
    }
    let mut parts: Vec<AnyPiece> = vec![
        column(PieceVec(head))
            .spacing(6.0)
            .align(HAlign::Center)
            .grow_w()
            .any(),
    ];
    // Ids by position (`about-link-0`, …) and `about-privacy`: what a walkthrough asserts.
    let mut links: Vec<AnyPiece> = about
        .links()
        .iter()
        .enumerate()
        .map(|(i, (t, url))| {
            link(text_source_fn(t), url.clone())
                .id(format!("about-link-{i}"))
                .any()
        })
        .collect();
    if let Some(p) = about.privacy_url() {
        links.push(
            link(day_fluent::tr("day-about-privacy"), p.to_string())
                .id("about-privacy")
                .any(),
        );
    }
    if !links.is_empty() {
        parts.push(section(PieceVec(links)).any());
    }
    let mut notes: Vec<AnyPiece> = Vec::new();
    if let Some(c) = about.credits_text() {
        let credits = c.clone();
        notes.push(
            rows::navigate(day_fluent::tr("day-about-acknowledgements"), move || {
                scroll(label(credits.clone()).padding(16.0))
            })
            .into_piece(inst.clone()),
        );
    }
    if let Some(l) = about.licenses_text() {
        let licenses = l.clone();
        notes.push(
            rows::navigate(day_fluent::tr("day-about-licenses"), move || {
                scroll(label(licenses.clone()).monospace().padding(16.0))
            })
            .into_piece(inst.clone()),
        );
    }
    if !notes.is_empty() {
        parts.push(section(PieceVec(notes)).any());
    }
    if let Some(c) = about.copyright_text() {
        parts.push(
            label(c)
                .font(day_spec::Font::Footnote)
                .secondary()
                .id("about-copyright")
                .any(),
        );
    }
    let body = form(PieceVec(parts))
        .padding(20.0)
        .grow_w()
        .id("about-page");
    // The About window fits its content like the settings panel and scrolls past the height
    // it can have; a pushed page scrolls.
    if inst.presentation == Presentation::Window {
        hugging_scroll(body).any()
    } else {
        scroll(body).any()
    }
}

/// A vertical scroll that hugs its content: it measures as tall as the content up to the
/// height it is offered, so a window that fits its content (`size_to_fit`) still shrinks to a
/// short page, and a page taller than the window can be scrolls instead of clipping. Day's
/// `scroll` takes the whole proposal, which would pin a fitted window at its ceiling.
fn hugging_scroll<P: Piece>(child: P) -> impl Piece {
    day_core::piece_fn(move |cx: &mut day_core::BuildCx| {
        let node = cx.native(
            day_spec::kinds::SCROLL,
            &day_spec::props::ScrollProps { horizontal: false },
            Rc::new(HuggingScrollLayout),
            day_core::Flex {
                grow_w: true,
                ..Default::default()
            },
            day_core::Boundary::Yes,
        );
        cx.under(node, |cx| {
            let _ = child.build(cx);
        });
        node
    })
}

/// The layout behind [`hugging_scroll`]: `day_core::ScrollLayout` with a content-hugging
/// measure.
struct HuggingScrollLayout;

impl day_core::Layout for HuggingScrollLayout {
    fn measure(
        &self,
        cx: &mut dyn day_core::LayoutOps,
        children: &[day_core::RNode],
        p: day_spec::Proposal,
    ) -> day_spec::Size {
        let cs = match children.first() {
            Some(&c) => cx.measure_child(c, day_spec::Proposal::new(p.width, None)),
            None => day_spec::Size::ZERO,
        };
        let height = match p.height {
            Some(h) => cs.height.min(h),
            None => cs.height,
        };
        day_spec::Size::new(p.width.unwrap_or(cs.width), height)
    }
    fn place(
        &self,
        cx: &mut dyn day_core::LayoutOps,
        children: &[day_core::RNode],
        bounds: day_spec::Rect,
    ) {
        let Some(&c) = children.first() else {
            return;
        };
        let mut width = bounds.size.width;
        let mut cs = cx.measure_child(c, day_spec::Proposal::new(Some(width), None));
        if cs.height > bounds.size.height {
            // A classic scroll bar takes room from the viewport; lay the content out beside it.
            let inset = cx.scroll_bar_inset(true);
            if inset > 0.0 {
                width = (width - inset).max(0.0);
                cs = cx.measure_child(c, day_spec::Proposal::new(Some(width), None));
            }
        }
        let content = day_spec::Size::new(width, cs.height.max(bounds.size.height));
        cx.place_child(c, day_spec::Rect::from_size(content));
        cx.set_scroll_content(content);
    }
}

impl Row {
    /// Build this row as a piece for `inst`'s presentation (what sections do internally).
    fn into_piece(self, inst: Rc<Installed>) -> AnyPiece {
        let desktop = inst.presentation == Presentation::Window;
        row_piece(inst, &self, desktop)
    }
}

// ===========================================================================
// Conformance cases (docs/testing.md)
// ===========================================================================

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

/// The settings surface's conformance cases.
#[cfg(feature = "conformance")]
pub mod conformance {
    use day_core::conformance::{Case, Drive};
    use day_spec::Cap;

    use super::*;

    static SOUND: Pref<bool> = Pref::new("conformance.sound", || true);
    static SCALE: Pref<f64> = Pref::new("conformance.scale", || 100.0);

    /// Choosing a theme applies it and persists; System clears the stored key again.
    #[day_macros::test(day_core)]
    fn settings_theme_applies_and_persists() -> Case {
        Case::new()
            .proves_cap(Cap::Appearance)
            .proves_duty("set_appearance")
            .requires(Cap::Appearance)
            .page(|| {
                let inst = Rc::new(Installed {
                    about: None,
                    groups: vec![Group::new("general", "General").row(appearance_row())],
                    presentation: Presentation::Window,
                    title: TextSource::Static("Settings".into()),
                    selected: Signal::new(String::new()),
                    path: Signal::new(Vec::new()),
                });
                let g = inst.groups[0].clone();
                column((group_page(inst, &g, true),))
            })
            .drive(|d: Drive| async move {
                d.select("theme-picker", 1).await?;
                d.wait_idle().await?;
                d.check(
                    with_store(|s| s.get("day.appearance")).as_deref() == Some("dark"),
                    "dark not saved",
                )?;
                d.select("theme-picker", 2).await?;
                d.wait_idle().await?;
                d.check(
                    with_store(|s| s.get("day.appearance")).is_none(),
                    "System (the default) did not clear the key",
                )
            })
    }

    /// A toggle row and a slider row round-trip through the store.
    #[day_macros::test(day_core)]
    fn settings_rows_persist() -> Case {
        Case::new()
            .page(|| {
                let inst = Rc::new(Installed {
                    about: None,
                    groups: vec![
                        Group::new("sound", "Sound")
                            .row(rows::toggle(&SOUND, "Sounds"))
                            .row(rows::slider(&SCALE, "Scale", 50.0..=200.0).step(10.0)),
                    ],
                    presentation: Presentation::Window,
                    title: TextSource::Static("Settings".into()),
                    selected: Signal::new(String::new()),
                    path: Signal::new(Vec::new()),
                });
                let g = inst.groups[0].clone();
                column((group_page(inst, &g, true),))
            })
            .drive(|d: Drive| async move {
                d.toggle("pref-conformance.sound", false).await?;
                d.wait_idle().await?;
                d.check(!SOUND.get_untracked(), "toggle did not write the pref")?;
                d.check(
                    with_store(|s| s.get("conformance.sound")).as_deref() == Some("false"),
                    "toggle did not persist",
                )?;
                d.set_value("pref-conformance.scale", 150.0).await?;
                d.wait_idle().await?;
                d.check(
                    (SCALE.get_untracked() - 150.0).abs() < 0.5,
                    "slider did not write the pref",
                )?;
                SOUND.reset();
                d.check(
                    with_store(|s| s.get("conformance.sound")).is_none(),
                    "reset did not remove the key",
                )
            })
    }

    static COUNT: Pref<i64> = Pref::new("conformance.count", || 3);

    /// A row's enabled gate reaches every interactive control the row builds: a stepper's two
    /// buttons stay disabled while the gate is off, however the bounds would allow a step.
    #[day_macros::test(day_core)]
    fn settings_row_enabled_gates_stepper() -> Case {
        let gate = Signal::new(false);
        Case::new()
            .page(move || {
                let inst = Rc::new(Installed {
                    about: None,
                    groups: vec![
                        Group::new("counts", "Counts")
                            .row(rows::stepper(&COUNT, "Count", 0..=10).enabled(gate)),
                    ],
                    presentation: Presentation::Window,
                    title: TextSource::Static("Settings".into()),
                    selected: Signal::new(String::new()),
                    path: Signal::new(Vec::new()),
                });
                let g = inst.groups[0].clone();
                column((
                    group_page(inst, &g, true),
                    button("Enable").action(move || gate.set(true)).id("enable"),
                ))
            })
            .drive(|d: Drive| async move {
                d.assert_enabled("pref-conformance.count-increment", false)
                    .await?;
                d.assert_enabled("pref-conformance.count-decrement", false)
                    .await?;
                d.tap("enable").await?;
                d.wait_idle().await?;
                d.assert_enabled("pref-conformance.count-increment", true)
                    .await?;
                d.assert_enabled("pref-conformance.count-decrement", true)
                    .await?;
                let initial = COUNT.get_untracked();
                d.assert_text("pref-conformance.count", &initial.to_string())
                    .await?;
                d.tap("pref-conformance.count-increment").await?;
                d.assert_text("pref-conformance.count", &(initial + 1).to_string())
                    .await?;
                d.tap("pref-conformance.count-decrement").await?;
                d.assert_text("pref-conformance.count", &initial.to_string())
                    .await
            })
    }

    day_core::tests! { settings_theme_applies_and_persists, settings_rows_persist, settings_row_enabled_gates_stepper }
}
