//! Dynamic piece registry (feature `dyn-registry`; docs/lite.md §4).
//!
//! The machine-readable surface of the piece layer: every registered constructor and
//! modifier is invokable by NAME with loosely-typed [`DynValue`] arguments, which is what
//! lets an interpreted language (day-lite's JS/TS, or any other embedder) drive real pieces
//! without compiling against the builder types. The registry ships the built-in vocabulary;
//! extension crates join it at startup through [`register_piece`] / [`register_modifier`],
//! so a superapp's compiled-in pieces become scriptable with no day-lite changes.
//!
//! Shape rules:
//! - A constructor produces a [`DynPiece`] that stays CONCRETE (its builder type) until a
//!   generic `Decorate` modifier erases it. Type-specific modifiers (`spacing`, `font`,
//!   `action`, …) therefore must precede generic ones (`padding`, `frame`, `id`, …) in a
//!   chain — violating that is a [`DynError::LateTyped`], reported with both names so the
//!   script-side error is actionable.
//! - Reactive arguments are [`DynValue::Fn`] callbacks (re-run under day-reactive tracking
//!   — a `get()` on a bridged signal inside one registers a real dependency) or
//!   [`DynValue::Signal`] handles (typed at creation; see [`DynSignal`]).
//! - Naming is **snake_case throughout** — constructors (`text_field`), modifiers
//!   (`corner_radius`), and string enum values alike (`large_title`, `top_leading`); the
//!   dyn surface mirrors day's Rust API one-to-one rather than re-casing per language.
//! - Everything is main-thread only, like the rest of the piece layer. The registry maps
//!   hold plain `fn` pointers so they can sit in globals; the values they build are not
//!   `Send` and never leave the UI thread.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Mutex;

use day_core::{Alignment, AnyPiece, PieceVec};
use day_reactive::Signal;
use day_spec::{Color, Edges, Font};

use crate::prelude::Decorate;
use crate::{
    Button, Column, Grid, GridRow, HAlign, Image, Label, Progress, Row, Slider, TextField, Toggle,
    VAlign, button, column, divider, each, grid, grid_row, image, label, progress, row, scroll,
    slider, spacer, text_field, toggle, when,
};

/// `Color` from 8-bit channels (the geometry type is `f64`-channel).
fn rgb8(r: u32, g: u32, b: u32, a: u32) -> Color {
    Color::rgba(
        r as f64 / 255.0,
        g as f64 / 255.0,
        b as f64 / 255.0,
        a as f64 / 255.0,
    )
}

/// A host callback (day-lite: a JS function). Invoked on the main thread; may be re-invoked
/// under reactive tracking, in which case signal reads inside it register dependencies.
pub type DynCallback = Rc<dyn Fn(&[DynValue]) -> DynValue>;

/// A signal bridged across the language boundary. The payload type is fixed at creation
/// from the initial value (day-lite maps JS bool/number/string onto the typed variants) so
/// control pieces get the `SignalRw` type they require without adapters.
#[derive(Clone)]
pub enum DynSignal {
    Bool(Signal<bool>),
    Num(Signal<f64>),
    Str(Signal<String>),
}

impl DynSignal {
    /// Tracked read, as a value.
    pub fn get(&self) -> DynValue {
        match self {
            DynSignal::Bool(s) => DynValue::Bool(s.get()),
            DynSignal::Num(s) => DynValue::Num(s.get()),
            DynSignal::Str(s) => DynValue::Str(s.get()),
        }
    }

    /// Write; a mismatched payload type is a no-op error for the caller to surface.
    pub fn set(&self, v: &DynValue) -> Result<(), DynError> {
        match (self, v) {
            (DynSignal::Bool(s), DynValue::Bool(b)) => {
                s.set(*b);
                Ok(())
            }
            (DynSignal::Num(s), DynValue::Num(n)) => {
                s.set(*n);
                Ok(())
            }
            (DynSignal::Str(s), DynValue::Str(t)) => {
                s.set(t.clone());
                Ok(())
            }
            _ => Err(DynError::Type {
                what: "signal.set",
                want: "the signal's payload type",
            }),
        }
    }
}

/// The loosely-typed argument/return value of the dynamic surface.
#[derive(Clone)]
pub enum DynValue {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    List(Vec<DynValue>),
    Map(Vec<(String, DynValue)>),
    Fn(DynCallback),
    Signal(DynSignal),
    Piece(DynPiece),
}

impl DynValue {
    fn kind(&self) -> &'static str {
        match self {
            DynValue::Null => "null",
            DynValue::Bool(_) => "bool",
            DynValue::Num(_) => "number",
            DynValue::Str(_) => "string",
            DynValue::List(_) => "list",
            DynValue::Map(_) => "map",
            DynValue::Fn(_) => "function",
            DynValue::Signal(_) => "signal",
            DynValue::Piece(_) => "piece",
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            DynValue::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            DynValue::Num(n) => Some(*n),
            _ => None,
        }
    }

    /// A stable structural serialization, used as the row key by `each` (a changed item
    /// gets a new key and thus a rebuilt row).
    pub fn key_string(&self) -> String {
        match self {
            DynValue::List(items) => {
                let inner: Vec<String> = items.iter().map(|v| v.key_string()).collect();
                format!("[{}]", inner.join(","))
            }
            DynValue::Map(entries) => {
                let inner: Vec<String> = entries
                    .iter()
                    .map(|(k, v)| format!("{k}:{}", v.key_string()))
                    .collect();
                format!("{{{}}}", inner.join(","))
            }
            other => other.display(),
        }
    }

    /// Render any value as display text (what `label`'s reactive closure produces).
    pub fn display(&self) -> String {
        match self {
            DynValue::Null => String::new(),
            DynValue::Bool(b) => b.to_string(),
            DynValue::Num(n) => {
                if n.fract() == 0.0 && n.abs() < 1e15 {
                    format!("{}", *n as i64)
                } else {
                    n.to_string()
                }
            }
            DynValue::Str(s) => s.clone(),
            other => format!("[{}]", other.kind()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DynError {
    /// No constructor/modifier registered under this name.
    Unknown { what: &'static str, name: String },
    /// An argument had the wrong shape.
    Type {
        what: &'static str,
        want: &'static str,
    },
    /// A type-specific modifier arrived after a generic one erased the builder.
    LateTyped {
        modifier: String,
        piece: &'static str,
    },
}

impl std::fmt::Display for DynError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DynError::Unknown { what, name } => write!(f, "unknown {what} `{name}`"),
            DynError::Type { what, want } => write!(f, "{what}: expected {want}"),
            DynError::LateTyped { modifier, piece } => write!(
                f,
                "`.{modifier}()` must come before generic modifiers — the {piece} builder \
                 was already erased (reorder the chain so type-specific calls are first)"
            ),
        }
    }
}

impl std::error::Error for DynError {}

/// A piece under dynamic construction: concrete until a generic modifier erases it.
#[derive(Clone)]
pub struct DynPiece(Rc<std::cell::RefCell<Option<Inner>>>);

enum Inner {
    Label(Label),
    Button(Button),
    Column(Column<PieceVec>),
    Row(Row<PieceVec>),
    Grid(Grid<PieceVec>),
    GridRow(GridRow<PieceVec>),
    TextField(TextField<Signal<String>>),
    Toggle(Toggle<Signal<bool>>),
    Slider(Slider<Signal<f64>>),
    Progress(Progress),
    Image(Image),
    Any(AnyPiece),
}

impl Inner {
    fn into_any(self) -> AnyPiece {
        match self {
            Inner::Label(p) => p.any(),
            Inner::Button(p) => p.any(),
            Inner::Column(p) => p.any(),
            Inner::Row(p) => p.any(),
            Inner::Grid(p) => p.any(),
            Inner::GridRow(p) => p.any(),
            Inner::TextField(p) => p.any(),
            Inner::Toggle(p) => p.any(),
            Inner::Slider(p) => p.any(),
            Inner::Progress(p) => p.any(),
            Inner::Image(p) => p.any(),
            Inner::Any(p) => p,
        }
    }
}

type CtorFn = fn(&[DynValue]) -> Result<DynPiece, DynError>;
type ModifierFn = fn(AnyPiece, &[DynValue]) -> Result<AnyPiece, DynError>;

/// What a registry entry is, for introspection (day-lite generates the JS API from this).
#[derive(Clone, Debug)]
pub struct SpecEntry {
    pub name: &'static str,
    pub kind: SpecKind,
    /// Human-oriented signature, e.g. `label(text | fn | signal)`.
    pub sig: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpecKind {
    Constructor,
    /// Generic `Decorate` modifier: applies to any piece, erases the builder.
    Modifier,
    /// Type-specific builder method; `sig` names the receiving piece.
    TypedModifier,
}

fn ctors() -> &'static Mutex<HashMap<&'static str, CtorFn>> {
    static MAP: std::sync::OnceLock<Mutex<HashMap<&'static str, CtorFn>>> =
        std::sync::OnceLock::new();
    MAP.get_or_init(|| Mutex::new(builtin_ctors()))
}

fn ext_modifiers() -> &'static Mutex<HashMap<&'static str, ModifierFn>> {
    static MAP: std::sync::OnceLock<Mutex<HashMap<&'static str, ModifierFn>>> =
        std::sync::OnceLock::new();
    MAP.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Register (or override) a constructor. Extension crates call this before launch.
pub fn register_piece(name: &'static str, ctor: CtorFn) {
    if let Ok(mut m) = ctors().lock() {
        m.insert(name, ctor);
    }
}

/// Register a generic modifier beyond the built-in `Decorate` set.
pub fn register_modifier(name: &'static str, f: ModifierFn) {
    if let Ok(mut m) = ext_modifiers().lock() {
        m.insert(name, f);
    }
}

/// Construct a piece by name.
pub fn construct(name: &str, args: &[DynValue]) -> Result<DynPiece, DynError> {
    let ctor = ctors()
        .lock()
        .ok()
        .and_then(|m| m.get(name).copied())
        .ok_or_else(|| DynError::Unknown {
            what: "piece",
            name: name.into(),
        })?;
    ctor(args)
}

impl DynPiece {
    fn new(inner: Inner) -> Self {
        DynPiece(Rc::new(std::cell::RefCell::new(Some(inner))))
    }

    /// Finish the chain: the erased piece, ready to hand to a parent or the mounter.
    /// A `DynPiece` is one-shot (pieces are consumed by building); a second take yields
    /// an empty spacer so a script bug degrades visibly rather than panicking.
    pub fn into_any(&self) -> AnyPiece {
        match self.0.borrow_mut().take() {
            Some(inner) => inner.into_any(),
            None => spacer().any(),
        }
    }

    /// Apply a modifier by name (type-specific first, then the generic `Decorate` set,
    /// then extension-registered generics).
    pub fn modify(&self, name: &str, args: &[DynValue]) -> Result<(), DynError> {
        let mut slot = self.0.borrow_mut();
        let inner = slot.take().unwrap_or_else(|| Inner::Any(spacer().any()));
        let next = apply_modifier(inner, name, args)?;
        *slot = Some(next);
        Ok(())
    }
}

// ---- argument helpers -------------------------------------------------------------------

fn want(args: &[DynValue], i: usize, what: &'static str, want_: &'static str) -> DynError {
    let _ = (args, i);
    DynError::Type { what, want: want_ }
}

fn num(args: &[DynValue], i: usize, what: &'static str) -> Result<f64, DynError> {
    args.get(i)
        .and_then(|v| v.as_f64())
        .ok_or(want(args, i, what, "a number"))
}

fn str_arg(args: &[DynValue], i: usize, what: &'static str) -> Result<String, DynError> {
    args.get(i)
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .ok_or(want(args, i, what, "a string"))
}

fn children(args: &[DynValue]) -> Result<PieceVec, DynError> {
    // Accept both `column(a, b, c)` and `column([a, b, c])`.
    let mut out = Vec::new();
    let mut push = |v: &DynValue| -> Result<(), DynError> {
        match v {
            DynValue::Piece(p) => {
                out.push(p.into_any());
                Ok(())
            }
            DynValue::Null => Ok(()), // holes from conditional JS spreads are fine
            _ => Err(DynError::Type {
                what: "children",
                want: "pieces",
            }),
        }
    };
    for a in args {
        match a {
            DynValue::List(items) => {
                for v in items {
                    push(v)?;
                }
            }
            other => push(other)?,
        }
    }
    Ok(PieceVec(out))
}

/// Text-ish argument: literal, reactive callback, or string signal.
enum TextArg {
    Lit(String),
    Cb(DynCallback),
    Sig(Signal<String>),
}

fn text_arg(args: &[DynValue], i: usize, what: &'static str) -> Result<TextArg, DynError> {
    match args.get(i) {
        Some(DynValue::Str(s)) => Ok(TextArg::Lit(s.clone())),
        Some(DynValue::Num(n)) => Ok(TextArg::Lit(DynValue::Num(*n).display())),
        Some(DynValue::Fn(f)) => Ok(TextArg::Cb(f.clone())),
        Some(DynValue::Signal(DynSignal::Str(s))) => Ok(TextArg::Sig(*s)),
        _ => Err(want(args, i, what, "a string, function, or string signal")),
    }
}

/// Reactive color argument for `background`/`foreground`: hex literal or callback.
fn color_of(v: &DynValue) -> Result<Color, DynError> {
    let s = v.as_str().ok_or(DynError::Type {
        what: "color",
        want: "a #rgb/#rrggbb/#aarrggbb hex string",
    })?;
    parse_color(s).ok_or(DynError::Type {
        what: "color",
        want: "a #rgb/#rrggbb/#aarrggbb hex string",
    })
}

fn parse_color(s: &str) -> Option<Color> {
    let h = s.strip_prefix('#')?;
    let v = u32::from_str_radix(h, 16).ok()?;
    Some(match h.len() {
        3 => rgb8(
            ((v >> 8) & 0xF) * 17,
            ((v >> 4) & 0xF) * 17,
            (v & 0xF) * 17,
            255,
        ),
        6 => rgb8((v >> 16) & 0xFF, (v >> 8) & 0xFF, v & 0xFF, 255),
        8 => rgb8(
            (v >> 16) & 0xFF,
            (v >> 8) & 0xFF,
            v & 0xFF,
            (v >> 24) & 0xFF,
        ),
        _ => return None,
    })
}

fn font_of(name: &str) -> Option<Font> {
    Some(match name {
        "large_title" => Font::LargeTitle,
        "title" => Font::Title,
        "title2" => Font::Title2,
        "title3" => Font::Title3,
        "headline" => Font::Headline,
        "body" => Font::Body,
        "callout" => Font::Callout,
        "subheadline" => Font::Subheadline,
        "footnote" => Font::Footnote,
        "caption" => Font::Caption,
        "caption2" => Font::Caption2,
        _ => return None,
    })
}

fn halign_of(name: &str) -> Option<HAlign> {
    Some(match name {
        "leading" => HAlign::Leading,
        "center" => HAlign::Center,
        "trailing" => HAlign::Trailing,
        _ => return None,
    })
}

fn valign_of(name: &str) -> Option<VAlign> {
    Some(match name {
        "top" => VAlign::Top,
        "center" => VAlign::Center,
        "bottom" => VAlign::Bottom,
        _ => return None,
    })
}

fn alignment_of(name: &str) -> Option<Alignment> {
    Some(match name {
        "top_leading" => Alignment::TopLeading,
        "top" => Alignment::Top,
        "top_trailing" => Alignment::TopTrailing,
        "leading" => Alignment::Leading,
        "center" => Alignment::Center,
        "trailing" => Alignment::Trailing,
        "bottom_leading" => Alignment::BottomLeading,
        "bottom" => Alignment::Bottom,
        "bottom_trailing" => Alignment::BottomTrailing,
        _ => return None,
    })
}

fn callback0(f: &DynCallback) -> impl Fn() + 'static {
    let f = f.clone();
    move || {
        let _ = f(&[]);
    }
}

// ---- constructors -----------------------------------------------------------------------

fn label_like(args: &[DynValue], what: &'static str) -> Result<Label, DynError> {
    Ok(match text_arg(args, 0, what)? {
        TextArg::Lit(s) => label(s),
        TextArg::Cb(f) => label(move || f(&[]).display()),
        TextArg::Sig(s) => label(s),
    })
}

fn builtin_ctors() -> HashMap<&'static str, CtorFn> {
    let mut m: HashMap<&'static str, CtorFn> = HashMap::new();
    m.insert("label", |a| {
        Ok(DynPiece::new(Inner::Label(label_like(a, "label")?)))
    });
    // Familiar alias for scripters; same piece.
    m.insert("text", |a| {
        Ok(DynPiece::new(Inner::Label(label_like(a, "text")?)))
    });
    m.insert("button", |a| {
        let b = match text_arg(a, 0, "button")? {
            TextArg::Lit(s) => button(s),
            TextArg::Cb(f) => button(move || f(&[]).display()),
            TextArg::Sig(s) => button(s),
        };
        Ok(DynPiece::new(Inner::Button(b)))
    });
    m.insert("column", |a| {
        Ok(DynPiece::new(Inner::Column(column(children(a)?))))
    });
    m.insert("row", |a| Ok(DynPiece::new(Inner::Row(row(children(a)?)))));
    m.insert("grid", |a| {
        Ok(DynPiece::new(Inner::Grid(grid(children(a)?))))
    });
    m.insert("grid_row", |a| {
        Ok(DynPiece::new(Inner::GridRow(grid_row(children(a)?))))
    });
    m.insert("scroll", |a| {
        let kids = children(a)?;
        let one: AnyPiece = if kids.0.len() == 1 {
            kids.0.into_iter().next().expect("len checked")
        } else {
            column(kids).any()
        };
        Ok(DynPiece::new(Inner::Any(scroll(one).any())))
    });
    m.insert("spacer", |_| Ok(DynPiece::new(Inner::Any(spacer().any()))));
    m.insert("divider", |_| {
        Ok(DynPiece::new(Inner::Any(divider().any())))
    });
    m.insert("toggle", |a| match a.first() {
        Some(DynValue::Signal(DynSignal::Bool(s))) => Ok(DynPiece::new(Inner::Toggle(toggle(*s)))),
        _ => Err(DynError::Type {
            what: "toggle",
            want: "a bool signal",
        }),
    });
    m.insert("slider", |a| match a.first() {
        Some(DynValue::Signal(DynSignal::Num(s))) => Ok(DynPiece::new(Inner::Slider(slider(*s)))),
        _ => Err(DynError::Type {
            what: "slider",
            want: "a number signal",
        }),
    });
    m.insert("text_field", |a| match a.first() {
        Some(DynValue::Signal(DynSignal::Str(s))) => {
            Ok(DynPiece::new(Inner::TextField(text_field(*s))))
        }
        _ => Err(DynError::Type {
            what: "text_field",
            want: "a string signal",
        }),
    });
    m.insert("progress", |a| match a.first() {
        Some(DynValue::Num(n)) => Ok(DynPiece::new(Inner::Progress(progress(*n)))),
        Some(DynValue::Fn(f)) => {
            let f = f.clone();
            Ok(DynPiece::new(Inner::Progress(progress(move || {
                f(&[]).as_f64().unwrap_or(0.0)
            }))))
        }
        _ => Err(DynError::Type {
            what: "progress",
            want: "a number or function",
        }),
    });
    m.insert("image", |a| {
        Ok(DynPiece::new(Inner::Image(image(str_arg(a, 0, "image")?))))
    });
    // Reactive structure: a conditional subtree and keyed reactive rows (§5.4). Rows are
    // keyed by each item's serialized value, so a changed item RE-BUILDS its row — the
    // simple, correct default for scripted UIs (no stale-binding hazard).
    m.insert("when", |a| {
        let (Some(DynValue::Fn(cond)), Some(DynValue::Fn(build))) = (a.first(), a.get(1)) else {
            return Err(DynError::Type {
                what: "when",
                want: "(condFn, buildFn)",
            });
        };
        let (cond, build) = (cond.clone(), build.clone());
        Ok(DynPiece::new(Inner::Any(
            when(
                move || matches!(cond(&[]), DynValue::Bool(true)),
                move || match build(&[]) {
                    DynValue::Piece(p) => p.into_any(),
                    _ => spacer().any(),
                },
            )
            .any(),
        )))
    });
    m.insert("each", |a| {
        let (Some(DynValue::Fn(items)), Some(DynValue::Fn(build_row))) = (a.first(), a.get(1))
        else {
            return Err(DynError::Type {
                what: "each",
                want: "(itemsFn, rowFn)",
            });
        };
        let (items, build_row) = (items.clone(), build_row.clone());
        Ok(DynPiece::new(Inner::Any(
            each(
                move || match items(&[]) {
                    DynValue::List(v) => v,
                    _ => Vec::new(),
                },
                |item: &DynValue| item.key_string(),
                move |slot| match build_row(&[slot.get()]) {
                    DynValue::Piece(p) => p.into_any(),
                    _ => spacer().any(),
                },
            )
            .any(),
        )))
    });
    m
}

// ---- modifiers --------------------------------------------------------------------------

fn apply_modifier(inner: Inner, name: &str, args: &[DynValue]) -> Result<Inner, DynError> {
    // Type-specific builder methods first, while the concrete type is still present.
    let inner = match (inner, name) {
        (Inner::Label(p), "font") => {
            // A number is a point size (`Font::System`); a string is a semantic style name.
            let font = match args.first() {
                Some(DynValue::Num(pt)) => Font::System(*pt),
                Some(DynValue::Str(name)) => font_of(name).ok_or(DynError::Type {
                    what: "font",
                    want: "a Font name (title, body, caption, …) or a point size",
                })?,
                _ => {
                    return Err(DynError::Type {
                        what: "font",
                        want: "a Font name or point size",
                    });
                }
            };
            return Ok(Inner::Label(p.font(font)));
        }
        (Inner::Button(p), "action") => match args.first() {
            Some(DynValue::Fn(f)) => return Ok(Inner::Button(p.action(callback0(f)))),
            _ => {
                return Err(DynError::Type {
                    what: "action",
                    want: "a function",
                });
            }
        },
        (Inner::Column(p), "spacing") => {
            return Ok(Inner::Column(p.spacing(num(args, 0, "spacing")?)));
        }
        (Inner::Row(p), "spacing") => return Ok(Inner::Row(p.spacing(num(args, 0, "spacing")?))),
        (Inner::Grid(p), "spacing") => return Ok(Inner::Grid(p.spacing(num(args, 0, "spacing")?))),
        (Inner::Column(p), "align") => {
            let a = str_arg(args, 0, "align")?;
            let al = halign_of(&a).ok_or(DynError::Type {
                what: "align",
                want: "leading | center | trailing",
            })?;
            return Ok(Inner::Column(p.align(al)));
        }
        (Inner::Row(p), "align") => {
            let a = str_arg(args, 0, "align")?;
            let al = valign_of(&a).ok_or(DynError::Type {
                what: "align",
                want: "top | center | bottom",
            })?;
            return Ok(Inner::Row(p.align(al)));
        }
        (Inner::Slider(p), "range") => {
            let lo = num(args, 0, "range")?;
            let hi = num(args, 1, "range")?;
            return Ok(Inner::Slider(p.range(lo..=hi)));
        }
        (Inner::Slider(p), "step") => return Ok(Inner::Slider(p.step(num(args, 0, "step")?))),
        (Inner::TextField(p), "placeholder") => {
            return Ok(Inner::TextField(p.placeholder(str_arg(
                args,
                0,
                "placeholder",
            )?)));
        }
        (Inner::TextField(p), "on_submit") => match args.first() {
            Some(DynValue::Fn(f)) => return Ok(Inner::TextField(p.on_submit(callback0(f)))),
            _ => {
                return Err(DynError::Type {
                    what: "on_submit",
                    want: "a function",
                });
            }
        },
        // A typed-modifier name arriving on an already-erased piece is the ordering error.
        (Inner::Any(p), n)
            if matches!(
                n,
                "font"
                    | "action"
                    | "spacing"
                    | "align"
                    | "range"
                    | "step"
                    | "placeholder"
                    | "on_submit"
            ) =>
        {
            let _ = p;
            return Err(DynError::LateTyped {
                modifier: n.into(),
                piece: "concrete",
            });
        }
        (inner, _) => inner,
    };

    // Generic Decorate modifiers: erase, wrap, stay erased.
    let p = inner.into_any();
    let out: AnyPiece = match name {
        "id" => p.id(str_arg(args, 0, "id")?),
        "padding" => p.padding(num(args, 0, "padding")?),
        "frame" => p.frame(num(args, 0, "frame")?, num(args, 1, "frame")?),
        "width" => p.width(num(args, 0, "width")?),
        "height" => p.height(num(args, 0, "height")?),
        "corner_radius" => p.corner_radius(num(args, 0, "corner_radius")?),
        "background" => match args.first() {
            Some(DynValue::Fn(f)) => {
                let f = f.clone();
                p.background(move || color_of(&f(&[])).unwrap_or(Color::CLEAR))
            }
            Some(v) => p.background(color_of(v)?),
            None => {
                return Err(DynError::Type {
                    what: "background",
                    want: "a color",
                });
            }
        },
        "on_tap" => match args.first() {
            Some(DynValue::Fn(f)) => p.on_tap(callback0(f)),
            _ => {
                return Err(DynError::Type {
                    what: "on_tap",
                    want: "a function",
                });
            }
        },
        "grow" => p.grow(),
        "grow_w" => p.grow_w(),
        "grow_h" => p.grow_h(),
        "grid_span" => p.grid_span(num(args, 0, "grid_span")? as usize),
        "grid_align" => {
            let a = str_arg(args, 0, "grid_align")?;
            p.grid_align(alignment_of(&a).ok_or(DynError::Type {
                what: "grid_align",
                want: "an alignment name",
            })?)
        }
        "overlay" => match args.first() {
            Some(DynValue::Piece(over)) => p.overlay(over.into_any()),
            _ => {
                return Err(DynError::Type {
                    what: "overlay",
                    want: "a piece",
                });
            }
        },
        "overlay_aligned" => {
            let a = str_arg(args, 0, "overlay_aligned")?;
            let al = alignment_of(&a).ok_or(DynError::Type {
                what: "overlay_aligned",
                want: "an alignment name",
            })?;
            match args.get(1) {
                Some(DynValue::Piece(over)) => p.overlay_aligned(al, over.into_any()),
                _ => {
                    return Err(DynError::Type {
                        what: "overlay_aligned",
                        want: "a piece",
                    });
                }
            }
        }
        "defers_system_gestures" => p.defers_system_gestures(Edges::ALL),
        "interactive_dismiss_disabled" => p.interactive_dismiss_disabled(),
        other => {
            let ext = ext_modifiers()
                .lock()
                .ok()
                .and_then(|m| m.get(other).copied());
            match ext {
                Some(f) => f(p, args)?,
                None => {
                    return Err(DynError::Unknown {
                        what: "modifier",
                        name: other.into(),
                    });
                }
            }
        }
    };
    Ok(Inner::Any(out))
}

/// Introspection: every name the dynamic surface answers to. day-lite generates the JS API
/// from this, so the script surface and the dispatch tables cannot drift.
pub fn catalog() -> Vec<SpecEntry> {
    let mut out: Vec<SpecEntry> = Vec::new();
    let c = |name, sig| SpecEntry {
        name,
        kind: SpecKind::Constructor,
        sig,
    };
    let g = |name, sig| SpecEntry {
        name,
        kind: SpecKind::Modifier,
        sig,
    };
    let t = |name, sig| SpecEntry {
        name,
        kind: SpecKind::TypedModifier,
        sig,
    };
    out.extend([
        c("label", "label(text | fn | signal)"),
        c("text", "text(text | fn | signal) — alias of label"),
        c("button", "button(title | fn | signal)"),
        c("column", "column(...pieces)"),
        c("row", "row(...pieces)"),
        c("grid", "grid(...grid_rows)"),
        c("grid_row", "grid_row(...pieces)"),
        c("scroll", "scroll(...pieces)"),
        c("spacer", "spacer()"),
        c("divider", "divider()"),
        c("toggle", "toggle(boolSignal)"),
        c("slider", "slider(numSignal)"),
        c("text_field", "text_field(strSignal)"),
        c("progress", "progress(fraction | fn)"),
        c("image", "image(name)"),
        c("when", "when(condFn, buildFn)"),
        c("each", "each(itemsFn, rowFn) — rows keyed by item value"),
        t("font", "label.font(name)"),
        t("action", "button.action(fn)"),
        t("spacing", "column|row|grid.spacing(n)"),
        t("align", "column|row.align(name)"),
        t("range", "slider.range(lo, hi)"),
        t("step", "slider.step(n)"),
        t("placeholder", "text_field.placeholder(s)"),
        t("on_submit", "text_field.on_submit(fn)"),
        g("id", "id(s)"),
        g("padding", "padding(n)"),
        g("frame", "frame(w, h)"),
        g("width", "width(n)"),
        g("height", "height(n)"),
        g("corner_radius", "corner_radius(n)"),
        g("background", "background(#hex | fn)"),
        g("on_tap", "on_tap(fn)"),
        g("grow", "grow()"),
        g("grow_w", "grow_w()"),
        g("grow_h", "grow_h()"),
        g("grid_span", "grid_span(n)"),
        g("grid_align", "grid_align(name)"),
        g("overlay", "overlay(piece)"),
        g("overlay_aligned", "overlay_aligned(name, piece)"),
        g("defers_system_gestures", "defers_system_gestures()"),
        g(
            "interactive_dismiss_disabled",
            "interactive_dismiss_disabled()",
        ),
    ]);
    if let Ok(m) = ctors().lock() {
        for name in m.keys() {
            if !out.iter().any(|e| e.name == *name) {
                out.push(SpecEntry {
                    name,
                    kind: SpecKind::Constructor,
                    sig: "(extension)",
                });
            }
        }
    }
    if let Ok(m) = ext_modifiers().lock() {
        for name in m.keys() {
            if !out.iter().any(|e| e.name == *name) {
                out.push(SpecEntry {
                    name,
                    kind: SpecKind::Modifier,
                    sig: "(extension)",
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructs_and_modifies_a_chain() {
        let title = construct("label", &[DynValue::Str("Hi".into())]).expect("label");
        title
            .modify("font", &[DynValue::Str("title".into())])
            .expect("font");
        title
            .modify("padding", &[DynValue::Num(8.0)])
            .expect("padding");
        let col = construct(
            "column",
            &[
                DynValue::Piece(title),
                DynValue::Piece(construct("spacer", &[]).unwrap()),
            ],
        )
        .expect("column");
        col.modify("spacing", &[DynValue::Num(12.0)])
            .expect("spacing");
        col.modify("id", &[DynValue::Str("root".into())])
            .expect("id");
        let _ = col.into_any();
    }

    #[test]
    fn typed_after_generic_is_a_clear_error() {
        let l = construct("label", &[DynValue::Str("x".into())]).unwrap();
        l.modify("padding", &[DynValue::Num(4.0)]).unwrap();
        let err = l
            .modify("font", &[DynValue::Str("body".into())])
            .unwrap_err();
        assert!(matches!(err, DynError::LateTyped { .. }));
    }

    #[test]
    fn unknown_names_error() {
        assert!(matches!(
            construct("blink", &[]),
            Err(DynError::Unknown { .. })
        ));
        let l = construct("label", &[DynValue::Str("x".into())]).unwrap();
        assert!(matches!(
            l.modify("blur", &[]),
            Err(DynError::Unknown { .. })
        ));
    }

    #[test]
    fn colors_parse() {
        assert!(parse_color("#fff").is_some());
        assert!(parse_color("#101024").is_some());
        assert!(parse_color("#80101024").is_some());
        assert!(parse_color("101024").is_none());
        assert!(parse_color("#12345").is_none());
    }
}
