//! day-android — the android-widget backend (DESIGN.md §9). jni + the DayBridge Java shim
//! (java/dev/day/bridge/ — the Java analogue of the Qt C++ shim; framework widgets only, zero
//! AndroidX). `Handle = AHandle(GlobalRef)`. Coordinates: day works in dp; `set_frame` scales
//! by density to px and `measure` scales back. The JVM owns the main loop: `Platform::run`
//! hands the pre-registered root straight to `ready` (the Activity already called `init`).

#![allow(clippy::missing_safety_doc)]

#[cfg(target_os = "android")]
pub use imp::*;

#[cfg(target_os = "android")]
mod imp {
    pub use jni;

    use std::any::Any;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::sync::OnceLock;

    use jni::objects::{GlobalRef, JObject, JString, JValue};
    use jni::{JNIEnv, JavaVM};
    use linkme::distributed_slice;

    use day_spec::props::*;
    use day_spec::{
        A11yProps, AnimSpec, Cap, DrawOp, Event, EventSink, Font, NodeId, PieceKind, Platform,
        Proposal, Rect, Registry, Renderer, Size, Support, Toolkit, WindowOptions, kinds,
    };

    pub const BRIDGE: &str = "dev/day/bridge/DayBridge";

    #[derive(Clone)]
    pub struct AHandle(pub GlobalRef);

    static JAVA_VM: OnceLock<JavaVM> = OnceLock::new();
    /// GlobalRef to the DayBridge class: FindClass from spawned native threads uses the SYSTEM
    /// class loader and cannot see app classes — cache the class on the main thread at init.
    static BRIDGE_CLASS: OnceLock<GlobalRef> = OnceLock::new();

    thread_local! {
        static SINK: RefCell<Option<Rc<dyn Fn(NodeId, Event)>>> = const { RefCell::new(None) };
        static DENSITY: Cell<f64> = const { Cell::new(1.0) };
        static ROOT: RefCell<Option<(AHandle, Size)>> = const { RefCell::new(None) };
    }

    pub fn emit(id: NodeId, ev: Event) {
        let sink = SINK.with(|s| s.borrow().clone());
        if let Some(sink) = sink {
            sink(id, ev);
        }
    }

    fn density() -> f64 {
        DENSITY.with(|d| d.get())
    }

    /// Run with an attached JNIEnv (public: external renderers use this too).
    pub fn with_env<R>(f: impl FnOnce(&mut JNIEnv) -> R) -> R {
        let vm = JAVA_VM.get().expect("day-android: init() not called");
        let mut guard = vm.attach_current_thread().expect("attach_current_thread");
        f(&mut guard)
    }

    /// Call a DayBridge static returning a View, as a global ref (public helper).
    pub fn make_view(env: &mut JNIEnv, method: &str, sig: &str, args: &[JValue]) -> GlobalRef {
        let obj = env
            .call_static_method(BRIDGE, method, sig, args)
            .expect("DayBridge call")
            .l()
            .expect("View");
        env.new_global_ref(obj).expect("global ref")
    }

    fn call_void(method: &str, sig: &str, args: &[JValue]) {
        with_env(|env| {
            let _ = env.call_static_method(BRIDGE, method, sig, args);
        });
    }

    fn measure_call(h: &AHandle, method: &str) -> f64 {
        with_env(|env| {
            env.call_static_method(
                BRIDGE,
                method,
                "(Landroid/view/View;)I",
                &[JValue::Object(h.0.as_obj())],
            )
            .expect("measure")
            .i()
            .unwrap_or(0) as f64
        })
    }

    /// Initialize globals from the Activity's nativeStart (called by `day::android_start`).
    pub fn init(env: &mut JNIEnv, root: JObject, density_: f32, w: i32, h: i32) {
        if let Ok(vm) = env.get_java_vm() {
            let _ = JAVA_VM.set(vm);
        }
        if let Ok(cls) = env.find_class(BRIDGE) {
            if let Ok(global) = env.new_global_ref(cls) {
                let _ = BRIDGE_CLASS.set(global);
            }
        }
        let d = density_ as f64;
        DENSITY.with(|x| x.set(d));
        let handle = AHandle(env.new_global_ref(root).expect("root global ref"));
        let size = Size::new(w as f64 / d, h as f64 / d);
        ROOT.with(|r| *r.borrow_mut() = Some((handle, size)));
    }

    /// The single native trampoline (the app's `nativeOnEvent` forwards here).
    /// Kinds: 0=press 1=text 2=toggle 3=value 4=select.
    pub fn dispatch_event(env: &mut JNIEnv, id: i64, kind: i32, num: f64, jstr: &JString) {
        let ev = match kind {
            0 => Event::Pressed,
            1 => {
                let text = env
                    .get_string(jstr)
                    .ok()
                    .map(|s| s.into())
                    .unwrap_or_default();
                Event::TextChanged(text)
            }
            2 => Event::ToggleChanged(num != 0.0),
            3 => Event::ValueChanged(num),
            4 => Event::SelectionChanged(num as i64),
            _ => return,
        };
        emit(NodeId(id as u64), ev);
    }

    /// Posted-closure trampoline (the app's `nativeRunPosted` forwards here).
    pub fn run_posted(token: i64) {
        let f: Box<Box<dyn FnOnce() + Send>> =
            unsafe { Box::from_raw(token as *mut Box<dyn FnOnce() + Send>) };
        f();
    }

    #[distributed_slice]
    pub static RENDERERS: [fn() -> Renderer<Android>];

    pub struct Android {
        registry: Registry<Android>,
    }

    impl Android {
        pub fn new() -> Self {
            let mut registry = Registry::default();
            for f in RENDERERS {
                registry.register(f());
            }
            Android { registry }
        }
    }

    impl Default for Android {
        fn default() -> Self {
            Self::new()
        }
    }

    fn jstr(env: &mut JNIEnv, s: &str) -> jni::objects::JString<'static> {
        // SAFETY: local ref used immediately within the same JNI frame.
        unsafe { std::mem::transmute(env.new_string(s).expect("new_string")) }
    }

    fn font_params(f: Font) -> (f32, bool) {
        match f {
            Font::Title => (24.0, true),
            Font::Headline => (16.0, true),
            Font::Body => (14.0, false),
            Font::Caption => (12.0, false),
            Font::System(pt) => (pt as f32, false),
        }
    }

    impl Toolkit for Android {
        type Handle = AHandle;

        fn capability(&self, _cap: Cap) -> Support {
            Support::Unsupported
        }

        fn realize(&mut self, kind: PieceKind, props: &dyn Any, id: NodeId) -> AHandle {
            let idj = id.0 as i64;
            match kind {
                kinds::CONTAINER => with_env(|env| {
                    AHandle(make_view(
                        env,
                        "makeContainer",
                        "()Landroid/view/View;",
                        &[],
                    ))
                }),
                kinds::SCROLL => with_env(|env| {
                    AHandle(make_view(env, "makeScroll", "()Landroid/view/View;", &[]))
                }),
                kinds::LABEL => {
                    let p = props.downcast_ref::<LabelProps>().unwrap();
                    let (dip, bold) = font_params(p.font);
                    with_env(|env| {
                        let s = jstr(env, &p.text);
                        let view = make_view(
                            env,
                            "makeLabel",
                            "(Ljava/lang/String;)Landroid/view/View;",
                            &[JValue::Object(&s)],
                        );
                        let _ = env.call_static_method(
                            BRIDGE,
                            "setLabelFont",
                            "(Landroid/view/View;FZ)V",
                            &[
                                JValue::Object(view.as_obj()),
                                JValue::Float(dip),
                                JValue::Bool(bold as u8),
                            ],
                        );
                        AHandle(view)
                    })
                }
                kinds::BUTTON => {
                    let p = props.downcast_ref::<ButtonProps>().unwrap();
                    with_env(|env| {
                        let s = jstr(env, &p.title);
                        AHandle(make_view(
                            env,
                            "makeButton",
                            "(JLjava/lang/String;)Landroid/view/View;",
                            &[JValue::Long(idj), JValue::Object(&s)],
                        ))
                    })
                }
                kinds::TOGGLE => {
                    let p = props.downcast_ref::<ToggleProps>().unwrap();
                    with_env(|env| {
                        AHandle(make_view(
                            env,
                            "makeToggle",
                            "(JZ)Landroid/view/View;",
                            &[JValue::Long(idj), JValue::Bool(p.on as u8)],
                        ))
                    })
                }
                kinds::SLIDER => {
                    let p = props.downcast_ref::<SliderProps>().unwrap();
                    with_env(|env| {
                        AHandle(make_view(
                            env,
                            "makeSlider",
                            "(JDDD)Landroid/view/View;",
                            &[
                                JValue::Long(idj),
                                JValue::Double(p.value),
                                JValue::Double(p.min),
                                JValue::Double(p.max),
                            ],
                        ))
                    })
                }
                kinds::TEXT_FIELD => {
                    let p = props.downcast_ref::<TextFieldProps>().unwrap();
                    with_env(|env| {
                        let v = jstr(env, &p.text);
                        let ph = jstr(env, &p.placeholder);
                        AHandle(make_view(
                            env,
                            "makeTextField",
                            "(JLjava/lang/String;Ljava/lang/String;)Landroid/view/View;",
                            &[JValue::Long(idj), JValue::Object(&v), JValue::Object(&ph)],
                        ))
                    })
                }
                kinds::DIVIDER => with_env(|env| {
                    AHandle(make_view(env, "makeDivider", "()Landroid/view/View;", &[]))
                }),
                kinds::CANVAS => with_env(|env| {
                    AHandle(make_view(env, "makeCanvas", "()Landroid/view/View;", &[]))
                }),
                kinds::IMAGE => {
                    let p = props.downcast_ref::<ImageProps>().unwrap();
                    with_env(|env| {
                        let s = jstr(env, &p.source);
                        AHandle(make_view(
                            env,
                            "makeImage",
                            "(Ljava/lang/String;)Landroid/view/View;",
                            &[JValue::Object(&s)],
                        ))
                    })
                }
                _ => {
                    if let Some(make) = self.registry.get(kind).map(|r| r.make) {
                        return make(self, props, id);
                    }
                    with_env(|env| {
                        let s = jstr(env, &format!("⟨{kind}⟩"));
                        AHandle(make_view(
                            env,
                            "makeLabel",
                            "(Ljava/lang/String;)Landroid/view/View;",
                            &[JValue::Object(&s)],
                        ))
                    })
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
                kinds::LABEL => {
                    if let Some(p) = patch.downcast_ref::<LabelPatch>() {
                        match p {
                            LabelPatch::Text(t) => with_env(|env| {
                                let s = jstr(env, t);
                                let _ = env.call_static_method(
                                    BRIDGE,
                                    "setLabel",
                                    "(Landroid/view/View;Ljava/lang/String;)V",
                                    &[JValue::Object(h.0.as_obj()), JValue::Object(&s)],
                                );
                            }),
                            LabelPatch::Font(f) => {
                                let (dip, bold) = font_params(*f);
                                call_void(
                                    "setLabelFont",
                                    "(Landroid/view/View;FZ)V",
                                    &[
                                        JValue::Object(h.0.as_obj()),
                                        JValue::Float(dip),
                                        JValue::Bool(bold as u8),
                                    ],
                                );
                            }
                            LabelPatch::Color(_) => {}
                        }
                    }
                }
                kinds::BUTTON => {
                    if let Some(p) = patch.downcast_ref::<ButtonPatch>() {
                        match p {
                            ButtonPatch::Title(t) => with_env(|env| {
                                let s = jstr(env, t);
                                let _ = env.call_static_method(
                                    BRIDGE,
                                    "setLabel",
                                    "(Landroid/view/View;Ljava/lang/String;)V",
                                    &[JValue::Object(h.0.as_obj()), JValue::Object(&s)],
                                );
                            }),
                            ButtonPatch::Enabled(e) => call_void(
                                "setEnabled",
                                "(Landroid/view/View;Z)V",
                                &[JValue::Object(h.0.as_obj()), JValue::Bool(*e as u8)],
                            ),
                        }
                    }
                }
                kinds::TOGGLE => {
                    if let Some(p) = patch.downcast_ref::<TogglePatch>() {
                        match p {
                            TogglePatch::On(on) => call_void(
                                "setToggle",
                                "(Landroid/view/View;Z)V",
                                &[JValue::Object(h.0.as_obj()), JValue::Bool(*on as u8)],
                            ),
                            TogglePatch::Enabled(e) => call_void(
                                "setEnabled",
                                "(Landroid/view/View;Z)V",
                                &[JValue::Object(h.0.as_obj()), JValue::Bool(*e as u8)],
                            ),
                        }
                    }
                }
                kinds::SLIDER => {
                    if let Some(p) = patch.downcast_ref::<SliderPatch>() {
                        match p {
                            SliderPatch::Value(v) => call_void(
                                "setSlider",
                                "(Landroid/view/View;DD)V",
                                &[
                                    JValue::Object(h.0.as_obj()),
                                    JValue::Double(*v),
                                    JValue::Double(0.0), // min recovered from the widget tag
                                ],
                            ),
                            SliderPatch::Enabled(e) => call_void(
                                "setEnabled",
                                "(Landroid/view/View;Z)V",
                                &[JValue::Object(h.0.as_obj()), JValue::Bool(*e as u8)],
                            ),
                        }
                    }
                }
                kinds::TEXT_FIELD => {
                    if let Some(p) = patch.downcast_ref::<TextFieldPatch>() {
                        match p {
                            TextFieldPatch::Text { text, from_native } => {
                                if !*from_native {
                                    with_env(|env| {
                                        let s = jstr(env, text);
                                        let _ = env.call_static_method(
                                            BRIDGE,
                                            "setTextField",
                                            "(Landroid/view/View;Ljava/lang/String;)V",
                                            &[JValue::Object(h.0.as_obj()), JValue::Object(&s)],
                                        );
                                    });
                                }
                            }
                            TextFieldPatch::Placeholder(t) => with_env(|env| {
                                let s = jstr(env, t);
                                let _ = env.call_static_method(
                                    BRIDGE,
                                    "setPlaceholder",
                                    "(Landroid/view/View;Ljava/lang/String;)V",
                                    &[JValue::Object(h.0.as_obj()), JValue::Object(&s)],
                                );
                            }),
                            TextFieldPatch::Enabled(e) => call_void(
                                "setEnabled",
                                "(Landroid/view/View;Z)V",
                                &[JValue::Object(h.0.as_obj()), JValue::Bool(*e as u8)],
                            ),
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

        fn release(&mut self, h: AHandle) {
            call_void(
                "removeChild",
                "(Landroid/view/View;)V",
                &[JValue::Object(h.0.as_obj())],
            );
        }

        fn insert(&mut self, parent: &AHandle, child: &AHandle, _index: usize) {
            call_void(
                "addChild",
                "(Landroid/view/View;Landroid/view/View;)V",
                &[
                    JValue::Object(parent.0.as_obj()),
                    JValue::Object(child.0.as_obj()),
                ],
            );
        }

        fn remove(&mut self, _parent: &AHandle, child: &AHandle) {
            call_void(
                "removeChild",
                "(Landroid/view/View;)V",
                &[JValue::Object(child.0.as_obj())],
            );
        }

        fn move_child(&mut self, parent: &AHandle, child: &AHandle, _to: usize) {
            self.remove(parent, child);
            self.insert(parent, child, 0);
        }

        fn measure(&mut self, h: &AHandle, kind: PieceKind, p: Proposal) -> Size {
            let d = density();
            match kind {
                kinds::LABEL => {
                    let natural_w = measure_call(h, "measureWidth") / d;
                    match p.width {
                        Some(pw) if natural_w > pw => {
                            let wpx = (pw * d).round() as i32;
                            let hh = with_env(|env| {
                                env.call_static_method(
                                    BRIDGE,
                                    "measureHeightForWidth",
                                    "(Landroid/view/View;I)I",
                                    &[JValue::Object(h.0.as_obj()), JValue::Int(wpx)],
                                )
                                .expect("hfw")
                                .i()
                                .unwrap_or(0) as f64
                            });
                            Size::new(pw, hh / d)
                        }
                        _ => Size::new(natural_w, measure_call(h, "measureHeight") / d),
                    }
                }
                kinds::SLIDER => Size::new(
                    p.width.unwrap_or(180.0),
                    (measure_call(h, "measureHeight") / d).max(24.0),
                ),
                kinds::TEXT_FIELD => Size::new(
                    p.width.unwrap_or(180.0),
                    (measure_call(h, "measureHeight") / d).max(40.0),
                ),
                kinds::DIVIDER => Size::new(p.width.unwrap_or(0.0), 1.0),
                _ => {
                    if let Some(measure) = self.registry.get(kind).and_then(|r| r.measure) {
                        return measure(self, h, p);
                    }
                    Size::new(
                        measure_call(h, "measureWidth") / d,
                        measure_call(h, "measureHeight") / d,
                    )
                }
            }
        }

        fn set_frame(&mut self, h: &AHandle, frame: Rect, _anim: Option<&AnimSpec>) {
            let d = density();
            call_void(
                "setFrame",
                "(Landroid/view/View;IIII)V",
                &[
                    JValue::Object(h.0.as_obj()),
                    JValue::Int((frame.origin.x * d).round() as i32),
                    JValue::Int((frame.origin.y * d).round() as i32),
                    JValue::Int((frame.size.width * d).round() as i32),
                    JValue::Int((frame.size.height * d).round() as i32),
                ],
            );
        }

        fn set_scroll_content(&mut self, h: &AHandle, content: Size) {
            let d = density();
            call_void(
                "setScrollContent",
                "(Landroid/view/View;II)V",
                &[
                    JValue::Object(h.0.as_obj()),
                    JValue::Int((content.width * d).round() as i32),
                    JValue::Int((content.height * d).round() as i32),
                ],
            );
        }

        fn set_event_sink(&mut self, sink: EventSink) {
            SINK.with(|s| *s.borrow_mut() = Some(Rc::from(sink)));
        }

        fn set_a11y(&mut self, h: &AHandle, a11y: &A11yProps) {
            if let Some(label) = &a11y.label {
                with_env(|env| {
                    let s = jstr(env, label);
                    let _ = env.call_static_method(
                        BRIDGE,
                        "setA11y",
                        "(Landroid/view/View;Ljava/lang/String;)V",
                        &[JValue::Object(h.0.as_obj()), JValue::Object(&s)],
                    );
                });
            }
        }

        fn replay(&mut self, h: &AHandle, ops: &[DrawOp], _size: Size) {
            let (nums, texts) = day_spec::encode_ops(ops);
            with_env(|env| {
                let arr = env
                    .new_double_array(nums.len() as i32)
                    .expect("double array");
                env.set_double_array_region(&arr, 0, &nums)
                    .expect("fill array");
                let joined = jstr(env, &texts.join("\n"));
                let _ = env.call_static_method(
                    BRIDGE,
                    "setCanvasOps",
                    "(Landroid/view/View;[DLjava/lang/String;)V",
                    &[
                        JValue::Object(h.0.as_obj()),
                        JValue::Object(&arr),
                        JValue::Object(&joined),
                    ],
                );
            });
        }

        fn snapshot_window(&mut self) -> Result<Vec<u8>, String> {
            Err("use `adb exec-out screencap -p` (device-level capture) on android-widget".into())
        }
    }

    impl Platform for Android {
        const TARGET: &'static str = "android-widget";
        const TOOLKIT: &'static str = "widget";

        fn run(self, _options: WindowOptions, ready: Box<dyn FnOnce(Self, AHandle, Size)>) {
            // The ActivityThread owns the loop; init() already registered the root.
            let (root, size) = ROOT
                .with(|r| r.borrow_mut().take())
                .expect("day-android: init() not called");
            ready(self, root, size);
        }

        fn post(f: Box<dyn FnOnce() + Send>) {
            let token = Box::into_raw(Box::new(f)) as i64;
            let vm = JAVA_VM.get().expect("day-android: init() not called");
            let mut env = vm.attach_current_thread().expect("attach");
            let cls = BRIDGE_CLASS
                .get()
                .expect("day-android: bridge class not cached");
            let jcls: &jni::objects::JClass = cls.as_obj().into();
            let res = env.call_static_method(jcls, "postMain", "(J)V", &[JValue::Long(token)]);
            if res.is_err() {
                let _ = env.exception_describe();
                let _ = env.exception_clear();
            }
        }
    }
}
