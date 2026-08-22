// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! day-piece-colorpicker — a color chooser bound two-way to a `Signal<Color>`, in two idioms
//! (docs/colorpicker.md; DESIGN.md §15 tier 1+shim).
//!
//! ```ignore
//! let tint = Signal::new(Color::hex(0xE86A3C));
//! row((
//!     vector(gv::nav_animation).tint(move || tint.get()).frame(28.0, 28.0),
//!     color_picker(tint).alpha(true),              // the platform's chooser where there is one
//!     color_picker(tint).composed(),               // Day's own panel, identical everywhere
//! ))
//! ```
//!
//! Both idioms give the app the same control — a **color well**: a swatch showing the current
//! color that opens a chooser when pressed. They differ only in who draws the chooser.
//!
//! - [`PickerIdiom::Native`] realizes a native leaf: `NSColorWell` onto the shared
//!   `NSColorPanel`, `UIColorWell` onto the iOS color picker, `GtkColorDialogButton`, a swatch
//!   button onto `QColorDialog`, the XAML `ColorPicker` in a button flyout, and
//!   `<input type="color">`. Each is the system chooser, chrome and all.
//! - [`PickerIdiom::Composed`] builds the whole thing out of ordinary Day pieces: a drawn swatch
//!   opening an [`unrouted`](day_pieces::Cover::unrouted) [`cover`] that holds a canvas
//!   saturation/brightness field, a hue strip, an opacity strip and a preset palette. Every part
//!   of it is Rust that already runs everywhere, so it is the same picker on all nine targets.
//!
//! [`PickerIdiom::Automatic`] — the default — is `Native` where the toolkit has a chooser and
//! `Composed` where it does not. Two toolkits have none at any layer: Android ships no color
//! picker in the framework, in Material, or in AndroidX, and HarmonyOS has none in ArkTS or in the
//! ArkUI NDK. Rather than each of those growing a hand-written dialog in its own language, they
//! get the composed panel — which is also why an app that wants ONE picker everywhere can just ask
//! for it.
//!
//! The value is Day's ordinary [`Color`], so the same signal drives `.tint(…)`, `.background(…)`,
//! a canvas fill or a gradient stop with no conversion — see [docs/color.md](../color/) for what
//! that currency does and does not carry across from a native pick.
//!
//! The native leaf also accepts `Event::TextChanged` carrying any form [`Color::parse`] reads
//! (`"#e86a3c"`, `"#e86a3c80"`, `"0.91 0.42 0.24 1"`) as a synthetic pick, so dayscript's `input:`
//! step drives it on every backend.

use day_core::{BuildCx, Flex, Piece, RNode, with_tree};
use day_pieces::prelude::*;
use day_pieces::{IntoText, TextSource};
use day_reactive::{Binding, bind_seeded, watch};
use day_spec::{Event, Support};

pub const KIND: &str = "day.piece.colorpicker";

/// The tag every in-process native arm reports a pick under. Across a JNI / C-ABI / JS boundary
/// the tag cannot be a `&'static str`, so it arrives empty and only the payload matters (§8.2) —
/// the front-end reads the text either way.
pub const PICK_TAG: &str = "colorpicker:value";

/// Full props (realize) for the NATIVE leaf. `alpha` and `title` are set once at build; only
/// `color` patches. The composed idiom realizes no leaf and never builds these.
#[derive(Clone, Debug, PartialEq)]
pub struct ColorProps {
    pub color: Color,
    /// Offer an opacity channel in the chooser. Off by default: most pickers hide the alpha
    /// slider unless asked, and a color that can go transparent behind the app's back is rarely
    /// what a tint or a brand swatch wants.
    pub alpha: bool,
    /// The chooser's title, where the platform shows one (`""` = the platform default).
    pub title: String,
}

impl Default for ColorProps {
    fn default() -> Self {
        ColorProps {
            color: Color::BLACK,
            alpha: false,
            title: String::new(),
        }
    }
}

/// The single imperative update: show `color` in the well (and in the chooser, if it is open).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ColorPatch {
    SetColor(Color),
}

/// Which chooser a [`ColorPicker`] opens.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PickerIdiom {
    /// The platform's own chooser where the toolkit has one; Day's composed panel where it does
    /// not (android-mdc and harmony-arkui). The default, and what an app wants unless it has a
    /// reason to pin the answer.
    #[default]
    Automatic,
    /// The platform's own chooser, on the nose: this realizes the native leaf whatever the
    /// backend is, so on a toolkit with no renderer for it the app gets Day's visible
    /// `⟨day.piece.colorpicker⟩` placeholder — the same answer any unrendered kind gives, and a
    /// gap a screenshot can see. Pin this only behind a [`support`] check; if you want "native
    /// where there is one", that is [`Automatic`](PickerIdiom::Automatic).
    Native,
    /// Day's composed panel, on every target. Ask for this when one identical color experience
    /// matters more than platform chrome — a design tool, a themed app, a branded editor.
    Composed,
}

/// Whether the compiled backend has a NATIVE color chooser — what [`PickerIdiom::Automatic`]
/// resolves against.
///
/// [`Support::Native`] on appkit, uikit, gtk, qt, xaml and web-dom. [`Support::Emulated`] on
/// android-mdc and harmony-arkui, where `Automatic` gives the composed panel instead.
///
/// This does not report whether the picker works — there is no target where it does not — so an
/// app showing a "not supported here" banner from this answer would be wrong. Use it to say
/// *which* picker the user gets, or ignore it, unless the app pins [`PickerIdiom::Native`]:
/// that realizes the leaf unconditionally and so DOES need this checked first.
pub fn support() -> Support {
    if cfg!(any(
        all(feature = "appkit", target_os = "macos"),
        all(feature = "uikit", target_os = "ios"),
        all(feature = "xaml", windows),
        all(feature = "dom", target_arch = "wasm32"),
        feature = "gtk",
        feature = "qt",
    )) {
        Support::Native
    } else {
        Support::Emulated
    }
}

/// The palette the composed panel offers when the app names none: one hue ring at full
/// saturation, plus the grayscale ends. Ordered by hue so the row reads as a spectrum.
fn default_presets() -> Vec<Color> {
    let mut v: Vec<Color> = (0..12)
        .map(|i| Color::hsv(i as f64 * 30.0, 0.78, 0.92))
        .collect();
    v.push(Color::WHITE);
    v.push(Color::rgb(0.6, 0.6, 0.62));
    v.push(Color::BLACK);
    v
}

/// A color well bound two-way to a [`Color`] signal. Build with [`color_picker`].
pub struct ColorPicker<C: Binding<Color>> {
    color: C,
    alpha: bool,
    idiom: PickerIdiom,
    title: Option<TextSource>,
    presets: Option<Vec<Color>>,
    key: String,
}

/// `color_picker(color)` — a swatch showing `color` that opens a color chooser. `color` is a
/// `Signal<Color>` or any other two-way binding (a day-model `Field`, mapped).
pub fn color_picker<C: Binding<Color>>(color: C) -> ColorPicker<C> {
    // web-dom's registry is populated at RUNTIME (no `linkme` on wasm), and a constructor always
    // runs before the node it returns is realized — so this is where the arm registers itself.
    #[cfg(all(feature = "dom", target_arch = "wasm32"))]
    dom_impl::register();
    ColorPicker {
        color,
        alpha: false,
        idiom: PickerIdiom::Automatic,
        title: None,
        presets: None,
        key: "color-picker".to_string(),
    }
}

impl<C: Binding<Color>> ColorPicker<C> {
    /// Offer an opacity channel, and let the bound signal carry a non-opaque alpha. Honored by
    /// the composed panel everywhere, and by every native chooser but one: a browser's
    /// `<input type="color">` gained an `alpha` attribute only recently, so the web arm sets it
    /// and stays opaque where the browser has not shipped it (docs/colorpicker.md).
    pub fn alpha(mut self, on: bool) -> Self {
        self.alpha = on;
        self
    }
    /// Which chooser this well opens (see [`PickerIdiom`]).
    pub fn idiom(mut self, idiom: PickerIdiom) -> Self {
        self.idiom = idiom;
        self
    }
    /// Pin this well to the platform's own chooser — [`PickerIdiom::Native`].
    pub fn native(self) -> Self {
        self.idiom(PickerIdiom::Native)
    }
    /// Pin this well to Day's composed panel — [`PickerIdiom::Composed`].
    pub fn composed(self) -> Self {
        self.idiom(PickerIdiom::Composed)
    }
    /// The chooser's heading. Native: the iOS picker's navigation title, the GTK dialog heading,
    /// `QColorDialog`'s window title, the XAML button's tooltip; AppKit sets the shared color
    /// panel's title. Composed: the panel's own heading. A constant, `Signal<String>` or closure,
    /// read once at build.
    pub fn title<M>(mut self, t: impl IntoText<M>) -> Self {
        self.title = Some(t.into_text());
        self
    }
    /// The swatch row the COMPOSED panel offers below its strips (ignored by the native
    /// choosers, which each have their own palette). Pass an empty vector to drop the row.
    pub fn presets(mut self, presets: Vec<Color>) -> Self {
        self.presets = Some(presets);
        self
    }
    /// The COMPOSED well's dayscript id (default `"color-picker"`).
    ///
    /// It goes here rather than on `Decorate::id` because what an app can reach from outside is
    /// the layout wrapper the piece returns, and an id on that tags a node no toolkit realizes —
    /// a `tap:` against it resolves to nothing while every step still reports ✓. Give two
    /// composed pickers on one page different keys so a script can tell them apart.
    ///
    /// The panel's own parts carry fixed ids (`color-picker-panel`, `-shade`, `-hue`,
    /// `-opacity`, `-presets`, `-value`, `-cancel`, `-done`); only one panel is ever open, so
    /// they need no disambiguation.
    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.key = key.into();
        self
    }
}

impl<C: Binding<Color>> Piece for ColorPicker<C> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let ColorPicker {
            color,
            alpha,
            idiom,
            title,
            presets,
            key,
        } = self;
        let title = title.map(|t| t.initial()).unwrap_or_default();
        let native = match idiom {
            PickerIdiom::Composed => false,
            PickerIdiom::Native => true,
            PickerIdiom::Automatic => support() == Support::Native,
        };
        if native {
            build_native(cx, color, alpha, title, key)
        } else {
            composed_well(
                color,
                alpha,
                title,
                presets.unwrap_or_else(default_presets),
                key,
            )
            .build(cx)
        }
    }
}

/// The native idiom: one leaf of [`KIND`], bound two-way.
fn build_native<C: Binding<Color>>(
    cx: &mut BuildCx,
    color: C,
    alpha: bool,
    title: String,
    key: String,
) -> RNode {
    let initial = color.peek();
    let node = cx.leaf(
        KIND,
        &ColorProps {
            color: initial,
            alpha,
            title,
        },
        Flex::default(),
    );
    // The leaf's dayscript id — one `.key` serves both idioms (the day-piece-stepper rule:
    // `Decorate::id` on the piece would tag a wrapper no toolkit realizes).
    with_tree(|t| t.set_id(node, key));
    // App writes → the native well. Every arm no-ops on an unchanged value, so a pick echoing
    // back through the signal never loops.
    let c2 = color.clone();
    bind_seeded(
        initial,
        move || c2.read(),
        move |c: &Color| {
            with_tree(|t| t.patch(node, Box::new(ColorPatch::SetColor(*c)), false));
        },
    );
    // Native picks (`Custom`, carrying the lossless component form) and dayscript's `input:` step
    // (`TextChanged`, carrying whatever a human typed) → the signal. Both go through
    // `Color::parse`, which reads hex and components alike, so there is one decode path.
    cx.on(node, move |ev| {
        let picked = match ev {
            Event::Custom { text, .. } => Color::parse(text),
            Event::TextChanged(s) => Color::parse(s),
            _ => None,
        };
        if let Some(c) = picked {
            // An opaque-only picker must not be able to clear the alpha the app set.
            color.write(if alpha { c } else { c.with_alpha(1.0) });
        }
    });
    node
}

// ===========================================================================
// The composed idiom — ordinary Day pieces, no native code, identical on all nine targets.
// ===========================================================================

/// The saturation/brightness field, in points. Fixed rather than fluid: a picker panel is a
/// fixed-size object on every platform that has one, and a known size is what lets a press
/// location become a value without the canvas having to report its own bounds first.
const FIELD_W: f64 = 264.0;
const FIELD_H: f64 = 160.0;
/// The hue and opacity strips.
const STRIP_H: f64 = 20.0;
/// One preset swatch.
const SWATCH: f64 = 26.0;
/// The panel card. `FIELD_W` plus its padding on both sides.
const CARD_PAD: f64 = 18.0;
const CARD_W: f64 = FIELD_W + CARD_PAD * 2.0;

/// The panel is a NEUTRAL DARK surface on every target, light appearance included, and its own
/// text color is stated rather than inherited.
///
/// Both halves of that are deliberate. A bright surround biases color judgment — the reason
/// every serious color tool sits its swatches on a dark neutral — so a picker that flipped to a
/// white card in light mode would make the same pick look like a different color. And a card
/// whose fill this piece chose cannot then take the platform's label color for its text: on a
/// light-appearance device that resolves to black, which is what put dark-on-dark text in the
/// first iOS screenshot of this panel.
const PANEL_SURFACE: Color = Color::rgb(0.13, 0.14, 0.17);
const PANEL_TEXT: Color = Color::rgb(0.93, 0.94, 0.96);

/// The well, in points: wide enough for `#rrggbbaa` at the caption size.
const WELL_W: f64 = 96.0;
const WELL_H: f64 = 26.0;

/// The well itself: a DRAWN swatch showing the current color and its hex, which presents the
/// panel when pressed.
///
/// Drawn rather than a `button(hex).tint(color)`, even though a tinted button is native and
/// carries press feedback and focus for free. What the composed idiom promises is ONE picker,
/// identical on every target — and a tinted button is the opposite of identical: AppKit
/// composites the color through the bezel, Material draws a filled container with its own
/// elevation, GTK and Qt apply it through their themes, and the web takes CSS. The color would
/// read differently on all nine. A canvas draws the color the app asked for.
///
/// The trade is keyboard activation: a drawn well takes a press, not a Return key. It carries a
/// `Button` role and a label so a screen reader still announces it correctly
/// (docs/colorpicker.md records the gap).
fn composed_well<C: Binding<Color>>(
    color: C,
    alpha: bool,
    title: String,
    presets: Vec<Color>,
    key: String,
) -> AnyPiece {
    let open: Signal<Option<String>> = Signal::new(None);
    let open_key = key.clone();
    // The id goes on the CANVAS, here, rather than being left to the app: what the app can reach
    // is the `zstack` this returns, and an id on that tags a layout wrapper the toolkit never
    // realizes — a dayscript `tap:` against it would resolve to nothing while every step still
    // reported ✓. The route key doubles as the id, so one name identifies the well both ways.
    let cw = color.clone();
    let well = canvas(move |d, size| {
        let c = cw.read();
        let r = Rect::new(0.0, 0.0, size.width, size.height);
        if c.a < 1.0 {
            checkerboard(d, size);
        }
        d.fill(Shape::RoundedRect(r, 7.0), c);
        d.stroke(
            Shape::RoundedRect(r.inset(0.5), 7.0),
            Color::BLACK.with_alpha(0.25),
            1.0,
        );
        d.text(
            &c.to_hex_string(),
            Point::new(size.width / 2.0, size.height / 2.0),
            TextStyle {
                size: 12.0,
                // Day's own readable-on-a-fill rule, the one `Button::tint` uses.
                color: day_spec::props::ButtonStyleSpec::on_tint(c),
                anchor: TextAnchor::Centered,
            },
        );
    })
    .on_tap(move || open.set(Some(open_key.clone())))
    .a11y(|a| a.role(Role::Button).label(day_l10n::t("day-color")))
    .id(key.clone())
    .frame(WELL_W, WELL_H);
    zstack((
        well,
        cover(open, move |_| {
            panel(color.clone(), alpha, title.clone(), presets.clone(), open)
        })
        // The panel's ground. `cover` paints this edge-to-edge (under the status bar and home
        // indicator); the emulated tier forces it opaque, so a dim scrim would read as flat gray
        // on six of the nine targets. A near-black ground that the card sits on reads the same
        // everywhere instead.
        .background(|_: &String| Color::rgba(0.06, 0.07, 0.09, 0.92))
        // OUT of the app's route space. A routed cover over the untyped `String` route claims
        // every segment — `String::from_key` accepts anything — so mounting this picker would
        // have made the host app's next `navigate("settings")` present a color panel keyed
        // "settings" instead of going to settings. A chooser is not a destination; the piece
        // opens and closes it, and Android's system back still dismisses it (that path is the
        // cover's own `NavBack` handler, not the route adapter).
        .unrouted(),
    ))
    .any()
}

/// The presented panel: the card, centered on the cover's surface.
fn panel<C: Binding<Color>>(
    color: C,
    alpha: bool,
    title: String,
    presets: Vec<Color>,
    open: Signal<Option<String>>,
) -> AnyPiece {
    // HSV is the panel's source of truth, not the bound color. Deriving hue from RGB on every
    // change would lose it the moment brightness reached zero (black has no hue), so the sliders
    // would jump back to red as the user dragged into the corner. Seeded once per presentation.
    let entry = color.peek();
    let (h0, s0, v0) = entry.to_hsv();
    let hue = Signal::new(h0);
    let sat = Signal::new(s0);
    let val = Signal::new(v0);
    let opacity = Signal::new(entry.a);

    // HSV → the bound color, live: the app sees the tint move as the user drags, the same as
    // every native chooser reports continuously. `watch` skips the initial run, so opening the
    // panel does not rewrite the signal with a round-tripped copy of what it already holds.
    watch(
        move || {
            Color::hsva(
                hue.get(),
                sat.get(),
                val.get(),
                if alpha { opacity.get() } else { 1.0 },
            )
        },
        {
            let color = color.clone();
            move |c: &Color, _| color.write(*c)
        },
    );

    let current = move || {
        Color::hsva(
            hue.get(),
            sat.get(),
            val.get(),
            if alpha { opacity.get() } else { 1.0 },
        )
    };

    let close = move || open.set(None);
    let cancel = move || {
        color.write(entry);
        open.set(None);
    };

    let mut rows: Vec<AnyPiece> = Vec::new();
    if !title.is_empty() {
        rows.push(label(title).font(Font::Headline).color(PANEL_TEXT).any());
    }
    rows.push(shade_field(hue, sat, val));
    rows.push(hue_strip(hue));
    if alpha {
        rows.push(opacity_strip(current, opacity));
    }
    rows.push(readout(current));
    if !presets.is_empty() {
        rows.push(preset_row(presets, hue, sat, val, opacity, alpha));
    }
    rows.push(
        row((
            button(day_l10n::t("day-cancel"))
                .action(cancel)
                .id("color-picker-cancel"),
            spacer(),
            button(day_l10n::t("day-done"))
                .action(close)
                .id("color-picker-done"),
        ))
        .grow_w()
        .any(),
    );

    let card = column(PieceVec(rows))
        .spacing(12.0)
        .align(HAlign::Leading)
        .padding(CARD_PAD)
        .background(PANEL_SURFACE)
        .corner_radius(16.0)
        .width(CARD_W)
        .id("color-picker-panel");

    // Centered both ways: spacers above and below push it to the middle vertically, and the
    // column's own cross-axis alignment centers it horizontally.
    //
    // NOT `row((spacer(), card, spacer()))` for the horizontal half, which is the obvious
    // spelling and the wrong one: a cover lays its content out once BEFORE the backend reports
    // the surface's size, so that row measures a 300pt card against a 0pt width and Day's
    // overflow diagnostic fires — naming this panel's ids on every single open. A cross-aligned
    // column has nothing to overflow.
    column((spacer(), card, spacer()))
        .align(HAlign::Center)
        .grow()
        .any()
}

/// The saturation/brightness field for the current hue: the pure hue, washed to white across and
/// to black down. Three fills, which is exactly how every native picker draws the same square —
/// and it stays crisp at any size because two of them are gradients rather than a bitmap.
fn shade_field(hue: Signal<f64>, sat: Signal<f64>, val: Signal<f64>) -> AnyPiece {
    let pick = move |p: Point| {
        sat.set((p.x / FIELD_W).clamp(0.0, 1.0));
        val.set(1.0 - (p.y / FIELD_H).clamp(0.0, 1.0));
    };
    canvas(move |d, size| {
        let r = Rect::new(0.0, 0.0, size.width, size.height);
        d.fill(Shape::Rect(r), Color::hsv(hue.get(), 1.0, 1.0));
        d.fill(
            Shape::Rect(r),
            LinearGradient::horizontal(Color::WHITE, Color::WHITE.with_alpha(0.0)),
        );
        d.fill(
            Shape::Rect(r),
            LinearGradient::vertical(Color::BLACK.with_alpha(0.0), Color::BLACK),
        );
        marker(
            d,
            Point::new(sat.get() * size.width, (1.0 - val.get()) * size.height),
            7.0,
        );
    })
    // Gestures go on the CANVAS, before any wrapper: `Event::Tap`/`Event::Drag` report a point in
    // the node's own space, and a wrapper's space is not the canvas's. Both are wired because a
    // press that never moves is a tap on some backends and a zero-length drag on others; they
    // write the same two values, so a backend that reports both costs nothing.
    .on_drag(move |drag| pick(drag.location))
    .on_tap_at(pick)
    .a11y(|a| a.label(day_l10n::t("day-color-shade")))
    .frame(FIELD_W, FIELD_H)
    .corner_radius(10.0)
    .id("color-picker-shade")
    .any()
}

/// The hue strip: one linear gradient through the six primaries and back to red.
fn hue_strip(hue: Signal<f64>) -> AnyPiece {
    let pick = move |p: Point| hue.set((p.x / FIELD_W).clamp(0.0, 1.0) * 360.0);
    canvas(move |d, size| {
        let r = Rect::new(0.0, 0.0, size.width, size.height);
        let stops: Vec<(f64, Color)> = (0..=6)
            .map(|i| (i as f64 / 6.0, Color::hsv(i as f64 * 60.0, 1.0, 1.0)))
            .collect();
        d.fill(
            Shape::Rect(r),
            LinearGradient::new(UnitPoint::LEADING, UnitPoint::TRAILING, stops),
        );
        slider_thumb(d, hue.get() / 360.0 * size.width, size.height);
    })
    .on_drag(move |drag| pick(drag.location))
    .on_tap_at(pick)
    .a11y(|a| a.label(day_l10n::t("day-color-hue")))
    .frame(FIELD_W, STRIP_H)
    .corner_radius(STRIP_H / 2.0)
    .id("color-picker-hue")
    .any()
}

/// The opacity strip: the current color faded across, over the checkerboard that is the only way
/// to tell "transparent" from "white" on a light ground.
fn opacity_strip(current: impl Fn() -> Color + 'static, opacity: Signal<f64>) -> AnyPiece {
    let pick = move |p: Point| opacity.set((p.x / FIELD_W).clamp(0.0, 1.0));
    canvas(move |d, size| {
        checkerboard(d, size);
        let opaque = current().with_alpha(1.0);
        d.fill(
            Shape::Rect(Rect::new(0.0, 0.0, size.width, size.height)),
            LinearGradient::horizontal(opaque.with_alpha(0.0), opaque),
        );
        slider_thumb(d, opacity.get() * size.width, size.height);
    })
    .on_drag(move |drag| pick(drag.location))
    .on_tap_at(pick)
    .a11y(|a| a.label(day_l10n::t("day-color-opacity")))
    .frame(FIELD_W, STRIP_H)
    .corner_radius(STRIP_H / 2.0)
    .id("color-picker-opacity")
    .any()
}

/// A wrapping row of preset swatches. `RowFit::Wrap` rather than a grid: the palette is however
/// many colors the app passed, and the row is however wide the card is (docs/size-classes.md).
fn preset_row(
    presets: Vec<Color>,
    hue: Signal<f64>,
    sat: Signal<f64>,
    val: Signal<f64>,
    opacity: Signal<f64>,
    alpha: bool,
) -> AnyPiece {
    let swatches: Vec<AnyPiece> = presets
        .into_iter()
        .map(|c| {
            canvas(move |d, size| {
                let r = Rect::new(0.0, 0.0, size.width, size.height);
                if c.a < 1.0 {
                    checkerboard(d, size);
                }
                d.fill(Shape::RoundedRect(r, 6.0), c);
                d.stroke(
                    Shape::RoundedRect(r.inset(0.5), 6.0),
                    Color::WHITE.with_alpha(0.22),
                    1.0,
                );
            })
            .on_tap(move || {
                let (h, s, v) = c.to_hsv();
                hue.set(h);
                sat.set(s);
                val.set(v);
                if alpha {
                    opacity.set(c.a);
                }
            })
            .frame(SWATCH, SWATCH)
            .any()
        })
        .collect();
    row(PieceVec(swatches))
        .spacing(6.0)
        .fit(RowFit::Wrap { run_spacing: 6.0 })
        .grow_w()
        .id("color-picker-presets")
        .any()
}

/// The current color, large, over a checkerboard, with its hex beside it.
fn readout(current: impl Fn() -> Color + Copy + 'static) -> AnyPiece {
    row((
        canvas(move |d, size| {
            checkerboard(d, size);
            d.fill(
                Shape::RoundedRect(Rect::new(0.0, 0.0, size.width, size.height), 7.0),
                current(),
            );
        })
        .frame(44.0, 28.0),
        label(move || current().to_hex_string())
            .monospace()
            .color(PANEL_TEXT)
            .id("color-picker-value"),
    ))
    .spacing(10.0)
    .align(VAlign::Center)
    .any()
}

/// The ring that marks the picked point in the shade field: white over a dark halo, so it stays
/// visible against both ends of the field it sits on.
fn marker(d: &mut Draw, at: Point, radius: f64) {
    let ring = |r: f64| Shape::Ellipse(Rect::new(at.x - r, at.y - r, r * 2.0, r * 2.0));
    d.stroke(ring(radius + 1.0), Color::BLACK.with_alpha(0.45), 3.0);
    d.stroke(ring(radius), Color::WHITE, 2.0);
}

/// The thumb on a strip: a full-height capsule, so it reads at 20 points tall.
fn slider_thumb(d: &mut Draw, x: f64, height: f64) {
    let r = Rect::new(x - 4.0, 0.0, 8.0, height);
    d.fill(Shape::RoundedRect(r, 4.0), Color::WHITE);
    d.stroke(
        Shape::RoundedRect(r.inset(0.5), 4.0),
        Color::BLACK.with_alpha(0.4),
        1.0,
    );
}

/// The transparency checkerboard every color tool draws behind a partly transparent swatch.
fn checkerboard(d: &mut Draw, size: Size) {
    const CELL: f64 = 6.0;
    d.fill(
        Shape::Rect(Rect::new(0.0, 0.0, size.width, size.height)),
        Color::rgb(0.86, 0.86, 0.88),
    );
    let cols = (size.width / CELL).ceil() as i64;
    let rows = (size.height / CELL).ceil() as i64;
    for row_i in 0..rows {
        for col in 0..cols {
            if (row_i + col) % 2 == 0 {
                continue;
            }
            let x = col as f64 * CELL;
            let y = row_i as f64 * CELL;
            d.fill(
                Shape::Rect(Rect::new(
                    x,
                    y,
                    CELL.min(size.width - x),
                    CELL.min(size.height - y),
                )),
                Color::rgb(0.66, 0.66, 0.69),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Per-toolkit native renderers — one file per backend, for the six toolkits that HAVE a color
// chooser. Every module registers a `Renderer` into its backend's `RENDERERS` slice link-time;
// `dom` registers at runtime from `color_picker`. android-mdc and harmony-arkui carry no arm at
// all: the composed panel above is their picker.
// ---------------------------------------------------------------------------

day_pieces::glue_modules!(appkit, gtk, qt, uikit, xaml, dom);

// --- Typed builders, forwarded through `Decorated` (docs/api-style.md) ---

/// [`ColorPicker`]'s own builders, reachable THROUGH a decoration (§5.2): `day_pieces::Decorated` forwards them
/// to the piece it wraps, so generic modifiers and typed ones chain in any order.
pub trait ColorPickerBuilder: Sized {
    fn alpha(self, on: bool) -> Self;
    fn idiom(self, idiom: PickerIdiom) -> Self;
    fn native(self) -> Self;
    fn composed(self) -> Self;
    fn title<M>(self, t: impl IntoText<M>) -> Self;
    fn presets(self, presets: Vec<Color>) -> Self;
    fn key(self, key: impl Into<String>) -> Self;
}

impl<C: Binding<Color>> ColorPickerBuilder for ColorPicker<C> {
    fn alpha(self, on: bool) -> Self {
        ColorPicker::alpha(self, on)
    }
    fn idiom(self, idiom: PickerIdiom) -> Self {
        ColorPicker::idiom(self, idiom)
    }
    fn native(self) -> Self {
        ColorPicker::native(self)
    }
    fn composed(self) -> Self {
        ColorPicker::composed(self)
    }
    fn title<M>(self, t: impl IntoText<M>) -> Self {
        ColorPicker::title(self, t)
    }
    fn presets(self, presets: Vec<Color>) -> Self {
        ColorPicker::presets(self, presets)
    }
    fn key(self, key: impl Into<String>) -> Self {
        ColorPicker::key(self, key)
    }
}

impl<Inner: ColorPickerBuilder + day_pieces::prelude::Piece> ColorPickerBuilder
    for day_pieces::Decorated<Inner>
{
    fn alpha(self, on: bool) -> Self {
        self.map_inner(|inner_piece| inner_piece.alpha(on))
    }
    fn idiom(self, idiom: PickerIdiom) -> Self {
        self.map_inner(|inner_piece| inner_piece.idiom(idiom))
    }
    fn native(self) -> Self {
        self.map_inner(|inner_piece| inner_piece.native())
    }
    fn composed(self) -> Self {
        self.map_inner(|inner_piece| inner_piece.composed())
    }
    fn title<M>(self, t: impl IntoText<M>) -> Self {
        self.map_inner(|inner_piece| inner_piece.title(t))
    }
    fn presets(self, presets: Vec<Color>) -> Self {
        self.map_inner(|inner_piece| inner_piece.presets(presets))
    }
    fn key(self, key: impl Into<String>) -> Self {
        self.map_inner(|inner_piece| inner_piece.key(key))
    }
}
