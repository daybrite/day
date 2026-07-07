//! day-arkui — the HarmonyOS Next **ArkUI** backend (target `ohos-arkui`; DESIGN.md §9).
//!
//! HarmonyOS has no AOSP layer; its UI framework is ArkUI. Day drives it through the **ArkUI Native
//! NodeAPI** (`day-arkui-sys`): every Piece becomes a real `ArkUI_NodeHandle` (Text / Button /
//! TextInput / Toggle / Slider / Stack), built natively and mounted into an ArkTS `NodeContent` slot.
//! Architecturally it mirrors `day-android` — a managed UI runtime (ArkTS) hosts the window, native
//! code (Rust) builds the tree over a thin bridge, and **day owns absolute layout**: containers are
//! `ARKUI_NODE_STACK` and each child gets an explicit position + size (in vp = day points).
//!
//! Off HarmonyOS the crate is empty (`cfg(target_env = "ohos")`), so the workspace still type-checks
//! on the host.

#![allow(clippy::missing_safety_doc)]

#[cfg(target_env = "ohos")]
pub use imp::*;

#[cfg(target_env = "ohos")]
mod imp {
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;
    use std::ffi::{CStr, CString};
    use std::os::raw::{c_char, c_int, c_void};
    use std::rc::Rc;

    use day_arkui_sys as ffi;
    use linkme::distributed_slice;

    use day_spec::props::*;
    use day_spec::{
        A11yProps, AnimSpec, Cap, DrawOp, Event, EventSink, Font, FontSpec, NodeId, PieceKind,
        Platform, Proposal, Rect, Registry, Renderer, Size, Support, Toolkit, WindowOptions, kinds,
    };

    /// An `ArkUI_NodeHandle`. day owns the tree, so the raw pointer is the identity.
    #[derive(Clone, Copy, PartialEq, Eq, Hash)]
    pub struct AHandle(pub *mut c_void);

    type Sink = Rc<dyn Fn(NodeId, Event)>;

    thread_local! {
        static SINK: RefCell<Option<Sink>> = const { RefCell::new(None) };
        /// The window root Stack + content size, set by [`init`] before `run`.
        static ROOT: RefCell<Option<(AHandle, Size)>> = const { RefCell::new(None) };
        static DENSITY: Cell<f64> = const { Cell::new(1.0) };
        /// Slider node ptr → (min, max), so ArkUI's 0..100 maps back to day's range.
        static SLIDER_RANGE: RefCell<HashMap<usize, (f64, f64)>> = RefCell::new(HashMap::new());
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

    /// day `Color` (0..1 components) → ArkUI ARGB `u32`.
    fn argb(c: day_spec::Color) -> u32 {
        let f = |x: f64| (x.clamp(0.0, 1.0) * 255.0).round() as u32;
        (f(c.a) << 24) | (f(c.r) << 16) | (f(c.g) << 8) | f(c.b)
    }

    /// Semantic [`Font`] → a vp point size (ArkUI's default length unit is vp ≈ day points).
    fn font_vp(f: FontSpec) -> f64 {
        match f.style {
            Font::LargeTitle => 34.0,
            Font::Title => 28.0,
            Font::Title2 => 22.0,
            Font::Title3 => 20.0,
            Font::Headline => 17.0,
            Font::Body => 17.0,
            Font::Callout => 16.0,
            Font::Subheadline => 15.0,
            Font::Footnote => 13.0,
            Font::Caption => 12.0,
            Font::Caption2 => 11.0,
            Font::System(pt) => pt,
        }
    }

    // day kind → the shim's node-kind code (see kind_map in shim.cpp).
    const K_STACK: c_int = 0;
    const K_TEXT: c_int = 1;
    const K_BUTTON: c_int = 2;
    const K_TEXT_INPUT: c_int = 3;
    const K_TOGGLE: c_int = 4;
    const K_SLIDER: c_int = 5;
    const K_SCROLL: c_int = 6;

    fn new_node(kind: c_int) -> AHandle {
        AHandle(unsafe { ffi::day_ark_node_new(kind) })
    }

    /// Set up the window root and density from the ArkTS host, before `launch_with`. Called by
    /// `day::arkui::start` (via the `day::arkui_main!` entry macro) with the `NodeContent` handle.
    #[allow(clippy::not_unsafe_ptr_arg_deref)] // `content` is a trusted NodeContent handle from ArkTS
    pub fn init(content: *mut c_void, w_vp: f64, h_vp: f64, density: f64) {
        DENSITY.with(|d| d.set(if density > 0.0 { density } else { 1.0 }));
        unsafe { ffi::day_ark_init() };
        // A Stack fills the window; day mounts its tree under it and positions children absolutely.
        let root = new_node(K_STACK);
        unsafe {
            ffi::day_ark_set_frame(root.0, 0.0, 0.0, w_vp, h_vp);
            ffi::day_ark_content_add(content, root.0);
        }
        ROOT.with(|r| *r.borrow_mut() = Some((root, Size::new(w_vp, h_vp))));
    }

    /// The native event callback the shim invokes (0=click 1=text 2=toggle 3=slider). `id` is the
    /// day NodeId delivered back as the ArkUI event userData.
    #[unsafe(no_mangle)]
    #[allow(clippy::not_unsafe_ptr_arg_deref)] // `text` is a valid C string from the ArkUI event
    pub extern "C" fn day_arkui_on_event(id: u64, kind: c_int, num: f64, text: *const c_char) {
        let node = NodeId(id);
        let ev = match kind {
            0 => Event::Pressed,
            1 => {
                let s = if text.is_null() {
                    String::new()
                } else {
                    unsafe { CStr::from_ptr(text) }
                        .to_string_lossy()
                        .into_owned()
                };
                Event::TextChanged(s)
            }
            2 => Event::ToggleChanged(num != 0.0),
            3 => {
                // ArkUI slider reports 0..100; map back to the node's day range.
                let (min, max) = SLIDER_RANGE
                    .with(|m| m.borrow().get(&(id as usize)).copied())
                    .unwrap_or((0.0, 1.0));
                Event::ValueChanged(min + (num / 100.0) * (max - min))
            }
            // File-picker answer (docs/files.md): `id` is the request id, `text` the chosen local
            // path (a cache copy for open, a docs URI for save) — empty means the user cancelled.
            5 => {
                let s = if text.is_null() {
                    String::new()
                } else {
                    unsafe { CStr::from_ptr(text) }
                        .to_string_lossy()
                        .into_owned()
                };
                let result = day_spec::present::PresentResult::decode(3, 0, s);
                emit(node, Event::PresentResult { req: id, result });
                return;
            }
            _ => return,
        };
        emit(node, ev);
    }

    /// The ArkTS host reports the app cache dir here (docs/files.md); it's the app-writable staging
    /// area for `save_file(..)`, since HarmonyOS's OS temp dir isn't writable by the app.
    #[unsafe(no_mangle)]
    #[allow(clippy::not_unsafe_ptr_arg_deref)] // `path` is a valid C string from the ArkTS host
    pub extern "C" fn day_arkui_set_cache_dir(path: *const c_char) {
        if !path.is_null() {
            let p = unsafe { CStr::from_ptr(path) }
                .to_string_lossy()
                .into_owned();
            if !p.is_empty() {
                day_spec::present::set_app_temp_dir(p);
            }
        }
    }

    /// The ArkUI backend. `new` collects any externally-registered renderers (§8.2), like the others.
    pub struct ArkUi {
        registry: Registry<ArkUi>,
    }

    #[distributed_slice]
    pub static RENDERERS: [fn() -> Renderer<ArkUi>];

    impl ArkUi {
        pub fn new() -> Self {
            let mut registry = Registry::default();
            for f in RENDERERS {
                registry.register(f());
            }
            ArkUi { registry }
        }
    }

    impl Default for ArkUi {
        fn default() -> Self {
            Self::new()
        }
    }

    /// Warn ONCE per kind that this backend has no registered renderer for `kind`, before falling
    /// back to a placeholder (an empty stack node). A missing renderer usually means the piece's
    /// `arkui` feature wasn't enabled. Deduped per kind so it doesn't spam the log.
    fn warn_missing_renderer(kind: PieceKind) {
        static SEEN: std::sync::Mutex<Option<std::collections::HashSet<&'static str>>> =
            std::sync::Mutex::new(None);
        let Ok(mut guard) = SEEN.lock() else { return };
        if guard
            .get_or_insert_with(std::collections::HashSet::new)
            .insert(kind)
        {
            eprintln!(
                "day: no renderer for piece kind \"{kind}\" on arkui \
                 — is the piece's arkui feature enabled? (rendering a placeholder)"
            );
        }
    }

    impl Toolkit for ArkUi {
        type Handle = AHandle;

        fn realize(&mut self, kind: PieceKind, props: &dyn Any, id: NodeId) -> AHandle {
            match kind {
                kinds::CONTAINER => {
                    let n = new_node(K_STACK);
                    if let Some(p) = props.downcast_ref::<ContainerProps>() {
                        unsafe {
                            if let Some(c) = p.background {
                                ffi::day_ark_set_bg_color(n.0, argb(c));
                            }
                            if p.corner_radius > 0.0 {
                                // NODE_BORDER_RADIUS in vp rounds the background (and clips content).
                                ffi::day_ark_set_corner_radius(n.0, p.corner_radius);
                            }
                        }
                    }
                    n
                }
                kinds::SCROLL => new_node(K_SCROLL),
                kinds::LABEL => {
                    let p = props.downcast_ref::<LabelProps>().unwrap();
                    let n = new_node(K_TEXT);
                    unsafe {
                        ffi::day_ark_set_text(n.0, cstr(&p.text).as_ptr());
                        ffi::day_ark_set_font_size(n.0, font_vp(p.font));
                        if let Some(c) = p.color {
                            ffi::day_ark_set_font_color(n.0, argb(c));
                        }
                    }
                    n
                }
                kinds::BUTTON => {
                    let p = props.downcast_ref::<ButtonProps>().unwrap();
                    let n = new_node(K_BUTTON);
                    unsafe {
                        ffi::day_ark_set_button_label(n.0, cstr(&p.title).as_ptr());
                        ffi::day_ark_register_event(n.0, 0, id.0);
                    }
                    n
                }
                kinds::TEXT_FIELD => {
                    let p = props.downcast_ref::<TextFieldProps>().unwrap();
                    let n = new_node(K_TEXT_INPUT);
                    unsafe {
                        ffi::day_ark_set_input_text(n.0, cstr(&p.text).as_ptr());
                        ffi::day_ark_set_placeholder(n.0, cstr(&p.placeholder).as_ptr());
                        ffi::day_ark_register_event(n.0, 1, id.0);
                    }
                    n
                }
                kinds::TOGGLE => {
                    let p = props.downcast_ref::<ToggleProps>().unwrap();
                    let n = new_node(K_TOGGLE);
                    unsafe {
                        ffi::day_ark_set_toggle(n.0, p.on as c_int);
                        ffi::day_ark_register_event(n.0, 2, id.0);
                    }
                    n
                }
                kinds::SLIDER => {
                    let p = props.downcast_ref::<SliderProps>().unwrap();
                    let n = new_node(K_SLIDER);
                    SLIDER_RANGE.with(|m| m.borrow_mut().insert(n.0 as usize, (p.min, p.max)));
                    let pct = normalize(p.value, p.min, p.max);
                    unsafe {
                        ffi::day_ark_set_slider(n.0, pct);
                        ffi::day_ark_register_event(n.0, 3, id.0);
                    }
                    n
                }
                _ => {
                    if let Some(r) = self.registry.get(kind) {
                        let make = r.make;
                        return make(self, props, id);
                    }
                    warn_missing_renderer(kind);
                    new_node(K_STACK)
                }
            }
        }

        fn update(
            &mut self,
            h: &AHandle,
            kind: PieceKind,
            patch: &dyn Any,
            _anim: Option<&AnimSpec>,
        ) {
            match kind {
                kinds::CONTAINER => {
                    if let Some(ContainerPatch::Background(Some(c))) =
                        patch.downcast_ref::<ContainerPatch>()
                    {
                        unsafe { ffi::day_ark_set_bg_color(h.0, argb(*c)) };
                    }
                }
                kinds::LABEL => {
                    if let Some(p) = patch.downcast_ref::<LabelPatch>() {
                        match p {
                            LabelPatch::Text(t) => unsafe {
                                ffi::day_ark_set_text(h.0, cstr(t).as_ptr())
                            },
                            LabelPatch::Color(c) => {
                                if let Some(c) = c {
                                    unsafe { ffi::day_ark_set_font_color(h.0, argb(*c)) };
                                }
                            }
                            LabelPatch::Font(f) => unsafe {
                                ffi::day_ark_set_font_size(h.0, font_vp(*f))
                            },
                        }
                    }
                }
                kinds::BUTTON => {
                    if let Some(ButtonPatch::Title(t)) = patch.downcast_ref::<ButtonPatch>() {
                        unsafe { ffi::day_ark_set_button_label(h.0, cstr(t).as_ptr()) };
                    }
                }
                kinds::TOGGLE => {
                    if let Some(TogglePatch::On(on)) = patch.downcast_ref::<TogglePatch>() {
                        unsafe { ffi::day_ark_set_toggle(h.0, *on as c_int) };
                    }
                }
                kinds::SLIDER => {
                    if let Some(SliderPatch::Value(v)) = patch.downcast_ref::<SliderPatch>() {
                        let (min, max) = SLIDER_RANGE
                            .with(|m| m.borrow().get(&(h.0 as usize)).copied())
                            .unwrap_or((0.0, 1.0));
                        unsafe { ffi::day_ark_set_slider(h.0, normalize(*v, min, max)) };
                    }
                }
                kinds::TEXT_FIELD => {
                    if let Some(TextFieldPatch::Text { text, from_native }) =
                        patch.downcast_ref::<TextFieldPatch>()
                    {
                        // A from_native echo would fight the user's caret — skip it (§4.4).
                        if !from_native {
                            unsafe { ffi::day_ark_set_input_text(h.0, cstr(text).as_ptr()) };
                        }
                    }
                }
                _ => {}
            }
        }

        fn release(&mut self, h: AHandle) {
            SLIDER_RANGE.with(|m| {
                m.borrow_mut().remove(&(h.0 as usize));
            });
            unsafe { ffi::day_ark_node_dispose(h.0) };
        }

        fn insert(&mut self, parent: &AHandle, child: &AHandle, index: usize) {
            unsafe { ffi::day_ark_insert_child(parent.0, child.0, index as c_int) };
        }

        fn remove(&mut self, parent: &AHandle, child: &AHandle) {
            unsafe { ffi::day_ark_remove_child(parent.0, child.0) };
        }

        fn move_child(&mut self, parent: &AHandle, child: &AHandle, to: usize) {
            self.remove(parent, child);
            self.insert(parent, child, to);
        }

        fn measure(&mut self, h: &AHandle, kind: PieceKind, p: Proposal) -> Size {
            match kind {
                kinds::LABEL | kinds::BUTTON => {
                    let (mut w, mut hh) = (0.0f64, 0.0f64);
                    unsafe {
                        ffi::day_ark_measure(
                            h.0,
                            p.width.unwrap_or(-1.0),
                            p.height.unwrap_or(-1.0),
                            &mut w,
                            &mut hh,
                        )
                    };
                    Size::new(w, hh)
                }
                kinds::TEXT_FIELD => Size::new(p.width.unwrap_or(200.0), 40.0),
                kinds::TOGGLE => Size::new(50.0, 30.0),
                kinds::SLIDER => Size::new(p.width.unwrap_or(200.0), 40.0),
                _ => {
                    if let Some(measure) = self.registry.get(kind).and_then(|r| r.measure) {
                        return measure(self, h, p);
                    }
                    Size::new(p.width.unwrap_or(0.0), p.height.unwrap_or(0.0))
                }
            }
        }

        fn set_frame(&mut self, h: &AHandle, frame: Rect, _anim: Option<&AnimSpec>) {
            unsafe {
                ffi::day_ark_set_frame(
                    h.0,
                    frame.origin.x,
                    frame.origin.y,
                    frame.size.width,
                    frame.size.height,
                )
            };
        }

        fn set_event_sink(&mut self, sink: EventSink) {
            SINK.with(|s| *s.borrow_mut() = Some(Rc::from(sink)));
        }

        fn set_a11y(&mut self, _h: &AHandle, _a11y: &A11yProps) {
            // ArkUI accessibility (accessibilityText/Level) — follow-up.
        }

        fn replay(&mut self, _h: &AHandle, _ops: &[DrawOp], _size: Size) {
            // Canvas display list (§11) — follow-up (ARKUI_NODE_CUSTOM + draw callback).
        }

        fn snapshot_window(&mut self) -> Result<Vec<u8>, String> {
            Err("use `hdc shell snapshot_display` on ohos-arkui".into())
        }

        /// Native file open/save via the ArkTS `@kit.CoreFileKit` DocumentViewPicker (docs/files.md).
        /// Alerts/prompts aren't wired on ArkUI yet, so those specs are ignored (like WinUI).
        fn present(&mut self, req: u64, spec: &day_spec::present::PresentSpec) {
            use day_spec::present::PresentSpec;
            match spec {
                PresentSpec::OpenFile { .. } => unsafe {
                    ffi::day_ark_present_file(
                        req,
                        0,
                        std::ptr::null(),
                        std::ptr::null(),
                        cstr(&spec.filters_joined()).as_ptr(),
                    );
                },
                PresentSpec::SaveFile {
                    suggested_name,
                    src_path,
                    ..
                } => unsafe {
                    ffi::day_ark_present_file(
                        req,
                        1,
                        cstr(suggested_name).as_ptr(),
                        cstr(src_path).as_ptr(),
                        cstr(&spec.filters_joined()).as_ptr(),
                    );
                },
                // Dialog / Prompt aren't implemented on ArkUI (a follow-up); ignore.
                _ => {}
            }
        }

        fn capability(&self, cap: Cap) -> Support {
            match cap {
                Cap::FileDialogs => Support::Native,
                _ => Support::Unsupported,
            }
        }
    }

    impl Platform for ArkUi {
        const TARGET: &'static str = "ohos-arkui";
        const TOOLKIT: &'static str = "arkui";

        fn run(self, _options: WindowOptions, ready: Box<dyn FnOnce(Self, AHandle, Size)>) {
            // The ArkTS ability owns the loop; init() already created + mounted the root.
            let (root, size) = ROOT
                .with(|r| r.borrow_mut().take())
                .expect("day-arkui: init() not called before run()");
            ready(self, root, size);
        }

        fn post(f: Box<dyn FnOnce() + Send>) {
            let data = Box::into_raw(Box::new(f)) as *mut c_void;
            unsafe { ffi::day_ark_post(run_posted, data) };
        }
    }

    extern "C" fn run_posted(data: *mut c_void) {
        let f = unsafe { Box::from_raw(data as *mut Box<dyn FnOnce() + Send>) };
        f();
    }

    /// Map a day slider value into ArkUI's default 0..100 range.
    fn normalize(v: f64, min: f64, max: f64) -> f64 {
        if max <= min {
            0.0
        } else {
            ((v - min) / (max - min) * 100.0).clamp(0.0, 100.0)
        }
    }

    use std::any::Any;
}
