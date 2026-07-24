//! Native input pieces: `picker` (a bound one-of-N selector — menu, segmented, or inline) and
//! `text_area` (a multi-line, auto-growing editor bound two-way to a `Signal<String>`).

use std::cell::RefCell;
use std::rc::Rc;

use day_core::*;
use day_reactive::{Signal, bind_seeded};
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
/// `.placeholder(_)` and the auto-growing height band with `.min_lines(_)` / `.max_lines(_)`.
pub struct TextArea {
    text: Signal<String>,
    placeholder: Option<TextSource>,
    min_lines: u32,
    max_lines: u32,
}

/// `text_area(text)` — a native multi-line editor whose contents mirror `text` in both directions.
pub fn text_area(text: Signal<String>) -> TextArea {
    TextArea {
        text,
        placeholder: None,
        min_lines: 1,
        max_lines: 0,
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
}

impl Piece for TextArea {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let TextArea {
            text,
            placeholder,
            min_lines,
            max_lines,
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
            },
            // A composer fills the available width; height is content-driven (the backend's
            // measure grows it between min/max lines), so it is NOT a height-growing leaf.
            Flex {
                grow_w: true,
                ..Default::default()
            },
        );
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
        cx.on(node, move |ev| {
            if let Event::TextChanged(t) = ev {
                *guard.borrow_mut() = Some(t.clone());
                text.set(t.clone());
            }
        });
        node
    }
}
