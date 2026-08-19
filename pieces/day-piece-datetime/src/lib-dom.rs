// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// Web (web-dom): `<input type="date">` and `<input type="time">` — the browser's own pickers,
// which on every desktop platform open the same system chooser the AppKit / GTK / Qt / XAML arms
// do, and on a phone browser raise the OS wheel. The least code of the eight arms.
//
// Two web-only truths:
//
// - **The wire format is fixed by HTML.** `input[type=date].value` is always `YYYY-MM-DD` and
//   `input[type=time].value` is `HH:MM` (or `HH:MM:SS` with a seconds step), regardless of the
//   locale the control DISPLAYS. So the parse below is against the ISO form, not a localized one,
//   and `DayDate::parse_iso` is exactly right for it.
// - **`Style` has no analogue.** A browser draws one date control; there is no
//   compact-versus-calendar choice to make, so `Style` is read and ignored the way every other
//   arm ignores what its platform lacks.
//
// Events need no `Custom` payload: `listen::INPUT` reports the element's value as
// `Event::TextChanged`, and the front end parses it through the same `parse_iso` the other arms
// use for their string round-trips.
// ---------------------------------------------------------------------------

use super::*;
use day_dom::{Dom, DomHandle, listen};
use day_spec::{NodeId, Proposal, Size};

fn iso_date(d: DayDate) -> String {
    format!("{:04}-{:02}-{:02}", d.year, d.month, d.day)
}

fn iso_time(t: DayTime) -> String {
    format!("{:02}:{:02}:{:02}", t.hour, t.minute, t.second)
}

fn make_date(backend: &mut Dom, p: &DateProps, _id: NodeId) -> DomHandle {
    let h = backend.element("input");
    backend.set_attr(&h, "type", "date");
    backend.set_attr(&h, "value", &iso_date(p.date));
    // `min`/`max` are the browser's own clamp — it refuses out-of-range values in the picker AND
    // on typed entry, which is the same guarantee `DayDate::clamped` gives the other arms.
    if let Some(min) = p.min {
        backend.set_attr(&h, "min", &iso_date(min));
    }
    if let Some(max) = p.max {
        backend.set_attr(&h, "max", &iso_date(max));
    }
    backend.set_attr(&h, "style", "width:100%;height:100%;box-sizing:border-box");
    backend.listen(&h, listen::INPUT);
    h
}

fn update_date(backend: &mut Dom, h: &DomHandle, patch: &DatePatch) {
    let DatePatch::SetDate(d) = patch;
    // Assigning `value` does not fire `input`, so there is no echo to guard against here.
    backend.set_attr(h, "value", &iso_date(*d));
}

fn make_time(backend: &mut Dom, p: &TimeProps, _id: NodeId) -> DomHandle {
    let h = backend.element("input");
    backend.set_attr(&h, "type", "time");
    // A seconds field appears only when the step admits one — that is how HTML spells it, and it
    // is the same `seconds` flag AppKit and Qt honor with a control element.
    if p.seconds {
        backend.set_attr(&h, "step", "1");
    }
    backend.set_attr(&h, "value", &iso_time(p.time));
    backend.set_attr(&h, "style", "width:100%;height:100%;box-sizing:border-box");
    backend.listen(&h, listen::INPUT);
    h
}

fn update_time(backend: &mut Dom, h: &DomHandle, patch: &TimePatch) {
    let TimePatch::SetTime(t) = patch;
    backend.set_attr(h, "value", &iso_time(*t));
}

/// A fixed-size control the browser draws; take what the layout proposes, with the same modest
/// default the other arms report as their intrinsic size.
fn measure(_backend: &mut Dom, _h: &DomHandle, p: Proposal) -> Size {
    Size::new(p.width.unwrap_or(160.0), p.height.unwrap_or(28.0))
}

// Each `dom_renderer!` defines its own `register()`, so the two arms live in their own modules
// and this one registers both — web-dom's registry is populated at runtime, unlike the link-time
// `renderer!` the other seven arms use.
mod date_arm {
    use super::*;
    day_pieces::dom_renderer!(day_dom::register_renderer, Dom,
        kind: DATE_KIND, props: DateProps, patch: DatePatch,
        make: make_date, update: update_date, measure: measure);
}

mod time_arm {
    use super::*;
    day_pieces::dom_renderer!(day_dom::register_renderer, Dom,
        kind: TIME_KIND, props: TimeProps, patch: TimePatch,
        make: make_time, update: update_time, measure: measure);
}

pub(crate) fn register() {
    date_arm::register();
    time_arm::register();
}
