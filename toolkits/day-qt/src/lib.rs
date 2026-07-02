//! day-qt — the Qt 6 Widgets backend (linux-qt / macos-qt / windows-qt; DESIGN.md §9), over
//! the day-qt-sys C++ shim. `Handle = QtHandle(*mut QWidget)`; absolute geometry; toggle is a
//! QCheckBox (Qt Widgets has no native switch — an explicitly documented divergence).

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::rc::Rc;

use day_qt_sys as ffi;
use linkme::distributed_slice;

use day_spec::props::*;
use day_spec::{
    A11yProps, AnimSpec, Cap, DrawOp, Event, EventSink, Font, NodeId, PieceKind, Platform,
    Proposal, Rect, Registry, Renderer, Size, Support, Toolkit, WindowOptions, kinds,
};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct QtHandle(pub *mut c_void);

pub type Handle = QtHandle;

thread_local! {
    static SINK: RefCell<Option<Rc<dyn Fn(NodeId, Event)>>> = const { RefCell::new(None) };
    /// Slider f64 range, keyed by node id (event callbacks) AND widget ptr (patch application).
    static RANGES: RefCell<HashMap<u64, (f64, f64)>> = RefCell::new(HashMap::new());
    static RANGES_BY_PTR: RefCell<HashMap<usize, (f64, f64)>> = RefCell::new(HashMap::new());
}

pub fn emit(id: NodeId, ev: Event) {
    let sink = SINK.with(|s| s.borrow().clone());
    if let Some(sink) = sink {
        sink(id, ev);
    }
}

fn cstr(s: &str) -> CString {
    CString::new(s).unwrap_or_default()
}

extern "C" fn on_press(id: u64) {
    emit(NodeId(id), Event::Pressed);
}
extern "C" fn on_toggle(id: u64, on: c_int) {
    emit(NodeId(id), Event::ToggleChanged(on != 0));
}
extern "C" fn on_text(id: u64, s: *const c_char) {
    let text = unsafe { CStr::from_ptr(s) }.to_string_lossy().into_owned();
    emit(NodeId(id), Event::TextChanged(text));
}
extern "C" fn on_slider(id: u64, v: c_int) {
    let (min, max) = RANGES.with(|r| r.borrow().get(&id).copied().unwrap_or((0.0, 1.0)));
    let value = min + (v as f64 / 1000.0) * (max - min);
    emit(NodeId(id), Event::ValueChanged(value));
}

fn slider_ticks(value: f64, min: f64, max: f64) -> c_int {
    if max <= min {
        return 0;
    }
    (((value - min) / (max - min)) * 1000.0).round() as c_int
}

/// Renderers registered by external Day Piece crates (§8.2).
#[distributed_slice]
pub static RENDERERS: [fn() -> Renderer<Qt>];

pub struct Qt {
    registry: Registry<Qt>,
    window: *mut c_void,
}

impl Qt {
    pub fn new() -> Self {
        let mut registry = Registry::default();
        for f in RENDERERS {
            registry.register(f());
        }
        Qt { registry, window: std::ptr::null_mut() }
    }
}

impl Default for Qt {
    fn default() -> Self {
        Self::new()
    }
}

fn content_of(parent: &QtHandle) -> *mut c_void {
    // Scroll areas expose an inner content widget; the shim returns null for non-scrolls.
    let inner = unsafe { ffi::day_qt_scroll_content(parent.0) };
    if inner.is_null() { parent.0 } else { inner }
}

fn font_params(f: Font) -> (f64, c_int) {
    match f {
        Font::Title => (22.0, 1),
        Font::Headline => (14.0, 1),
        Font::Body => (13.0, 0),
        Font::Caption => (11.0, 0),
        Font::System(pt) => (pt, 0),
    }
}

impl Toolkit for Qt {
    type Handle = QtHandle;

    fn capability(&self, cap: Cap) -> Support {
        match cap {
            Cap::Snapshot => Support::Native,
            _ => Support::Unsupported,
        }
    }

    fn realize(&mut self, kind: PieceKind, props: &dyn std::any::Any, id: NodeId) -> QtHandle {
        unsafe {
            match kind {
                kinds::CONTAINER => QtHandle(ffi::day_qt_container_new()),
                kinds::SCROLL => QtHandle(ffi::day_qt_scroll_new()),
                kinds::LABEL => {
                    let p = props.downcast_ref::<LabelProps>().unwrap();
                    let w = ffi::day_qt_label_new(cstr(&p.text).as_ptr());
                    let (pt, bold) = font_params(p.font);
                    ffi::day_qt_label_set_font(w, pt, bold);
                    QtHandle(w)
                }
                kinds::BUTTON => {
                    let p = props.downcast_ref::<ButtonProps>().unwrap();
                    QtHandle(ffi::day_qt_button_new(cstr(&p.title).as_ptr(), id.0, on_press))
                }
                kinds::TOGGLE => {
                    let p = props.downcast_ref::<ToggleProps>().unwrap();
                    QtHandle(ffi::day_qt_checkbox_new(p.on as c_int, id.0, on_toggle))
                }
                kinds::SLIDER => {
                    let p = props.downcast_ref::<SliderProps>().unwrap();
                    RANGES.with(|r| r.borrow_mut().insert(id.0, (p.min, p.max)));
                    let w = ffi::day_qt_slider_new(
                        slider_ticks(p.value, p.min, p.max),
                        id.0,
                        on_slider,
                    );
                    RANGES_BY_PTR.with(|r| r.borrow_mut().insert(w as usize, (p.min, p.max)));
                    QtHandle(w)
                }
                kinds::TEXT_FIELD => {
                    let p = props.downcast_ref::<TextFieldProps>().unwrap();
                    QtHandle(ffi::day_qt_lineedit_new(
                        cstr(&p.text).as_ptr(),
                        cstr(&p.placeholder).as_ptr(),
                        id.0,
                        on_text,
                    ))
                }
                kinds::DIVIDER => QtHandle(ffi::day_qt_separator_new()),
                kinds::CANVAS => QtHandle(ffi::day_qt_canvas_new()),
                kinds::IMAGE => {
                    let p = props.downcast_ref::<ImageProps>().unwrap();
                    let path = std::env::var("DAY_ASSET_ROOT")
                        .map(|r| std::path::Path::new(&r).join(&p.source))
                        .ok()
                        .filter(|p| p.exists())
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    QtHandle(ffi::day_qt_image_new(cstr(&path).as_ptr()))
                }
                _ => {
                    if let Some(make) = self.registry.get(kind).map(|r| r.make) {
                        return make(self, props, id);
                    }
                    QtHandle(ffi::day_qt_label_new(cstr(&format!("⟨{kind}⟩")).as_ptr()))
                }
            }
        }
    }

    fn update(&mut self, h: &QtHandle, kind: PieceKind, patch: &dyn std::any::Any, _anim: Option<&AnimSpec>) {
        unsafe {
            match kind {
                kinds::LABEL => {
                    if let Some(p) = patch.downcast_ref::<LabelPatch>() {
                        match p {
                            LabelPatch::Text(t) => {
                                ffi::day_qt_label_set_text(h.0, cstr(t).as_ptr())
                            }
                            LabelPatch::Font(f) => {
                                let (pt, bold) = font_params(*f);
                                ffi::day_qt_label_set_font(h.0, pt, bold);
                            }
                            LabelPatch::Color(_) => {}
                        }
                    }
                }
                kinds::BUTTON => {
                    if let Some(p) = patch.downcast_ref::<ButtonPatch>() {
                        match p {
                            ButtonPatch::Title(t) => {
                                ffi::day_qt_button_set_title(h.0, cstr(t).as_ptr())
                            }
                            ButtonPatch::Enabled(e) => ffi::day_qt_set_enabled(h.0, *e as c_int),
                        }
                    }
                }
                kinds::TOGGLE => {
                    if let Some(p) = patch.downcast_ref::<TogglePatch>() {
                        match p {
                            TogglePatch::On(on) => ffi::day_qt_checkbox_set(h.0, *on as c_int),
                            TogglePatch::Enabled(e) => ffi::day_qt_set_enabled(h.0, *e as c_int),
                        }
                    }
                }
                kinds::SLIDER => {
                    if let Some(p) = patch.downcast_ref::<SliderPatch>() {
                        match p {
                            SliderPatch::Value(v) => {
                                let (min, max) = RANGES_BY_PTR
                                    .with(|r| r.borrow().get(&(h.0 as usize)).copied())
                                    .unwrap_or((0.0, 1.0));
                                ffi::day_qt_slider_set(h.0, slider_ticks(*v, min, max));
                            }
                            SliderPatch::Enabled(e) => ffi::day_qt_set_enabled(h.0, *e as c_int),
                        }
                    }
                }
                kinds::TEXT_FIELD => {
                    if let Some(p) = patch.downcast_ref::<TextFieldPatch>() {
                        match p {
                            TextFieldPatch::Text { text, from_native } => {
                                if !*from_native {
                                    ffi::day_qt_lineedit_set_text(h.0, cstr(text).as_ptr());
                                }
                            }
                            TextFieldPatch::Placeholder(t) => {
                                ffi::day_qt_lineedit_set_placeholder(h.0, cstr(t).as_ptr())
                            }
                            TextFieldPatch::Enabled(e) => ffi::day_qt_set_enabled(h.0, *e as c_int),
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
    }

    fn release(&mut self, h: QtHandle) {
        unsafe {
            ffi::day_qt_remove_child(h.0);
            ffi::day_qt_delete(h.0);
        }
    }

    fn insert(&mut self, parent: &QtHandle, child: &QtHandle, _index: usize) {
        unsafe { ffi::day_qt_add_child(content_of(parent), child.0) };
    }

    fn remove(&mut self, _parent: &QtHandle, child: &QtHandle) {
        unsafe { ffi::day_qt_remove_child(child.0) };
    }

    fn move_child(&mut self, _parent: &QtHandle, _child: &QtHandle, _to: usize) {}

    fn measure(&mut self, h: &QtHandle, kind: PieceKind, p: Proposal) -> Size {
        let mut w = 0.0;
        let mut hh = 0.0;
        unsafe { ffi::day_qt_size_hint(h.0, &mut w, &mut hh) };
        match kind {
            kinds::LABEL => {
                let width = match p.width {
                    Some(pw) => w.min(pw),
                    None => w,
                };
                if p.width.is_some() && w > width {
                    let hfw =
                        unsafe { ffi::day_qt_label_height_for_width(h.0, width.round() as c_int) };
                    Size::new(width.ceil(), (hfw as f64).max(hh))
                } else {
                    Size::new(w.ceil(), hh.ceil())
                }
            }
            kinds::SLIDER => Size::new(p.width.unwrap_or(180.0), hh.max(20.0)),
            kinds::TEXT_FIELD => Size::new(p.width.unwrap_or(180.0), hh.max(24.0)),
            kinds::DIVIDER => Size::new(p.width.unwrap_or(0.0), 2.0),
            _ => {
                if let Some(measure) = self.registry.get(kind).and_then(|r| r.measure) {
                    measure(self, h, p)
                } else {
                    Size::new(p.width.unwrap_or(w), p.height.unwrap_or(hh))
                }
            }
        }
    }

    fn set_frame(&mut self, h: &QtHandle, frame: Rect, _anim: Option<&AnimSpec>) {
        unsafe {
            ffi::day_qt_set_geometry(
                h.0,
                frame.origin.x.round() as c_int,
                frame.origin.y.round() as c_int,
                frame.size.width.round() as c_int,
                frame.size.height.round() as c_int,
            )
        };
    }

    fn set_scroll_content(&mut self, h: &QtHandle, content: Size) {
        unsafe {
            ffi::day_qt_scroll_set_content_size(
                h.0,
                content.width.round() as c_int,
                content.height.round() as c_int,
            )
        };
    }

    fn set_event_sink(&mut self, sink: EventSink) {
        SINK.with(|s| *s.borrow_mut() = Some(Rc::from(sink)));
    }

    fn set_a11y(&mut self, h: &QtHandle, a11y: &A11yProps) {
        unsafe {
            if let Some(id) = &a11y.identifier {
                ffi::day_qt_set_object_name(h.0, cstr(id).as_ptr());
            }
            if let Some(label) = &a11y.label {
                ffi::day_qt_set_tooltip(h.0, cstr(label).as_ptr());
            }
        }
    }

    fn replay(&mut self, h: &QtHandle, ops: &[DrawOp], _size: Size) {
        let (nums, texts) = day_spec::encode_ops(ops);
        let joined = cstr(&texts.join("\n"));
        unsafe {
            ffi::day_qt_canvas_set_ops(h.0, nums.as_ptr(), nums.len() as c_int, joined.as_ptr())
        };
    }

    fn snapshot_window(&mut self) -> Result<Vec<u8>, String> {
        if self.window.is_null() {
            return Err("no window".into());
        }
        let path = std::env::temp_dir().join(format!("day-qt-snap-{}.png", std::process::id()));
        let cpath = cstr(path.to_str().unwrap_or("/tmp/day-qt-snap.png"));
        let rc = unsafe { ffi::day_qt_snapshot_png(self.window, cpath.as_ptr()) };
        if rc != 0 {
            return Err("grab failed".into());
        }
        let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        let _ = std::fs::remove_file(&path);
        Ok(bytes)
    }
}

extern "C" fn run_posted(data: *mut c_void) {
    let f: Box<Box<dyn FnOnce() + Send>> = unsafe { Box::from_raw(data as *mut _) };
    f();
}

impl Platform for Qt {
    const TARGET: &'static str = if cfg!(target_os = "macos") {
        "macos-qt"
    } else if cfg!(target_os = "windows") {
        "windows-qt"
    } else {
        "linux-qt"
    };
    const TOOLKIT: &'static str = "qt";

    fn run(mut self, options: WindowOptions, ready: Box<dyn FnOnce(Self, QtHandle, Size)>) {
        unsafe {
            let app = ffi::day_qt_app_new();
            let window = ffi::day_qt_window_new(
                cstr(&options.title).as_ptr(),
                options.size.width as c_int,
                options.size.height as c_int,
            );
            self.window = window;
            ready(self, QtHandle(window), options.size);
            ffi::day_qt_window_show(window);
            ffi::day_qt_app_run(app);
        }
    }

    fn post(f: Box<dyn FnOnce() + Send>) {
        let data = Box::into_raw(Box::new(f)) as *mut c_void;
        unsafe { ffi::day_qt_post(run_posted, data) };
    }
}
