//! day-winui — the Windows backend (target `windows-winui`; DESIGN.md §1, §9), over the
//! day-winui-sys C++/WinRT XAML-Islands shim. `Handle = WinHandle(*mut UIElement)`; every day
//! node is a real `Windows.UI.Xaml` control (TextBlock, Button, ToggleSwitch, Slider, TextBox,
//! ComboBox) hosted inside a `DesktopWindowXamlSource`. day owns layout — containers are XAML
//! `Canvas`es and children are placed by absolute frame — exactly like the GTK/AppKit/Qt
//! backends. Native events (Click/Toggled/ValueChanged/TextChanged) funnel through the shim's
//! id-keyed callbacks into day's event sink.

#![cfg(windows)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::rc::Rc;

use day_winui_sys as ffi;
use linkme::distributed_slice;

use day_spec::props::*;
use day_spec::{
    A11yProps, AnimSpec, Cap, Event, EventSink, Font, NodeId, PieceKind, Platform, Proposal, Rect,
    Registry, Renderer, Size, Support, Toolkit, WindowOptions, kinds,
};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct WinHandle(pub *mut c_void);

pub type Handle = WinHandle;

/// The day-core event sink (node-id keyed).
type Sink = Rc<dyn Fn(NodeId, Event)>;

thread_local! {
    static SINK: RefCell<Option<Sink>> = const { RefCell::new(None) };
    /// Slider f64 range, keyed by node id (event callbacks) and handle ptr (patch application).
    static RANGES: RefCell<HashMap<u64, (f64, f64)>> = RefCell::new(HashMap::new());
    static RANGES_BY_PTR: RefCell<HashMap<usize, (f64, f64)>> = RefCell::new(HashMap::new());
}

/// Emit an event into day-core's queue (public for external Day Piece renderers).
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
pub static RENDERERS: [fn() -> Renderer<WinUi>];

pub struct WinUi {
    registry: Registry<WinUi>,
    window: *mut c_void,
}

impl WinUi {
    pub fn new() -> Self {
        let mut registry = Registry::default();
        for f in RENDERERS {
            registry.register(f());
        }
        WinUi {
            registry,
            window: std::ptr::null_mut(),
        }
    }
}

impl Default for WinUi {
    fn default() -> Self {
        Self::new()
    }
}

/// day font intents → (XAML FontSize in DIPs, bold).
fn font_params(f: Font) -> (f64, c_int) {
    match f {
        Font::Title => (24.0, 1),
        Font::Headline => (18.0, 1),
        Font::Body => (14.0, 0),
        Font::Caption => (12.0, 0),
        Font::System(pt) => (pt, 0),
    }
}

/// Natural (unconstrained) desired size from the shim's XAML Measure.
fn natural(h: *mut c_void) -> Size {
    let mut w = 0.0;
    let mut hh = 0.0;
    unsafe { ffi::day_winui_measure(h, -1.0, -1.0, &mut w, &mut hh) };
    Size::new(w, hh)
}

impl Toolkit for WinUi {
    type Handle = WinHandle;

    fn capability(&self, cap: Cap) -> Support {
        match cap {
            Cap::Snapshot => Support::Native,
            _ => Support::Unsupported,
        }
    }

    fn realize(&mut self, kind: PieceKind, props: &dyn std::any::Any, id: NodeId) -> WinHandle {
        unsafe {
            match kind {
                kinds::CONTAINER => {
                    let h = ffi::day_winui_container_new();
                    if let Some(p) = props.downcast_ref::<ContainerProps>()
                        && let Some(bg) = p.background
                    {
                        ffi::day_winui_container_set_bg(h, argb(bg));
                    }
                    WinHandle(h)
                }
                kinds::SCROLL => WinHandle(ffi::day_winui_scroll_new()),
                kinds::CANVAS => WinHandle(ffi::day_winui_canvas_new()),
                kinds::LABEL => {
                    let p = props.downcast_ref::<LabelProps>().unwrap();
                    let h = ffi::day_winui_label_new(cstr(&p.text).as_ptr());
                    let (pt, bold) = font_params(p.font);
                    ffi::day_winui_label_set_font(h, pt, bold);
                    WinHandle(h)
                }
                kinds::BUTTON => {
                    let p = props.downcast_ref::<ButtonProps>().unwrap();
                    let h = ffi::day_winui_button_new(cstr(&p.title).as_ptr(), id.0, on_press);
                    ffi::day_winui_set_enabled(h, p.enabled as c_int);
                    WinHandle(h)
                }
                kinds::TOGGLE => {
                    let p = props.downcast_ref::<ToggleProps>().unwrap();
                    let h = ffi::day_winui_toggle_new(p.on as c_int, id.0, on_toggle);
                    ffi::day_winui_set_enabled(h, p.enabled as c_int);
                    WinHandle(h)
                }
                kinds::SLIDER => {
                    let p = props.downcast_ref::<SliderProps>().unwrap();
                    RANGES.with(|r| r.borrow_mut().insert(id.0, (p.min, p.max)));
                    let h = ffi::day_winui_slider_new(
                        slider_ticks(p.value, p.min, p.max),
                        id.0,
                        on_slider,
                    );
                    RANGES_BY_PTR.with(|r| r.borrow_mut().insert(h as usize, (p.min, p.max)));
                    ffi::day_winui_set_enabled(h, p.enabled as c_int);
                    WinHandle(h)
                }
                kinds::TEXT_FIELD => {
                    let p = props.downcast_ref::<TextFieldProps>().unwrap();
                    let h = ffi::day_winui_textbox_new(
                        cstr(&p.text).as_ptr(),
                        cstr(&p.placeholder).as_ptr(),
                        id.0,
                        on_text,
                    );
                    ffi::day_winui_set_enabled(h, p.enabled as c_int);
                    WinHandle(h)
                }
                kinds::DIVIDER => WinHandle(ffi::day_winui_divider_new()),
                kinds::IMAGE => {
                    let p = props.downcast_ref::<ImageProps>().unwrap();
                    WinHandle(ffi::day_winui_image_new(
                        cstr(&image_uri(&p.source)).as_ptr(),
                    ))
                }
                _ => {
                    if let Some(make) = self.registry.get(kind).map(|r| r.make) {
                        return make(self, props, id);
                    }
                    WinHandle(ffi::day_winui_label_new(
                        cstr(&format!("⟨{kind}⟩")).as_ptr(),
                    ))
                }
            }
        }
    }

    fn update(
        &mut self,
        h: &WinHandle,
        kind: PieceKind,
        patch: &dyn std::any::Any,
        _anim: Option<&AnimSpec>,
    ) {
        unsafe {
            match kind {
                kinds::LABEL => {
                    if let Some(p) = patch.downcast_ref::<LabelPatch>() {
                        match p {
                            LabelPatch::Text(t) => {
                                ffi::day_winui_label_set_text(h.0, cstr(t).as_ptr())
                            }
                            LabelPatch::Font(f) => {
                                let (pt, bold) = font_params(*f);
                                ffi::day_winui_label_set_font(h.0, pt, bold);
                            }
                            LabelPatch::Color(_) => {} // XAML Foreground token is a follow-up
                        }
                    }
                }
                kinds::BUTTON => {
                    if let Some(p) = patch.downcast_ref::<ButtonPatch>() {
                        match p {
                            ButtonPatch::Title(t) => {
                                ffi::day_winui_button_set_title(h.0, cstr(t).as_ptr())
                            }
                            ButtonPatch::Enabled(e) => ffi::day_winui_set_enabled(h.0, *e as c_int),
                        }
                    }
                }
                kinds::TOGGLE => {
                    if let Some(p) = patch.downcast_ref::<TogglePatch>() {
                        match p {
                            TogglePatch::On(on) => ffi::day_winui_toggle_set(h.0, *on as c_int),
                            TogglePatch::Enabled(e) => ffi::day_winui_set_enabled(h.0, *e as c_int),
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
                                ffi::day_winui_slider_set(h.0, slider_ticks(*v, min, max));
                            }
                            SliderPatch::Enabled(e) => ffi::day_winui_set_enabled(h.0, *e as c_int),
                        }
                    }
                }
                kinds::TEXT_FIELD => {
                    if let Some(p) = patch.downcast_ref::<TextFieldPatch>() {
                        match p {
                            TextFieldPatch::Text { text, from_native } => {
                                if !*from_native {
                                    ffi::day_winui_textbox_set_text(h.0, cstr(text).as_ptr());
                                }
                            }
                            TextFieldPatch::Placeholder(t) => {
                                ffi::day_winui_textbox_set_placeholder(h.0, cstr(t).as_ptr())
                            }
                            TextFieldPatch::Enabled(e) => {
                                ffi::day_winui_set_enabled(h.0, *e as c_int)
                            }
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

    fn release(&mut self, h: WinHandle) {
        RANGES_BY_PTR.with(|r| r.borrow_mut().remove(&(h.0 as usize)));
        unsafe { ffi::day_winui_delete(h.0) };
    }

    fn insert(&mut self, parent: &WinHandle, child: &WinHandle, _index: usize) {
        unsafe { ffi::day_winui_add_child(parent.0, child.0) };
    }

    fn remove(&mut self, parent: &WinHandle, child: &WinHandle) {
        unsafe { ffi::day_winui_remove_child(parent.0, child.0) };
    }

    fn move_child(&mut self, _parent: &WinHandle, _child: &WinHandle, _to: usize) {
        // Absolute frames don't overlap: sibling z-order is irrelevant.
    }

    fn measure(&mut self, h: &WinHandle, kind: PieceKind, p: Proposal) -> Size {
        match kind {
            kinds::LABEL => {
                let nat = natural(h.0);
                match p.width {
                    Some(pw) if nat.width > pw => {
                        // Height-for-width: re-measure wrapped at the proposed width.
                        let mut w = 0.0;
                        let mut hh = 0.0;
                        unsafe { ffi::day_winui_measure(h.0, pw, -1.0, &mut w, &mut hh) };
                        Size::new(pw.ceil(), hh.ceil())
                    }
                    _ => Size::new(nat.width.ceil(), nat.height.ceil()),
                }
            }
            kinds::SLIDER => Size::new(p.width.unwrap_or(180.0), natural(h.0).height.max(24.0)),
            kinds::TEXT_FIELD => Size::new(p.width.unwrap_or(180.0), natural(h.0).height.max(28.0)),
            kinds::DIVIDER => Size::new(p.width.unwrap_or(0.0), 1.0),
            _ => {
                if let Some(measure) = self.registry.get(kind).and_then(|r| r.measure) {
                    return measure(self, h, p);
                }
                let nat = natural(h.0);
                Size::new(
                    p.width.unwrap_or(nat.width).ceil(),
                    p.height.unwrap_or(nat.height).ceil(),
                )
            }
        }
    }

    fn set_frame(&mut self, h: &WinHandle, frame: Rect, _anim: Option<&AnimSpec>) {
        unsafe {
            ffi::day_winui_set_geometry(
                h.0,
                frame.origin.x.round() as c_int,
                frame.origin.y.round() as c_int,
                frame.size.width.round() as c_int,
                frame.size.height.round() as c_int,
            )
        };
    }

    fn set_event_sink(&mut self, sink: EventSink) {
        SINK.with(|s| *s.borrow_mut() = Some(Rc::from(sink)));
    }

    fn set_a11y(&mut self, h: &WinHandle, a11y: &A11yProps) {
        if let Some(id) = &a11y.identifier {
            unsafe { ffi::day_winui_set_name(h.0, cstr(id).as_ptr()) };
        }
    }

    fn snapshot_window(&mut self) -> Result<Vec<u8>, String> {
        if self.window.is_null() {
            return Err("no window".into());
        }
        let path = std::env::temp_dir().join(format!("day-winui-snap-{}.png", std::process::id()));
        let cpath = cstr(&path.to_string_lossy());
        let rc = unsafe { ffi::day_winui_snapshot_png(self.window, cpath.as_ptr()) };
        if rc != 0 {
            return Err(format!("snapshot failed (rc={rc})"));
        }
        let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        let _ = std::fs::remove_file(&path);
        Ok(bytes)
    }
}

fn argb(c: day_spec::Color) -> u32 {
    let a = (c.a.clamp(0.0, 1.0) * 255.0) as u32;
    let r = (c.r.clamp(0.0, 1.0) * 255.0) as u32;
    let g = (c.g.clamp(0.0, 1.0) * 255.0) as u32;
    let b = (c.b.clamp(0.0, 1.0) * 255.0) as u32;
    (a << 24) | (r << 16) | (g << 8) | b
}

/// Resolve an asset name to a `file:///` URI the XAML `BitmapImage` can load (§18.2).
fn image_uri(source: &str) -> String {
    let path = std::env::var("DAY_ASSET_ROOT")
        .map(|r| std::path::Path::new(&r).join(source))
        .ok()
        .filter(|p| p.exists());
    match path {
        Some(p) => format!("file:///{}", p.to_string_lossy().replace('\\', "/")),
        None => String::new(),
    }
}

extern "C" fn run_posted(data: *mut c_void) {
    let f: Box<Box<dyn FnOnce() + Send>> = unsafe { Box::from_raw(data as *mut _) };
    f();
}

impl Platform for WinUi {
    const TARGET: &'static str = "windows-winui";
    const TOOLKIT: &'static str = "winui";

    fn run(mut self, options: WindowOptions, ready: Box<dyn FnOnce(Self, WinHandle, Size)>) {
        unsafe {
            let win = ffi::day_winui_window_new(
                cstr(&options.title).as_ptr(),
                options.size.width as c_int,
                options.size.height as c_int,
            );
            if win.is_null() {
                eprintln!("day-winui: could not create the XAML window (see error above)");
                return;
            }
            self.window = win;
            let root = ffi::day_winui_window_root(win);
            ready(self, WinHandle(root), options.size);
            ffi::day_winui_window_show(win);
            ffi::day_winui_run(win);
        }
    }

    fn post(f: Box<dyn FnOnce() + Send>) {
        let data = Box::into_raw(Box::new(f)) as *mut c_void;
        unsafe { ffi::day_winui_post(run_posted, data) };
    }
}
