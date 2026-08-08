// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Native input pieces: `picker` (a bound one-of-N selector — menu, segmented, or inline) and
//! `text_area` (a multi-line, auto-growing editor bound two-way to a `Signal<String>`).

use std::cell::RefCell;
use std::rc::Rc;

use day_core::*;
use day_reactive::{Signal, bind, bind_seeded};
use day_spec::{Event, kinds};

use crate::*;

// ---------------------------------------------------------------------------
// Picker (kinds::PICKER, docs/picker.md) — built-in since 2026-07.
// ---------------------------------------------------------------------------

/// A native picker bound two-way to `selected`. Style via `.menu()`/`.segmented()`/`.inline()`.
pub struct Picker {
    options: Vec<String>,
    selected: Signal<usize>,
    style: day_spec::props::PickerStyle,
}

/// `picker(["A", "B", "C"], choice).segmented()` — options are fixed, `selected` is the bound index.
pub fn picker<S: Into<String>>(
    options: impl IntoIterator<Item = S>,
    selected: Signal<usize>,
) -> Picker {
    Picker {
        options: options.into_iter().map(Into::into).collect(),
        selected,
        style: day_spec::props::PickerStyle::Menu,
    }
}

impl Picker {
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

impl Piece for Picker {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let Picker {
            options,
            selected,
            style,
        } = self;
        let initial = day_spec::props::PickerProps {
            options,
            selected: selected.get_untracked(),
            style,
        };
        let node = cx.leaf(kinds::PICKER, &initial, Flex::default());
        bind_seeded(
            initial.selected,
            move || selected.get(),
            move |v: &usize| {
                with_tree(|t| {
                    t.patch(
                        node,
                        Box::new(day_spec::props::PickerPatch::Selected(*v)),
                        false,
                    )
                });
            },
        );
        cx.on(node, move |ev| {
            if let Event::SelectionChanged(i) = ev
                && *i >= 0
            {
                selected.set_rw(*i as usize);
            }
        });
        node
    }
}

// ---------------------------------------------------------------------------
// Text area (kinds::TEXT_AREA, docs/textarea.md) — built-in since 2026-07.
// ---------------------------------------------------------------------------

/// A native multi-line text editor bound two-way to `text`. Configure a prompt with
/// `.placeholder(_)`, the auto-growing height band with `.min_lines(_)` / `.max_lines(_)`, and the
/// native editor attributes with `.editable(_)` / `.selectable(_)` / `.spellcheck(_)` (each accepts
/// a constant or a reactive `bool`, and updates live). A backend that can't honor an attribute
/// answers the matching `Cap::Text{Editable,Selectable,SpellCheck}` with `Support::Unsupported`.
pub struct TextArea {
    text: Signal<String>,
    placeholder: Option<TextSource>,
    min_lines: u32,
    max_lines: u32,
    editable: Reactive<bool>,
    selectable: Reactive<bool>,
    spellcheck: Reactive<bool>,
    on_submit: Option<Rc<dyn Fn()>>,
}

/// `text_area(text)` — a native multi-line editor whose contents mirror `text` in both directions.
pub fn text_area(text: Signal<String>) -> TextArea {
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

impl TextArea {
    /// The empty-state prompt shown when the editor is empty (a constant, `Signal<String>`, or
    /// closure — evaluated once for the initial value; not reactive after build).
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
    /// default) means unbounded — the editor keeps growing and never scrolls.
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

    /// Submit on Enter: a plain Enter runs `f` instead of inserting a newline — the chat-composer
    /// contract. Shift+Enter still inserts a line break on the desktop toolkits; Android's soft
    /// keyboard shows a Send action; iOS's return key submits. Backends without the intercept
    /// (web-dom today) keep inserting newlines, so pair this with a visible send button. The
    /// bound `text` signal is already up to date when `f` runs.
    pub fn on_submit(mut self, f: impl Fn() + 'static) -> Self {
        self.on_submit = Some(Rc::new(f));
        self
    }
}

impl Piece for TextArea {
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
        let initial = text.get_untracked();
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
            // measure grows it between min/max lines), so it is NOT a height-growing leaf.
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
        // that arrived FROM the native widget so bind_seeded does not patch it straight back.
        let guard: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
        let g = guard.clone();
        bind_seeded(
            initial,
            move || text.get(),
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
                *guard.borrow_mut() = Some(t.clone());
                text.set(t.clone());
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
