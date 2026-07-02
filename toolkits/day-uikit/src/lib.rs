//! day-uikit — the ios-uikit backend (DESIGN.md §9). objc2, pure Rust, no shim.
//!
//! `Handle = Retained<UIView>`; UIKit is top-left/y-down so day frames apply directly. The app
//! boots via `UIApplicationMain` + a `define_class!` app delegate (pane's proven pattern: the
//! delegate class is force-registered before `UIApplicationMain`, and exposes `window`/
//! `setWindow:` for the no-scene-manifest compat path). iOS-only (`cfg(target_os = "ios")`);
//! host builds see an empty crate.

#![allow(unused_unsafe)]

#[cfg(target_os = "ios")]
pub use imp::*;

#[cfg(target_os = "ios")]
mod imp {
    use std::any::Any;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::ffi::{c_char, c_int};
    use std::ptr::NonNull;
    use std::rc::Rc;

    use linkme::distributed_slice;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, NSObjectProtocol};
    use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
    use objc2_core_foundation::{CGPoint, CGRect, CGSize};
    use objc2_foundation::{NSObject, NSString};
    // UIApplicationMain is "deprecated" in objc2 only as a rename to the private
    // `UIApplication::__main` binding; the classic entry point is what we want.
    #[allow(deprecated)]
    use objc2_ui_kit::UIApplicationMain;
    use objc2_ui_kit::{
        UIApplication, UIApplicationDelegate, UIButton, UIButtonType, UIColor, UIControl,
        UIControlEvents, UIControlState, UILabel, UIScreen, UIScrollView, UISlider, UISwitch,
        UITextBorderStyle, UITextField, UIView, UIViewController, UIWindow,
    };

    use day_spec::props::*;
    use day_spec::{
        A11yProps, AnimSpec, Cap, DrawOp, Event, EventSink, Font, NodeId, PieceKind, Platform,
        Proposal, Rect, Registry, Renderer, Size, Support, Toolkit, WindowOptions, kinds,
    };

    pub type Handle = Retained<UIView>;

    /// The day-core event sink (node-id keyed).
    type Sink = Rc<dyn Fn(NodeId, Event)>;

    thread_local! {
        static SINK: RefCell<Option<Sink>> = const { RefCell::new(None) };
        static TARGETS: RefCell<HashMap<usize, Retained<DayTarget>>> = RefCell::new(HashMap::new());
        static WINDOW: RefCell<Option<Retained<UIWindow>>> = const { RefCell::new(None) };
        #[allow(clippy::type_complexity)]
        static PENDING: RefCell<Option<(Uikit, WindowOptions, Box<dyn FnOnce(Uikit, Handle, Size)>)>> =
            RefCell::new(None);
    }

    pub fn emit(id: NodeId, ev: Event) {
        let sink = SINK.with(|s| s.borrow().clone());
        if let Some(sink) = sink {
            sink(id, ev);
        }
    }

    fn ptr_of(v: &UIView) -> usize {
        (v as *const UIView).cast::<()>() as usize
    }
    fn view_of<T: AsRef<UIView>>(x: Retained<T>) -> Handle {
        Retained::from(x.as_ref())
    }

    // -----------------------------------------------------------------------
    // DayTarget — target/action trampoline, node-id keyed
    // -----------------------------------------------------------------------

    struct TargetIvars {
        node: NodeId,
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "DayUIKitTarget"]
        #[ivars = TargetIvars]
        struct DayTarget;

        unsafe impl NSObjectProtocol for DayTarget {}

        impl DayTarget {
            #[unsafe(method(fire:))]
            fn fire(&self, sender: &UIControl) {
                let node = self.ivars().node;
                let obj: &AnyObject = sender.as_ref();
                if let Some(sw) = obj.downcast_ref::<UISwitch>() {
                    emit(node, Event::ToggleChanged(unsafe { sw.isOn() }));
                } else if let Some(sl) = obj.downcast_ref::<UISlider>() {
                    emit(node, Event::ValueChanged(unsafe { sl.value() } as f64));
                } else if let Some(tf) = obj.downcast_ref::<UITextField>() {
                    let s = unsafe { tf.text() }.map(|s| s.to_string()).unwrap_or_default();
                    emit(node, Event::TextChanged(s));
                } else {
                    emit(node, Event::Pressed);
                }
            }
        }
    );

    impl DayTarget {
        fn new(mtm: MainThreadMarker, node: NodeId) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(TargetIvars { node });
            unsafe { msg_send![super(this), init] }
        }
    }

    // -----------------------------------------------------------------------
    // DayCanvasView — replay in drawRect (§11)
    // -----------------------------------------------------------------------

    thread_local! {
        static OPS: RefCell<HashMap<usize, Vec<day_spec::DrawOp>>> = RefCell::new(HashMap::new());
    }

    struct CanvasIvars;

    define_class!(
        #[unsafe(super(UIView))]
        #[thread_kind = MainThreadOnly]
        #[name = "DayCanvasView"]
        #[ivars = CanvasIvars]
        struct DayCanvasView;

        impl DayCanvasView {
            #[unsafe(method(drawRect:))]
            fn draw_rect(&self, _dirty: CGRect) {
                let ptr = (self as *const DayCanvasView).cast::<UIView>() as usize;
                let ops = OPS.with(|m| m.borrow().get(&ptr).cloned()).unwrap_or_default();
                for op in &ops {
                    draw_op(op);
                }
            }
        }
    );

    impl DayCanvasView {
        fn new(mtm: MainThreadMarker) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(CanvasIvars);
            let v: Retained<Self> = unsafe { msg_send![super(this), init] };
            unsafe {
                v.setBackgroundColor(Some(&UIColor::clearColor()));
                v.setOpaque(false);
            }
            v
        }
    }

    fn uicolor(c: day_spec::Color) -> Retained<UIColor> {
        unsafe { UIColor::colorWithRed_green_blue_alpha(c.r, c.g, c.b, c.a) }
    }

    fn cg(r: day_spec::Rect) -> CGRect {
        CGRect::new(
            CGPoint::new(r.origin.x, r.origin.y),
            CGSize::new(r.size.width, r.size.height),
        )
    }

    fn draw_op(op: &day_spec::DrawOp) {
        use day_spec::DrawOp;
        unsafe {
            match op {
                DrawOp::Fill(shape, color) => {
                    uicolor(*color).setFill();
                    if let Some(p) = bezier(shape) {
                        p.fill();
                    }
                }
                DrawOp::Stroke(shape, color, width) => {
                    uicolor(*color).setStroke();
                    if let Some(p) = bezier(shape) {
                        p.setLineWidth(*width);
                        p.stroke();
                    }
                }
                DrawOp::Text {
                    text,
                    at,
                    size,
                    color,
                    centered,
                } => {
                    let font = objc2_ui_kit::UIFont::systemFontOfSize(*size);
                    let col = uicolor(*color);
                    let keys: [&NSString; 2] = [
                        objc2_ui_kit::NSFontAttributeName,
                        objc2_ui_kit::NSForegroundColorAttributeName,
                    ];
                    let objs: [&AnyObject; 2] =
                        [font.as_ref() as &AnyObject, col.as_ref() as &AnyObject];
                    let attrs =
                        objc2_foundation::NSDictionary::from_slices::<NSString>(&keys, &objs);
                    let ns = NSString::from_str(text);
                    let mut origin = CGPoint::new(at.x, at.y);
                    if *centered {
                        let sz: CGSize = msg_send![&ns, sizeWithAttributes: &*attrs];
                        origin.x -= sz.width / 2.0;
                        origin.y -= sz.height / 2.0;
                    }
                    let _: () = msg_send![&ns, drawAtPoint: origin, withAttributes: &*attrs];
                }
            }
        }
    }

    fn bezier(shape: &day_spec::Shape) -> Option<Retained<objc2_ui_kit::UIBezierPath>> {
        use day_spec::Shape;
        use objc2_ui_kit::UIBezierPath;
        unsafe {
            Some(match shape {
                Shape::Rect(r) => UIBezierPath::bezierPathWithRect(cg(*r)),
                Shape::RoundedRect(r, rad) => {
                    UIBezierPath::bezierPathWithRoundedRect_cornerRadius(cg(*r), *rad)
                }
                Shape::Ellipse(r) => UIBezierPath::bezierPathWithOvalInRect(cg(*r)),
                Shape::Arc {
                    rect,
                    start_deg,
                    sweep_deg,
                } => {
                    let center = CGPoint::new(
                        rect.origin.x + rect.size.width / 2.0,
                        rect.origin.y + rect.size.height / 2.0,
                    );
                    let radius = rect.size.width.min(rect.size.height) / 2.0;
                    UIBezierPath::bezierPathWithArcCenter_radius_startAngle_endAngle_clockwise(
                        center,
                        radius,
                        start_deg.to_radians(),
                        (start_deg + sweep_deg).to_radians(),
                        true,
                    )
                }
                Shape::Line(a, b) => {
                    let p = UIBezierPath::bezierPath();
                    p.moveToPoint(CGPoint::new(a.x, a.y));
                    p.addLineToPoint(CGPoint::new(b.x, b.y));
                    p
                }
                Shape::Polygon(_) => return None,
            })
        }
    }

    // -----------------------------------------------------------------------
    // The backend
    // -----------------------------------------------------------------------

    #[distributed_slice]
    pub static RENDERERS: [fn() -> Renderer<Uikit>];

    pub struct Uikit {
        registry: Registry<Uikit>,
    }

    impl Uikit {
        pub fn new() -> Self {
            let mut registry = Registry::default();
            for f in RENDERERS {
                registry.register(f());
            }
            Uikit { registry }
        }
    }

    impl Default for Uikit {
        fn default() -> Self {
            Self::new()
        }
    }

    fn mtm() -> MainThreadMarker {
        MainThreadMarker::new().expect("day-uikit: not on the main thread")
    }

    fn uifont_size(f: Font) -> (f64, bool) {
        match f {
            Font::Title => (28.0, true),
            Font::Headline => (17.0, true),
            Font::Body => (17.0, false),
            Font::Caption => (12.0, false),
            Font::System(pt) => (pt, false),
        }
    }

    fn apply_font(label: &UILabel, font: Font) {
        let (pt, bold) = uifont_size(font);
        unsafe {
            let f = if bold {
                objc2_ui_kit::UIFont::boldSystemFontOfSize(pt)
            } else {
                objc2_ui_kit::UIFont::systemFontOfSize(pt)
            };
            label.setFont(Some(&f));
        }
    }

    impl Toolkit for Uikit {
        type Handle = Handle;

        fn capability(&self, _cap: Cap) -> Support {
            Support::Unsupported
        }

        fn realize(&mut self, kind: PieceKind, props: &dyn Any, id: NodeId) -> Handle {
            let mtm = mtm();
            match kind {
                kinds::CONTAINER => view_of(unsafe { UIView::new(mtm) }),
                kinds::SCROLL => {
                    let sv = unsafe { UIScrollView::new(mtm) };
                    view_of(sv)
                }
                kinds::LABEL => {
                    let p = props.downcast_ref::<LabelProps>().unwrap();
                    let label = unsafe { UILabel::new(mtm) };
                    unsafe {
                        label.setText(Some(&NSString::from_str(&p.text)));
                        label.setNumberOfLines(0);
                    }
                    apply_font(&label, p.font);
                    view_of(label)
                }
                kinds::BUTTON => {
                    let p = props.downcast_ref::<ButtonProps>().unwrap();
                    let target = DayTarget::new(mtm, id);
                    let btn = unsafe { UIButton::buttonWithType(UIButtonType::System, mtm) };
                    unsafe {
                        btn.setTitle_forState(
                            Some(&NSString::from_str(&p.title)),
                            UIControlState::Normal,
                        );
                        let tobj: &AnyObject = target.as_ref();
                        btn.addTarget_action_forControlEvents(
                            Some(tobj),
                            sel!(fire:),
                            UIControlEvents::TouchUpInside,
                        );
                    }
                    let view = view_of(btn);
                    TARGETS.with(|m| m.borrow_mut().insert(ptr_of(&view), target));
                    view
                }
                kinds::TOGGLE => {
                    let p = props.downcast_ref::<ToggleProps>().unwrap();
                    let target = DayTarget::new(mtm, id);
                    let sw = unsafe { UISwitch::new(mtm) };
                    unsafe {
                        sw.setOn(p.on);
                        let tobj: &AnyObject = target.as_ref();
                        sw.addTarget_action_forControlEvents(
                            Some(tobj),
                            sel!(fire:),
                            UIControlEvents::ValueChanged,
                        );
                    }
                    let view = view_of(sw);
                    TARGETS.with(|m| m.borrow_mut().insert(ptr_of(&view), target));
                    view
                }
                kinds::SLIDER => {
                    let p = props.downcast_ref::<SliderProps>().unwrap();
                    let target = DayTarget::new(mtm, id);
                    let sl = unsafe { UISlider::new(mtm) };
                    unsafe {
                        sl.setMinimumValue(p.min as f32);
                        sl.setMaximumValue(p.max as f32);
                        sl.setValue(p.value as f32);
                        let tobj: &AnyObject = target.as_ref();
                        sl.addTarget_action_forControlEvents(
                            Some(tobj),
                            sel!(fire:),
                            UIControlEvents::ValueChanged,
                        );
                    }
                    let view = view_of(sl);
                    TARGETS.with(|m| m.borrow_mut().insert(ptr_of(&view), target));
                    view
                }
                kinds::TEXT_FIELD => {
                    let p = props.downcast_ref::<TextFieldProps>().unwrap();
                    let target = DayTarget::new(mtm, id);
                    let tf = unsafe { UITextField::new(mtm) };
                    unsafe {
                        tf.setText(Some(&NSString::from_str(&p.text)));
                        tf.setPlaceholder(Some(&NSString::from_str(&p.placeholder)));
                        tf.setBorderStyle(UITextBorderStyle::RoundedRect);
                        let tobj: &AnyObject = target.as_ref();
                        tf.addTarget_action_forControlEvents(
                            Some(tobj),
                            sel!(fire:),
                            UIControlEvents::EditingChanged,
                        );
                    }
                    let view = view_of(tf);
                    TARGETS.with(|m| m.borrow_mut().insert(ptr_of(&view), target));
                    view
                }
                kinds::DIVIDER => {
                    let v = unsafe { UIView::new(mtm) };
                    unsafe { v.setBackgroundColor(Some(&UIColor::separatorColor())) };
                    view_of(v)
                }
                kinds::CANVAS => view_of(DayCanvasView::new(mtm)),
                kinds::IMAGE => {
                    let p = props.downcast_ref::<ImageProps>().unwrap();
                    let iv = unsafe { objc2_ui_kit::UIImageView::new(mtm) };
                    // Bundle path: <app>/assets/<name> (staged by day xcode-backend).
                    let bundle = unsafe { objc2_foundation::NSBundle::mainBundle() };
                    if let Some(res) = unsafe { bundle.resourcePath() } {
                        let path = format!("{}/assets/{}", res, p.source);
                        if let Some(img) = unsafe {
                            objc2_ui_kit::UIImage::imageWithContentsOfFile(&NSString::from_str(
                                &path,
                            ))
                        } {
                            unsafe { iv.setImage(Some(&img)) };
                        }
                    }
                    view_of(iv)
                }
                _ => {
                    if let Some(make) = self.registry.get(kind).map(|r| r.make) {
                        return make(self, props, id);
                    }
                    let label = unsafe { UILabel::new(mtm) };
                    unsafe { label.setText(Some(&NSString::from_str(&format!("⟨{kind}⟩")))) };
                    view_of(label)
                }
            }
        }

        fn update(
            &mut self,
            h: &Handle,
            kind: PieceKind,
            patch: &dyn Any,
            _anim: Option<&AnimSpec>,
        ) {
            match kind {
                kinds::LABEL => {
                    if let (Some(p), Some(label)) = (
                        patch.downcast_ref::<LabelPatch>(),
                        (**h).downcast_ref::<UILabel>(),
                    ) {
                        match p {
                            LabelPatch::Text(t) => unsafe {
                                label.setText(Some(&NSString::from_str(t)))
                            },
                            LabelPatch::Font(f) => apply_font(label, *f),
                            LabelPatch::Color(_) => {}
                        }
                    }
                }
                kinds::BUTTON => {
                    if let (Some(p), Some(btn)) = (
                        patch.downcast_ref::<ButtonPatch>(),
                        (**h).downcast_ref::<UIButton>(),
                    ) {
                        match p {
                            ButtonPatch::Title(t) => unsafe {
                                btn.setTitle_forState(
                                    Some(&NSString::from_str(t)),
                                    UIControlState::Normal,
                                )
                            },
                            ButtonPatch::Enabled(e) => unsafe { btn.setEnabled(*e) },
                        }
                    }
                }
                kinds::TOGGLE => {
                    if let (Some(p), Some(sw)) = (
                        patch.downcast_ref::<TogglePatch>(),
                        (**h).downcast_ref::<UISwitch>(),
                    ) {
                        match p {
                            TogglePatch::On(on) => {
                                if unsafe { sw.isOn() } != *on {
                                    unsafe { sw.setOn(*on) };
                                }
                            }
                            TogglePatch::Enabled(e) => unsafe { sw.setEnabled(*e) },
                        }
                    }
                }
                kinds::SLIDER => {
                    if let (Some(p), Some(sl)) = (
                        patch.downcast_ref::<SliderPatch>(),
                        (**h).downcast_ref::<UISlider>(),
                    ) {
                        match p {
                            SliderPatch::Value(v) => {
                                if (unsafe { sl.value() } as f64 - v).abs() > 0.001 {
                                    unsafe { sl.setValue(*v as f32) };
                                }
                            }
                            SliderPatch::Enabled(e) => unsafe { sl.setEnabled(*e) },
                        }
                    }
                }
                kinds::TEXT_FIELD => {
                    if let (Some(p), Some(tf)) = (
                        patch.downcast_ref::<TextFieldPatch>(),
                        (**h).downcast_ref::<UITextField>(),
                    ) {
                        match p {
                            TextFieldPatch::Text { text, from_native } => {
                                let cur = unsafe { tf.text() }
                                    .map(|s| s.to_string())
                                    .unwrap_or_default();
                                if !*from_native && cur != *text {
                                    unsafe { tf.setText(Some(&NSString::from_str(text))) };
                                }
                            }
                            TextFieldPatch::Placeholder(t) => unsafe {
                                tf.setPlaceholder(Some(&NSString::from_str(t)))
                            },
                            TextFieldPatch::Enabled(e) => unsafe { tf.setEnabled(*e) },
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
            TARGETS.with(|m| {
                m.borrow_mut().remove(&ptr_of(&h));
            });
            unsafe { h.removeFromSuperview() };
        }

        fn insert(&mut self, parent: &Handle, child: &Handle, _index: usize) {
            unsafe { parent.addSubview(child) };
        }

        fn remove(&mut self, _parent: &Handle, child: &Handle) {
            unsafe { child.removeFromSuperview() };
        }

        fn move_child(&mut self, parent: &Handle, child: &Handle, _to: usize) {
            unsafe { parent.addSubview(child) };
        }

        fn measure(&mut self, h: &Handle, kind: PieceKind, p: Proposal) -> Size {
            let fit = |w: f64, hh: f64| {
                let s = unsafe { h.sizeThatFits(CGSize::new(w, hh)) };
                Size::new(s.width.ceil(), s.height.ceil())
            };
            match kind {
                kinds::LABEL => {
                    let w = p.width.unwrap_or(1.0e6);
                    let s = fit(w, 1.0e6);
                    Size::new(s.width.min(w), s.height)
                }
                kinds::BUTTON | kinds::TOGGLE => fit(1.0e6, 1.0e6),
                kinds::SLIDER => {
                    Size::new(p.width.unwrap_or(180.0), fit(1.0e6, 1.0e6).height.max(31.0))
                }
                kinds::TEXT_FIELD => {
                    Size::new(p.width.unwrap_or(180.0), fit(1.0e6, 1.0e6).height.max(34.0))
                }
                kinds::DIVIDER => Size::new(p.width.unwrap_or(0.0), 1.0),
                _ => {
                    if let Some(measure) = self.registry.get(kind).and_then(|r| r.measure) {
                        measure(self, h, p)
                    } else {
                        let s = fit(1.0e6, 1.0e6);
                        Size::new(p.width.unwrap_or(s.width), p.height.unwrap_or(s.height))
                    }
                }
            }
        }

        fn set_frame(&mut self, h: &Handle, frame: Rect, _anim: Option<&AnimSpec>) {
            let f = CGRect::new(
                CGPoint::new(frame.origin.x, frame.origin.y),
                CGSize::new(frame.size.width, frame.size.height),
            );
            unsafe { h.setFrame(f) };
        }

        fn set_scroll_content(&mut self, h: &Handle, content: Size) {
            if let Some(sv) = (**h).downcast_ref::<UIScrollView>() {
                unsafe { sv.setContentSize(CGSize::new(content.width, content.height)) };
            }
        }

        fn set_event_sink(&mut self, sink: EventSink) {
            SINK.with(|s| *s.borrow_mut() = Some(Rc::from(sink)));
        }

        fn set_a11y(&mut self, h: &Handle, a11y: &A11yProps) {
            unsafe {
                if let Some(id) = &a11y.identifier {
                    let ns = NSString::from_str(id);
                    let _: () = msg_send![&**h, setAccessibilityIdentifier: &*ns];
                }
                if let Some(label) = &a11y.label {
                    let ns = NSString::from_str(label);
                    let _: () = msg_send![&**h, setAccessibilityLabel: &*ns];
                }
            }
        }

        fn replay(&mut self, h: &Handle, ops: &[DrawOp], _size: Size) {
            OPS.with(|m| m.borrow_mut().insert(ptr_of(h), ops.to_vec()));
            unsafe { h.setNeedsDisplay() };
        }

        fn snapshot_window(&mut self) -> Result<Vec<u8>, String> {
            Err("use `simctl io booted screenshot` (device-level capture) on ios-uikit".into())
        }
    }

    // -----------------------------------------------------------------------
    // App delegate + Platform (UIApplicationMain)
    // -----------------------------------------------------------------------

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "DayAppDelegate"]
        struct AppDelegate;

        unsafe impl NSObjectProtocol for AppDelegate {}

        unsafe impl UIApplicationDelegate for AppDelegate {
            // The no-scene-manifest compat path reads `delegate.window` (pane's hard-won lesson).
            #[unsafe(method(window))]
            fn window(&self) -> *mut UIWindow {
                WINDOW.with(|w| {
                    w.borrow()
                        .as_ref()
                        .map(|r| &**r as *const UIWindow as *mut UIWindow)
                        .unwrap_or(std::ptr::null_mut())
                })
            }
            #[unsafe(method(setWindow:))]
            fn set_window(&self, window: *mut UIWindow) {
                let retained = unsafe { window.as_ref() }.map(Retained::from);
                WINDOW.with(|w| *w.borrow_mut() = retained);
            }

            // Classic (pre-UIScene) window setup: fine for day's single-window shell.
            #[allow(deprecated)]
            #[unsafe(method(application:didFinishLaunchingWithOptions:))]
            fn did_finish_launching(&self, _app: &UIApplication, _opts: *mut AnyObject) -> bool {
                let mtm = MainThreadMarker::new().unwrap();
                let bounds = UIScreen::mainScreen(mtm).bounds();
                let window = unsafe { UIWindow::initWithFrame(UIWindow::alloc(mtm), bounds) };
                let vc = unsafe { UIViewController::new(mtm) };
                let holder = unsafe { UIView::initWithFrame(UIView::alloc(mtm), bounds) };
                let root_view = unsafe { UIView::initWithFrame(UIView::alloc(mtm), bounds) };
                unsafe {
                    holder.setBackgroundColor(Some(&UIColor::systemBackgroundColor()));
                    holder.addSubview(&root_view);
                    vc.setView(Some(&holder));
                    window.setRootViewController(Some(&vc));
                    window.makeKeyAndVisible();
                }
                WINDOW.with(|w| *w.borrow_mut() = Some(window.clone()));

                // Safe area as root padding (§7.7 MVP): valid once the window is key.
                let insets = unsafe { window.safeAreaInsets() };
                let inner = CGRect::new(
                    CGPoint::new(insets.left, insets.top),
                    CGSize::new(
                        bounds.size.width - insets.left - insets.right,
                        bounds.size.height - insets.top - insets.bottom,
                    ),
                );
                unsafe { root_view.setFrame(inner) };

                let (backend, _options, ready) = PENDING
                    .with(|p| p.borrow_mut().take())
                    .expect("day-uikit: run() not called");
                let size = Size::new(inner.size.width, inner.size.height);
                ready(backend, view_of(root_view), size);
                true
            }
        }
    );

    impl Platform for Uikit {
        const TARGET: &'static str = "ios-uikit";
        const TOOLKIT: &'static str = "uikit";

        fn run(self, options: WindowOptions, ready: Box<dyn FnOnce(Self, Handle, Size)>) {
            PENDING.with(|p| *p.borrow_mut() = Some((self, options, ready)));
            // Force-register the delegate class: UIApplicationMain looks it up by name before
            // any Rust code touches it (pane's exact fix).
            let _ = <AppDelegate as objc2::ClassType>::class();
            let arg0 = c"day".as_ptr() as *mut c_char;
            let mut argv = [arg0];
            let argv_ptr = NonNull::new(argv.as_mut_ptr()).unwrap();
            let delegate = NSString::from_str("DayAppDelegate");
            #[allow(deprecated)]
            unsafe {
                UIApplicationMain(1 as c_int, argv_ptr, None, Some(&delegate));
            }
        }

        fn post(f: Box<dyn FnOnce() + Send>) {
            dispatch2::DispatchQueue::main().exec_async(f);
        }
    }
}
