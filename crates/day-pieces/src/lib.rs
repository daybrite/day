// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! day-pieces: the built-in piece library (DESIGN.md §5.3).
//!
//! Every constructor is a plain function returning a piece value; builder methods configure;
//! `build` runs once. Dynamic attributes become seeded bindings writing sparse typed patches
//! through the thread-local tree.
//!
//! The vocabulary is split across sibling modules (one logical group each) and re-exported here,
//! so the public API stays flat (`day_pieces::button`, `day_pieces::stack`, …) regardless of
//! which module a piece is defined in.

// External-piece registration surface (§8.2): the `renderer!` macro + `fill_measure`, plus the
// re-exports the macro expands to (so a piece needs only a `day-pieces` dependency, not linkme).
pub mod async_image;
pub mod colorpicker;
pub mod combobox;
pub mod datetime;
pub mod pager;
pub mod pullrefresh;
pub mod searchfield;
pub mod stepper;
pub use async_image::{AsyncImage, AsyncImageBuilder, async_image};
pub mod render;

// The dynamic piece registry (docs/lite.md §4 in https://github.com/daybrite/day-lite): drive
// pieces by name with loosely-typed values, the surface interpreted languages (day-lite) build
// UIs through.
#[cfg(feature = "dyn-registry")]
pub mod dynreg;
/// `#[day_pieces::builder(FooBuilder)]`: a piece's builder trait, generated from its inherent
/// impl (docs/api-style.md "Typed builders and erasure").
pub use day_core::{ElementId, IntoElementKey, id};
pub use day_macros::builder;
pub use day_spec::Renderer;
pub use linkme;
pub use render::fill_measure;

// The piece vocabulary: one logical group per module, re-exported flat (see each module's docs).
mod canvas;
mod commands;
pub use commands::Command;
mod containers;
mod decorators;
mod dialogs;
mod file_access;
pub use file_access::FileAccess;
#[cfg(feature = "conformance")]
pub mod conformance;
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
mod split;
mod status;
mod structure;
mod tabs;
mod tokens;
mod toolbar;
pub use tabs::*;
pub use tokens::*;

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
pub use split::*;
pub use status::{StatusItem, StatusItemHandle, status_item};
pub use structure::*;
// The ambient environment and the `Ambient` state trait (docs/state.md) live in day-core, so an
// app's `*-core` crate (which depends on day-core but not on day-pieces) can `impl Ambient`
// for its view-model. Re-exported here so `day_pieces::environment` and the prelude are unchanged.
pub use day_core::{Ambient, app_environment, environment, focused_environment, with_environment};
// Canvas export (docs/canvas.md "Exporting"): what this toolkit can render a `Drawing` to.
pub use day_core::{CanvasExports, RenderBitmap, RenderPdf, canvas_exports};
pub use toolbar::*;

pub use colorpicker::{ColorPicker, ColorPickerBuilder, PickerIdiom, color_picker};
pub use combobox::{ComboBox, ComboBoxBuilder, combo_box};
pub use datetime::{
    DatePicker, DatePickerBuilder, DayDate, DayTime, TimePicker, TimePickerBuilder, date_picker,
    time_picker,
};
pub use pager::{Pager, PagerState, pager};
pub use pullrefresh::{PullRefresh, PullRefreshBuilder, pull_to_refresh};
pub use searchfield::{SearchField, SearchFieldBuilder, search_field};
pub use stepper::{Stepper, StepperIdiom, stepper};

pub mod prelude {
    pub use crate::colorpicker::{ColorPicker, ColorPickerBuilder, PickerIdiom, color_picker};
    pub use crate::combobox::{ComboBox, ComboBoxBuilder, combo_box};
    pub use crate::datetime::{
        DatePicker, DatePickerBuilder, DayDate, DayTime, TimePicker, TimePickerBuilder,
        date_picker, time_picker,
    };
    pub use crate::pager::{Pager, PagerState, pager};
    pub use crate::pullrefresh::{PullRefresh, PullRefreshBuilder, pull_to_refresh};
    pub use crate::searchfield::{SearchField, SearchFieldBuilder, search_field};
    pub use crate::stepper::{Stepper, StepperIdiom, stepper};
    pub use crate::{AsyncImage, AsyncImageBuilder, async_image};

    pub use crate::{DocumentTabs, TabActions, TabSet, document_tabs};
    pub use day_core::{ElementId, IntoElementKey, id};
    // Model-driven rows (docs/model.md): the store-as-RowSource surface.
    pub use crate::TextStyle;
    pub use crate::ToolbarEntry;
    pub use crate::routes;
    pub use crate::{
        A11yBuilder, Alert, Ambient, BackRequest, BackResponse, Binding, ButtonBuilder,
        ColumnBuilder, Command, Confirm, ContentList, Corner, Cover, Decorate, Decorated, Delete,
        Drag, Draw, FileAccess, FileUrl, Form, FormSection, Grid, GridRow, HAlign, Inspector,
        IntoFocusBinding, IntoFraction, IntoImageSource, IntoReactive, IntoText, ItemSlot,
        LabelBuilder, Labeled, Link, List, ListItem, ListItemBuilder, Loaded, MenuContent,
        MenuEntry, Modifier, Move, NativeRef, Nav, NavContent, NavItem, NavStack, NavStyle,
        OpenFile, Pan, PathBuilder, Pinch, ProgressBuilder, Prompt, Reactive, Reorder,
        ReorderVerdict, Route, RoutePath, RowBuilder, RowFit, SaveFile, Search, SearchReveal,
        Sections, ShapeKind, ShapePiece, StatusItem, SwipeAction, TextBuilder, VAlign,
        VectorWeight, When, ZStack, adaptive, alert, app_environment, app_menu, arc, button,
        canvas, capsule, circle, column, confirm, cover, current_route, divider, dock_menu, each,
        ellipse, environment, focused_environment, form, frame_clock, grid, grid_row, image,
        inspector, item, items, keyed, label, labeled, line, link, list, list_item, load,
        menu_item, menu_role, menu_separator, nav, nav_back, nav_items, nav_link, nav_link_to,
        nav_stack, navigate, navigate_to, open_file, open_link, picker, picker_optional, polygon,
        progress, prompt, rectangle, rounded_rectangle, route, route_param, route_params, row,
        save_file, scroll, section, secure_field, segment, shape, shape_group, shape_group_fn,
        slider, spacer, spinner, split, status_item, sub_menu, swipe_action, switch, text_area,
        text_field, toggle, toolbar_button, toolbar_label, toolbar_menu, toolbar_segmented,
        toolbar_separator, toolbar_toggle, vector, when, with_environment, zstack,
    };
    pub use crate::{Space, TextPreset};
    // Canvas export (docs/canvas.md "Exporting"): record a drawing once, render it to a bitmap
    // or a PDF through the toolkit's own painter, and ask what this target can do first.
    pub use crate::{CanvasExports, Drawing, PdfDocument, canvas_exports};
    pub use day_spec::ExportError;
    // The hierarchical tree (docs/tree.md): the piece, its sources, and its verdict enum.
    pub use crate::{
        Branches, MoveVerdict, NodeSource, TreeBuilder, TreeConn, TreePiece, branches, tree,
    };
    // Typed builder traits (docs/api-style.md "Typed builders and erasure"): each piece's own
    // builders, forwarded through `Decorated` so they still chain after a generic modifier. A
    // piece implements exactly one, so the names they share (`title`, `style`, `align`, …) never
    // become ambiguous at a call site. (`LabelBuilder`, `ButtonBuilder`, `ColumnBuilder` and
    // `RowBuilder` sit in the list above, in alphabetical company.)
    pub use crate::{
        AdaptiveBuilder, CoverBuilder, DividerBuilder, FormSectionBuilder, GridBuilder,
        GridRowBuilder, ImageBuilder, InspectorBuilder, LinkBuilder, ListBuilder, NavBuilder,
        NavStackBuilder, PickerBuilder, ScrollBuilder, ShapePieceBuilder, SliderBuilder,
        SplitBuilder, TextAreaBuilder, TextFieldBuilder, ToggleBuilder, VectorBuilder, WhenBuilder,
        ZStackBuilder,
    };
    #[cfg(feature = "model")]
    pub use crate::{ModelSlot, Rows, StoreRows, StoreTree, StoreTrees};
    pub use crate::{Picker, TextArea};
    pub use day_core::{
        Alignment, AnyPiece, BuildCx, Either, Piece, PieceFn, PieceSeq, RNode, ScrollTarget,
        announce, announce_urgent, invalidate_size, open_url, piece_fn, with_animation,
    };
    pub use day_geometry::{Affine, Animatable, Color, Insets, Point, Rect, Size, Transform};
    pub use day_reactive::{
        AnyBinding, Effect, Memo, Scope, Setter, Signal, Trigger, batch, bind, untrack, watch,
    };
    // `Nav::presentation` takes one (docs/size-classes.md); apps that leave the
    // presentation automatic never name it.
    pub use day_spec::props::NavPresentation;
    pub use day_spec::props::PickerStyle;
    pub use day_spec::props::RowHeight;
    pub use day_spec::props::TextAlign;
    pub use day_spec::props::{PaneEdge, SplitAxis};
    pub use day_spec::{AnimSpec, AnimSpec as Animation, Curve};
    pub use day_spec::{AssetName, FontFamily, ImageName};
    pub use day_spec::{DragPhase, Edges, GestureKind};
    // Text entry (docs/textfield.md): what a field collects and what its action key says.
    pub use day_spec::{
        DrawOp, LinearGradient, Paint, RadialGradient, Shape, TextAnchor, TextVAlign, UnitPoint,
    };
    pub use day_spec::{InputPurpose, SubmitLabel};
    // Canvas fonts and the platform font list (docs/fonts.md).
    pub use day_spec::{CanvasFont, FontFace, FontFamilyInfo, TextMetrics};
    pub use day_spec::{Font, FontSpec, FontWeight, Role};
    // The styled-text document (docs/texteditor.md): what `.markdown()` parses into, what a
    // label's runs are, and what `day-piece-texteditor` edits, plus the Markdown / HTML / RTF
    // codecs over it.
    pub use day_spec::{
        LabelStyle, Symbol, ToolbarItem, ToolbarItemKind, ToolbarPlacement, ToolbarValue,
    };
    pub use day_spec::{
        ListStyle, ParagraphAlign, ParagraphRun, ParagraphStyle, RunStyle, StyledText, TextRun,
        Underline,
    };
    pub use day_spec::{MenuBarRole, MenuItem, MenuRole, Shortcut};
    pub use std::time::Duration;
}
