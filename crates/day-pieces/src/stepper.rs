// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! day_pieces::stepper: a numeric stepper field, a text field with increment/decrement arrows,
//! bound two-way to any `Binding<f64>` (docs/stepper.md).
//!
//! Two idioms, decided automatically. `Native` realizes a leaf where the platform ships the
//! widget: an `NSTextField` + `NSStepper` composite on AppKit (macOS has no combined
//! control; the pair is the platform idiom, see any inspector in Keynote), `GtkSpinButton`,
//! and a `QDoubleSpinBox` shim. `Composed` builds the same field from ordinary Day pieces
//! (a − button, a text field, a + button), which is what every backend without an arm gets
//! (uikit, material, arkui, dom, xaml and mock), so the piece works on all nine targets.
//!
//! The native leaf also accepts `Event::TextChanged` (a typed value, dayscript's `input:`
//! step) and `Event::ValueChanged`/`ValueCommitted` (dayscript's `set_value:`), and mirrors
//! its state into the dayscript probe (`assert_text` sees the display text, `assert_value`
//! the number); the frontend reports that explicitly, because day-core's probe
//! inspection only knows the builtin patch types.

use crate::prelude::*;
use day_core::{BuildCx, Flex, Piece, RNode, with_tree};
use day_reactive::{Binding, bind_seeded};
use day_spec::{Event, Support};
use std::rc::Rc;

pub use day_spec::controls::stepper::*;

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
pub fn support() -> day_spec::Support {
    let support = day_core::capability(day_spec::Cap::Stepper);
    if support == day_spec::Support::Unsupported {
        day_spec::Support::Emulated
    } else {
        support
    }
}

/// The display form of `v` at `decimals` fraction digits: what the field shows, what the
/// probe's text reports, and what the composed field parses back.
pub fn fmt_value(v: f64, decimals: u32) -> String {
    format!("{v:.prec$}", prec = decimals as usize)
}

type ParseValue = Rc<dyn Fn(&str) -> Option<f64>>;

#[derive(Clone)]
struct ValueFormat {
    display: Rc<dyn Fn(f64) -> String>,
    parse: ParseValue,
}

/// A stepper field bound two-way to a numeric binding. Build with [`stepper`].
pub struct Stepper<V: Binding<f64>> {
    value: V,
    min: f64,
    max: f64,
    step: f64,
    decimals: u32,
    format: Option<ValueFormat>,
    idiom: StepperIdiom,
    key: day_core::ElementId,
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
        format: None,
        idiom: StepperIdiom::Automatic,
        key: day_core::id!(stepper),
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
    /// Format the editable text and parse committed edits back to the bound number.
    /// `display` can return a generated Fluent message (`Into<String>`). Reads made by
    /// the formatter, including the locale, are reactive. `parse` returns `None` for
    /// invalid input; non-finite results are rejected and finite results clamp to the range.
    /// Return or focus loss restores the formatted value, including after a rejected edit.
    ///
    /// Custom formatting uses the composed − / field / + control on every toolkit,
    /// overriding the idiom and `.decimals()` settings. Numeric stepping and scripted
    /// `set_value` still use the bound number, not the displayed units.
    pub fn format<T: Into<String>>(
        mut self,
        display: impl Fn(f64) -> T + 'static,
        parse: impl Fn(&str) -> Option<f64> + 'static,
    ) -> Self {
        self.format = Some(ValueFormat {
            display: Rc::new(move |v| display(v).into()),
            parse: Rc::new(parse),
        });
        self
    }
    /// Which control this renders as (see [`StepperIdiom`]).
    pub fn idiom(mut self, idiom: StepperIdiom) -> Self {
        self.idiom = idiom;
        self
    }
    /// Pin the platform's own widget, [`StepperIdiom::Native`], unless `.format()` is set.
    pub fn native(self) -> Self {
        self.idiom(StepperIdiom::Native)
    }
    /// Pin Day's composed row, [`StepperIdiom::Composed`].
    pub fn composed(self) -> Self {
        self.idiom(StepperIdiom::Composed)
    }
    /// The field's dayscript id (default `"stepper"`; its − and + buttons are
    /// `<id>-dec` and `<id>-inc`). Names the interactive field in both idioms, rather than
    /// the composed layout row that a generic `Decorate::id` would tag.
    pub fn id(mut self, id: day_core::ElementId) -> Self {
        self.key = id;
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
        if native && self.format.is_none() {
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
    with_tree(|t| t.set_id(node, key.as_str().to_owned()));
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
            // Other custom events include SYNTHESIZED_TEXT, the paint half of scripted
            // typing. TextChanged below owns that edit; accepting both commits it twice.
            Event::Custom { tag, text, .. } if *tag == VALUE_TAG || tag.is_empty() => {
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
        format,
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
        format: Option<ValueFormat>,
    }
    impl<V: Binding<f64>> Clone for FieldBinding<V> {
        fn clone(&self) -> Self {
            FieldBinding {
                value: self.value.clone(),
                min: self.min,
                max: self.max,
                decimals: self.decimals,
                format: self.format.clone(),
            }
        }
    }
    impl<V: Binding<f64>> Binding<String> for FieldBinding<V> {
        fn read(&self) -> String {
            let value = self.value.read().clamp(self.min, self.max);
            match &self.format {
                Some(format) => (format.display)(value),
                None => fmt_value(value, self.decimals),
            }
        }
        fn peek(&self) -> String {
            day_reactive::untrack(|| self.read())
        }
        fn write(&self, s: String) {
            self.write_commit(s);
        }
        fn write_preview(&self, _s: String) {}
        fn write_commit(&self, s: String) {
            let parsed = match &self.format {
                Some(format) => (format.parse)(&s).filter(|v| v.is_finite()),
                None => s.trim().parse::<f64>().ok(),
            };
            if let Some(v) = parsed {
                self.value.write_commit(v.clamp(self.min, self.max));
            }
        }
    }

    // Glyph-sized buttons: a stock Material button is 88 dp wide, and two of them beside the
    // field made the composed stepper a 240 dp control that clipped in a 280 dp inspector.
    // The buttons carry ids derived from the key, so a script can press them as a person would.
    let (dec_id, inc_id) = (
        day_core::id!(stepper_decrement, key.as_str()),
        day_core::id!(stepper_increment, key.as_str()),
    );
    // dayscript's `set_value:` lands `ValueChanged`/`ValueCommitted` on the id, which the
    // field itself has no use for (it carries text); the composed idiom takes them the way
    // the native leaf does, so a scripted value reaches the binding on every toolkit.
    // The id goes on the field itself, before the width (which wraps it in a frame node), so
    // the events land on the node the handler is on.
    let binding = FieldBinding {
        value: value.clone(),
        min,
        max,
        decimals,
        format,
    };
    // A rejected edit or a differently spelled version of the same number does not
    // change the signal. Restore the formatted text explicitly at the end of editing.
    let restore = binding.format.as_ref().map(|_| {
        let binding = binding.clone();
        Rc::new(move || binding.peek()) as Rc<dyn Fn() -> String>
    });
    let field = Valued {
        inner: text_field(binding).id(key),
        value,
        clamp,
        restore,
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
    restore: Option<Rc<dyn Fn() -> String>>,
}

impl<P: Piece, V: Binding<f64>, C: Fn(f64) -> f64 + 'static> Piece for Valued<P, V, C> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let Valued {
            inner,
            value,
            clamp,
            restore,
        } = self;
        let node = inner.build(cx);
        cx.on(node, move |ev| match ev {
            Event::ValueChanged(v) => value.write_preview(clamp(*v)),
            Event::ValueCommitted(v) => value.write_commit(clamp(*v)),
            Event::Submitted | Event::FocusChanged(false) => {
                if let Some(display) = &restore {
                    let text = display();
                    with_tree(|t| {
                        t.patch(
                            node,
                            Box::new(day_spec::props::TextFieldPatch::Text {
                                text,
                                from_native: false,
                            }),
                            false,
                        )
                    });
                }
            }
            _ => {}
        });
        node
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use day_mock::MockToolkit;
    use day_reactive::{Signal, flush_sync};
    use day_spec::WindowOptions;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Records binding writes even when the value is unchanged: a duplicate commit creates
    /// a second history unit in bindings such as Day-Sketch's inspector.
    #[derive(Clone)]
    struct Writes {
        value: Signal<f64>,
        events: Rc<RefCell<Vec<(&'static str, f64)>>>,
    }

    impl Binding<f64> for Writes {
        fn read(&self) -> f64 {
            self.value.get()
        }
        fn peek(&self) -> f64 {
            self.value.get_untracked()
        }
        fn write(&self, value: f64) {
            self.write_commit(value);
        }
        fn write_preview(&self, value: f64) {
            self.events.borrow_mut().push(("preview", value));
            self.value.set(value);
        }
        fn write_commit(&self, value: f64) {
            self.events.borrow_mut().push(("commit", value));
            self.value.set(value);
        }
    }

    fn boot(native: bool) -> (Writes, RNode) {
        day_core::uninstall_tree();
        let writes = Writes {
            value: Signal::new(3.0),
            events: Rc::default(),
        };
        let binding = writes.clone();
        let (mock, _) = MockToolkit::new();
        day_core::launch_with(mock, WindowOptions::default(), move || {
            let field = stepper(binding).range(0.0..=10.0);
            if native {
                field.native()
            } else {
                field.composed()
            }
            .id(day_core::id!(fixture_stepper))
            .any()
        });
        let node = with_tree(|t| t.find_by_id(day_core::id!(fixture_stepper).as_str())).unwrap();
        (writes, node)
    }

    fn emit(node: RNode, event: Event) {
        day_core::enqueue_event(day_core::rnode_to_id(node), event);
        flush_sync();
    }

    #[test]
    fn formatted_edits_commit_once_clamp_and_restore_rejected_text() {
        for idiom in [
            StepperIdiom::Automatic,
            StepperIdiom::Native,
            StepperIdiom::Composed,
        ] {
            day_core::uninstall_tree();
            let writes = Writes {
                value: Signal::new(0.4),
                events: Rc::default(),
            };
            let binding = writes.clone();
            let (mock, probe) = MockToolkit::new();
            day_core::launch_with(mock, WindowOptions::default(), move || {
                // Synthetic formatting fixture; the website app exercises real Fluent messages.
                stepper(binding)
                    .range(0.0..=1.0)
                    .step(0.01)
                    .format(
                        |v| format!("{:.0}%", v * 100.0),
                        |text| {
                            text.trim()
                                .trim_end_matches('%')
                                .trim()
                                .parse::<f64>()
                                .ok()
                                .map(|v| v / 100.0)
                        },
                    )
                    .idiom(idiom)
                    .id(day_core::id!(fixture_stepper))
                    .any()
            });
            let node = with_tree(|t| t.find_by_id("fixture_stepper")).unwrap();
            let text = || probe.find_by_kind("day.text_field")[0].1.text.clone();
            assert_eq!(text(), "40%");
            assert!(
                probe.find_by_kind(KIND).is_empty(),
                "custom text always uses the composed control"
            );
            emit(node, Event::TextChanged("73%".into()));
            assert!(
                writes.events.borrow().is_empty(),
                "unfinished typing is not a numeric edit"
            );
            emit(node, Event::Submitted);
            emit(node, Event::FocusChanged(false));
            assert_eq!(*writes.events.borrow(), [("commit", 0.73)]);
            assert_eq!(text(), "73%");
            for invalid in ["invalid", "NaN%", "inf%", "-inf%"] {
                emit(node, Event::TextChanged(invalid.into()));
                emit(node, Event::Submitted);
                assert_eq!(writes.peek(), 0.73);
                assert_eq!(text(), "73%");
                assert_eq!(writes.events.borrow().len(), 1);
            }
            emit(node, Event::TextChanged("150%".into()));
            emit(node, Event::FocusChanged(false));
            assert_eq!(writes.peek(), 1.0);
            assert_eq!(text(), "100%");
            emit(node, Event::TextChanged("100".into()));
            emit(node, Event::Submitted);
            assert_eq!(
                text(),
                "100%",
                "canonical text is restored even when the number is unchanged"
            );
            assert_eq!(writes.events.borrow().len(), 3);
        }
    }

    #[test]
    fn formatted_reads_track_dependencies_and_numeric_events_keep_bound_units() {
        day_core::uninstall_tree();
        let value = Signal::new(0.4);
        let suffix = Signal::new("%");
        let (mock, probe) = MockToolkit::new();
        day_core::launch_with(mock, WindowOptions::default(), move || {
            stepper(value)
                .range(0.0..=1.0)
                .step(0.01)
                .format(
                    move |v| format!("{:.0}{}", v * 100.0, suffix.get()),
                    |_| None,
                )
                .id(day_core::id!(fixture_stepper))
                .any()
        });
        let text = || probe.find_by_kind("day.text_field")[0].1.text.clone();
        suffix.set(" %");
        flush_sync();
        assert_eq!(
            text(),
            "40 %",
            "formatter dependencies update without a numeric change"
        );
        let node = with_tree(|t| t.find_by_id("fixture_stepper")).unwrap();
        emit(node, Event::ValueChanged(0.6));
        assert_eq!(value.get(), 0.6);
        assert_eq!(text(), "60 %");
        emit(node, Event::ValueCommitted(0.7));
        assert_eq!(text(), "70 %");
        let inc = day_core::id!(stepper_increment, "fixture_stepper");
        let button = with_tree(|t| t.find_by_id(inc.as_str())).unwrap();
        emit(button, Event::Pressed);
        assert!((value.get() - 0.71).abs() < 1e-10);
        assert_eq!(text(), "71 %");
    }

    #[test]
    fn scripted_typing_commits_once_in_both_idioms() {
        for native in [true, false] {
            let (writes, node) = boot(native);
            // Use the actual dayscript input path: a native leaf receives SYNTHESIZED_TEXT
            // to paint the characters, followed by TextChanged for the edit itself.
            day_core::synthesize_text(node, "7".into());
            flush_sync();
            emit(node, Event::Submitted);
            assert_eq!(writes.peek(), 7.0);
            assert_eq!(
                *writes.events.borrow(),
                [("commit", 7.0)],
                "native={native}"
            );
        }
    }

    #[test]
    fn native_custom_events_accept_only_value_tags_and_the_legacy_bridge() {
        let (writes, node) = boot(true);
        emit(node, Event::custom(day_core::SYNTHESIZED_TEXT, "8"));
        emit(node, Event::custom("fixture:unrelated", "9"));
        assert_eq!(writes.peek(), 3.0);
        assert!(writes.events.borrow().is_empty());
        emit(node, Event::custom(VALUE_TAG, "6"));
        emit(node, Event::custom("", "12"));
        assert_eq!(*writes.events.borrow(), [("commit", 6.0), ("commit", 10.0)]);
        assert_eq!(writes.peek(), 10.0);
    }

    #[test]
    fn scripted_values_keep_the_preview_and_commit_boundary() {
        for native in [true, false] {
            let (writes, node) = boot(native);
            emit(node, Event::ValueChanged(12.0));
            emit(node, Event::ValueCommitted(12.0));
            assert_eq!(
                *writes.events.borrow(),
                [("preview", 10.0), ("commit", 10.0)],
                "native={native}"
            );
            assert_eq!(writes.peek(), 10.0);
        }
    }
}

/// The stepper's conformance cases (docs/testing.md), in both idioms.
#[cfg(feature = "conformance")]
pub mod conformance {
    use crate::*;
    use day_core::AnyPiece;
    use day_core::conformance::{Case, Drive};
    use day_reactive::Signal;

    use super::{KIND, stepper};

    fn page(composed: bool) -> AnyPiece {
        let value = Signal::new(3.0f64);
        // Each idiom asked for explicitly: a toolkit with no native arm renders a placeholder
        // for `.native()`, and the native cases skip there.
        let field = stepper(value)
            .range(0.0..=10.0)
            .id(day_core::id!(stepper::count));
        column((
            label(move || format!("value {}", value.get())).id(day_core::id!(stepper::value)),
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
                d.assert_text("stepper::count", "3").await?;
                d.set_value("stepper::count", 7.0).await?;
                d.assert_text("stepper::value", "value 7").await?;
                d.assert_text("stepper::count", "7").await
            })
    }

    /// The range holds: a value past the top lands on it.
    #[day_macros::test(day_core)]
    fn stepper_clamps_to_range() -> Case {
        Case::new()
            .proves(KIND)
            .page(|| page(false))
            .drive(|d: Drive| async move {
                d.set_value("stepper::count", 42.0).await?;
                d.assert_text("stepper::value", "value 10").await
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
                d.input("stepper::count", "5").await?;
                d.submit("stepper::count").await?;
                d.assert_text("stepper::value", "value 5").await?;
                d.tap(day_core::id!(stepper_increment, "stepper::count").as_str())
                    .await?;
                d.assert_text("stepper::value", "value 6").await?;
                d.assert_text("stepper::count", "6").await?;
                // A scripted value lands on the field's id, as it does on the native leaf.
                d.set_value("stepper::count", 9.0).await?;
                d.assert_text("stepper::value", "value 9").await?;
                d.assert_text("stepper::count", "9").await
            })
    }
}
