// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! day-pieces — the built-in piece library (DESIGN.md §5.3).
//!
//! Every constructor is a plain function returning a piece value; builder methods configure;
//! `build` runs once. Dynamic attributes become seeded bindings writing sparse typed patches
//! through the thread-local tree.
//!
//! The vocabulary is split across sibling modules (one logical group each) and re-exported here,
//! so the public API stays flat — `day_pieces::button`, `day_pieces::stack`, … — regardless of
//! which module a piece is defined in.

// External-piece registration surface (§8.2): the `renderer!` macro + `fill_measure`, plus the
// re-exports the macro expands to (so a piece needs only a `day-pieces` dependency, not linkme).
pub mod render;

// The dynamic piece registry (docs/lite.md §4): drive pieces by name with loosely-typed
// values — the surface interpreted languages (day-lite) build real UIs through.
#[cfg(feature = "dyn-registry")]
pub mod dynreg;
pub use day_spec::Renderer;
pub use linkme;
pub use render::fill_measure;

// The piece vocabulary — one logical group per module, re-exported flat (see each module's docs).
mod canvas;
mod containers;
mod decorators;
mod dialogs;
mod forms;
mod image;
mod inputs;
mod leaves;
/// Inline markdown → styled runs (docs/markdown.md). The parser moved to day-spec, beside
/// the `TextRun`s it produces and the other format codecs; this keeps the old path working.
pub use day_spec::markdown;
mod inspector;
mod menus;
mod nav;
mod shapes;
mod sources;
mod structure;
mod toolbar;

pub use canvas::*;
pub use containers::*;
pub use decorators::*;
pub use dialogs::*;
pub use forms::*;
pub use image::*;
pub use inputs::*;
pub use inspector::*;
pub use leaves::*;
pub use menus::*;
pub use nav::*;
pub use shapes::*;
pub use sources::*;
pub use structure::*;
pub use toolbar::*;

pub mod prelude {
    // Model-driven rows (docs/model.md): the store-as-RowSource surface.
    pub use crate::TextStyle;
    pub use crate::ToolbarEntry;
    pub use crate::routes;
    pub use crate::{
        A11yBuilder, Alert, BackRequest, BackResponse, Binding, ButtonBuilder, ColumnBuilder,
        Confirm, Corner, Cover, Decorate, Decorated, Drag, Draw, FileUrl, FormSection, Grid,
        GridRow, HAlign, Inspector, IntoFocusBinding, IntoFraction, IntoReactive, IntoText,
        ItemSlot, LabelBuilder, Link, List, MenuEntry, Modifier, NativeRef, NavItem, OpenFile, Pan,
        PathBuilder, Pinch, Prompt, Reactive, Reorder, Route, RoutePath, RowBuilder, RowFit,
        SaveFile, Selector, SelectorStyle, ShapeKind, ShapePiece, Stack, TextBuilder, VAlign,
        VectorWeight, When, ZStack, alert, app_menu, app_menu_reactive, arc, button, canvas,
        capsule, circle, column, confirm, cover, current_route, divider, each, ellipse,
        environment, form, frame_clock, grid, grid_row, image, inspector, item, items, label,
        labeled, line, link, list, menu_item, menu_role, menu_separator, nav_back, nav_link,
        nav_link_to, navigate, navigate_to, open_file, picker, polygon, progress, prompt,
        rectangle, rounded_rectangle, route, route_param, route_params, row, save_file, scroll,
        section, segment, selector, shape, shape_group, shape_group_fn, slider, spacer, spinner,
        stack, sub_menu, text_area, text_field, toggle, toolbar, toolbar_button,
        toolbar_flexible_space, toolbar_label, toolbar_menu, toolbar_reactive, toolbar_segmented,
        toolbar_separator, toolbar_sidebar_toggle, toolbar_space, toolbar_toggle, vector, when,
        with_environment, zstack,
    };
    // Typed builder traits (docs/api-style.md "Typed builders and erasure"): each piece's own
    // builders, forwarded through `Decorated` so they still chain after a generic modifier. A
    // piece implements exactly one, so the names they share (`title`, `style`, `align`, …) never
    // become ambiguous at a call site. (`LabelBuilder`, `ButtonBuilder`, `ColumnBuilder` and
    // `RowBuilder` sit in the list above, in alphabetical company.)
    pub use crate::{
        CoverBuilder, FormSectionBuilder, GridBuilder, GridRowBuilder, ImageBuilder,
        InspectorBuilder, LinkBuilder, ListBuilder, PickerBuilder, ScrollBuilder, SelectorBuilder,
        ShapePieceBuilder, SliderBuilder, StackBuilder, TextAreaBuilder, TextFieldBuilder,
        ToggleBuilder, VectorBuilder, WhenBuilder, ZStackBuilder,
    };
    #[cfg(feature = "model")]
    pub use crate::{ModelSlot, Rows, StoreRows};
    pub use crate::{Picker, TextArea};
    pub use day_core::{
        Alignment, AnyPiece, BuildCx, Either, Piece, PieceSeq, PieceVec, RNode, ScrollTarget,
        invalidate_size, open_url, piece_fn, with_animation,
    };
    pub use day_geometry::{Affine, Animatable, Color, Insets, Point, Rect, Size, Transform};
    pub use day_reactive::{
        Effect, Memo, Scope, Setter, Signal, Trigger, batch, bind, untrack, watch,
    };
    // `Selector::presentation` takes one (docs/size-classes.md); apps that leave the
    // presentation automatic never name it.
    pub use day_spec::props::NavPresentation;
    pub use day_spec::props::PickerStyle;
    pub use day_spec::props::RowHeight;
    pub use day_spec::props::TextAlign;
    pub use day_spec::{AnimSpec, AnimSpec as Animation, Curve};
    pub use day_spec::{AssetName, FontFamily, ImageName};
    pub use day_spec::{DragPhase, Edges, GestureKind};
    pub use day_spec::{
        DrawOp, LinearGradient, Paint, RadialGradient, Shape, TextAnchor, UnitPoint,
    };
    pub use day_spec::{Font, FontSpec, FontWeight, Role};
    // The styled-text document (docs/texteditor.md): what `.markdown()` parses into, what a
    // label's runs are, and what `day-piece-texteditor` edits — plus the Markdown / HTML / RTF
    // codecs over it.
    pub use day_spec::{
        ListStyle, ParagraphAlign, ParagraphRun, ParagraphStyle, RunStyle, StyledText, TextRun,
        Underline,
    };
    pub use day_spec::{MenuBarRole, MenuItem, MenuRole, Shortcut};
    pub use day_spec::{Symbol, ToolbarItem, ToolbarItemKind, ToolbarValue};
    pub use std::time::Duration;
}
