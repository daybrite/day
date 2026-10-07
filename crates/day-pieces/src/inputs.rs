// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Native input pieces: `picker` (a bound one-of-N selector: menu, segmented, or inline) and
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

/// A native picker bound two-way to `selected`. Style via `.menu()`/`.segmented()`/`.inline()`.
pub struct Picker<Sel: Binding<usize>> {
    options: Vec<String>,
    separators_before: Vec<usize>,
    reactive_options: Option<Rc<dyn Fn() -> Vec<String>>>,
    selected: Sel,
    style: day_spec::props::PickerStyle,
    enabled: Reactive<bool>,
}

/// `picker(["A", "B", "C"], choice).segmented()`: options are fixed, `selected` is the bound
/// index: a `Signal<usize>`, or any other two-way binding (a day-model `Field`, a `Mapped` view).
pub fn picker<S: Into<String>, Sel: Binding<usize>>(
    options: impl IntoIterator<Item = S>,
    selected: Sel,
) -> Picker<Sel> {
    Picker {
        options: options.into_iter().map(Into::into).collect(),
        separators_before: Vec::new(),
        reactive_options: None,
        selected,
        style: day_spec::props::PickerStyle::Menu,
        enabled: Reactive::Const(true),
    }
}

impl<Sel: Binding<usize>> Picker<Sel> {
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
    /// Recompute the option labels reactively, for choices that come from data (the open
    /// documents, a live count) rather than from a fixed list. The labels passed to
    /// [`picker`] seed the control; every later change patches the native items in place,
    /// keeping the selected index where it still exists.
    ///
    /// The count may change too, so a shrinking list can strand the app's `selected`
    /// binding past the end; the backend clamps its own selection, and the app is expected
    /// to write a valid index. Reach for this only when the options really do change, since
    /// rebuilding a native menu costs more than moving a mark.
    pub fn options_reactive(mut self, f: impl Fn() -> Vec<String> + 'static) -> Self {
        self.reactive_options = Some(Rc::new(f));
        self
    }
}

impl<Sel: Binding<usize>> Piece for Picker<Sel> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let Picker {
            options,
            reactive_options,
            separators_before,
            selected,
            style,
            enabled,
        } = self;
        let initial = day_spec::props::PickerProps {
            options,
            separators_before,
            selected: selected.peek(),
            style,
            enabled: enabled.get_untracked(),
        };
        let node = cx.leaf(kinds::PICKER, &initial, Flex::default());
        // A reactive `enabled` patches on change; a constant is applied once at realize.
        if let Reactive::Dyn(_) = &enabled {
            bind(
                move || enabled.get(),
                move |e: &bool| {
                    with_tree(|t| {
                        t.patch(
                            node,
                            Box::new(day_spec::props::PickerPatch::Enabled(*e)),
                            false,
                        )
                    });
                },
            );
        }
        // Data-driven labels: patch the native items whenever they change. Always a
        // remeasure, since the option strings are the control's intrinsic width, in every style.
        if let Some(f) = reactive_options {
            bind_seeded(
                initial.options.clone(),
                move || f(),
                move |opts: &Vec<String>| {
                    with_tree(|t| {
                        t.patch(
                            node,
                            Box::new(day_spec::props::PickerPatch::Options(opts.clone())),
                            true,
                        )
                    });
                },
            );
        }
        // A menu-style picker's intrinsic size follows the selected value (the collapsed
        // control renders it), so its selection patch must remeasure; without that the
        // control keeps the width of the build-time value and ellipsizes anything longer.
        // Segmented and inline render every option at once; selection moves a mark only.
        let affects_size = matches!(initial.style, day_spec::props::PickerStyle::Menu);
        let sel = selected.clone();
        bind_seeded(
            initial.selected,
            move || sel.read(),
            move |v: &usize| {
                with_tree(|t| {
                    t.patch(
                        node,
                        Box::new(day_spec::props::PickerPatch::Selected(*v)),
                        affects_size,
                    )
                });
            },
        );
        cx.on(node, move |ev| {
            if let Event::SelectionChanged(i) = ev
                && *i >= 0
            {
                selected.write(*i as usize);
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
    /// contract. Shift+Enter still inserts a line break on the desktop toolkits; Android's soft
    /// keyboard shows a Send action; iOS's return key submits. Backends without the intercept
    /// (web-dom today) keep inserting newlines, so pair this with a visible send button. The
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

/// [`Picker`]'s own builders, reachable through a decoration (§5.2): `Decorated` forwards them
/// to the piece it wraps, so generic modifiers and typed ones chain in any order.
pub trait PickerBuilder: Sized {
    fn separators_before(self, indexes: impl IntoIterator<Item = usize>) -> Self;
    fn menu(self) -> Self;
    fn segmented(self) -> Self;
    fn inline(self) -> Self;
    fn style(self, style: day_spec::props::PickerStyle) -> Self;
    fn enabled<M>(self, v: impl IntoReactive<bool, M>) -> Self;
}

impl<Sel: Binding<usize>> PickerBuilder for Picker<Sel> {
    fn separators_before(self, indexes: impl IntoIterator<Item = usize>) -> Self {
        Picker::separators_before(self, indexes)
    }
    fn menu(self) -> Self {
        Picker::menu(self)
    }
    fn segmented(self) -> Self {
        Picker::segmented(self)
    }
    fn inline(self) -> Self {
        Picker::inline(self)
    }
    fn style(self, style: day_spec::props::PickerStyle) -> Self {
        Picker::style(self, style)
    }
    fn enabled<M>(self, v: impl IntoReactive<bool, M>) -> Self {
        Picker::enabled(self, v)
    }
}

impl<Inner: PickerBuilder + Piece> PickerBuilder for Decorated<Inner> {
    fn separators_before(self, indexes: impl IntoIterator<Item = usize>) -> Self {
        self.map_inner(|inner| inner.separators_before(indexes))
    }
    fn menu(self) -> Self {
        self.map_inner(|inner_piece| inner_piece.menu())
    }
    fn segmented(self) -> Self {
        self.map_inner(|inner_piece| inner_piece.segmented())
    }
    fn inline(self) -> Self {
        self.map_inner(|inner_piece| inner_piece.inline())
    }
    fn style(self, style: day_spec::props::PickerStyle) -> Self {
        self.map_inner(|inner_piece| inner_piece.style(style))
    }
    fn enabled<M>(self, v: impl IntoReactive<bool, M>) -> Self {
        self.map_inner(|inner_piece| inner_piece.enabled(v))
    }
}

/// [`TextArea`]'s own builders, reachable through a decoration (§5.2): `Decorated` forwards them
/// to the piece it wraps, so generic modifiers and typed ones chain in any order.
pub trait TextAreaBuilder: Sized {
    fn placeholder<M>(self, t: impl IntoText<M>) -> Self;
    fn min_lines(self, lines: u32) -> Self;
    fn max_lines(self, lines: u32) -> Self;
    fn editable<M>(self, v: impl IntoReactive<bool, M>) -> Self;
    fn spellcheck<M>(self, v: impl IntoReactive<bool, M>) -> Self;
    fn on_submit(self, f: impl Fn() + 'static) -> Self;
}

impl<S: Binding<String>> TextAreaBuilder for TextArea<S> {
    fn placeholder<M>(self, t: impl IntoText<M>) -> Self {
        TextArea::placeholder(self, t)
    }
    fn min_lines(self, lines: u32) -> Self {
        TextArea::min_lines(self, lines)
    }
    fn max_lines(self, lines: u32) -> Self {
        TextArea::max_lines(self, lines)
    }
    fn editable<M>(self, v: impl IntoReactive<bool, M>) -> Self {
        TextArea::editable(self, v)
    }
    fn spellcheck<M>(self, v: impl IntoReactive<bool, M>) -> Self {
        TextArea::spellcheck(self, v)
    }
    fn on_submit(self, f: impl Fn() + 'static) -> Self {
        TextArea::on_submit(self, f)
    }
}

impl<Inner: TextAreaBuilder + Piece> TextAreaBuilder for Decorated<Inner> {
    fn placeholder<M>(self, t: impl IntoText<M>) -> Self {
        self.map_inner(|inner_piece| inner_piece.placeholder(t))
    }
    fn min_lines(self, lines: u32) -> Self {
        self.map_inner(|inner_piece| inner_piece.min_lines(lines))
    }
    fn max_lines(self, lines: u32) -> Self {
        self.map_inner(|inner_piece| inner_piece.max_lines(lines))
    }
    fn editable<M>(self, v: impl IntoReactive<bool, M>) -> Self {
        self.map_inner(|inner_piece| inner_piece.editable(v))
    }
    fn spellcheck<M>(self, v: impl IntoReactive<bool, M>) -> Self {
        self.map_inner(|inner_piece| inner_piece.spellcheck(v))
    }
    fn on_submit(self, f: impl Fn() + 'static) -> Self {
        self.map_inner(|inner_piece| inner_piece.on_submit(f))
    }
}

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

    const FRUIT: [&str; 3] = ["Apple", "Pear", "Plum"];

    /// A menu picker shows its selection natively and reports a choice to its signal.
    #[day_macros::test(day_core)]
    fn picker_select() -> Case {
        let choice = Signal::new(0usize);
        Case::new()
            .proves(kinds::PICKER)
            .page(move || {
                column((
                    picker(FRUIT, choice).id("picker"),
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
                    picker(FRUIT, choice).segmented().id("picker"),
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
                    picker(FRUIT, choice).inline().id("picker"),
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
                    picker(FRUIT, choice).id("picker"),
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
                    picker(["One", "Two"], choice)
                        .options_reactive(move || {
                            let mut v = vec!["One".to_owned(), "Two".to_owned()];
                            if more.get() {
                                v.push("Three".to_owned());
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
                    picker(FRUIT, a).enabled(move || enabled.get()).id("menu"),
                    picker(FRUIT, b)
                        .segmented()
                        .enabled(move || enabled.get())
                        .id("segmented"),
                    picker(FRUIT, c)
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

    day_core::tests! {
        picker_select,
        picker_segmented,
        picker_inline,
        picker_follows_signal,
        picker_options_reactive,
        picker_disabled,
        text_area_binding,
        text_area_multiline,
        text_area_read_only,
        text_area_unselectable,
    }
}
