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
pub mod ext;
#[cfg(target_env = "ohos")]
pub use ext::*;

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
        A11yProps, AnimSpec, Cap, DrawOp, Event, EventSink, Font, FontSpec, GestureKind, NodeId,
        PieceKind, Platform, Point, Proposal, Rect, Registry, Renderer, Size, Support, Toolkit,
        WindowOptions, kinds,
    };

    /// An `ArkUI_NodeHandle`. day owns the tree, so the raw pointer is the identity.
    #[derive(Clone, Copy, PartialEq, Eq, Hash)]
    pub struct AHandle(pub *mut c_void);

    type Sink = Rc<dyn Fn(NodeId, Event)>;

    thread_local! {
        /// Navigation state (docs/navigation.md): the single app nav host (its day NodeId +
        /// ArkUI node pointer), the host's attached page children in order (page ptr → day
        /// NodeId, so a Pushed patch can re-home the just-attached last page), pages re-homed
        /// into ArkTS NodeContents (page ptr → key), how many pops Day itself initiated (a
        /// `navPopped` for one of those must NOT sync back), and the current native stack depth.
        static NAV_HOST: std::cell::Cell<Option<(u64, usize)>> = const { std::cell::Cell::new(None) };
        static NAV_ATTACHED: RefCell<Vec<(usize, u64)>> = const { RefCell::new(Vec::new()) };
        static NAV_PUSHED: RefCell<HashMap<usize, u64>> = RefCell::new(HashMap::new());
        static NAV_EXPECT_POP: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
        static NAV_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
        /// NAV_PAGE node ptr → day NodeId (recorded at realize; consumed by insert/push).
        static NAV_PAGE_IDS: RefCell<HashMap<usize, u64>> = RefCell::new(HashMap::new());
        static SINK: RefCell<Option<Sink>> = const { RefCell::new(None) };
        /// The window root Stack + content size, set by [`init`] before `run`.
        static ROOT: RefCell<Option<(AHandle, Size)>> = const { RefCell::new(None) };
        static DENSITY: Cell<f64> = const { Cell::new(1.0) };
        /// Dark mode (docs/localization + theming): resolved once at init — DAY_THEME (the CI
        /// forced theme) wins, else DAY_ARKUI_DARK (the system color mode the ArkTS host reports
        /// via setEnv before start()). ArkUI's C-API nodes do NOT re-theme hardcoded colors, so
        /// every neutral day-arkui paints branches on this flag.
        static IS_DARK: Cell<bool> = const { Cell::new(false) };
        /// Slider node ptr → (min, max), so ArkUI's 0..100 maps back to day's range.
        static SLIDER_RANGE: RefCell<HashMap<usize, (f64, f64)>> = RefCell::new(HashMap::new());
        /// A NAV_MENU row's synthetic click id → (menu node, row index). A tap on a menu row is a
        /// plain NODE_ON_CLICK, so we register it against a fresh synthetic id and translate the
        /// click back into `SelectionChanged(index)` against the MENU host (day-android does the
        /// same with a per-row listener). See [`day_arkui_on_event`].
        static MENU_ROWS: RefCell<HashMap<u64, (NodeId, i64)>> = RefCell::new(HashMap::new());
        /// Monotonic synthetic-id counter for menu rows (kept out of day's NodeId space by using the
        /// high bit, which day-core never allocates).
        static SYNTH: Cell<u64> = const { Cell::new(1u64 << 63) };
        /// LIST host node ptr → its day NodeId, so `attach_list` (which only gets the handle) can
        /// key the source by the id the native adapter callbacks report.
        static LIST_NODE: RefCell<HashMap<usize, u64>> = RefCell::new(HashMap::new());
        /// LIST host NodeId → its injected row-pull source (docs/list.md).
        static LIST_SOURCES: RefCell<HashMap<u64, day_spec::ListSource>> =
            RefCell::new(HashMap::new());
        /// TABS_PAGE node ptrs: their layout is owned by the parent Swiper, so `set_frame` sizes
        /// them but does not position them.
        static TABS_PAGES: RefCell<std::collections::HashSet<usize>> =
            RefCell::new(std::collections::HashSet::new());
        /// Node ids with a Tap gesture (docs/shapes.md): a NODE_ON_CLICK on these emits `Event::Tap`
        /// (not `Event::Pressed`) — how a canvas/shape `.on_tap` (e.g. day-piece-rating's stars)
        /// receives taps on ArkUI. See [`Toolkit::enable_gesture`] + [`day_arkui_on_event`].
        static TAP_NODES: RefCell<std::collections::HashSet<u64>> =
            RefCell::new(std::collections::HashSet::new());
        /// Tap-node handle ptr → its node id, so `release` (which only gets the handle) can drop the
        /// matching TAP_NODES entry (else a recycling list would grow the set unbounded).
        static TAP_HANDLES: RefCell<HashMap<usize, u64>> = RefCell::new(HashMap::new());
    }

    /// Build a NAV_MENU: a scrollable column of CONVENTIONAL navigation rows — leading-aligned
    /// label, trailing chevron, hairline separators (the HarmonyOS settings-list idiom) — not
    /// buttons. Each row's tap becomes a synthetic click that [`day_arkui_on_event`] translates
    /// to `SelectionChanged(index)` against `menu`.
    fn build_nav_menu(menu: NodeId, items: &[String]) -> AHandle {
        let scroll = new_node(K_SCROLL);
        let col = new_node(K_COLUMN);
        let mut pos: c_int = 0;
        for (i, title) in items.iter().enumerate() {
            // A Row (vertically centered children) carries the whole-row click target.
            let row = new_node(K_ROW);
            let label = new_node(K_TEXT);
            let chevron = new_node(K_TEXT);
            let synth = SYNTH.with(|c| {
                let v = c.get();
                c.set(v + 1);
                v
            });
            MENU_ROWS.with(|m| m.borrow_mut().insert(synth, (menu, i as i64)));
            unsafe {
                ffi::day_ark_set_text(label.0, cstr(title).as_ptr());
                ffi::day_ark_set_font_size(label.0, 16.0);
                ffi::day_ark_set_font_color(label.0, theme_color(0xE500_0000, 0xE6FF_FFFF));
                ffi::day_ark_set_flex_grow(label.0, 1.0);
                ffi::day_ark_set_text(chevron.0, cstr("\u{203a}").as_ptr());
                ffi::day_ark_set_font_size(chevron.0, 20.0);
                ffi::day_ark_set_font_color(chevron.0, theme_color(0x4D00_0000, 0x66FF_FFFF));
                ffi::day_ark_insert_child(row.0, label.0, 0);
                ffi::day_ark_insert_child(row.0, chevron.0, 1);
                ffi::day_ark_style_row(row.0, 52.0);
                ffi::day_ark_register_event(row.0, 0, synth);
                ffi::day_ark_insert_child(col.0, row.0, pos);
            }
            pos += 1;
            if i + 1 < items.len() {
                let sep = new_node(K_STACK);
                unsafe {
                    ffi::day_ark_menu_separator(sep.0, theme_color(0x1400_0000, 0x24FF_FFFF));
                    ffi::day_ark_insert_child(col.0, sep.0, pos);
                }
                pos += 1;
            }
        }
        unsafe { ffi::day_ark_insert_child(scroll.0, col.0, 0) };
        scroll
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
            Font::Custom(_, pt) => pt,
        }
    }

    /// Apply a `Font::Custom` family (§18.4): the family was registered by the
    /// platform/ohos scaffold's EntryAbility (from rawfile `day/fonts.json`), so NODE_FONT_FAMILY resolves it
    /// by name; ArkUI falls back to the default family when it doesn't.
    fn apply_custom_family(node: *mut c_void, spec: FontSpec) {
        if let Font::Custom(family, _) = spec.style {
            unsafe { ffi::day_ark_set_font_family(node, cstr(family).as_ptr()) };
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
    const K_COLUMN: c_int = 7;
    const K_ROW: c_int = 15;
    const K_LOADING: c_int = 8; // indeterminate spinner
    const K_IMAGE: c_int = 9;
    const K_CANVAS: c_int = 10; // custom node + on-draw
    const K_PROGRESS: c_int = 11; // determinate bar
    const K_SWIPER: c_int = 12;
    const K_LIST: c_int = 13;
    // 14 = ARKUI_NODE_LIST_ITEM, created inside the shim's list adapter (never via new_node here).

    fn new_node(kind: c_int) -> AHandle {
        AHandle(unsafe { ffi::day_ark_node_new(kind) })
    }

    /// The theme-adaptive pick: `light` under the light theme, `dark` under dark.
    fn theme_color(light: u32, dark: u32) -> u32 {
        if IS_DARK.with(|d| d.get()) {
            dark
        } else {
            light
        }
    }

    /// Set up the window root and density from the ArkTS host, before `launch_with`. Called by
    /// `day::arkui::start` (via the `day::arkui_main!` entry macro) with the `NodeContent` handle.
    #[allow(clippy::not_unsafe_ptr_arg_deref)] // `content` is a trusted NodeContent handle from ArkTS
    pub fn init(content: *mut c_void, w_vp: f64, h_vp: f64, density: f64) {
        DENSITY.with(|d| d.set(if density > 0.0 { density } else { 1.0 }));
        let dark = match std::env::var("DAY_THEME").ok().as_deref() {
            Some("dark") => true,
            Some("light") => false,
            _ => std::env::var("DAY_ARKUI_DARK").ok().as_deref() == Some("1"),
        };
        IS_DARK.with(|d| d.set(dark));
        unsafe { ffi::day_ark_init() };
        // Serve bundled data resources (§18.3) from the app's rawfile store. Registered once here;
        // the opener is a no-op until the ArkTS host hands us its resourceManager (see below).
        day_spec::resource::set_resource_opener(open_resource);
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
        // A NAV_MENU row click arrives with a synthetic id — translate it to a SelectionChanged
        // against the menu host before the normal per-node dispatch.
        if kind == 0
            && let Some((menu, index)) = MENU_ROWS.with(|m| m.borrow().get(&id).copied())
        {
            emit(menu, Event::SelectionChanged(index));
            return;
        }
        // A node with a registered Tap gesture emits `Event::Tap`, not `Event::Pressed`.
        if kind == 0 && TAP_NODES.with(|s| s.borrow().contains(&id)) {
            emit(NodeId(id), Event::Tap(Point::ZERO));
            return;
        }
        let node = NodeId(id);
        let ev = match kind {
            0 => Event::Pressed,
            // 6 = SelectionChanged (swiper tab / menu row), carried as the index in `num`.
            6 => Event::SelectionChanged(num as i64),
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

    /// Recycling-list row count, called from the NodeAdapter (docs/list.md).
    #[unsafe(no_mangle)]
    pub extern "C" fn day_arkui_list_count(host_id: u64) -> u32 {
        LIST_SOURCES.with(|m| {
            m.borrow()
                .get(&host_id)
                .map(|s| (s.len)() as u32)
                .unwrap_or(0)
        })
    }

    /// Build (or rebind) row `index`'s content into the native cell `cell` (an inner Stack). The
    /// adapter reuses cells, so a repeat `cell` pointer is a rebind (day-core keys its cell cache by
    /// the raw handle). Called on the JS/main thread from the adapter's add callback.
    #[unsafe(no_mangle)]
    #[allow(clippy::not_unsafe_ptr_arg_deref)] // `cell` is a live ArkUI_NodeHandle from the adapter
    pub extern "C" fn day_arkui_list_bind(host_id: u64, index: u32, cell: *mut c_void) {
        let source = LIST_SOURCES.with(|m| m.borrow().get(&host_id).cloned());
        if let Some(source) = source {
            (source.bind_row)(index as usize, cell as day_spec::RawHandle);
        }
    }

    /// A NavDestination disappeared on the ArkTS side (docs/navigation.md). For a pop DAY
    /// initiated (NavPatch::Popped) this is just the acknowledgement; for a NATIVE back
    /// (system gesture / title-bar back button) sync the route state: the toolkit already
    /// popped, so the host receives `NavBack { already_popped: true }`.
    #[unsafe(no_mangle)]
    pub extern "C" fn day_arkui_nav_popped(_key: u64) {
        NAV_DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
        let expected = NAV_EXPECT_POP.with(|e| {
            let v = e.get();
            if v > 0 {
                e.set(v - 1);
                true
            } else {
                false
            }
        });
        if !expected && let Some((host_id, _)) = NAV_HOST.with(|c| c.get()) {
            emit(
                NodeId(host_id),
                Event::NavBack {
                    already_popped: true,
                },
            );
        }
    }

    /// A destination's content area changed (vp): relayout that page in its real bounds.
    #[unsafe(no_mangle)]
    pub extern "C" fn day_arkui_nav_area(key: u64, w: f64, h: f64) {
        if w > 0.0 && h > 0.0 {
            emit(NodeId(key), Event::FrameChanged(Size::new(w, h)));
        }
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
                            if p.role == Some(day_spec::SurfaceRole::SectionCard) {
                                // A translucent neutral fill reads as a subtle card on BOTH the
                                // light and dark ArkUI themes (no public semantic-fill API).
                                ffi::day_ark_set_bg_color(
                                    n.0,
                                    theme_color(0x1480_8080, 0x2EFF_FFFF),
                                );
                            } else if let Some(c) = p.background {
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
                kinds::IMAGE => {
                    let p = props.downcast_ref::<ImageProps>().unwrap();
                    let n = new_node(K_IMAGE);
                    // Resolve `image("name")` through the app's rawfile store — the only resource
                    // root the OpenHarmony NDK can address from native code (app.media is ArkTS-only,
                    // §18.3). The CLI stages each image uncompressed to resources/rawfile/day/<name>
                    // normalized to PNG, so a bare `source` (no extension) maps to `day/<source>.png`.
                    let src = format!("resource://RAWFILE/day/{}.png", p.source);
                    unsafe { ffi::day_ark_set_image_src(n.0, cstr(&src).as_ptr()) };
                    // Scaling (§18.3): ArkUI_ObjectFit CONTAIN=0 (fit) / COVER=1 (fill) / FILL=3.
                    let fit = match p.content_mode {
                        ContentMode::Fit => 0,
                        ContentMode::Fill => 1,
                        ContentMode::Stretch => 3,
                    };
                    unsafe { ffi::day_ark_set_image_fit(n.0, fit) };
                    n
                }
                kinds::LABEL => {
                    let p = props.downcast_ref::<LabelProps>().unwrap();
                    let n = new_node(K_TEXT);
                    unsafe {
                        ffi::day_ark_set_text(n.0, cstr(&p.text).as_ptr());
                        ffi::day_ark_set_font_size(n.0, font_vp(p.font));
                        if let Some(c) = p.color {
                            ffi::day_ark_set_font_color(n.0, argb(c));
                        } else if IS_DARK.with(|d| d.get()) {
                            // Text defaults don't re-theme through the C API — give un-colored
                            // labels the dark theme's primary text color.
                            ffi::day_ark_set_font_color(n.0, 0xE6FF_FFFF);
                        }
                    }
                    apply_custom_family(n.0, p.font);
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
                // A 1-vp hairline: a thin Stack tinted with a faint separator colour.
                kinds::DIVIDER => {
                    let n = new_node(K_STACK);
                    unsafe {
                        ffi::day_ark_set_bg_color(n.0, theme_color(0x3300_0000, 0x33FF_FFFF))
                    };
                    n
                }
                // Determinate bar (ARKUI_NODE_PROGRESS) vs indeterminate spinner (LOADING_PROGRESS).
                kinds::PROGRESS => {
                    let p = props.downcast_ref::<ProgressProps>().unwrap();
                    match p.value {
                        Some(v) => {
                            let n = new_node(K_PROGRESS);
                            unsafe { ffi::day_ark_set_progress(n.0, v) };
                            n
                        }
                        None => new_node(K_LOADING),
                    }
                }
                // Navigation host + pages (docs/navigation.md): the host Stack shows the ROOT
                // page; every LATER page is re-homed into an ArkTS `NavDestination` (HarmonyOS's
                // own Navigation/NavPathStack) when its NavPatch::Pushed arrives — native push
                // transition, title bar, and system back gesture included. Pages carry an opaque
                // background so transitions don't bleed.
                kinds::NAV => {
                    let n = new_node(K_STACK);
                    NAV_HOST.with(|c| c.set(Some((id.0, n.0 as usize))));
                    n
                }
                kinds::NAV_PAGE => {
                    let n = new_node(K_STACK);
                    unsafe {
                        ffi::day_ark_set_bg_color(n.0, theme_color(0xFFFF_FFFF, 0xFF1A_1A1C))
                    };
                    NAV_PAGE_IDS.with(|m| m.borrow_mut().insert(n.0 as usize, id.0));
                    n
                }
                // A scrollable column of tappable rows; each row's tap becomes SelectionChanged(index)
                // against this menu host (via a synthetic click id, see day_arkui_on_event).
                kinds::NAV_MENU => {
                    let p = props.downcast_ref::<NavMenuProps>().unwrap();
                    build_nav_menu(id, &p.items)
                }
                // Tabs: a native Swiper pager (swipe + dot indicator). Each TABS_PAGE is a Swiper
                // child (the Swiper owns their horizontal layout, so their set_frame skips position).
                kinds::TABS => {
                    let p = props.downcast_ref::<TabsProps>().unwrap();
                    let n = new_node(K_SWIPER);
                    unsafe {
                        ffi::day_ark_swiper_setup(n.0);
                        ffi::day_ark_set_swiper_index(n.0, p.selected as c_int);
                        ffi::day_ark_register_event(n.0, 6, id.0);
                    }
                    n
                }
                kinds::TABS_PAGE => {
                    let n = new_node(K_STACK);
                    TABS_PAGES.with(|s| s.borrow_mut().insert(n.0 as usize));
                    n
                }
                // Canvas: a custom node whose on-draw callback replays the encoded display list.
                kinds::CANVAS => {
                    let n = new_node(K_CANVAS);
                    unsafe { ffi::day_ark_canvas_init(n.0) };
                    n
                }
                // Recycling list: an ARKUI_NODE_LIST driven by a NodeAdapter (attach_list injects the
                // row source; the adapter binds cells on demand). See attach_list / the adapter cbs.
                kinds::LIST => {
                    let p = props.downcast_ref::<ListProps>().unwrap();
                    let row_h = match p.row_height {
                        RowHeight::Uniform(h) => h,
                        RowHeight::Automatic => 0.0,
                    };
                    let n = new_node(K_LIST);
                    LIST_NODE.with(|m| m.borrow_mut().insert(n.0 as usize, id.0));
                    unsafe { ffi::day_ark_list_init(n.0, id.0, row_h) };
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
                // Navigation (docs/navigation.md): drive the ArkTS Navigation/NavPathStack.
                kinds::NAV => {
                    if let Some(p) = patch.downcast_ref::<NavPatch>() {
                        match p {
                            NavPatch::Pushed { title } => {
                                // The just-attached LAST page child becomes a NavDestination:
                                // detach it from the host Stack and mount it into the fresh
                                // NodeContent the ArkTS push callback returns.
                                let last = NAV_ATTACHED.with(|v| v.borrow().last().copied());
                                if let Some((page, key)) = last {
                                    unsafe {
                                        ffi::day_ark_remove_child(h.0, page as *mut _);
                                    }
                                    let rc = unsafe {
                                        ffi::day_ark_nav_push(
                                            page as *mut _,
                                            key,
                                            cstr(title).as_ptr(),
                                        )
                                    };
                                    if rc == 0 {
                                        NAV_PUSHED.with(|m| m.borrow_mut().insert(page, key));
                                        NAV_DEPTH.with(|d| d.set(d.get() + 1));
                                    } else {
                                        // No ArkTS bridge (old host page): fall back to the
                                        // stacked-children presentation.
                                        unsafe {
                                            ffi::day_ark_add_child(h.0, page as *mut _);
                                        }
                                    }
                                }
                            }
                            NavPatch::Popped => {
                                // Pop natively only if a destination is actually up and not
                                // already popped by a native back (the NavBack sync path).
                                let outstanding =
                                    NAV_DEPTH.with(|d| d.get()) > NAV_EXPECT_POP.with(|e| e.get());
                                if outstanding {
                                    NAV_EXPECT_POP.with(|e| e.set(e.get() + 1));
                                    unsafe { ffi::day_ark_nav_pop() };
                                }
                            }
                            NavPatch::Title(t) => unsafe {
                                ffi::day_ark_nav_set_title(cstr(t).as_ptr());
                            },
                        }
                    }
                }
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
                            LabelPatch::Font(f) => {
                                unsafe { ffi::day_ark_set_font_size(h.0, font_vp(*f)) };
                                apply_custom_family(h.0, *f);
                            }
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
                kinds::PROGRESS => {
                    if let Some(ProgressPatch::Value(Some(v))) =
                        patch.downcast_ref::<ProgressPatch>()
                    {
                        unsafe { ffi::day_ark_set_progress(h.0, *v) };
                    }
                }
                kinds::TABS => {
                    if let Some(TabsPatch::Selected(i)) = patch.downcast_ref::<TabsPatch>() {
                        unsafe { ffi::day_ark_set_swiper_index(h.0, *i as c_int) };
                    }
                }
                kinds::LIST => match patch.downcast_ref::<ListPatch>() {
                    Some(ListPatch::Reload) => unsafe { ffi::day_ark_list_reload(h.0) },
                    Some(ListPatch::ScrollToEnd) => unsafe { ffi::day_ark_list_scroll_to_end(h.0) },
                    _ => {}
                },
                _ => {}
            }
        }

        fn release(&mut self, h: AHandle) {
            let key = h.0 as usize;
            NAV_PAGE_IDS.with(|m| {
                m.borrow_mut().remove(&key);
            });
            SLIDER_RANGE.with(|m| {
                m.borrow_mut().remove(&key);
            });
            TABS_PAGES.with(|s| {
                s.borrow_mut().remove(&key);
            });
            if let Some(nid) = TAP_HANDLES.with(|m| m.borrow_mut().remove(&key)) {
                TAP_NODES.with(|s| {
                    s.borrow_mut().remove(&nid);
                });
            }
            if let Some(nid) = LIST_NODE.with(|m| m.borrow_mut().remove(&key)) {
                LIST_SOURCES.with(|m| {
                    m.borrow_mut().remove(&nid);
                });
            }
            unsafe { ffi::day_ark_node_dispose(h.0) };
        }

        fn insert(&mut self, parent: &AHandle, child: &AHandle, index: usize) {
            // Track page attachment order under the nav host: the next NavPatch::Pushed
            // re-homes the most recently attached page into a NavDestination.
            if NAV_HOST
                .with(|c| c.get())
                .is_some_and(|(_, hp)| hp == parent.0 as usize)
                && let Some(id) =
                    NAV_PAGE_IDS.with(|m| m.borrow().get(&(child.0 as usize)).copied())
            {
                NAV_ATTACHED.with(|v| v.borrow_mut().push((child.0 as usize, id)));
            }
            unsafe { ffi::day_ark_insert_child(parent.0, child.0, index as c_int) };
        }

        fn remove(&mut self, parent: &AHandle, child: &AHandle) {
            let cp = child.0 as usize;
            NAV_ATTACHED.with(|v| v.borrow_mut().retain(|(p, _)| *p != cp));
            if let Some(key) = NAV_PUSHED.with(|m| m.borrow_mut().remove(&cp)) {
                // The page lives in an ArkTS NodeContent (NavDestination), not under the host.
                unsafe { ffi::day_ark_nav_remove(key, child.0) };
                return;
            }
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
                kinds::DIVIDER => Size::new(p.width.unwrap_or(0.0), 1.0),
                kinds::PROGRESS => Size::new(p.width.unwrap_or(40.0), p.height.unwrap_or(20.0)),
                // These fill their container (host owns scroll/paging; content is laid out inside).
                kinds::NAV_MENU => Size::new(p.width.unwrap_or(240.0), p.height.unwrap_or(400.0)),
                kinds::LIST => Size::new(p.width.unwrap_or(0.0), p.height.unwrap_or(0.0)),
                _ => {
                    if let Some(measure) = self.registry.get(kind).and_then(|r| r.measure) {
                        return measure(self, h, p);
                    }
                    Size::new(p.width.unwrap_or(0.0), p.height.unwrap_or(0.0))
                }
            }
        }

        fn set_frame(&mut self, h: &AHandle, frame: Rect, _anim: Option<&AnimSpec>) {
            // A Swiper owns its pages' horizontal placement — size them, but don't position them
            // (a NODE_POSITION would fight the pager transform).
            if TABS_PAGES.with(|s| s.borrow().contains(&(h.0 as usize))) {
                unsafe { ffi::day_ark_set_size(h.0, frame.size.width, frame.size.height) };
                return;
            }
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

        fn set_a11y(&mut self, h: &AHandle, a11y: &A11yProps) {
            // The screen-reader label; `hidden`/`decorative` drop the node + subtree from the tree.
            let label = a11y.label.as_deref().unwrap_or("");
            let hidden = (a11y.hidden || a11y.decorative) as c_int;
            unsafe { ffi::day_ark_set_a11y(h.0, cstr(label).as_ptr(), hidden) };
        }

        fn enable_gesture(&mut self, h: &AHandle, node: NodeId, kind: GestureKind) {
            // Tap is a NODE_ON_CLICK that emits `Event::Tap` (tracked in TAP_NODES so the shared
            // click receiver knows to send Tap, not Pressed). Long-press / drag aren't wired on
            // ArkUI yet — a piece that needs them degrades to no gesture.
            if matches!(kind, GestureKind::Tap) {
                TAP_NODES.with(|s| s.borrow_mut().insert(node.0));
                TAP_HANDLES.with(|m| m.borrow_mut().insert(h.0 as usize, node.0));
                unsafe { ffi::day_ark_register_event(h.0, 0, node.0) };
            }
        }

        fn replay(&mut self, h: &AHandle, ops: &[DrawOp], _size: Size) {
            // Encode the display list the shared way (day-android uses the same encoder) and hand it
            // to the custom node; its on-draw callback replays it with OH_Drawing (§11).
            let (nums, texts) = day_spec::encode_ops(ops);
            let joined = cstr(&texts.join("\u{1f}"));
            unsafe {
                ffi::day_ark_set_canvas_ops(h.0, nums.as_ptr(), nums.len() as u32, joined.as_ptr())
            };
        }

        fn adopt(&mut self, raw: day_spec::RawHandle) -> AHandle {
            // A recycling LIST cell's inner Stack, created natively and handed back through the
            // adapter's bind callback — day mounts + rebinds the row's content into it.
            AHandle(raw)
        }

        fn attach_list(&mut self, host: &AHandle, source: day_spec::ListSource) {
            if let Some(nid) = LIST_NODE.with(|m| m.borrow().get(&(host.0 as usize)).copied()) {
                LIST_SOURCES.with(|m| m.borrow_mut().insert(nid, source));
            }
            unsafe { ffi::day_ark_list_reload(host.0) };
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

    /// Keeps a native rawfile view (an mmap region or heap copy) alive for a [`Resource`]'s lifetime,
    /// releasing it via the shim when dropped.
    struct ResGuard(*mut c_void);

    impl Drop for ResGuard {
        fn drop(&mut self) {
            unsafe { ffi::day_ark_res_close(self.0) };
        }
    }

    /// The rawfile-backed data-resource opener (§18.3), registered once in [`init`]. Serves
    /// `resource("numbers.bin")` from the app's `resources/rawfile/day/<name>` store via the
    /// OpenHarmony `OH_ResourceManager_*` API — zero-copy where the entry is mmap-able, else a copy.
    ///
    /// Returns `None` until the ArkTS entry ability has handed the native side its `resourceManager`
    /// (the shim's `registerResourceManager`); without it there is no `NativeResourceManager` to read
    /// through, so no data resources are available.
    fn open_resource(name: &str) -> Option<day_spec::resource::Resource> {
        if unsafe { ffi::day_ark_res_available() } == 0 {
            return None;
        }
        // OpenRawFile addresses entries relative to the rawfile root, so the lookup key for a staged
        // resource is `day/<name>` (the CLI stages data uncompressed under resources/rawfile/day/).
        let path = cstr(&format!("day/{name}"));
        let mut data: *const u8 = std::ptr::null();
        let mut len: usize = 0;
        let mut handle: *mut c_void = std::ptr::null_mut();
        let ok = unsafe { ffi::day_ark_res_open(path.as_ptr(), &mut data, &mut len, &mut handle) };
        if ok == 0 || data.is_null() {
            return None;
        }
        // Safety: `data`/`len` describe a valid immutable region owned by the native token `handle`;
        // `ResGuard` keeps it mapped until the `Resource` drops, then releases it via the shim.
        Some(unsafe {
            day_spec::resource::Resource::from_raw(data, len, Box::new(ResGuard(handle)))
        })
    }

    use std::any::Any;
}
