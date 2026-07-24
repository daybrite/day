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
mod menus;
mod nav;
mod shapes;
mod sources;
mod structure;

pub use canvas::*;
pub use containers::*;
pub use decorators::*;
pub use dialogs::*;
pub use forms::*;
pub use image::*;
pub use inputs::*;
pub use leaves::*;
pub use menus::*;
pub use nav::*;
pub use shapes::*;
pub use sources::*;
pub use structure::*;

pub mod prelude {
    pub use crate::TextStyle;
    pub use crate::routes;
    pub use crate::{
        A11yBuilder, Alert, ButtonStyle, Confirm, Corner, Cover, Decorate, Drag, Draw, FileUrl,
        FilledButtonStyle, FormSection, Grid, GridRow, HAlign, IntoFocusBinding, IntoFraction,
        IntoReactive, IntoText, ItemSlot, Link, List, MenuEntry, Modifier, NativeRef, OpenFile,
        Prompt, Reactive, Route, RoutePath, SaveFile, Selector, SelectorStyle, ShapeKind,
        ShapePiece, SignalRw, Stack, VAlign, ZStack, alert, app_menu, arc, button, canvas, capsule,
        circle, column, confirm, cover, current_route, divider, each, ellipse, environment, form,
        frame_clock, grid, grid_row, image, label, labeled, line, link, list, menu_item, menu_role,
        menu_separator, nav_back, nav_link, nav_link_to, navigate, navigate_to, open_file, picker,
        polygon, progress, prompt, rectangle, rounded_rectangle, route, route_param, route_params,
        row, save_file, scroll, section, selector, shape, shape_group, shape_group_fn, slider,
        spacer, spinner, stack, sub_menu, text_area, text_field, toggle, when, with_environment,
        zstack,
    };
    pub use crate::{Picker, TextArea};
    pub use day_core::{
        Alignment, AnyPiece, BuildCx, Piece, PieceSeq, PieceVec, RNode, ScrollTarget,
        invalidate_size, open_url, piece_fn, with_animation,
    };
    pub use day_geometry::{Affine, Animatable, Color, Insets, Point, Rect, Size, Transform};
    pub use day_reactive::{
        Effect, Memo, Scope, Setter, Signal, Trigger, batch, bind, untrack, watch,
    };
    pub use day_spec::props::PickerStyle;
    pub use day_spec::props::RowHeight;
    pub use day_spec::{AnimSpec, AnimSpec as Animation, Curve};
    pub use day_spec::{AssetName, FontFamily, ImageName};
    pub use day_spec::{DragPhase, Edges, GestureKind};
    pub use day_spec::{
        DrawOp, LinearGradient, Paint, RadialGradient, Shape, TextAnchor, UnitPoint,
    };
    pub use day_spec::{Font, FontSpec, FontWeight, Role};
    pub use day_spec::{MenuItem, MenuRole, Shortcut};
    pub use std::time::Duration;
}
