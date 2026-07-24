//! Input adapters that let a constructor accept a plain value, a `Signal`, or a closure
//! interchangeably: `IntoText`/`TextSource` (text), `IntoFraction`/`FractionSource` (0–1 numbers),
//! `SignalRw` (two-way bindings), and `IntoFocusBinding` (focus state).

use std::rc::Rc;

use day_core::*;
use day_reactive::{Signal, bind_seeded};
use day_spec::props::*;

// ---------------------------------------------------------------------------
// Text sources (§12.2's IntoText, M1 subset — Fluent joins at M6)
// ---------------------------------------------------------------------------

pub enum TextSource {
    Static(String),
    Dyn(Rc<dyn Fn() -> String>),
}

impl TextSource {
    pub fn initial(&self) -> String {
        match self {
            TextSource::Static(s) => s.clone(),
            TextSource::Dyn(f) => day_reactive::untrack(|| f()),
        }
    }
    /// Install the text binding for a realized node (no-op for static text).
    pub(crate) fn bind_to(
        self,
        node: RNode,
        make_patch: impl Fn(String) -> Box<dyn std::any::Any> + 'static,
        affects_size: bool,
    ) {
        if let TextSource::Dyn(f) = self {
            let seed = day_reactive::untrack(|| f());
            bind_seeded(
                seed,
                move || f(),
                move |t: &String| {
                    let patch = make_patch(t.clone());
                    with_tree(|tr| tr.patch(node, patch, affects_size));
                },
            );
        }
    }
}

/// Disjoint-marker conversion (the coherent form of §12.2's IntoText):
/// literals, `String`, `Signal<String>`, and closures all convert, each under its own marker.
pub trait IntoText<M> {
    fn into_text(self) -> TextSource;
}

pub struct StaticMark;
pub struct SignalMark;
pub struct FnMark;

/// A focus-binding target for [`Decorate::focused`] (docs/focus.md): either a `Signal<bool>`
/// (one control) or a `(Signal<Option<K>>, K)` pair (one control of a group sharing a signal).
/// The two-marker split is the same E0119 dodge as [`IntoText`].
pub trait IntoFocusBinding<M> {
    /// Split into (desired-focus tracked read, native-change write-back).
    #[allow(clippy::type_complexity)]
    fn into_focus_binding(self) -> (Box<dyn Fn() -> bool>, Box<dyn Fn(bool)>);
}

pub struct FocusBoolMark;
pub struct FocusGroupMark;

impl IntoFocusBinding<FocusBoolMark> for Signal<bool> {
    fn into_focus_binding(self) -> (Box<dyn Fn() -> bool>, Box<dyn Fn(bool)>) {
        (Box::new(move || self.get()), Box::new(move |f| self.set(f)))
    }
}

impl<K: Copy + PartialEq + 'static> IntoFocusBinding<FocusGroupMark> for (Signal<Option<K>>, K) {
    fn into_focus_binding(self) -> (Box<dyn Fn() -> bool>, Box<dyn Fn(bool)>) {
        let (sig, key) = self;
        (
            Box::new(move || sig.get() == Some(key)),
            Box::new(move |f| {
                if f {
                    sig.set(Some(key));
                } else if sig.get_untracked() == Some(key) {
                    // Only clear if the signal still names THIS control — when focus moved to a
                    // sibling, the paired gain (dispatched first, docs/focus.md) already wrote
                    // the new value and the group signal never passes through `None`.
                    sig.set(None);
                }
            }),
        )
    }
}

impl IntoText<StaticMark> for &str {
    fn into_text(self) -> TextSource {
        TextSource::Static(self.to_owned())
    }
}
impl IntoText<StaticMark> for String {
    fn into_text(self) -> TextSource {
        TextSource::Static(self)
    }
}
impl IntoText<SignalMark> for Signal<String> {
    fn into_text(self) -> TextSource {
        TextSource::Dyn(Rc::new(move || self.get()))
    }
}
impl<F, S> IntoText<FnMark> for F
where
    F: Fn() -> S + 'static,
    S: Into<String>,
{
    fn into_text(self) -> TextSource {
        TextSource::Dyn(Rc::new(move || self().into()))
    }
}

// ---------------------------------------------------------------------------
// Fraction sources — the read-only numeric analogue of TextSource, for `progress`.
// ---------------------------------------------------------------------------

pub enum FractionSource {
    Static(f64),
    Dyn(Rc<dyn Fn() -> f64>),
}

impl FractionSource {
    /// Untracked seed value, clamped to `0.0..=1.0`.
    pub fn initial(&self) -> f64 {
        let v = match self {
            FractionSource::Static(v) => *v,
            FractionSource::Dyn(f) => day_reactive::untrack(|| f()),
        };
        v.clamp(0.0, 1.0)
    }
    /// Install the fraction binding (no-op for static values). Writes are clamped so a
    /// backend never sees an out-of-range fraction.
    pub(crate) fn bind_to(self, node: RNode) {
        if let FractionSource::Dyn(f) = self {
            let seed = day_reactive::untrack(|| f()).clamp(0.0, 1.0);
            bind_seeded(
                seed,
                move || f().clamp(0.0, 1.0),
                move |v: &f64| {
                    with_tree(|tr| tr.patch(node, Box::new(ProgressPatch::Value(Some(*v))), false));
                },
            );
        }
    }
}

/// Disjoint-marker conversion (like [`IntoText`]) so `progress(_)` accepts a constant `f64`,
/// a `Signal<f64>`, or a closure. Reuses the same marker types.
pub trait IntoFraction<M> {
    fn into_fraction(self) -> FractionSource;
}

impl IntoFraction<StaticMark> for f64 {
    fn into_fraction(self) -> FractionSource {
        FractionSource::Static(self)
    }
}
impl IntoFraction<SignalMark> for Signal<f64> {
    fn into_fraction(self) -> FractionSource {
        FractionSource::Dyn(Rc::new(move || self.get()))
    }
}
impl<F> IntoFraction<FnMark> for F
where
    F: Fn() -> f64 + 'static,
{
    fn into_fraction(self) -> FractionSource {
        FractionSource::Dyn(Rc::new(self))
    }
}

// ---------------------------------------------------------------------------
// Two-way binding surface (§5.3)
// ---------------------------------------------------------------------------

pub trait SignalRw<T: 'static>: Clone + 'static {
    /// Tracked read.
    fn get_rw(&self) -> T;
    fn get_untracked_rw(&self) -> T;
    fn set_rw(&self, v: T);
}

impl<T: Clone + 'static> SignalRw<T> for Signal<T> {
    fn get_rw(&self) -> T {
        self.get()
    }
    fn get_untracked_rw(&self) -> T {
        self.get_untracked()
    }
    fn set_rw(&self, v: T) {
        self.set(v);
    }
}
