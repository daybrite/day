// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
//! Inks on AppKit (docs/color.md "How a toolkit follows the appearance").
//!
//! Every ink becomes one `NSColor` that AppKit resolves for the appearance it is drawn in: a
//! fixed ink an sRGB color, a role the system's own dynamic color, and a light/dark pair (or a
//! role with its alpha replaced, which `colorWithAlphaComponent:` would freeze) a color made
//! with `colorWithName:dynamicProvider:`. Labels, attributed runs, decorations, image and button
//! tints and the surface fills `drawRect` paints take that color and follow an appearance
//! switch with nothing re-applied.
//!
//! Colors handed to a `CALayer` are `CGColor`s, which hold one appearance. The views that own
//! such a layer color re-set it when their effective appearance changes: a surface's border
//! and shadow and a cover's page from `viewDidChangeEffectiveAppearance`, a tinted button
//! (a plain `NSButton`) from [`refresh_layers`], which every appearance change calls.
use super::*;
use day_spec::{Color, Ink, SystemColor};
use objc2::rc::Weak;
use objc2_app_kit::NSAppearance;
use std::ptr::NonNull;

// ---------------------------------------------------------------------------
// Pure logic (unit-tested below)
// ---------------------------------------------------------------------------

/// Whether an appearance name is a dark one, given AppKit's best match for it between aqua and
/// dark aqua (`bestMatchFromAppearancesWithNames:`): high contrast and vibrant variants match
/// the side they belong to, so only the name of the match is compared.
pub(super) fn is_dark_match(best_match: Option<&str>) -> bool {
    best_match.is_some_and(|n| n.contains("DarkAqua"))
}

/// What a light/dark pair, or a role's two sides, map to under `f`: a pair stays a pair. The
/// readable title color of a tinted button is `on_tint` of each side.
pub(super) fn map_pair(pair: (Color, Color), f: impl Fn(Color) -> Color) -> Ink {
    let (light, dark) = (f(pair.0), f(pair.1));
    if light == dark {
        Ink::Fixed(light)
    } else {
        Ink::adaptive(light, dark)
    }
}

// ---------------------------------------------------------------------------
// Roles
// ---------------------------------------------------------------------------

/// The AppKit color of a role (docs/color.md "Roles"), a system color AppKit resolves for the
/// appearance it is drawn in. `GroupedBackground` is the window background, as System Settings
/// draws its grouped pages; `ElevatedSurface` is `underPageBackgroundColor`, the slightly
/// lighter ground AppKit puts behind raised content in dark mode.
pub(super) fn role_color(role: SystemColor) -> Retained<NSColor> {
    match role {
        SystemColor::Accent => NSColor::controlAccentColor(),
        SystemColor::Text => NSColor::labelColor(),
        SystemColor::SecondaryText => NSColor::secondaryLabelColor(),
        SystemColor::TertiaryText => NSColor::tertiaryLabelColor(),
        SystemColor::Placeholder => NSColor::placeholderTextColor(),
        SystemColor::Link => NSColor::linkColor(),
        SystemColor::Separator => NSColor::separatorColor(),
        SystemColor::ControlFill => {
            // The system fills are macOS 14+; `controlColor` before that.
            if <NSColor as objc2::ClassType>::class()
                .responds_to(objc2::sel!(quaternarySystemFillColor))
            {
                NSColor::quaternarySystemFillColor()
            } else {
                NSColor::controlColor()
            }
        }
        SystemColor::Background | SystemColor::GroupedBackground => {
            NSColor::windowBackgroundColor()
        }
        SystemColor::Surface => NSColor::controlBackgroundColor(),
        SystemColor::ElevatedSurface => NSColor::underPageBackgroundColor(),
        SystemColor::Selection => NSColor::selectedTextBackgroundColor(),
        SystemColor::Destructive => NSColor::systemRedColor(),
        SystemColor::Success => NSColor::systemGreenColor(),
        SystemColor::Warning => NSColor::systemOrangeColor(),
    }
}

// ---------------------------------------------------------------------------
// Inks as native colors
// ---------------------------------------------------------------------------

/// Aqua or dark aqua, whichever `appearance` is closest to. Day's inks have two sides, and the
/// surfaces `drawRect` paints resolve under the standard appearance rather than a vibrant one
/// (see `DayFlipped`'s `drawRect:`).
pub(super) fn standard_appearance(appearance: &NSAppearance) -> Option<Retained<NSAppearance>> {
    let names = objc2_foundation::NSArray::from_slice(&[
        // SAFETY: AppKit's constant appearance names.
        unsafe { objc2_app_kit::NSAppearanceNameAqua },
        unsafe { objc2_app_kit::NSAppearanceNameDarkAqua },
    ]);
    appearance
        .bestMatchFromAppearancesWithNames(&names)
        .and_then(|n| NSAppearance::appearanceNamed(&n))
}

/// Whether `appearance` is a dark one.
fn is_dark(appearance: &NSAppearance) -> bool {
    let names = objc2_foundation::NSArray::from_slice(&[
        // SAFETY: AppKit's constant appearance names.
        unsafe { objc2_app_kit::NSAppearanceNameAqua },
        unsafe { objc2_app_kit::NSAppearanceNameDarkAqua },
    ]);
    let best = appearance.bestMatchFromAppearancesWithNames(&names);
    is_dark_match(best.map(|n| n.to_string()).as_deref())
}

/// The `NSColor` of an ink, resolved by AppKit wherever it is drawn: an sRGB color for a fixed
/// ink, the system color for a role, and a dynamic color for a pair or a role at another
/// alpha.
///
/// Public for pieces with a native view of their own (an attributed string's foreground, a
/// text view's colors): a color from here follows a light/dark switch and a per-window
/// appearance with nothing re-applied.
#[must_use]
pub fn nsink(ink: Ink) -> Retained<NSColor> {
    match ink {
        Ink::Fixed(c) => nscolor(c),
        Ink::System { role, alpha: None } => role_color(role),
        Ink::Adaptive { light, dark } => {
            let (light, dark) = (nscolor(light), nscolor(dark));
            let provider = block2::RcBlock::new(move |appearance: NonNull<NSAppearance>| {
                // SAFETY: AppKit passes a valid appearance for the length of the call.
                let appearance = unsafe { appearance.as_ref() };
                let pick = if is_dark(appearance) { &dark } else { &light };
                // The block keeps both colors alive, so a +0 pointer to one is valid for as
                // long as the dynamic color can call it.
                NonNull::from(&**pick)
            });
            // SAFETY: the provider returns a valid color for every appearance.
            unsafe { NSColor::colorWithName_dynamicProvider(None, &provider) }
        }
        Ink::System {
            role,
            alpha: Some(alpha),
        } => {
            // `colorWithAlphaComponent:` on a system color keeps only the appearance it was
            // made in, so the alpha is applied per appearance, to the role resolved in it.
            let provider = block2::RcBlock::new(move |appearance: NonNull<NSAppearance>| {
                // SAFETY: as above.
                let appearance = unsafe { appearance.as_ref() };
                let c = color_in(appearance, Ink::system(role)).with_alpha(alpha);
                // Returned autoreleased, the block return convention for an object.
                let ptr = Retained::autorelease_return(nscolor(c));
                // SAFETY: the pointer of a `Retained`, which is never null.
                unsafe { NonNull::new_unchecked(ptr) }
            });
            // SAFETY: the provider returns a valid color for every appearance.
            unsafe { NSColor::colorWithName_dynamicProvider(None, &provider) }
        }
    }
}

/// An `NSColor` in sRGB, as a Day color; `None` for a pattern or another color with no sRGB
/// form.
pub(super) fn srgb(color: &NSColor) -> Option<Color> {
    let c = color.colorUsingColorSpace(&objc2_app_kit::NSColorSpace::sRGBColorSpace())?;
    Some(Color::rgba(
        c.redComponent(),
        c.greenComponent(),
        c.blueComponent(),
        c.alphaComponent(),
    ))
}

/// The color an ink shows under the current drawing appearance: inside `drawRect:` that is the
/// view's own, inside `performAsCurrentDrawingAppearance` the one performed as.
pub(super) fn ink_now(ink: Ink) -> Color {
    match ink {
        Ink::Fixed(c) => c,
        _ => srgb(&nsink(ink)).unwrap_or_else(|| ink.resolved()),
    }
}

/// The color an ink shows in `appearance` (its standard counterpart).
pub(super) fn color_in(appearance: &NSAppearance, ink: Ink) -> Color {
    if let Ink::Fixed(c) = ink {
        return c;
    }
    let out = Cell::new(ink.resolved());
    let block = block2::StackBlock::new(|| out.set(ink_now(ink)));
    match standard_appearance(appearance) {
        Some(standard) => standard.performAsCurrentDrawingAppearance(&block),
        None => appearance.performAsCurrentDrawingAppearance(&block),
    }
    out.get()
}

/// The color an ink shows in `view`, for a property that takes a concrete color (a layer's).
pub(super) fn color_for(view: &NSView, ink: Ink) -> Color {
    match ink {
        Ink::Fixed(c) => c,
        _ => color_in(&view.effectiveAppearance(), ink),
    }
}

/// A `CGColor` of an ink as `view` shows it now; set again when the view's appearance changes.
pub(super) fn cg_for(view: &NSView, ink: Ink) -> Retained<objc2_core_graphics::CGColor> {
    nscolor(color_for(view, ink)).CGColor()
}

/// The ink's light and dark sides, roles resolved natively in aqua and dark aqua.
fn pair_now(ink: Ink) -> (Color, Color) {
    match ink {
        Ink::Fixed(c) => (c, c),
        Ink::Adaptive { light, dark } => (light, dark),
        Ink::System { .. } => {
            let side = |name: &objc2_app_kit::NSAppearanceName| {
                NSAppearance::appearanceNamed(name)
                    .map_or_else(|| ink.resolved(), |a| color_in(&a, ink))
            };
            // SAFETY: AppKit's constant appearance names.
            let (aqua, dark) = unsafe {
                (
                    objc2_app_kit::NSAppearanceNameAqua,
                    objc2_app_kit::NSAppearanceNameDarkAqua,
                )
            };
            (side(aqua), side(dark))
        }
    }
}

/// `f` applied to the ink's color on each side: a pair that follows the appearance (a role is
/// read for both sides now, so a later accent change does not reach it).
pub(super) fn map_ink(ink: Ink, f: impl Fn(Color) -> Color) -> Ink {
    match ink {
        Ink::Fixed(c) => Ink::Fixed(f(c)),
        _ => map_pair(pair_now(ink), f),
    }
}

// ---------------------------------------------------------------------------
// Layer colors that follow the appearance
// ---------------------------------------------------------------------------

thread_local! {
    /// Tinted buttons whose layer carries the tint (`apply_button_style`): plain `NSButton`s,
    /// so nothing of theirs hears an appearance change, and [`refresh_layers`] re-sets them.
    static TINTED: RefCell<Vec<Weak<NSButton>>> = const { RefCell::new(Vec::new()) };
}

/// Remember a tinted button whose ink follows the appearance.
pub(super) fn track_tinted(btn: &NSButton) {
    TINTED.with(|t| {
        let mut t = t.borrow_mut();
        t.retain(|w| {
            w.load()
                .is_some_and(|b| !std::ptr::eq(Retained::as_ptr(&b), std::ptr::from_ref(btn)))
        });
        t.push(Weak::new(btn));
    });
}

/// Re-set the layer colors no view of Day's own re-sets: called on every appearance change
/// (`set_appearance`, a window's appearance, the system's switch).
pub(super) fn refresh_layers() {
    let live: Vec<Retained<NSButton>> = TINTED.with(|t| {
        let mut t = t.borrow_mut();
        t.retain(|w| w.load().is_some());
        t.iter().filter_map(Weak::load).collect()
    });
    for btn in live {
        let style = BUTTON_STYLES.with(|m| m.borrow().get(&ptr_of(&btn)).copied());
        if let Some(day_spec::props::ButtonStyleSpec::Tinted(ink)) = style
            && let Some(layer) = btn.layer()
        {
            layer.setBackgroundColor(Some(&cg_for(&btn, ink)));
        }
    }
}

/// The same, on the next main-queue turn as well: AppKit settles a view's effective
/// appearance after the change that caused it.
pub(super) fn refresh_layers_soon() {
    refresh_layers();
    dispatch2::DispatchQueue::main().exec_async(|| ffi_guard::contain((), refresh_layers));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_matches_by_the_best_match_name() {
        assert!(is_dark_match(Some("NSAppearanceNameDarkAqua")));
        assert!(!is_dark_match(Some("NSAppearanceNameAqua")));
        assert!(!is_dark_match(None));
    }

    #[test]
    fn a_mapped_pair_stays_a_pair_unless_both_sides_agree() {
        let white = Color::rgb(1.0, 1.0, 1.0);
        let black = Color::rgb(0.0, 0.0, 0.0);
        let on = |c: Color| {
            if c.r > 0.5 { black } else { white }
        };
        assert_eq!(map_pair((white, black), on), Ink::adaptive(black, white));
        assert_eq!(map_pair((black, black), on), Ink::Fixed(white));
    }
}
