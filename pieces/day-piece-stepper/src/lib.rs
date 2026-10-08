// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! day-piece-stepper: a numeric stepper field, a text field with increment/decrement arrows,
//! bound two-way to any `Binding<f64>` (docs/stepper.md).
//!
//! Two idioms, decided automatically. `Native` realizes a leaf where the platform ships the
//! widget: an `NSTextField` + `NSStepper` composite on AppKit (macOS has no combined
//! control; the pair is the platform idiom, see any inspector in Keynote), `GtkSpinButton`,
//! and a `QDoubleSpinBox` shim. `Composed` builds the same field from ordinary Day pieces
//! (a − button, a text field, a + button), which is what every backend without an arm gets
//! (uikit, mdc, arkui, dom, xaml and mock), so the piece works on all nine targets.
//!
//! The native leaf also accepts `Event::TextChanged` (a typed value, dayscript's `input:`
//! step) and `Event::ValueChanged`/`ValueCommitted` (dayscript's `set_value:`), and mirrors
//! its state into the dayscript probe (`assert_text` sees the display text, `assert_value`
//! the number); a satellite piece must report that itself, because day-core's probe
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

/// Full props (realize) for the native leaf. Everything but `value` is set once at build.
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
    /// The platform's own widget, taken literally: on a toolkit with no renderer it draws Day's
    /// visible placeholder. Pin it only behind a [`support`] check.
    Native,
    /// Day's own − / field / + row, identical on every target.
    Composed,
    /// [`Native`](StepperIdiom::Native) where an arm exists, otherwise
    /// [`Composed`](StepperIdiom::Composed). The default.
    #[default]
    Automatic,
}

/// Whether the compiled backend has a native stepper arm, which is what
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

/// The display form of `v` at `decimals` fraction digits: what the field shows, what the
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

/// `stepper(value)`: a numeric field with increment/decrement arrows. `value` is a
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
    /// Fraction digits shown (default 0, integers).
    pub fn decimals(mut self, decimals: u32) -> Self {
        self.decimals = decimals;
        self
    }
    /// Which control this renders as (see [`StepperIdiom`]).
    pub fn idiom(mut self, idiom: StepperIdiom) -> Self {
        self.idiom = idiom;
        self
    }
    /// Pin the platform's own widget, [`StepperIdiom::Native`].
    pub fn native(self) -> Self {
        self.idiom(StepperIdiom::Native)
    }
    /// Pin Day's composed row, [`StepperIdiom::Composed`].
    pub fn composed(self) -> Self {
        self.idiom(StepperIdiom::Composed)
    }
    /// The composed field's dayscript id (default `"stepper"`; its − and + buttons are
    /// `<key>-dec` and `<key>-inc`). It goes here rather than on
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
    // The leaf's dayscript id: one `.key` drives both idioms (`Decorate::id` on the piece
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
    // is a commit, one undoable unit per click, exactly like a built-in slider's committed
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

/// The composed idiom: a − button, a text field and a + button, ordinary pieces on every target.
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

    /// The field's binding: reads format the bound value, keystrokes are previews the value
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

    // Glyph-sized buttons: a stock Material button is 88 dp wide, and two of them beside the
    // field made the composed stepper a 240 dp control that clipped in a 280 dp inspector.
    // The buttons carry ids derived from the key, so a script can press them as a person would.
    let (dec_id, inc_id) = (format!("{key}-dec"), format!("{key}-inc"));
    // dayscript's `set_value:` lands `ValueChanged`/`ValueCommitted` on the id, which the
    // field itself has no use for (it carries text); the composed idiom takes them the way
    // the native leaf does, so a scripted value reaches the binding on every toolkit.
    // The id goes on the field itself, before the width (which wraps it in a frame node), so
    // the events land on the node the handler is on.
    let field = Valued {
        inner: text_field(FieldBinding {
            value: value.clone(),
            min,
            max,
            decimals,
        })
        .id(key),
        value,
        clamp,
    }
    .width(56.0);
    row((
        button("−").compact().action(dec).id(dec_id),
        field,
        button("+").compact().action(inc).id(inc_id),
    ))
    .spacing(4.0)
    .align(VAlign::Center)
    .build(cx)
}

/// The composed field with the numeric value events the native leaf answers (`set_value:`).
struct Valued<P: Piece, V: Binding<f64>, C: Fn(f64) -> f64 + 'static> {
    inner: P,
    value: V,
    clamp: C,
}

impl<P: Piece, V: Binding<f64>, C: Fn(f64) -> f64 + 'static> Piece for Valued<P, V, C> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let Valued {
            inner,
            value,
            clamp,
        } = self;
        let node = inner.build(cx);
        cx.on(node, move |ev| match ev {
            Event::ValueChanged(v) => value.write_preview(clamp(*v)),
            Event::ValueCommitted(v) => value.write_commit(clamp(*v)),
            _ => {}
        });
        node
    }
}

day_pieces::glue_modules!(appkit, gtk, qt);

/// The stepper's conformance cases (docs/testing.md), in both idioms.
#[cfg(feature = "conformance")]
pub mod conformance {
    use day_core::AnyPiece;
    use day_core::conformance::{Case, Drive};
    use day_pieces::*;
    use day_reactive::Signal;

    use super::{KIND, stepper};

    fn page(composed: bool) -> AnyPiece {
        let value = Signal::new(3.0f64);
        // Each idiom asked for explicitly: a toolkit with no native arm renders a placeholder
        // for `.native()`, and the native cases skip there.
        let field = stepper(value).range(0.0..=10.0).key("count");
        column((
            label(move || format!("value {}", value.get())).id("value"),
            if composed {
                field.composed().any()
            } else {
                field.native().any()
            },
        ))
        .any()
    }

    /// A value set on the stepper reaches its binding and shows in the field.
    #[day_macros::test(day_core)]
    fn stepper_sets_value() -> Case {
        Case::new()
            .proves(KIND)
            .page(|| page(false))
            .drive(|d: Drive| async move {
                d.assert_text("count", "3").await?;
                d.set_value("count", 7.0).await?;
                d.assert_text("value", "value 7").await?;
                d.assert_text("count", "7").await
            })
    }

    /// The range holds: a value past the top lands on it.
    #[day_macros::test(day_core)]
    fn stepper_clamps_to_range() -> Case {
        Case::new()
            .proves(KIND)
            .page(|| page(false))
            .drive(|d: Drive| async move {
                d.set_value("count", 42.0).await?;
                d.assert_text("value", "value 10").await
            })
    }

    /// The composed idiom (− / field / +) behaves the same on every toolkit.
    #[day_macros::test(day_core)]
    fn stepper_composed_sets_value() -> Case {
        // Proves no kind: the composed idiom is ordinary pieces, never the native leaf.
        Case::new()
            .page(|| page(true))
            .drive(|d: Drive| async move {
                // Typed text commits on submit, as Return does.
                d.input("count", "5").await?;
                d.submit("count").await?;
                d.assert_text("value", "value 5").await?;
                d.tap("count-inc").await?;
                d.assert_text("value", "value 6").await?;
                d.assert_text("count", "6").await?;
                // A scripted value lands on the field's id, as it does on the native leaf.
                d.set_value("count", 9.0).await?;
                d.assert_text("value", "value 9").await?;
                d.assert_text("count", "9").await
            })
    }

    day_core::tests! {
        stepper_sets_value,
        stepper_clamps_to_range,
        stepper_composed_sets_value,
    }
}
