// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Native input pieces: `picker` (a typed one-of-N selector: menu, segmented, or inline) and
//! `text_area` (a multi-line, auto-growing editor bound two-way to a `Signal<String>`).

use std::cell::RefCell;
use std::rc::Rc;

use day_core::*;
use day_reactive::{bind, bind_seeded};
use day_spec::{Event, kinds};

use crate::*;

// ---------------------------------------------------------------------------
// Picker (kinds::PICKER, docs/picker.md), built-in since 2026-07.
// ---------------------------------------------------------------------------

/// The options of a [`picker`]: `(value, title)` pairs, fixed or derived.
pub enum PickerOptions<T: 'static> {
    Fixed(Vec<(T, TextSource)>),
    Derived(Rc<dyn Fn() -> Vec<(T, TextSource)>>),
}

impl<T: Clone + 'static> PickerOptions<T> {
    /// The current `(values, titles)`: a tracked read, of the derived closure and of every
    /// title (so a localized title follows the locale).
    fn resolve(&self) -> (Vec<T>, Vec<String>) {
        let pairs = match self {
            PickerOptions::Fixed(v) => v.clone(),
            PickerOptions::Derived(f) => f(),
        };
        let mut values = Vec::with_capacity(pairs.len());
        let mut titles = Vec::with_capacity(pairs.len());
        for (v, t) in pairs {
            values.push(v);
            titles.push(t.resolve());
        }
        (values, titles)
    }
}

/// Anything [`picker`] takes as its options: an array or `Vec` of `(value, title)` pairs, or a
/// closure that derives the list and re-runs whenever its reactive reads change. Titles take the
/// same sources as a label. The two-level marker is the same E0119 dodge as
/// [`IntoText`](crate::IntoText).
pub trait IntoPickerOptions<T: 'static, M> {
    fn into_picker_options(self) -> PickerOptions<T>;
}

/// Marker for a fixed list of options.
pub struct FixedOptionsMark;
/// Marker for a derived list of options.
pub struct DerivedOptionsMark;

impl<T: 'static, X: IntoText<M>, M, const N: usize> IntoPickerOptions<T, (FixedOptionsMark, M)>
    for [(T, X); N]
{
    fn into_picker_options(self) -> PickerOptions<T> {
        PickerOptions::Fixed(self.into_iter().map(|(v, t)| (v, t.into_text())).collect())
    }
}

impl<T: 'static, X: IntoText<M>, M> IntoPickerOptions<T, (FixedOptionsMark, M)> for Vec<(T, X)> {
    fn into_picker_options(self) -> PickerOptions<T> {
        PickerOptions::Fixed(self.into_iter().map(|(v, t)| (v, t.into_text())).collect())
    }
}

impl<T: 'static, X: IntoText<M>, M, F> IntoPickerOptions<T, (DerivedOptionsMark, M)> for F
where
    F: Fn() -> Vec<(T, X)> + 'static,
{
    fn into_picker_options(self) -> PickerOptions<T> {
        PickerOptions::Derived(Rc::new(move || {
            self()
                .into_iter()
                .map(|(v, t)| (v, t.into_text()))
                .collect()
        }))
    }
}

/// How a picker reads and writes its selection: a required value ([`picker`]) or an optional
/// one ([`picker_optional`]). Implemented by the two adapters below, never by apps.
pub trait PickerSelection<T: 'static>: Clone + 'static {
    /// Tracked read of the current value, `None` when nothing is chosen.
    fn current(&self) -> Option<T>;
    /// The user chose `value`.
    fn choose(&self, value: T);
}

/// A [`picker`]'s selection: a binding to `T`.
#[derive(Clone)]
pub struct Required<Sel>(Sel);

impl<T: 'static, Sel: Binding<T>> PickerSelection<T> for Required<Sel> {
    fn current(&self) -> Option<T> {
        Some(self.0.read())
    }
    fn choose(&self, value: T) {
        self.0.write(value);
    }
}

/// A [`picker_optional`]'s selection: a binding to `Option<T>`.
#[derive(Clone)]
pub struct Optional<Sel>(Sel);

impl<T: 'static, Sel: Binding<Option<T>>> PickerSelection<T> for Optional<Sel> {
    fn current(&self) -> Option<T> {
        self.0.read()
    }
    fn choose(&self, value: T) {
        self.0.write(Some(value));
    }
}

/// A native one-of-N selector bound two-way to a typed value (docs/picker.md). Style via
/// `.menu()`/`.segmented()`/`.inline()`.
pub struct Picker<T: Clone + PartialEq + 'static, S: PickerSelection<T>> {
    options: PickerOptions<T>,
    separators_before: Vec<usize>,
    selected: S,
    style: day_spec::props::PickerStyle,
    enabled: Reactive<bool>,
}

/// `picker(sort, [(Sort::Name, res::str::name()), (Sort::Date, res::str::date())]).segmented()`:
/// `selected` is a two-way binding to the chosen value (a `Signal<T>`, a day-model `Field`, a
/// `Mapped` view), and the options are `(value, title)` pairs, fixed or derived from a closure.
/// The mapping between values and the native control's indexes lives here, once.
///
/// Selection rules (docs/picker.md): the picker never writes the binding on its own, only a
/// user's choice does. A bound value that is not among the options shows no selection where
/// the toolkit can show none, and the first option otherwise; empty options disable the
/// control; when the options change, the native index is re-derived from the value, and a
/// native choice that arrives after the options changed is mapped against the options the
/// control was last given.
pub fn picker<T, Sel, M>(
    selected: Sel,
    options: impl IntoPickerOptions<T, M>,
) -> Picker<T, Required<Sel>>
where
    T: Clone + PartialEq + 'static,
    Sel: Binding<T>,
{
    Picker {
        options: options.into_picker_options(),
        separators_before: Vec::new(),
        selected: Required(selected),
        style: day_spec::props::PickerStyle::Menu,
        enabled: Reactive::Const(true),
    }
}

/// [`picker`] over an optional value: `None` is "nothing chosen" (no selection where the
/// toolkit can show none), and a user's choice writes `Some(value)`.
pub fn picker_optional<T, Sel, M>(
    selected: Sel,
    options: impl IntoPickerOptions<T, M>,
) -> Picker<T, Optional<Sel>>
where
    T: Clone + PartialEq + 'static,
    Sel: Binding<Option<T>>,
{
    Picker {
        options: options.into_picker_options(),
        separators_before: Vec::new(),
        selected: Optional(selected),
        style: day_spec::props::PickerStyle::Menu,
        enabled: Reactive::Const(true),
    }
}

#[day_macros::builder(PickerBuilder)]
impl<T: Clone + PartialEq + 'static, S: PickerSelection<T>> Picker<T, S> {
    /// Whether the picker takes input (default `true`); a constant or a reactive `bool`.
    pub fn enabled<M>(mut self, v: impl IntoReactive<bool, M>) -> Self {
        self.enabled = v.into_reactive();
        self
    }
    /// Insert native menu separators before these option indexes. Indexes still refer to
    /// options, so separators never change the selection binding. AppKit/UIKit menu style
    /// honors this; other backends and styles display the same options without grouping.
    pub fn separators_before(mut self, indexes: impl IntoIterator<Item = usize>) -> Self {
        self.separators_before = indexes.into_iter().collect();
        self.separators_before.sort_unstable();
        self.separators_before.dedup();
        self
    }
    pub fn menu(mut self) -> Self {
        self.style = day_spec::props::PickerStyle::Menu;
        self
    }
    pub fn segmented(mut self) -> Self {
        self.style = day_spec::props::PickerStyle::Segmented;
        self
    }
    pub fn inline(mut self) -> Self {
        self.style = day_spec::props::PickerStyle::Inline;
        self
    }
    pub fn style(mut self, style: day_spec::props::PickerStyle) -> Self {
        self.style = style;
        self
    }
}

impl<T: Clone + PartialEq + 'static, S: PickerSelection<T>> Piece for Picker<T, S> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let Picker {
            options,
            separators_before,
            selected,
            style,
            enabled,
        } = self;
        let (values0, titles0) = day_reactive::untrack(|| options.resolve());
        debug_assert!(
            values0
                .iter()
                .enumerate()
                .all(|(i, v)| !values0[..i].contains(v)),
            "day: a picker's option values must be distinct"
        );
        // The options the control was last given: what a native index is mapped against.
        let values: Rc<RefCell<Vec<T>>> = Rc::new(RefCell::new(values0));
        let index_of = {
            let values = values.clone();
            move |v: Option<T>| -> Option<usize> {
                let v = v?;
                values.borrow().iter().position(|x| *x == v)
            }
        };
        let initial_index = index_of(day_reactive::untrack(|| selected.current())).unwrap_or(0);
        let enabled_now = enabled.get_untracked() && !titles0.is_empty();
        let initial = day_spec::props::PickerProps {
            options: titles0,
            separators_before,
            selected: initial_index,
            style,
            enabled: enabled_now,
        };
        let node = cx.leaf(kinds::PICKER, &initial, Flex::default());
        let patch = move |p: day_spec::props::PickerPatch, affects_size: bool| {
            with_tree(|t| t.patch(node, Box::new(p), affects_size));
        };
        // Availability: the app's value, and never with no options to choose from.
        {
            let enabled = enabled.clone();
            let values = values.clone();
            bind_seeded(
                enabled_now,
                move || enabled.get() && !values.borrow().is_empty(),
                move |e: &bool| patch(day_spec::props::PickerPatch::Enabled(*e), false),
            );
        }
        // A menu-style picker's intrinsic size follows the selected value (the collapsed
        // control renders it), so its selection patch must remeasure; without that the
        // control keeps the width of the build-time value and ellipsizes anything longer.
        // Segmented and inline render every option at once; selection moves a mark only.
        let affects_size = matches!(initial.style, day_spec::props::PickerStyle::Menu);
        // The options, live: a derived closure re-running, or a title following the locale.
        // The native items are patched (always a remeasure, since the option strings are the
        // control's intrinsic width), and the selected index re-derived from the value, since
        // the value's position may have moved.
        {
            let (values, sel) = (values.clone(), selected.clone());
            let index_of = index_of.clone();
            bind_seeded(
                initial.options.clone(),
                move || {
                    let (vs, titles) = options.resolve();
                    *values.borrow_mut() = vs;
                    titles
                },
                move |titles: &Vec<String>| {
                    patch(day_spec::props::PickerPatch::Options(titles.clone()), true);
                    if let Some(i) = index_of(day_reactive::untrack(|| sel.current())) {
                        patch(day_spec::props::PickerPatch::Selected(i), affects_size);
                    }
                },
            );
        }
        // The value, live: the app writing its binding moves the native selection. A value
        // absent from the options patches nothing (the control keeps showing what it shows).
        {
            let sel = selected.clone();
            let index_of = index_of.clone();
            bind_seeded(
                Some(initial.selected),
                move || index_of(sel.current()),
                move |i: &Option<usize>| {
                    if let Some(i) = i {
                        patch(day_spec::props::PickerPatch::Selected(*i), affects_size);
                    }
                },
            );
        }
        // The user's choice: a native index, mapped against the options the control was last
        // given; an index outside them (a stale event) is dropped.
        cx.on(node, move |ev| {
            if let Event::SelectionChanged(i) = ev
                && *i >= 0
                && let Some(v) = values.borrow().get(*i as usize).cloned()
            {
                selected.choose(v);
            }
        });
        node
    }
}

// ---------------------------------------------------------------------------
// Text area (kinds::TEXT_AREA, docs/textarea.md), built-in since 2026-07.
// ---------------------------------------------------------------------------

/// A native multi-line text editor bound two-way to `text`. Configure a prompt with
/// `.placeholder(_)`, the auto-growing height band with `.min_lines(_)` / `.max_lines(_)`, and the
/// native editor attributes with `.editable(_)` / `.selectable(_)` / `.spellcheck(_)` (each accepts
/// a constant or a reactive `bool`, and updates live). A backend that can't honor an attribute
/// answers the matching `Cap::Text{Editable,Selectable,SpellCheck}` with `Support::Unsupported`.
pub struct TextArea<S: Binding<String>> {
    text: S,
    placeholder: Option<TextSource>,
    min_lines: u32,
    max_lines: u32,
    editable: Reactive<bool>,
    selectable: Reactive<bool>,
    spellcheck: Reactive<bool>,
    on_submit: Option<Rc<dyn Fn()>>,
}

/// `text_area(text)`: a native multi-line editor whose contents mirror `text` in both
/// directions; `text` is a `Signal<String>` or any other two-way binding (a day-model `Field`).
pub fn text_area<S: Binding<String>>(text: S) -> TextArea<S> {
    TextArea {
        text,
        placeholder: None,
        min_lines: 1,
        max_lines: 0,
        editable: true.into_reactive(),
        selectable: true.into_reactive(),
        spellcheck: true.into_reactive(),
        on_submit: None,
    }
}

#[day_macros::builder(TextAreaBuilder)]
impl<S: Binding<String>> TextArea<S> {
    /// The empty-state prompt shown when the editor is empty (a constant, `Signal<String>`, or
    /// closure, evaluated once for the initial value; not reactive after build).
    pub fn placeholder<M>(mut self, t: impl IntoText<M>) -> Self {
        self.placeholder = Some(t.into_text());
        self
    }

    /// The minimum height, in text lines (default 1): the editor never shrinks below this.
    pub fn min_lines(mut self, lines: u32) -> Self {
        self.min_lines = lines.max(1);
        self
    }

    /// The maximum height, in text lines, before the editor scrolls internally. `0` (the
    /// default) means unbounded: the editor keeps growing and never scrolls.
    pub fn max_lines(mut self, lines: u32) -> Self {
        self.max_lines = lines;
        self
    }

    /// Whether the user can edit the text (default `true`; `false` = read-only). Reactive.
    pub fn editable<M>(mut self, v: impl IntoReactive<bool, M>) -> Self {
        self.editable = v.into_reactive();
        self
    }

    /// Whether the text can be selected and copied (default `true`). Reactive. `Unsupported` on
    /// backends where selection is always on (GTK).
    pub fn selectable<M>(mut self, v: impl IntoReactive<bool, M>) -> Self {
        self.selectable = v.into_reactive();
        self
    }

    /// Whether spell-check / autocorrect highlighting is on (default `true`). Reactive.
    /// `Unsupported` on backends with no built-in spell-check (GTK, Qt).
    pub fn spellcheck<M>(mut self, v: impl IntoReactive<bool, M>) -> Self {
        self.spellcheck = v.into_reactive();
        self
    }

    /// Submit on Enter: a plain Enter runs `f` instead of inserting a newline, the chat-composer
    /// contract. Shift+Enter still inserts a line break on desktop and ArkUI; Android and
    /// ArkUI show a Send action on the soft keyboard; iOS's return key submits. Backends without
    /// the intercept (web-dom today) keep inserting newlines, so pair this with a visible send button. The
    /// bound `text` signal is already up to date when `f` runs.
    pub fn on_submit(mut self, f: impl Fn() + 'static) -> Self {
        self.on_submit = Some(Rc::new(f));
        self
    }
}

impl<S: Binding<String>> Piece for TextArea<S> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let TextArea {
            text,
            placeholder,
            min_lines,
            max_lines,
            editable,
            selectable,
            spellcheck,
            on_submit,
        } = self;
        let initial = text.peek();
        let ph = placeholder.map(|p| p.initial()).unwrap_or_default();
        let node = cx.leaf(
            kinds::TEXT_AREA,
            &day_spec::props::TextAreaProps {
                text: initial.clone(),
                placeholder: ph,
                min_lines,
                // A 0 max is "unbounded"; a non-zero max is floored to min so the band is
                // never inverted.
                max_lines: if max_lines == 0 {
                    0
                } else {
                    max_lines.max(min_lines)
                },
                editable: editable.get_untracked(),
                selectable: selectable.get_untracked(),
                spellcheck: spellcheck.get_untracked(),
                submit_on_enter: on_submit.is_some(),
            },
            // A composer fills the available width; height is content-driven (the backend's
            // measure grows it between min/max lines), so it is not a height-growing leaf.
            Flex {
                grow_w: true,
                ..Default::default()
            },
        );
        // Live attributes: only a reactive source needs a binding (a constant is applied once at
        // realize). Each patches the backend when its value changes.
        if let Reactive::Dyn(_) = &editable {
            bind(
                move || editable.get(),
                move |v: &bool| {
                    with_tree(|t| {
                        t.patch(
                            node,
                            Box::new(day_spec::props::TextAreaPatch::SetEditable(*v)),
                            false,
                        )
                    });
                },
            );
        }
        if let Reactive::Dyn(_) = &selectable {
            bind(
                move || selectable.get(),
                move |v: &bool| {
                    with_tree(|t| {
                        t.patch(
                            node,
                            Box::new(day_spec::props::TextAreaPatch::SetSelectable(*v)),
                            false,
                        )
                    });
                },
            );
        }
        if let Reactive::Dyn(_) = &spellcheck {
            bind(
                move || spellcheck.get(),
                move |v: &bool| {
                    with_tree(|t| {
                        t.patch(
                            node,
                            Box::new(day_spec::props::TextAreaPatch::SetSpellCheck(*v)),
                            false,
                        )
                    });
                },
            );
        }
        // Controlled input with origin tracking (§4.4): the echo guard remembers the last value
        // that arrived from the native widget so bind_seeded does not patch it straight back.
        let guard: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
        let g = guard.clone();
        let tx = text.clone();
        bind_seeded(
            initial,
            move || tx.read(),
            move |t: &String| {
                let from_native = g.borrow_mut().take().as_deref() == Some(t.as_str());
                if !from_native {
                    with_tree(|tr| {
                        tr.patch(
                            node,
                            Box::new(day_spec::props::TextAreaPatch::SetText(t.clone())),
                            true,
                        )
                    });
                }
            },
        );
        cx.on(node, move |ev| match ev {
            Event::TextChanged(t) => {
                // Native edits skip the write-back patch below to preserve the caret/IME.
                // Keep inspection/dayscript text current even when no toolkit patch is sent.
                with_tree(|tree| tree.set_probe_value(node, 0.0, t.clone()));
                *guard.borrow_mut() = Some(t.clone());
                text.write(t.clone());
            }
            Event::Submitted => {
                if let Some(f) = &on_submit {
                    f();
                }
            }
            _ => {}
        });
        node
    }
}

// --- Typed builders, forwarded through `Decorated` (docs/api-style.md) ---

// ---------------------------------------------------------------------------
// Conformance cases (docs/testing.md)
// ---------------------------------------------------------------------------

/// The picker's and text area's `#[day::test]` cases, next to their constructors.
#[cfg(feature = "conformance")]
pub(crate) mod conformance {
    use day_core::conformance::{Case, Drive};
    use day_reactive::Signal;
    use day_spec::{Cap, kinds};

    use crate::*;

    const FRUIT: [(usize, &str); 3] = [(0, "Apple"), (1, "Pear"), (2, "Plum")];

    /// A menu picker shows its selection natively and reports a choice to its signal.
    #[day_macros::test(day_core)]
    fn picker_select() -> Case {
        let choice = Signal::new(0usize);
        Case::new()
            .proves(kinds::PICKER)
            .page(move || {
                column((
                    picker(choice, FRUIT).id("picker"),
                    label(move || choice.get().to_string()).id("index"),
                ))
                .spacing(8.0)
            })
            .shot("default")
            .drive(|d: Drive| async move {
                d.assert_text("picker", "Apple").await?;
                d.select("picker", 2).await?;
                d.assert_text("index", "2").await?;
                d.assert_text("picker", "Plum").await
            })
    }

    /// A segmented picker selects the same way.
    #[day_macros::test(day_core)]
    fn picker_segmented() -> Case {
        let choice = Signal::new(1usize);
        Case::new()
            .proves(kinds::PICKER)
            .page(move || {
                column((
                    picker(choice, FRUIT).segmented().id("picker"),
                    label(move || choice.get().to_string()).id("index"),
                ))
                .spacing(8.0)
            })
            .shot("default")
            .drive(|d: Drive| async move {
                d.assert_text("picker", "Pear").await?;
                d.select("picker", 0).await?;
                d.assert_text("index", "0").await?;
                d.assert_text("picker", "Apple").await
            })
    }

    /// An inline picker selects the same way.
    #[day_macros::test(day_core)]
    fn picker_inline() -> Case {
        let choice = Signal::new(0usize);
        Case::new()
            .proves(kinds::PICKER)
            .page(move || {
                column((
                    picker(choice, FRUIT).inline().id("picker"),
                    label(move || choice.get().to_string()).id("index"),
                ))
                .spacing(8.0)
            })
            .drive(|d: Drive| async move {
                d.select("picker", 1).await?;
                d.assert_text("index", "1").await?;
                d.assert_text("picker", "Pear").await
            })
    }

    /// A write to the picker's signal moves the native selection.
    #[day_macros::test(day_core)]
    fn picker_follows_signal() -> Case {
        let choice = Signal::new(0usize);
        Case::new()
            .proves(kinds::PICKER)
            .page(move || {
                column((
                    picker(choice, FRUIT).id("picker"),
                    button("Plum").action(move || choice.set(2)).id("set"),
                ))
                .spacing(8.0)
            })
            .drive(|d: Drive| async move {
                d.tap("set").await?;
                d.assert_text("picker", "Plum").await
            })
    }

    /// Options from data replace the native items and keep the selection where it exists.
    #[day_macros::test(day_core)]
    fn picker_options_reactive() -> Case {
        let choice = Signal::new(1usize);
        let more = Signal::new(false);
        Case::new()
            .proves(kinds::PICKER)
            .page(move || {
                column((
                    picker(choice, move || {
                        let mut v = vec![(0usize, "One"), (1, "Two")];
                        if more.get() {
                            v.push((2, "Three"));
                        }
                        v
                    })
                    .id("picker"),
                    button("More").action(move || more.set(true)).id("more"),
                ))
                .spacing(8.0)
            })
            .drive(|d: Drive| async move {
                d.assert_text("picker", "Two").await?;
                d.tap("more").await?;
                d.select("picker", 2).await?;
                d.assert_text("picker", "Three").await
            })
    }

    /// A disabled picker reports itself disabled, natively too, in every style, until enabled.
    #[day_macros::test(day_core)]
    fn picker_disabled() -> Case {
        let (a, b, c) = (
            Signal::new(0usize),
            Signal::new(0usize),
            Signal::new(0usize),
        );
        let enabled = Signal::new(false);
        Case::new()
            .proves(kinds::PICKER)
            .page(move || {
                column((
                    picker(a, FRUIT).enabled(move || enabled.get()).id("menu"),
                    picker(b, FRUIT)
                        .segmented()
                        .enabled(move || enabled.get())
                        .id("segmented"),
                    picker(c, FRUIT)
                        .inline()
                        .enabled(move || enabled.get())
                        .id("inline"),
                    button("Enable")
                        .action(move || enabled.set(true))
                        .id("enable"),
                ))
                .spacing(8.0)
            })
            .shot("disabled")
            .drive(|d: Drive| async move {
                for id in ["menu", "segmented", "inline"] {
                    d.assert_enabled(id, false).await?;
                }
                d.tap("enable").await?;
                for id in ["menu", "segmented", "inline"] {
                    d.assert_enabled(id, true).await?;
                }
                Ok(())
            })
    }

    /// A text area mirrors its signal both ways.
    #[day_macros::test(day_core)]
    fn text_area_binding() -> Case {
        let text = Signal::new(String::new());
        Case::new()
            .proves(kinds::TEXT_AREA)
            .page(move || {
                column((
                    text_area(text).placeholder("Notes").id("area"),
                    label(move || text.get().chars().count().to_string()).id("len"),
                    button("Set")
                        .action(move || text.set("Set from Day".into()))
                        .id("set"),
                ))
                .spacing(8.0)
            })
            .drive(|d: Drive| async move {
                d.input("area", "typed").await?;
                d.assert_text("len", "5").await?;
                d.tap("set").await?;
                d.assert_text("area", "Set from Day").await
            })
    }

    /// Lines survive the round trip: a text area holds and shows a newline as a newline.
    #[day_macros::test(day_core)]
    fn text_area_multiline() -> Case {
        let text = Signal::new("first\nsecond".to_owned());
        Case::new()
            .proves(kinds::TEXT_AREA)
            .page(move || text_area(text).min_lines(3).id("area"))
            .drive(|d: Drive| async move {
                d.assert_text("area", "first\nsecond").await?;
                d.input("area", "one\ntwo\nthree").await?;
                d.assert_text("area", "one\ntwo\nthree").await
            })
    }

    /// Submission observes the latest bound text and may clear the composer. This checks
    /// the public event contract; native drivers separately prove keyboard/IME routing.
    #[day_macros::test(day_core)]
    fn text_area_submit() -> Case {
        let text = Signal::new(String::new());
        let sent = Signal::new(String::new());
        let count = Signal::new(0u32);
        Case::new()
            .proves(kinds::TEXT_AREA)
            .page(move || {
                column((
                    text_area(text)
                        .min_lines(3)
                        .on_submit(move || {
                            sent.set(text.get_untracked());
                            count.update(|n| *n += 1);
                            text.set(String::new());
                        })
                        .id("area"),
                    label(move || sent.get()).id("sent"),
                    label(move || count.get().to_string()).id("count"),
                ))
            })
            .drive(|d: Drive| async move {
                d.input("area", "first\n東京").await?;
                d.assert_text("count", "0").await?;
                d.submit("area").await?;
                d.assert_text("sent", "first\n東京").await?;
                d.assert_text("count", "1").await?;
                d.assert_text("area", "").await?;
                d.input("area", "next").await?;
                d.submit("area").await?;
                d.assert_text("sent", "next").await?;
                d.assert_text("count", "2").await?;
                d.assert_text("area", "").await
            })
    }

    /// A read-only text area shows what its signal holds.
    #[day_macros::test(day_core)]
    fn text_area_read_only() -> Case {
        let text = Signal::new("Read me".to_owned());
        Case::new()
            .proves(kinds::TEXT_AREA)
            .proves_cap(Cap::TextEditable)
            .requires(Cap::TextEditable)
            .page(move || {
                column((
                    text_area(text).editable(false).id("area"),
                    button("Replace")
                        .action(move || text.set("Replaced".into()))
                        .id("replace"),
                ))
                .spacing(8.0)
            })
            .drive(|d: Drive| async move {
                d.assert_text("area", "Read me").await?;
                d.tap("replace").await?;
                d.assert_text("area", "Replaced").await
            })
    }

    /// A text area whose selection is turned off still shows its text; skipped where the
    /// toolkit's editor always allows selection (`Cap::TextSelectable`).
    #[day_macros::test(day_core)]
    fn text_area_unselectable() -> Case {
        let text = Signal::new("Fixed".to_owned());
        Case::new()
            .proves(kinds::TEXT_AREA)
            .proves_cap(Cap::TextSelectable)
            .requires(Cap::TextSelectable)
            .page(move || text_area(text).selectable(false).editable(false).id("area"))
            .drive(|d: Drive| async move { d.assert_text("area", "Fixed").await })
    }

    /// Changing editor policies must preserve content and programmatic updates. Native input
    /// rejection/selection needs the OS-input drivers; Drive::input injects a Day event.
    #[day_macros::test(day_core)]
    fn text_area_attributes_reactive() -> Case {
        let text = Signal::new("First\nsecond".to_owned());
        let editable = Signal::new(true);
        let selectable = Signal::new(true);
        Case::new()
            .proves(kinds::TEXT_AREA)
            .requires(Cap::TextEditable)
            .requires(Cap::TextSelectable)
            .page(move || {
                column((
                    text_area(text)
                        .editable(editable)
                        .selectable(selectable)
                        .id("area"),
                    label(move || text.get()).id("model"),
                    button("Lock").id("lock").action(move || {
                        editable.set(false);
                        selectable.set(false);
                    }),
                    button("Replace")
                        .id("replace")
                        .action(move || text.set("Updated\n東京".into())),
                    button("Unlock").id("unlock").action(move || {
                        selectable.set(true);
                        editable.set(true);
                    }),
                ))
            })
            .drive(|d: Drive| async move {
                d.tap("lock").await?;
                d.assert_text("area", "First\nsecond").await?;
                d.tap("replace").await?;
                d.assert_text("area", "Updated\n東京").await?;
                d.assert_text("model", "Updated\n東京").await?;
                d.tap("unlock").await?;
                d.assert_text("area", "Updated\n東京").await?;
                d.input("area", "Back to editing").await?;
                d.assert_text("model", "Back to editing").await
            })
    }

    day_core::tests! {
        picker_select,
        picker_segmented,
        picker_inline,
        picker_follows_signal,
        picker_options_reactive,
        picker_disabled,
        text_area_binding,
        text_area_multiline,
        text_area_submit,
        text_area_read_only,
        text_area_unselectable,
        text_area_attributes_reactive,
    }
}
