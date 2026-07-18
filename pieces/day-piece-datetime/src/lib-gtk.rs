// ---------------------------------------------------------------------------
// GTK: COMPOSED from native primitives — GTK4/libadwaita have no stock date or time picker
// (support() reports Emulated). Compact date = GtkMenuButton (label = the locale-formatted date,
// via g_date_time_format "%x") opening a GtkCalendar in a GtkPopover — the GNOME-idiom popover
// chooser. Inline date = GtkCalendar. Time = linked GtkSpinButtons (h/m[/s]). GtkCalendar has no
// min/max, so bounds ride the piece's own clamp: an out-of-range pick clamps in the signal and the
// resulting patch snaps the calendar back. Echo-guarded like the picker's GTK renderer.
// ---------------------------------------------------------------------------

use super::*;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use day_gtk::Gtk;
use day_spec::{NodeId, Proposal, Size};
use gtk4::prelude::*;

struct DateState {
    calendar: gtk4::Calendar,
    button: Option<gtk4::MenuButton>,
    suppress: Rc<Cell<bool>>,
}

struct TimeState {
    hour: gtk4::SpinButton,
    minute: gtk4::SpinButton,
    second: Option<gtk4::SpinButton>,
    suppress: Rc<Cell<bool>>,
}

thread_local! {
    static DATES: RefCell<HashMap<usize, DateState>> = RefCell::new(HashMap::new());
    static TIMES: RefCell<HashMap<usize, TimeState>> = RefCell::new(HashMap::new());
}

fn key(w: &gtk4::Widget) -> usize {
    w.as_ptr() as usize
}

fn glib_date(d: DayDate) -> Option<gtk4::glib::DateTime> {
    gtk4::glib::DateTime::from_local(d.year, d.month as i32, d.day as i32, 0, 0, 0.0).ok()
}

fn calendar_date(c: &gtk4::Calendar) -> Option<DayDate> {
    let dt = c.date();
    DayDate::new(dt.year(), dt.month() as u8, dt.day_of_month() as u8)
}

/// The compact button's label: the locale's date format (g_date_time_format "%x"), ISO fallback.
fn date_label(d: DayDate) -> String {
    glib_date(d)
        .and_then(|dt| dt.format("%x").ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| d.to_string())
}

fn select_suppressed(c: &gtk4::Calendar, suppress: &Rc<Cell<bool>>, d: DayDate) {
    if let Some(dt) = glib_date(d) {
        suppress.set(true);
        c.select_day(&dt);
        suppress.set(false);
    }
}

fn measure_widget(h: &gtk4::Widget) -> Size {
    let (_, nat_w, _, _) = h.measure(gtk4::Orientation::Horizontal, -1);
    let (_, nat_h, _, _) = h.measure(gtk4::Orientation::Vertical, -1);
    Size::new((nat_w as f64).max(60.0), (nat_h as f64).max(22.0))
}

mod date_renderer {
    use super::*;

    fn make(_backend: &mut Gtk, p: &DateProps, id: NodeId) -> gtk4::Widget {
        let suppress = Rc::new(Cell::new(false));
        let calendar = gtk4::Calendar::new();
        select_suppressed(&calendar, &suppress, p.date);
        {
            let suppress = suppress.clone();
            calendar.connect_day_selected(move |c| {
                if suppress.get() {
                    return;
                }
                if let Some(d) = calendar_date(c) {
                    day_gtk::emit(id, Event::custom("datepicker:value", d.to_string()));
                }
            });
        }
        let (root, button) = if p.style == Style::Inline {
            (calendar.clone().upcast::<gtk4::Widget>(), None)
        } else {
            let btn = gtk4::MenuButton::new();
            btn.set_label(&date_label(p.date));
            let pop = gtk4::Popover::new();
            pop.set_child(Some(&calendar));
            btn.set_popover(Some(&pop));
            (btn.clone().upcast::<gtk4::Widget>(), Some(btn))
        };
        // Keep the compact label in sync with USER picks too (patches only cover signal changes
        // that differ — a pick echoes through the signal and patches back idempotently).
        if let Some(btn) = &button {
            let btn = btn.clone();
            calendar.connect_day_selected(move |c| {
                if let Some(d) = calendar_date(c) {
                    btn.set_label(&date_label(d));
                }
            });
        }
        DATES.with(|m| {
            m.borrow_mut().insert(
                key(&root),
                DateState {
                    calendar,
                    button,
                    suppress,
                },
            )
        });
        root
    }

    fn update(_backend: &mut Gtk, h: &gtk4::Widget, patch: &DatePatch) {
        let DatePatch::SetDate(d) = patch;
        DATES.with(|m| {
            let m = m.borrow();
            let Some(st) = m.get(&key(h)) else {
                return;
            };
            if calendar_date(&st.calendar) != Some(*d) {
                select_suppressed(&st.calendar, &st.suppress, *d);
            }
            if let Some(btn) = &st.button {
                btn.set_label(&date_label(*d));
            }
        });
    }

    fn measure(_backend: &mut Gtk, h: &gtk4::Widget, _p: Proposal) -> Size {
        measure_widget(h)
    }

    day_pieces::renderer!(day_gtk::RENDERERS, Gtk,
        kind: DATE_KIND, props: DateProps, patch: DatePatch,
        make: make, update: update, measure: measure);
}

mod time_renderer {
    use super::*;

    fn spin(upper: f64, value: f64) -> gtk4::SpinButton {
        let s = gtk4::SpinButton::with_range(0.0, upper, 1.0);
        s.set_wrap(true);
        s.set_orientation(gtk4::Orientation::Vertical);
        s.set_value(value);
        s
    }

    fn current_secs(st: &TimeState) -> i64 {
        st.hour.value() as i64 * 3600
            + st.minute.value() as i64 * 60
            + st.second.as_ref().map_or(0, |s| s.value() as i64)
    }

    fn make(_backend: &mut Gtk, p: &TimeProps, id: NodeId) -> gtk4::Widget {
        // Both styles compose the same linked spin buttons — GNOME's own time-setting idiom
        // (GTK has no clock widget at all); `Inline` vs `Compact` only matters where a real
        // embedded chooser exists.
        let suppress = Rc::new(Cell::new(false));
        let bx = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        bx.add_css_class("linked");
        bx.set_halign(gtk4::Align::Start);
        let t = p.time;
        let hour = spin(23.0, t.hour as f64);
        let minute = spin(59.0, t.minute as f64);
        let second = p.seconds.then(|| spin(59.0, t.second as f64));
        bx.append(&hour);
        bx.append(&gtk4::Label::new(Some(":")));
        bx.append(&minute);
        if let Some(s) = &second {
            bx.append(&gtk4::Label::new(Some(":")));
            bx.append(s);
        }
        let root = bx.upcast::<gtk4::Widget>();
        let state = TimeState {
            hour,
            minute,
            second,
            suppress: suppress.clone(),
        };
        let k = key(&root);
        for s in [
            Some(&state.hour),
            Some(&state.minute),
            state.second.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            let suppress = suppress.clone();
            s.connect_value_changed(move |_| {
                if suppress.get() {
                    return;
                }
                let secs = TIMES.with(|m| m.borrow().get(&k).map(current_secs));
                if let Some(secs) = secs {
                    let t = DayTime::from_seconds_of_day(secs);
                    day_gtk::emit(id, Event::custom("timepicker:value", t.to_string()));
                }
            });
        }
        TIMES.with(|m| m.borrow_mut().insert(k, state));
        root
    }

    fn update(_backend: &mut Gtk, h: &gtk4::Widget, patch: &TimePatch) {
        let TimePatch::SetTime(t) = patch;
        TIMES.with(|m| {
            let m = m.borrow();
            let Some(st) = m.get(&key(h)) else {
                return;
            };
            st.suppress.set(true);
            st.hour.set_value(t.hour as f64);
            st.minute.set_value(t.minute as f64);
            if let Some(s) = &st.second {
                s.set_value(t.second as f64);
            }
            st.suppress.set(false);
        });
    }

    fn measure(_backend: &mut Gtk, h: &gtk4::Widget, _p: Proposal) -> Size {
        measure_widget(h)
    }

    day_pieces::renderer!(day_gtk::RENDERERS, Gtk,
        kind: TIME_KIND, props: TimeProps, patch: TimePatch,
        make: make, update: update, measure: measure);
}
