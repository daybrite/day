//! day-gtk — the GTK 4 backend (linux-gtk / macos-gtk; DESIGN.md §9). gtk4-rs, pure Rust.
//!
//! `Handle = gtk4::Widget` (GObject-refcounted, `!Send`). Containers are `GtkFixed`; day's
//! layout positions children via `fixed.move_()` + `set_size_request` (hop's proven pattern).
//! Native signals connect once at realize, capturing the NodeId and emitting into the day sink.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gtk4::prelude::*;
use linkme::distributed_slice;

use day_spec::props::*;
use day_spec::{
    A11yProps, AnimSpec, Cap, DrawOp, Event, EventSink, Font, NodeId, PieceKind, Platform,
    Proposal, Rect, Registry, Renderer, Size, Support, Toolkit, kinds,
};

pub type Handle = gtk4::Widget;

/// The day-core event sink (node-id keyed).
type Sink = Rc<dyn Fn(NodeId, Event)>;

thread_local! {
    static SINK: RefCell<Option<Sink>> = const { RefCell::new(None) };
    static OPS: RefCell<HashMap<usize, Vec<DrawOp>>> = RefCell::new(HashMap::new());
}

fn cairo_set_color(cr: &gtk4::cairo::Context, bits: f64) {
    let v = bits as u32;
    cr.set_source_rgba(
        ((v >> 16) & 0xff) as f64 / 255.0,
        ((v >> 8) & 0xff) as f64 / 255.0,
        (v & 0xff) as f64 / 255.0,
        ((v >> 24) & 0xff) as f64 / 255.0,
    );
}

fn cairo_draw(cr: &gtk4::cairo::Context, ops: &[DrawOp]) {
    let (nums, texts) = day_spec::encode_ops(ops);
    let mut ti = 0;
    for chunk in nums.chunks(9) {
        let (k, a, b, c, d, e, f, g, col) = (
            chunk[0] as i32,
            chunk[1],
            chunk[2],
            chunk[3],
            chunk[4],
            chunk[5],
            chunk[6],
            chunk[7],
            chunk[8],
        );
        cairo_set_color(cr, col);
        match k {
            0 | 1 => {
                cr.rectangle(a, b, c, d);
                if k == 0 {
                    let _ = cr.fill();
                } else {
                    cr.set_line_width(g);
                    let _ = cr.stroke();
                }
            }
            2 => {
                cr.rectangle(a, b, c, d); // rounded post-MVP refinement
                let _ = cr.fill();
            }
            3 | 4 => {
                cr.save().ok();
                cr.translate(a + c / 2.0, b + d / 2.0);
                cr.scale(c / 2.0, d / 2.0);
                cr.arc(0.0, 0.0, 1.0, 0.0, std::f64::consts::TAU);
                cr.restore().ok();
                if k == 3 {
                    let _ = cr.fill();
                } else {
                    cr.set_line_width(g);
                    let _ = cr.stroke();
                }
            }
            5 => {
                let (cx_, cy) = (a + c / 2.0, b + d / 2.0);
                let radius = c.min(d) / 2.0;
                let start = e.to_radians();
                let end = (e + f).to_radians();
                cr.set_line_width(g);
                cr.set_line_cap(gtk4::cairo::LineCap::Round);
                cr.arc(cx_, cy, radius, start, end);
                let _ = cr.stroke();
            }
            6 => {
                cr.set_line_width(g);
                cr.move_to(a, b);
                cr.line_to(c, d);
                let _ = cr.stroke();
            }
            7 => {
                let text = texts.get(ti).cloned().unwrap_or_default();
                ti += 1;
                cr.set_font_size(e);
                let (mut x, mut y) = (a, b);
                if f > 0.5
                    && let Ok(ext) = cr.text_extents(&text)
                {
                    x -= ext.width() / 2.0;
                    y += ext.height() / 2.0;
                }
                cr.move_to(x, y);
                let _ = cr.show_text(&text); // toy API; PangoCairo refinement is a TODO (§11)
            }
            _ => {}
        }
    }
}

/// Emit an event into day-core's queue (public for external Day Piece renderers).
pub fn emit(id: NodeId, ev: Event) {
    let sink = SINK.with(|s| s.borrow().clone());
    if let Some(sink) = sink {
        sink(id, ev);
    }
}

/// Renderers registered by external Day Piece crates (§8.2).
#[distributed_slice]
pub static RENDERERS: [fn() -> Renderer<Gtk>];

pub struct Gtk {
    registry: Registry<Gtk>,
    window_fixed: Option<gtk4::Fixed>,
}

impl Gtk {
    pub fn new() -> Self {
        let mut registry = Registry::default();
        for f in RENDERERS {
            registry.register(f());
        }
        Gtk {
            registry,
            window_fixed: None,
        }
    }
}

impl Default for Gtk {
    fn default() -> Self {
        Self::new()
    }
}

fn apply_font(label: &gtk4::Label, font: Font) {
    let (size_pt, bold) = match font {
        Font::Title => (24.0, true),
        Font::Headline => (15.0, true),
        Font::Body => (13.0, false),
        Font::Caption => (11.0, false),
        Font::System(pt) => (pt, false),
    };
    let weight = if bold { "bold" } else { "normal" };
    // Pango attributes via markup-free CSS is heavyweight; use attributes list.
    let attrs = gtk4::pango::AttrList::new();
    let mut size = gtk4::pango::AttrSize::new((size_pt * gtk4::pango::SCALE as f64) as i32);
    size.set_start_index(0);
    attrs.insert(size);
    if bold {
        let mut w = gtk4::pango::AttrInt::new_weight(gtk4::pango::Weight::Bold);
        w.set_start_index(0);
        attrs.insert(w);
    }
    let _ = weight;
    label.set_attributes(Some(&attrs));
}

/// If `parent` is a scrolled window, children go into its content fixed. NOTE: GTK4 auto-wraps
/// non-scrollable children in a GtkViewport, so `sw.child()` is the viewport, not our Fixed.
fn content_of(parent: &Handle) -> Handle {
    if let Some(sw) = parent.downcast_ref::<gtk4::ScrolledWindow>()
        && let Some(child) = sw.child()
    {
        if let Some(vp) = child.downcast_ref::<gtk4::Viewport>()
            && let Some(inner) = vp.child()
        {
            return inner;
        }
        return child;
    }
    parent.clone()
}

impl Toolkit for Gtk {
    type Handle = Handle;

    fn capability(&self, cap: Cap) -> Support {
        match cap {
            Cap::Snapshot => Support::Native,
            _ => Support::Unsupported,
        }
    }

    fn realize(&mut self, kind: PieceKind, props: &dyn std::any::Any, id: NodeId) -> Handle {
        match kind {
            kinds::CONTAINER => gtk4::Fixed::new().upcast(),
            kinds::SCROLL => {
                let sw = gtk4::ScrolledWindow::new();
                sw.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
                sw.set_child(Some(&gtk4::Fixed::new()));
                sw.upcast()
            }
            kinds::LABEL => {
                let p = props.downcast_ref::<LabelProps>().unwrap();
                let label = gtk4::Label::new(Some(&p.text));
                label.set_xalign(0.0);
                label.set_yalign(0.0);
                label.set_wrap(true);
                label.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
                apply_font(&label, p.font);
                label.upcast()
            }
            kinds::BUTTON => {
                let p = props.downcast_ref::<ButtonProps>().unwrap();
                let btn = gtk4::Button::with_label(&p.title);
                btn.connect_clicked(move |_| emit(id, Event::Pressed));
                btn.upcast()
            }
            kinds::TOGGLE => {
                let p = props.downcast_ref::<ToggleProps>().unwrap();
                let sw = gtk4::Switch::new();
                sw.set_active(p.on);
                sw.connect_active_notify(move |s| emit(id, Event::ToggleChanged(s.is_active())));
                sw.upcast()
            }
            kinds::SLIDER => {
                let p = props.downcast_ref::<SliderProps>().unwrap();
                let step = p.step.unwrap_or((p.max - p.min) / 1000.0).max(1e-9);
                let scale =
                    gtk4::Scale::with_range(gtk4::Orientation::Horizontal, p.min, p.max, step);
                scale.set_value(p.value);
                scale.set_draw_value(false);
                scale.connect_value_changed(move |s| emit(id, Event::ValueChanged(s.value())));
                scale.upcast()
            }
            kinds::TEXT_FIELD => {
                let p = props.downcast_ref::<TextFieldProps>().unwrap();
                let entry = gtk4::Entry::new();
                entry.set_text(&p.text);
                entry.set_placeholder_text(Some(&p.placeholder));
                entry.connect_changed(move |e| emit(id, Event::TextChanged(e.text().to_string())));
                entry.upcast()
            }
            kinds::DIVIDER => gtk4::Separator::new(gtk4::Orientation::Horizontal).upcast(),
            kinds::CANVAS => {
                let area = gtk4::DrawingArea::new();
                area.set_draw_func(|area, cr, _w, _h| {
                    let ptr = area.as_ptr() as usize;
                    let ops = OPS
                        .with(|m| m.borrow().get(&ptr).cloned())
                        .unwrap_or_default();
                    cairo_draw(cr, &ops);
                });
                area.upcast()
            }
            kinds::IMAGE => {
                let p = props.downcast_ref::<ImageProps>().unwrap();
                let pic = gtk4::Picture::new();
                if let Ok(root) = std::env::var("DAY_ASSET_ROOT") {
                    let path = std::path::Path::new(&root).join(&p.source);
                    if path.exists() {
                        pic.set_filename(Some(&path));
                    }
                }
                pic.upcast()
            }
            _ => {
                if let Some(make) = self.registry.get(kind).map(|r| r.make) {
                    return make(self, props, id);
                }
                gtk4::Label::new(Some(&format!("⟨{kind}⟩"))).upcast()
            }
        }
    }

    fn update(
        &mut self,
        h: &Handle,
        kind: PieceKind,
        patch: &dyn std::any::Any,
        _anim: Option<&AnimSpec>,
    ) {
        match kind {
            kinds::LABEL => {
                if let (Some(p), Some(label)) = (
                    patch.downcast_ref::<LabelPatch>(),
                    h.downcast_ref::<gtk4::Label>(),
                ) {
                    match p {
                        LabelPatch::Text(t) => {
                            if label.text() != t.as_str() {
                                label.set_text(t);
                            }
                        }
                        LabelPatch::Font(f) => apply_font(label, *f),
                        LabelPatch::Color(_) => {}
                    }
                }
            }
            kinds::BUTTON => {
                if let (Some(p), Some(btn)) = (
                    patch.downcast_ref::<ButtonPatch>(),
                    h.downcast_ref::<gtk4::Button>(),
                ) {
                    match p {
                        ButtonPatch::Title(t) => btn.set_label(t),
                        ButtonPatch::Enabled(e) => btn.set_sensitive(*e),
                    }
                }
            }
            kinds::TOGGLE => {
                if let (Some(p), Some(sw)) = (
                    patch.downcast_ref::<TogglePatch>(),
                    h.downcast_ref::<gtk4::Switch>(),
                ) {
                    match p {
                        TogglePatch::On(on) => {
                            if sw.is_active() != *on {
                                sw.set_active(*on);
                            }
                        }
                        TogglePatch::Enabled(e) => sw.set_sensitive(*e),
                    }
                }
            }
            kinds::SLIDER => {
                if let (Some(p), Some(scale)) = (
                    patch.downcast_ref::<SliderPatch>(),
                    h.downcast_ref::<gtk4::Scale>(),
                ) {
                    match p {
                        SliderPatch::Value(v) => {
                            if (scale.value() - v).abs() > 0.001 {
                                scale.set_value(*v);
                            }
                        }
                        SliderPatch::Enabled(e) => scale.set_sensitive(*e),
                    }
                }
            }
            kinds::TEXT_FIELD => {
                if let (Some(p), Some(entry)) = (
                    patch.downcast_ref::<TextFieldPatch>(),
                    h.downcast_ref::<gtk4::Entry>(),
                ) {
                    match p {
                        TextFieldPatch::Text { text, from_native } => {
                            if !*from_native && entry.text() != text.as_str() {
                                entry.set_text(text);
                            }
                        }
                        TextFieldPatch::Placeholder(t) => entry.set_placeholder_text(Some(t)),
                        TextFieldPatch::Enabled(e) => entry.set_sensitive(*e),
                    }
                }
            }
            _ => {
                if let Some(update) = self.registry.get(kind).map(|r| r.update) {
                    update(self, h, patch);
                }
            }
        }
    }

    fn release(&mut self, h: Handle) {
        if let Some(parent) = h.parent()
            && let Some(fixed) = parent.downcast_ref::<gtk4::Fixed>()
        {
            fixed.remove(&h);
        }
    }

    fn insert(&mut self, parent: &Handle, child: &Handle, _index: usize) {
        if let Some(fixed) = content_of(parent).downcast_ref::<gtk4::Fixed>() {
            fixed.put(child, 0.0, 0.0);
        }
    }

    fn remove(&mut self, parent: &Handle, child: &Handle) {
        if let Some(fixed) = content_of(parent).downcast_ref::<gtk4::Fixed>() {
            fixed.remove(child);
        }
    }

    fn move_child(&mut self, _parent: &Handle, _child: &Handle, _to: usize) {
        // Absolute layout: z-order = insertion order; nothing to do for non-overlapping frames.
    }

    fn measure(&mut self, h: &Handle, kind: PieceKind, p: Proposal) -> Size {
        match kind {
            kinds::LABEL => {
                // Height-for-width through GTK's measure protocol.
                let (_, nat_w, _, _) = h.measure(gtk4::Orientation::Horizontal, -1);
                let w = match p.width {
                    Some(pw) => (nat_w as f64).min(pw),
                    None => nat_w as f64,
                };
                let (_, nat_h, _, _) = h.measure(gtk4::Orientation::Vertical, w.round() as i32);
                Size::new(w.ceil(), nat_h as f64)
            }
            kinds::SLIDER => {
                let (_, nat_h, _, _) = h.measure(gtk4::Orientation::Vertical, -1);
                Size::new(p.width.unwrap_or(180.0), (nat_h as f64).max(24.0))
            }
            kinds::TEXT_FIELD => {
                let (_, nat_h, _, _) = h.measure(gtk4::Orientation::Vertical, -1);
                Size::new(p.width.unwrap_or(180.0), (nat_h as f64).max(24.0))
            }
            kinds::DIVIDER => Size::new(p.width.unwrap_or(0.0), 1.0),
            _ => {
                if let Some(measure) = self.registry.get(kind).and_then(|r| r.measure) {
                    return measure(self, h, p);
                }
                let (_, nat_w, _, _) = h.measure(gtk4::Orientation::Horizontal, -1);
                let (_, nat_h, _, _) = h.measure(gtk4::Orientation::Vertical, -1);
                Size::new(nat_w as f64, nat_h as f64)
            }
        }
    }

    fn set_frame(&mut self, h: &Handle, frame: Rect, _anim: Option<&AnimSpec>) {
        if let Some(parent) = h.parent()
            && let Some(fixed) = parent.downcast_ref::<gtk4::Fixed>()
        {
            fixed.move_(h, frame.origin.x, frame.origin.y);
        }
        h.set_size_request(
            frame.size.width.round() as i32,
            frame.size.height.round() as i32,
        );
    }

    fn set_scroll_content(&mut self, h: &Handle, content: Size) {
        let inner = content_of(h);
        if inner.as_ptr() != h.as_ptr() {
            inner.set_size_request(content.width.round() as i32, content.height.round() as i32);
        }
    }

    fn set_event_sink(&mut self, sink: EventSink) {
        SINK.with(|s| *s.borrow_mut() = Some(Rc::from(sink)));
    }

    fn set_a11y(&mut self, h: &Handle, a11y: &A11yProps) {
        if let Some(id) = &a11y.identifier {
            h.set_widget_name(id); // GtkInspector-visible only (§13's honest table)
        }
        if let Some(label) = &a11y.label {
            // Full GtkAccessible property plumbing lands with M6; tooltip is the M3 stopgap.
            h.set_tooltip_text(Some(label));
        }
    }

    fn replay(&mut self, h: &Handle, ops: &[DrawOp], _size: Size) {
        OPS.with(|m| m.borrow_mut().insert(h.as_ptr() as usize, ops.to_vec()));
        h.queue_draw();
    }

    fn snapshot_window(&mut self) -> Result<Vec<u8>, String> {
        let fixed = self.window_fixed.as_ref().ok_or("no window")?;
        let widget: &gtk4::Widget = fixed.upcast_ref();
        let paintable = gtk4::WidgetPaintable::new(Some(widget));
        let w = widget.width() as f64;
        let h = widget.height() as f64;
        if w <= 0.0 || h <= 0.0 {
            return Err("zero-size window".into());
        }
        let snapshot = gtk4::Snapshot::new();
        use gtk4::gdk::prelude::PaintableExt;
        paintable.snapshot(&snapshot, w, h);
        let node = snapshot.to_node().ok_or("empty render node")?;
        let native = widget.native().ok_or("no native")?;
        let renderer = native.renderer().ok_or("no renderer")?;
        let texture = renderer.render_texture(&node, None);
        Ok(texture.save_to_png_bytes().to_vec())
    }
}

impl Platform for Gtk {
    const TARGET: &'static str = if cfg!(target_os = "macos") {
        "macos-gtk"
    } else {
        "linux-gtk"
    };
    const TOOLKIT: &'static str = "gtk";

    fn run(self, options: WindowOptions, ready: Box<dyn FnOnce(Self, Handle, Size)>) {
        let app = gtk4::Application::builder()
            .application_id("dev.day.app")
            .build();
        let state = RefCell::new(Some((self, ready, options)));
        // Take on first activate (FnOnce payload inside an Fn handler).
        app.connect_activate(move |app| {
            let Some((mut backend, ready, options)) = state.borrow_mut().take() else {
                return;
            };
            let window = gtk4::ApplicationWindow::new(app);
            window.set_title(Some(&options.title));
            window.set_default_size(options.size.width as i32, options.size.height as i32);
            window.set_resizable(false); // live resize needs the custom layout manager (post-MVP)
            let fixed = gtk4::Fixed::new();
            window.set_child(Some(&fixed));
            backend.window_fixed = Some(fixed.clone());
            ready(backend, fixed.upcast(), options.size);
            window.present();
        });
        app.run_with_args::<&str>(&[]);
    }

    fn post(f: Box<dyn FnOnce() + Send>) {
        gtk4::glib::MainContext::default().invoke(f);
    }
}

use day_spec::WindowOptions;
