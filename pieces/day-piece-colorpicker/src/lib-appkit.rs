// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// AppKit: `NSColorWell` — the swatch control macOS has for exactly this — wired to the shared
// `NSColorPanel`, which is the full system chooser (wheel, sliders, palettes, image spectrum,
// crayons, and the screen eyedropper).
//
// Three AppKit facts this arm has to live with:
//
// - **The panel is a singleton.** `NSColorPanel.sharedColorPanel` is one per process, so
//   `showsAlpha` is a process-wide setting rather than a per-well one. A well that wants alpha
//   turns it on; one that does not leaves it alone rather than turning it off, because clearing it
//   would silently disable the alpha slider for some *other* well that asked for it. The piece's
//   own front-end is what actually enforces opacity (it drops a non-opaque pick when
//   `.alpha(false)`), so the panel setting is a hint, not the guarantee.
// - **`NSColorWell.supportsAlpha` is macOS 14.** Day's floor is 13, so it is called only behind a
//   `respondsToSelector:` probe and the panel setting carries the older systems.
// - **An `NSColor` is not necessarily RGB.** A pick from the CMYK tab, the gray tab or a named
//   system color has no red component at all, and `-redComponent` on one raises. Every read goes
//   through `colorUsingColorSpace:sRGBColorSpace` first, which is also what pins the numbers to
//   sRGB rather than to whatever space the display is in. Colors that cannot convert (a pattern
//   color — an IMAGE, which AppKit lets the user drag into a well) are dropped: there is nothing
//   in Day's `Color` to put one in. docs/color.md carries that gap.
// ---------------------------------------------------------------------------

use super::*;
use std::cell::RefCell;
use std::collections::HashMap;

use day_appkit::AppKit;
use day_spec::{NodeId, Proposal, Size};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSColor, NSColorPanel, NSColorSpace, NSColorWell, NSColorWellStyle, NSView};
use objc2_foundation::NSObject;

struct TargetIvars {
    node: NodeId,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DayColorWellTarget"]
    #[ivars = TargetIvars]
    struct WellTarget;

    unsafe impl NSObjectProtocol for WellTarget {}

    impl WellTarget {
        #[unsafe(method(fire:))]
        fn fire(&self, sender: &AnyObject) {
            let Some(well) = sender.downcast_ref::<NSColorWell>() else {
                return;
            };
            if let Some(c) = to_day_color(&well.color()) {
                day_appkit::emit(self.ivars().node, Event::custom(PICK_TAG, c.to_string()));
            }
        }
    }
);

impl WellTarget {
    fn new(mtm: MainThreadMarker, node: NodeId) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TargetIvars { node });
        unsafe { msg_send![super(this), init] }
    }
}

thread_local! {
    static TARGETS: RefCell<HashMap<usize, Retained<WellTarget>>> = RefCell::new(HashMap::new());
}

/// An `NSColor` in whatever space the user picked in, read as sRGB. `None` for a color with no
/// component representation at all (a pattern/image color).
fn to_day_color(c: &NSColor) -> Option<Color> {
    let srgb_space = NSColorSpace::sRGBColorSpace();
    let srgb = c.colorUsingColorSpace(&srgb_space)?;
    Some(Color::rgba(
        srgb.redComponent(),
        srgb.greenComponent(),
        srgb.blueComponent(),
        srgb.alphaComponent(),
    ))
}

fn to_ns_color(c: Color) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(c.r, c.g, c.b, c.a)
}

fn make(backend: &mut AppKit, p: &ColorProps, id: NodeId) -> Retained<NSView> {
    let mtm = backend.mtm();
    // `Default` rather than `Minimal`: the bordered well is the one that reads as a control at a
    // glance next to a button, and it is the style that accepts a dragged color.
    let well = NSColorWell::colorWellWithStyle(NSColorWellStyle::Default, mtm);
    well.setColor(&to_ns_color(p.color));
    if p.alpha {
        // macOS 14+ per-well switch; the panel setting below is what carries macOS 13.
        if well.respondsToSelector(sel!(setSupportsAlpha:)) {
            well.setSupportsAlpha(true);
        }
        // Never `setShowsAlpha(false)` in the `else` arm: the panel is process-wide and another
        // well may have asked for alpha (see the header).
        NSColorPanel::sharedColorPanel(mtm).setShowsAlpha(true);
    }
    if !p.title.is_empty() {
        NSColorPanel::sharedColorPanel(mtm).setTitle(&objc2_foundation::NSString::from_str(&p.title));
    }
    let target = WellTarget::new(mtm, id);
    unsafe {
        well.setTarget(Some(&target));
        well.setAction(Some(sel!(fire:)));
    }
    let view: Retained<NSView> = Retained::from(<NSColorWell as AsRef<NSView>>::as_ref(&well));
    TARGETS.with(|m| {
        m.borrow_mut()
            .insert((view.as_ref() as *const NSView) as usize, target)
    });
    view
}

fn update(_backend: &mut AppKit, h: &Retained<NSView>, patch: &ColorPatch) {
    let ColorPatch::SetColor(c) = patch;
    let Some(well) = h.downcast_ref::<NSColorWell>() else {
        return;
    };
    // No-op on an unchanged value: `setColor:` fires the well's action, so writing back the color
    // that just arrived FROM the well would round-trip forever.
    if to_day_color(&well.color()) != Some(*c) {
        well.setColor(&to_ns_color(*c));
    }
}

/// A color well has no text, so its `fittingSize` is the bare swatch. The floor keeps it a
/// pressable target where a layout proposes nothing.
fn measure(_backend: &mut AppKit, h: &Retained<NSView>, _p: Proposal) -> Size {
    let s = h.fittingSize();
    Size::new(s.width.ceil().max(44.0), s.height.ceil().max(24.0))
}

/// Drop the retained target when the well goes away.
///
/// Without this the map grows by one entry per realized well, and — worse — its key is the view's
/// ADDRESS, which the allocator reuses: a later view landing on a freed address would inherit the
/// dead node's target and report picks against a node that no longer exists.
fn release(_backend: &mut AppKit, h: &Retained<NSView>) {
    TARGETS.with(|m| {
        m.borrow_mut()
            .remove(&((h.as_ref() as *const NSView) as usize));
    });
}

day_pieces::renderer!(day_appkit::RENDERERS, AppKit,
    kind: KIND, props: ColorProps, patch: ColorPatch,
    make: make, update: update, measure: measure, release: release);
