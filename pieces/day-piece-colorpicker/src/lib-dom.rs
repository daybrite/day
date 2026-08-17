// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// Web (web-dom): `<input type="color">` — the browser's own color picker, which on every desktop
// platform IS the system chooser the AppKit / GTK / Qt / XAML arms open. The least code of the
// eight arms, and the most native result.
//
// Two web-only truths, both in docs/colorpicker.md:
//
// - **Alpha is new.** The `alpha` attribute (and the `colorspace` one beside it) reached the HTML
//   spec only recently, and browsers are still shipping it. The attribute is set when the piece
//   asked for alpha; a browser that has not implemented it ignores it and the control stays
//   opaque, which is the same degradation an app gets from any not-yet-shipped attribute. There
//   is no reliable feature test that predicts the *UI* (a browser can accept the property and
//   still draw an opaque-only picker), so this arm sets it and lets the value speak: an opaque
//   pick simply arrives with `a = 1`.
// - **The value is 8-bit.** `input.value` is always `#rrggbb`, so a pick made on the web comes
//   back quantized even though Day's `Color` is float. Nothing here can widen that.
//
// Events do not need a `Custom` payload: `listen::INPUT` reports the element's value as
// `Event::TextChanged`, and the piece's front-end parses hex and components through the same
// `Color::parse`. That is the whole event wiring.
// ---------------------------------------------------------------------------

use super::*;
use day_dom::{Dom, DomHandle, listen};
use day_spec::{NodeId, Proposal, Size};

fn make(backend: &mut Dom, p: &ColorProps, _id: NodeId) -> DomHandle {
    let h = backend.element("input");
    backend.set_attr(&h, "type", "color");
    backend.set_attr(&h, "value", &opaque_hex(p.color));
    if p.alpha {
        // day-dom's boolean-attribute convention: `"-"` sets, `""` removes.
        backend.set_attr(&h, "alpha", "-");
    }
    if !p.title.is_empty() {
        backend.set_attr(&h, "title", &p.title);
    }
    backend.set_attr(&h, "style", "width:100%;height:100%;padding:0;border:0");
    // `input` fires as the user drags inside the picker, `change` when they commit. Listening to
    // both would report the same value twice; `input` is the one that keeps a bound tint live.
    backend.listen(&h, listen::INPUT);
    h
}

fn update(backend: &mut Dom, h: &DomHandle, patch: &ColorPatch) {
    let ColorPatch::SetColor(c) = patch;
    // Assigning `value` does not fire `input`, so there is no echo to guard against here.
    backend.set_attr(h, "value", &opaque_hex(*c));
}

/// `<input type="color">` only ever accepts `#rrggbb` as its `value` — an 8-digit form is invalid
/// and the browser resets the control to black. Alpha rides the separate `alpha` attribute (and
/// comes back in the reported value where the browser supports it).
fn opaque_hex(c: Color) -> String {
    c.with_alpha(1.0).to_hex_string()
}

/// The swatch is a fixed-size control the browser draws; take what the layout proposes, with the
/// same modest default the other arms report as their intrinsic size.
fn measure(_backend: &mut Dom, _h: &DomHandle, p: Proposal) -> Size {
    Size::new(p.width.unwrap_or(56.0), p.height.unwrap_or(28.0))
}

// Defines `register()`, which `color_picker()` calls — web-dom's registry is populated at runtime,
// unlike the link-time `renderer!` the other seven arms use.
day_pieces::dom_renderer!(day_dom::register_renderer, Dom,
    kind: KIND, props: ColorProps, patch: ColorPatch,
    make: make, update: update, measure: measure);
