// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! day-piece-stepper — a numeric STEPPER FIELD: a text field with increment/decrement arrows,
//! bound two-way to any `Binding<f64>` (docs/stepper.md).
//!
//! Two idioms, decided automatically. NATIVE realizes a leaf where the platform ships the
//! widget: an `NSTextField` + `NSStepper` composite on AppKit (macOS has no combined
//! control — the pair IS the platform idiom, see any inspector in Keynote), `GtkSpinButton`,
//! and a `QDoubleSpinBox` shim. COMPOSED builds the same field from ordinary Day pieces
//! (a − button, a text field, a + button), which is what every backend without an arm gets —
//! uikit, mdc, arkui, dom, xaml and mock — so the piece works on all nine targets.
//!
//! The native leaf also accepts `Event::TextChanged` (a typed value — dayscript's `input:`
//! step) and `Event::ValueChanged`/`ValueCommitted` (dayscript's `set_value:`), and mirrors
//! its state into the dayscript probe (`assert_text` sees the display text, `assert_value`
//! the number) — a satellite piece must report that itself, because day-core's probe
//! inspection only knows the builtin patch types.

use day_core::{BuildCx, Flex, Piece, RNode, with_tree};
use day_pieces::prelude::*;
use day_reactive::{Binding, bind_seeded};
use day_spec::{Event, Support};

pub const KIND: &str = "day.piece.stepper";

/// The tag every in-process native arm reports a value under. Across a native boundary the
/// tag arrives empty and only the payload matters (§8.2); the front-end reads the text
/// either way.
pub const VALUE_TAG: &str = "stepper:value";

/// Full props (realize) for the NATIVE leaf. Everything but `value` is set once at build.
#[derive(Clone, Debug, PartialEq)]
pub struct StepperProps {
    pub value: f64,
    pub min: f64,
    pub max: f64,
    pub step: f64,
    /// Fraction digits the field shows (and the native widget's display precision).
    pub decimals: u32,
}

impl Default for StepperProps {
    fn default() -> Self {
        StepperProps {
            value: 0.0,
            min: 0.0,
            max: 100.0,
            step: 1.0,
            decimals: 0,
        }
    }
}

/// The single imperative update: show `value` in the field.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StepperPatch {
    SetValue(f64),
}

/// Which control this stepper renders as (the colorpicker's idiom shape).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StepperIdiom {
    /// The platform's own widget — literal: on a toolkit with no renderer it draws Day's
    /// visible placeholder. Pin it only behind a [`support`] check.
    Native,
    /// Day's own − / field / + row, identical on every target.
    Composed,
    /// [`Native`](StepperIdiom::Native) where an arm exists, otherwise
    /// [`Composed`](StepperIdiom::Composed). The default.
    #[default]
    Automatic,
}

/// Whether the compiled backend has a native stepper arm — what
/// [`StepperIdiom::Automatic`] resolves against. [`Support::Native`] on appkit, gtk and qt;
/// [`Support::Emulated`] everywhere else, where `Automatic` composes the field instead.
pub fn support() -> Support {
    if cfg!(any(
        all(feature = "appkit", target_os = "macos"),
        feature = "gtk",
        feature = "qt",
    )) {
        Support::Native
    } else {
        Support::Emulated
    }
}

/// The display form of `v` at `decimals` fraction digits — what the field shows, what the
/// probe's text reports, and what the composed field parses back.
pub fn fmt_value(v: f64, decimals: u32) -> String {
    format!("{v:.prec$}", prec = decimals as usize)
}

/// A stepper field bound two-way to a numeric binding. Build with [`stepper`].
pub struct Stepper<V: Binding<f64>> {
    value: V,
    min: f64,
    max: f64,
    step: f64,
    decimals: u32,
    idiom: StepperIdiom,
    key: String,
}

/// `stepper(value)` — a numeric field with increment/decrement arrows. `value` is a
/// `Signal<f64>`, a day-model `Field`, or any other two-way binding; a step click commits one
/// unit through it (`write_commit`), so under an undo stack each click is one undoable step.
pub fn stepper<V: Binding<f64>>(value: V) -> Stepper<V> {
    Stepper {
        value,
        min: 0.0,
        max: 100.0,
        step: 1.0,
        decimals: 0,
        idiom: StepperIdiom::Automatic,
        key: "stepper".to_string(),
    }
}

impl<V: Binding<f64>> Stepper<V> {
    /// The value's bounds (default `0.0..=100.0`). Typed and stepped values both clamp.
    pub fn range(mut self, range: std::ops::RangeInclusive<f64>) -> Self {
        self.min = *range.start();
        self.max = *range.end();
        self
    }
    /// One arrow click's increment (default 1).
    pub fn step(mut self, step: f64) -> Self {
        self.step = step.max(f64::EPSILON);
        self
    }
    /// Fraction digits shown (default 0 — integers).
    pub fn decimals(mut self, decimals: u32) -> Self {
        self.decimals = decimals;
        self
    }
    /// Which control this renders as (see [`StepperIdiom`]).
    pub fn idiom(mut self, idiom: StepperIdiom) -> Self {
        self.idiom = idiom;
        self
    }
    /// Pin the platform's own widget — [`StepperIdiom::Native`].
    pub fn native(self) -> Self {
        self.idiom(StepperIdiom::Native)
    }
    /// Pin Day's composed row — [`StepperIdiom::Composed`].
    pub fn composed(self) -> Self {
        self.idiom(StepperIdiom::Composed)
    }
    /// The COMPOSED field's dayscript id (default `"stepper"`). It goes here rather than on
    /// `Decorate::id` for the same reason the color well's does: what the app can reach from
    /// outside is the row wrapper, and an id on that tags a node no toolkit realizes. The
    /// native leaf takes this as its id too, so one name drives both idioms.
    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.key = key.into();
        self
    }
}

impl<V: Binding<f64>> Piece for Stepper<V> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let native = match self.idiom {
            StepperIdiom::Composed => false,
            StepperIdiom::Native => true,
            StepperIdiom::Automatic => support() == Support::Native,
        };
        if native {
            build_native(self, cx)
        } else {
            build_composed(self, cx)
        }
    }
}

/// The native idiom: one leaf of [`KIND`], bound two-way, probe kept current by hand.
fn build_native<V: Binding<f64>>(stepper: Stepper<V>, cx: &mut BuildCx) -> RNode {
    let Stepper {
        value,
        min,
        max,
        step,
        decimals,
        key,
        ..
    } = stepper;
    let clamp = move |v: f64| v.clamp(min, max);
    let initial = clamp(value.peek());
    let node = cx.leaf(
        KIND,
        &StepperProps {
            value: initial,
            min,
            max,
            step,
            decimals,
        },
        Flex::default(),
    );
    // The leaf's dayscript id — one `.key` drives both idioms (`Decorate::id` on the piece
    // would tag the wrapper the composed row returns, which no toolkit realizes).
    with_tree(|t| t.set_id(node, key));
    let note_probe = move |v: f64| {
        with_tree(|t| t.set_probe_value(node, v, fmt_value(v, decimals)));
    };
    note_probe(initial);
    // App writes → the native widget. Every arm no-ops on an unchanged value, so a step
    // echoing back through the binding never loops.
    {
        let v = value.clone();
        bind_seeded(
            initial,
            move || clamp(v.read()),
            move |val: &f64| {
                with_tree(|t| t.patch(node, Box::new(StepperPatch::SetValue(*val)), false));
                note_probe(*val);
            },
        );
    }
    // Native steps and typed edits (`Custom`), dayscript's `input:` (`TextChanged`) and
    // `set_value:` (`ValueChanged`/`ValueCommitted`) → the binding. A step or a settled edit
    // is a COMMIT — one undoable unit per click, exactly like a built-in slider's committed
    // value; only `ValueChanged` stays a preview.
    cx.on(node, move |ev| {
        match ev {
            Event::Custom { text, .. } => {
                if let Ok(v) = text.trim().parse::<f64>() {
                    value.write_commit(clamp(v));
                }
            }
            Event::TextChanged(s) => {
                if let Ok(v) = s.trim().parse::<f64>() {
                    value.write_commit(clamp(v));
                }
            }
            Event::ValueChanged(v) => value.write_preview(clamp(*v)),
            Event::ValueCommitted(v) => value.write_commit(clamp(*v)),
            _ => {}
        };
    });
    node
}

/// The composed idiom: a − button, a text field, a + button — ordinary pieces, every target.
fn build_composed<V: Binding<f64>>(stepper: Stepper<V>, cx: &mut BuildCx) -> RNode {
    let Stepper {
        value,
        min,
        max,
        step,
        decimals,
        key,
        ..
    } = stepper;
    let clamp = move |v: f64| v.clamp(min, max);
    let stepped = {
        let value = value.clone();
        move |dir: f64| {
            let v = clamp(value.peek() + dir * step);
            value.write_commit(v);
        }
    };
    let dec = {
        let stepped = stepped.clone();
        move || stepped(-1.0)
    };
    let inc = move || stepped(1.0);

    /// The field's seam: reads format the bound value, keystrokes are previews the value
    /// must not follow (half-typed numbers are not values), and the committed text (Return,
    /// focus loss, dayscript `submit:`) parses, clamps, and writes through.
    struct FieldBinding<V: Binding<f64>> {
        value: V,
        min: f64,
        max: f64,
        decimals: u32,
    }
    impl<V: Binding<f64>> Clone for FieldBinding<V> {
        fn clone(&self) -> Self {
            FieldBinding {
                value: self.value.clone(),
                min: self.min,
                max: self.max,
                decimals: self.decimals,
            }
        }
    }
    impl<V: Binding<f64>> Binding<String> for FieldBinding<V> {
        fn read(&self) -> String {
            fmt_value(self.value.read().clamp(self.min, self.max), self.decimals)
        }
        fn peek(&self) -> String {
            fmt_value(self.value.peek().clamp(self.min, self.max), self.decimals)
        }
        fn write(&self, s: String) {
            self.write_commit(s);
        }
        fn write_preview(&self, _s: String) {}
        fn write_commit(&self, s: String) {
            if let Ok(v) = s.trim().parse::<f64>() {
                self.value.write_commit(v.clamp(self.min, self.max));
            }
        }
    }

    row((
        button("−").action(dec),
        text_field(FieldBinding {
            value,
            min,
            max,
            decimals,
        })
        .id(key)
        .width(56.0),
        button("+").action(inc),
    ))
    .spacing(4.0)
    .align(VAlign::Center)
    .build(cx)
}

day_pieces::glue_modules!(appkit, gtk, qt);
