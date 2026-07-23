//! day-mock — the headless toolkit (DESIGN.md §3.2, §21.2 M0–M1).
//!
//! Records every toolkit call into a compact op log (golden-diffable), performs deterministic
//! measurement (8pt/char × 16pt line labels, fixed control sizes), and lets tests inject
//! native events through the real sink. The op log is the contract for the fine-grained
//! guarantees: "exactly one op per state change" and "bounded measure calls" are assertions
//! over this log.

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use day_spec::props::*;
use day_spec::*;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct MockHandle(pub u64);

#[derive(Default, Debug, Clone)]
pub struct MockWidget {
    pub kind: &'static str,
    pub node: u64,
    pub text: String,
    pub placeholder: String,
    pub value: f64,
    pub flag: bool,
    pub enabled: bool,
    pub children: Vec<u64>,
    pub frame: Rect,
    pub a11y: A11yProps,
    pub scroll_content: Size,
    /// The scroll offset after the last `scroll_to` (docs/scroll.md), computed with the same
    /// minimal-reveal clamp every real backend applies — probe-visible for tests.
    pub scroll_offset: Point,
    pub ops: Vec<DrawOp>,
    /// Surface style from a `background`/`corner_radius` decorator (probe-visible for tests).
    pub background: Option<Color>,
    pub corner_radius: f64,
    pub clips: bool,
    /// Semantic theme-adaptive surface (a form section card) — probe-visible for tests.
    pub surface_role: Option<day_spec::SurfaceRole>,
    /// A label's resolved font spec (probe-visible so tests can assert e.g. `Font::Custom` flow).
    pub font: Option<day_spec::FontSpec>,
    /// Last focus state driven through the `focus` duty (docs/focus.md) — probe-visible.
    pub focused: bool,
    /// Last opacity applied via `set_opacity` (§8.4) — `None` until touched (probe-visible).
    pub opacity: Option<f64>,
    /// Last transform applied via `set_transform` (§8.4) — probe-visible.
    pub transform: Option<day_spec::Transform>,
    /// The most recent animation intent seen on ANY seam for this widget (`update`/`set_frame`/
    /// `set_opacity`/`set_transform`). Lets tests assert `with_animation` threaded the intent.
    pub last_anim: Option<AnimSpec>,
}

#[derive(Default)]
pub struct MockState {
    next: u64,
    pub widgets: HashMap<u64, MockWidget>,
    pub log: Vec<String>,
    pub sink: Option<EventSink>,
    /// (kind, proposal) measure-call counter for the M1 bounded-measure tests.
    pub measure_calls: usize,
    /// Recycling-list row-pull sources, keyed by LIST host handle (docs/list.md). A test drives
    /// the "viewport" through [`MockProbe::list_bind`], simulating what a native list would do.
    pub list_sources: HashMap<u64, ListSource>,
    /// The app menu as last applied (docs/menus.md) — item titles, probe-visible.
    pub app_menu: Vec<String>,
    /// Context menus by widget handle (docs/menus.md) — item titles per handle.
    pub context_menus: HashMap<u64, Vec<String>>,
}

impl MockState {
    fn log(&mut self, s: String) {
        self.log.push(s);
    }
}

/// The mock backend. Cloneable observer half: construct with [`MockToolkit::new`] and keep the
/// returned [`MockProbe`] to inspect state after day-core takes ownership of the toolkit.
pub struct MockToolkit {
    pub state: Rc<RefCell<MockState>>,
}

#[derive(Clone)]
pub struct MockProbe {
    pub state: Rc<RefCell<MockState>>,
}

impl MockToolkit {
    pub fn new() -> (Self, MockProbe) {
        let state = Rc::new(RefCell::new(MockState::default()));
        (
            MockToolkit {
                state: state.clone(),
            },
            MockProbe { state },
        )
    }
}

impl MockProbe {
    pub fn log(&self) -> Vec<String> {
        self.state.borrow().log.clone()
    }
    pub fn clear_log(&self) {
        let mut s = self.state.borrow_mut();
        s.log.clear();
        s.measure_calls = 0;
    }
    pub fn measure_calls(&self) -> usize {
        self.state.borrow().measure_calls
    }
    /// Ops excluding measures (mutation ops only).
    pub fn mutations(&self) -> Vec<String> {
        self.state
            .borrow()
            .log
            .iter()
            .filter(|l| !l.starts_with("measure"))
            .cloned()
            .collect()
    }
    pub fn widget(&self, h: MockHandle) -> MockWidget {
        self.state
            .borrow()
            .widgets
            .get(&h.0)
            .cloned()
            .unwrap_or_default()
    }
    pub fn find_by_kind(&self, kind: &str) -> Vec<(MockHandle, MockWidget)> {
        let mut v: Vec<_> = self
            .state
            .borrow()
            .widgets
            .iter()
            .filter(|(_, w)| w.kind == kind)
            .map(|(k, w)| (MockHandle(*k), w.clone()))
            .collect();
        v.sort_by_key(|(h, _)| h.0);
        v
    }
    /// Row count a `LIST` host would query from its data-source.
    pub fn list_len(&self, host: MockHandle) -> usize {
        let f = self
            .state
            .borrow()
            .list_sources
            .get(&host.0)
            .map(|s| s.len.clone());
        f.map(|f| f()).unwrap_or(0)
    }

    /// Simulate the native list binding row `index` into a physical `cell` — Day builds the row
    /// the first time a cell is used and rebinds (slot-write) when it is recycled. Drives the real
    /// day-core driver, so tests exercise the whole recycling path. (The source Rc is cloned out
    /// before the call so the re-entrant `with_tree`/toolkit work holds no MockState borrow.)
    pub fn list_bind(&self, host: MockHandle, index: usize, cell: MockHandle) {
        let f = self
            .state
            .borrow()
            .list_sources
            .get(&host.0)
            .map(|s| s.bind_row.clone());
        if let Some(f) = f {
            f(index, cell.0 as RawHandle);
        }
    }

    /// Inject a native event through the real sink (as the toolkit trampoline would).
    pub fn emit(&self, node: NodeId, event: Event) {
        let sink = self.state.borrow_mut().sink.take();
        if let Some(sink) = sink {
            sink(node, event);
            self.state.borrow_mut().sink.get_or_insert(sink);
        } else {
            panic!("day-mock: no event sink installed");
        }
    }

    /// The current op-log length — pair with [`Self::log_since`] to scope assertions.
    pub fn log_len(&self) -> usize {
        self.state.borrow().log.len()
    }

    /// The op-log entries recorded after `mark` (from [`Self::log_len`]).
    pub fn log_since(&self, mark: usize) -> Vec<String> {
        self.state.borrow().log[mark..].to_vec()
    }
}

fn fmt_size(s: Size) -> String {
    format!("{}x{}", s.width, s.height)
}
fn fmt_rect(r: Rect) -> String {
    format!(
        "({},{} {}x{})",
        r.origin.x, r.origin.y, r.size.width, r.size.height
    )
}

/// Deterministic text metrics: 8pt per char, 16pt line height, greedy wrap.
pub fn text_size(text: &str, proposal: Proposal, wraps: bool) -> Size {
    let needed = 8.0 * text.chars().count() as f64;
    match (proposal.width, wraps) {
        (Some(w), true) if needed > w && w > 0.0 => {
            let lines = (needed / w).ceil();
            Size::new(w, 16.0 * lines)
        }
        _ => Size::new(needed, 16.0),
    }
}

impl Toolkit for MockToolkit {
    type Handle = MockHandle;

    fn capability(&self, cap: Cap) -> Support {
        match cap {
            Cap::Snapshot => Support::Native,
            // The mock "runs" backend-executed animation by recording the intent (probe-visible).
            Cap::Animation => Support::Native,
            _ => Support::Unsupported,
        }
    }

    fn realize(&mut self, kind: PieceKind, props: &dyn Any, id: NodeId) -> MockHandle {
        let mut s = self.state.borrow_mut();
        s.next += 1;
        let h = s.next;
        let mut w = MockWidget {
            kind,
            node: id.0,
            enabled: true,
            ..Default::default()
        };
        let mut detail = String::new();
        if let Some(p) = props.downcast_ref::<LabelProps>() {
            w.text = p.text.clone();
            w.font = Some(p.font);
            detail = format!(" text={:?}", p.text);
        } else if let Some(p) = props.downcast_ref::<ButtonProps>() {
            w.text = p.title.clone();
            w.enabled = p.enabled;
            detail = format!(" title={:?}", p.title);
        } else if let Some(p) = props.downcast_ref::<ToggleProps>() {
            w.flag = p.on;
        } else if let Some(p) = props.downcast_ref::<SliderProps>() {
            w.value = p.value;
        } else if let Some(p) = props.downcast_ref::<TextFieldProps>() {
            w.text = p.text.clone();
            w.placeholder = p.placeholder.clone();
        } else if let Some(p) = props.downcast_ref::<CanvasProps>() {
            w.ops = p.ops.clone();
        } else if let Some(p) = props.downcast_ref::<ContainerProps>() {
            w.background = p.background;
            w.corner_radius = p.corner_radius;
            w.clips = p.clips;
            w.surface_role = p.role;
            if p.background.is_some() || p.corner_radius > 0.0 || p.clips {
                detail = format!(
                    " bg={:?} radius={} clips={}",
                    p.background, p.corner_radius, p.clips
                );
            }
        } else if let Some(p) = props.downcast_ref::<ProgressProps>() {
            // `flag` records indeterminate-ness; `value` the determinate fraction.
            w.flag = p.value.is_none();
            w.value = p.value.unwrap_or(0.0);
            detail = format!(" value={:?}", p.value);
        } else if let Some(p) = props.downcast_ref::<NavProps>() {
            w.text = p.title.clone();
            w.flag = p.split;
            detail = format!(" title={:?} split={}", p.title, p.split);
        } else if let Some(p) = props.downcast_ref::<NavPageProps>() {
            w.text = p.title.clone();
            w.flag = p.sidebar;
            detail = format!(" title={:?} sidebar={}", p.title, p.sidebar);
        } else if let Some(p) = props.downcast_ref::<NavMenuProps>() {
            w.text = p.items.join("|");
            w.value = p.selected.map(|i| i as f64).unwrap_or(-1.0);
            detail = format!(" items={:?} selected={:?}", p.items, p.selected);
        } else if let Some(p) = props.downcast_ref::<TabsProps>() {
            w.text = p.titles.join("|");
            w.value = p.selected as f64;
            detail = format!(" titles={:?} selected={}", p.titles, p.selected);
        } else if let Some(p) = props.downcast_ref::<TabsPageProps>() {
            w.text = p.title.clone();
            detail = format!(" title={:?}", p.title);
        } else if let Some(p) = props.downcast_ref::<PickerProps>() {
            w.text = p.options.join("|");
            w.value = p.selected as f64;
            detail = format!(" options={:?} selected={}", p.options, p.selected);
        } else if let Some(p) = props.downcast_ref::<TextAreaProps>() {
            w.text = p.text.clone();
            w.placeholder = p.placeholder.clone();
            detail = format!(" lines={}..{}", p.min_lines, p.max_lines);
        }
        s.log(format!("realize {kind} #{h}{detail}"));
        s.widgets.insert(h, w);
        MockHandle(h)
    }

    fn update(
        &mut self,
        h: &MockHandle,
        kind: PieceKind,
        patch: &dyn Any,
        anim: Option<&AnimSpec>,
    ) {
        let mut s = self.state.borrow_mut();
        let detail;
        {
            let w = s.widgets.get_mut(&h.0).expect("update on unknown widget");
            if anim.is_some() {
                w.last_anim = anim.copied();
            }
            detail = if let Some(p) = patch.downcast_ref::<LabelPatch>() {
                match p {
                    LabelPatch::Text(t) => {
                        w.text = t.clone();
                        format!("text={t:?}")
                    }
                    LabelPatch::Color(_) => "color".into(),
                    LabelPatch::Font(f) => {
                        w.font = Some(*f);
                        "font".into()
                    }
                }
            } else if let Some(p) = patch.downcast_ref::<ButtonPatch>() {
                match p {
                    ButtonPatch::Title(t) => {
                        w.text = t.clone();
                        format!("title={t:?}")
                    }
                    ButtonPatch::Enabled(e) => {
                        w.enabled = *e;
                        format!("enabled={e}")
                    }
                }
            } else if let Some(p) = patch.downcast_ref::<TogglePatch>() {
                match p {
                    TogglePatch::On(v) => {
                        w.flag = *v;
                        format!("on={v}")
                    }
                    TogglePatch::Enabled(e) => {
                        w.enabled = *e;
                        format!("enabled={e}")
                    }
                }
            } else if let Some(p) = patch.downcast_ref::<SliderPatch>() {
                match p {
                    SliderPatch::Value(v) => {
                        w.value = *v;
                        format!("value={v}")
                    }
                    SliderPatch::Enabled(e) => {
                        w.enabled = *e;
                        format!("enabled={e}")
                    }
                }
            } else if let Some(p) = patch.downcast_ref::<TextFieldPatch>() {
                match p {
                    TextFieldPatch::Text { text, from_native } => {
                        if !*from_native {
                            w.text = text.clone();
                        }
                        format!("text={text:?} from_native={from_native}")
                    }
                    TextFieldPatch::Placeholder(t) => {
                        w.placeholder = t.clone();
                        format!("placeholder={t:?}")
                    }
                    TextFieldPatch::Enabled(e) => {
                        w.enabled = *e;
                        format!("enabled={e}")
                    }
                }
            } else if let Some(p) = patch.downcast_ref::<CanvasProps>() {
                w.ops = p.ops.clone();
                format!("canvas ops={}", w.ops.len())
            } else if let Some(ProgressPatch::Value(v)) = patch.downcast_ref::<ProgressPatch>() {
                w.flag = v.is_none();
                w.value = v.unwrap_or(0.0);
                format!("value={v:?}")
            } else if let Some(PickerPatch::Selected(i)) = patch.downcast_ref::<PickerPatch>() {
                if let Some(w) = s.widgets.get_mut(&h.0) {
                    w.value = *i as f64;
                }
                format!("picker.selected {i}")
            } else if let Some(TextAreaPatch::SetText(t)) = patch.downcast_ref::<TextAreaPatch>() {
                if let Some(w) = s.widgets.get_mut(&h.0) {
                    w.text = t.clone();
                }
                format!("textarea.text {t:?}")
            } else if let Some(NavMenuPatch::Selected(sel)) = patch.downcast_ref::<NavMenuPatch>() {
                w.value = sel.map(|i| i as f64).unwrap_or(-1.0);
                format!("menu selected={sel:?}")
            } else if let Some(TabsPatch::Selected(i)) = patch.downcast_ref::<TabsPatch>() {
                w.value = *i as f64;
                format!("tab selected={i}")
            } else if let Some(p) = patch.downcast_ref::<NavPatch>() {
                match p {
                    NavPatch::Pushed { title } => {
                        w.text = title.clone();
                        format!("nav pushed title={title:?}")
                    }
                    NavPatch::Popped => "nav popped".into(),
                    NavPatch::Title(t) => {
                        w.text = t.clone();
                        format!("nav title={t:?}")
                    }
                }
            } else if let Some(ContainerPatch::Background(c)) =
                patch.downcast_ref::<ContainerPatch>()
            {
                w.background = *c;
                format!("bg={c:?}")
            } else if let Some(p) = patch.downcast_ref::<ListPatch>() {
                match p {
                    ListPatch::Reload => "list reload".into(),
                    ListPatch::RowSizeInvalidated(i) => format!("list row-size-invalidated {i}"),
                    ListPatch::ScrollToEnd => {
                        // Record that the host was asked to follow its last row (probe-visible).
                        w.flag = true;
                        "list scroll-to-end".into()
                    }
                }
            } else {
                "?".into()
            };
        }
        s.log(format!("update {kind} #{} {detail}", h.0));
    }

    fn release(&mut self, h: MockHandle) {
        let mut s = self.state.borrow_mut();
        s.widgets.remove(&h.0);
        s.log(format!("release #{}", h.0));
    }

    fn insert(&mut self, parent: &MockHandle, child: &MockHandle, index: usize) {
        let mut s = self.state.borrow_mut();
        {
            let p = s
                .widgets
                .get_mut(&parent.0)
                .expect("insert into unknown parent");
            let idx = index.min(p.children.len());
            p.children.insert(idx, child.0);
        }
        s.log(format!(
            "insert #{} into #{} at {}",
            child.0, parent.0, index
        ));
    }

    fn remove(&mut self, parent: &MockHandle, child: &MockHandle) {
        let mut s = self.state.borrow_mut();
        {
            let p = s
                .widgets
                .get_mut(&parent.0)
                .expect("remove from unknown parent");
            p.children.retain(|&c| c != child.0);
        }
        s.log(format!("remove #{} from #{}", child.0, parent.0));
    }

    fn move_child(&mut self, parent: &MockHandle, child: &MockHandle, to: usize) {
        let mut s = self.state.borrow_mut();
        {
            let p = s
                .widgets
                .get_mut(&parent.0)
                .expect("move in unknown parent");
            p.children.retain(|&c| c != child.0);
            let idx = to.min(p.children.len());
            p.children.insert(idx, child.0);
        }
        s.log(format!("move #{} in #{} to {}", child.0, parent.0, to));
    }

    fn measure(&mut self, h: &MockHandle, kind: PieceKind, p: Proposal) -> Size {
        let mut s = self.state.borrow_mut();
        s.measure_calls += 1;
        let w = s.widgets.get(&h.0).cloned().unwrap_or_default();
        let size = match kind {
            kinds::LABEL => text_size(&w.text, p, true),
            kinds::BUTTON => {
                let t = text_size(&w.text, Proposal::UNCONSTRAINED, false);
                Size::new(t.width + 16.0, 24.0)
            }
            kinds::TOGGLE => Size::new(51.0, 31.0),
            kinds::SLIDER => Size::new(p.width.unwrap_or(200.0), 24.0),
            kinds::TEXT_FIELD => Size::new(p.width.unwrap_or(200.0), 24.0),
            kinds::DIVIDER => Size::new(p.width.unwrap_or(0.0), 1.0),
            kinds::IMAGE => Size::new(32.0, 32.0),
            // Indeterminate spinner is a fixed square; determinate bar fills width.
            kinds::PROGRESS if w.flag => Size::new(20.0, 20.0),
            kinds::PROGRESS => Size::new(p.width.unwrap_or(200.0), 4.0),
            _ => Size::new(p.width.unwrap_or(10.0), p.height.unwrap_or(10.0)),
        };
        s.log(format!(
            "measure {kind} #{} {:?} -> {}",
            h.0,
            p.cache_key(),
            fmt_size(size)
        ));
        size
    }

    fn set_frame(&mut self, h: &MockHandle, frame: Rect, anim: Option<&AnimSpec>) {
        let mut s = self.state.borrow_mut();
        if let Some(w) = s.widgets.get_mut(&h.0) {
            w.frame = frame;
            if anim.is_some() {
                w.last_anim = anim.copied();
            }
        }
        let a = if anim.is_some() { " animated" } else { "" };
        s.log(format!("set_frame #{} {}{}", h.0, fmt_rect(frame), a));
    }

    fn set_opacity(&mut self, h: &MockHandle, opacity: f64, anim: Option<&AnimSpec>) {
        let mut s = self.state.borrow_mut();
        if let Some(w) = s.widgets.get_mut(&h.0) {
            w.opacity = Some(opacity);
            if anim.is_some() {
                w.last_anim = anim.copied();
            }
        }
        let a = if anim.is_some() { " animated" } else { "" };
        s.log(format!("set_opacity #{} {:.3}{}", h.0, opacity, a));
    }

    fn set_transform(&mut self, h: &MockHandle, t: day_spec::Transform, anim: Option<&AnimSpec>) {
        let mut s = self.state.borrow_mut();
        if let Some(w) = s.widgets.get_mut(&h.0) {
            w.transform = Some(t);
            if anim.is_some() {
                w.last_anim = anim.copied();
            }
        }
        let a = if anim.is_some() { " animated" } else { "" };
        s.log(format!(
            "set_transform #{} tx={:.1},ty={:.1},sx={:.2},sy={:.2},rot={:.1}{}",
            h.0, t.tx, t.ty, t.sx, t.sy, t.rotate_deg, a
        ));
    }

    fn set_scroll_content(&mut self, h: &MockHandle, content: Size) {
        let mut s = self.state.borrow_mut();
        if let Some(w) = s.widgets.get_mut(&h.0) {
            w.scroll_content = content;
        }
        s.log(format!("set_scroll_content #{} {}", h.0, fmt_size(content)));
    }

    fn scroll_to(&mut self, h: &MockHandle, target: Rect, _animated: bool) {
        let mut s = self.state.borrow_mut();
        if let Some(w) = s.widgets.get_mut(&h.0) {
            // Minimal scroll that makes `target` (content space) visible, clamped to range.
            let (vw, vh) = (w.frame.size.width, w.frame.size.height);
            let (cw, ch) = (w.scroll_content.width, w.scroll_content.height);
            let clamp = |cur: f64, lo: f64, hi_edge: f64, view: f64, content: f64| -> f64 {
                let mut o = cur;
                if hi_edge > o + view {
                    o = hi_edge - view;
                }
                if lo < o {
                    o = lo;
                }
                o.clamp(0.0, (content - view).max(0.0))
            };
            let o = w.scroll_offset;
            w.scroll_offset = Point::new(
                clamp(
                    o.x,
                    target.origin.x,
                    target.origin.x + target.size.width,
                    vw,
                    cw,
                ),
                clamp(
                    o.y,
                    target.origin.y,
                    target.origin.y + target.size.height,
                    vh,
                    ch,
                ),
            );
        }
        s.log(format!("scroll_to #{} {}", h.0, fmt_rect(target)));
    }

    fn scroll_offset(&mut self, h: &MockHandle) -> Point {
        self.state
            .borrow()
            .widgets
            .get(&h.0)
            .map(|w| w.scroll_offset)
            .unwrap_or(Point::ZERO)
    }

    fn enable_gesture(&mut self, h: &MockHandle, _node: NodeId, kind: GestureKind) {
        self.state
            .borrow_mut()
            .log(format!("enable_gesture #{} {:?}", h.0, kind));
    }

    fn focus(&mut self, h: &MockHandle, _node: NodeId, focused: bool) {
        let mut st = self.state.borrow_mut();
        st.log(format!("focus #{} {}", h.0, focused));
        if let Some(w) = st.widgets.get_mut(&h.0) {
            w.focused = focused;
        }
    }

    fn set_event_sink(&mut self, sink: EventSink) {
        self.state.borrow_mut().sink = Some(sink);
    }

    fn attach_list(&mut self, host: &MockHandle, source: ListSource) {
        let mut s = self.state.borrow_mut();
        s.list_sources.insert(host.0, source);
        s.log(format!("attach_list #{}", host.0));
    }

    fn adopt(&mut self, raw: RawHandle) -> MockHandle {
        // A recycling list's cell: register a container widget so row content can attach to it.
        let h = raw as u64;
        let mut s = self.state.borrow_mut();
        s.widgets.entry(h).or_insert_with(|| MockWidget {
            kind: kinds::LIST_CELL,
            node: h,
            enabled: true,
            ..Default::default()
        });
        MockHandle(h)
    }

    fn set_a11y(&mut self, h: &MockHandle, a11y: &A11yProps) {
        let mut s = self.state.borrow_mut();
        if let Some(w) = s.widgets.get_mut(&h.0) {
            w.a11y = a11y.clone();
        }
        s.log(format!("a11y #{} id={:?}", h.0, a11y.identifier));
    }

    fn replay(&mut self, h: &MockHandle, ops: &[DrawOp], size: Size) {
        let mut s = self.state.borrow_mut();
        if let Some(w) = s.widgets.get_mut(&h.0) {
            w.ops = ops.to_vec();
        }
        s.log(format!(
            "replay #{} {} ops in {}",
            h.0,
            ops.len(),
            fmt_size(size)
        ));
    }

    fn snapshot_window(&mut self) -> Result<Vec<u8>, String> {
        Ok(vec![0x89, b'P', b'N', b'G'])
    }

    fn present(&mut self, req: u64, spec: &day_spec::present::PresentSpec) {
        // No native UI; day-core's PENDING registry holds the spec. Log for op-log asserts;
        // tests answer via day_core::respond_presentation / pending_presentation.
        self.state
            .borrow_mut()
            .log(format!("present req={req} title={:?}", spec.title()));
    }

    fn dismiss(&mut self, req: u64) {
        self.state.borrow_mut().log(format!("dismiss req={req}"));
    }

    fn open_url(&mut self, url: &str) {
        // No browser to launch; record it so op-log assertions can verify a `link` fired.
        self.state.borrow_mut().log(format!("open_url {url}"));
    }

    // The remaining duties, implemented observably so mock stays a COMPLETE conformance probe
    // (a duty a piece exercises must never vanish into a trait default here).

    fn set_app_menu(&mut self, items: &[day_spec::MenuItem]) {
        let mut s = self.state.borrow_mut();
        s.app_menu = items.iter().map(menu_title).collect();
        s.log(format!("set_app_menu [{} items]", items.len()));
    }

    fn set_context_menu(&mut self, h: &MockHandle, _node: NodeId, items: &[day_spec::MenuItem]) {
        let mut s = self.state.borrow_mut();
        if items.is_empty() {
            s.context_menus.remove(&h.0);
        } else {
            s.context_menus
                .insert(h.0, items.iter().map(menu_title).collect());
        }
        s.log(format!("set_context_menu #{} [{} items]", h.0, items.len()));
    }

    fn supports_lifecycle(&self, _phase: day_spec::Lifecycle) -> bool {
        // Headless CI stands in for every platform: claim the full lifecycle so tests can
        // exercise mobile-only phases (day-core synthesizes delivery).
        true
    }

    fn read_a11y(&self, h: &MockHandle) -> day_spec::A11ySnapshot {
        // Echo what set_a11y recorded, so `a11y_audit` diffs cleanly against expectations.
        let s = self.state.borrow();
        let Some(w) = s.widgets.get(&h.0) else {
            return day_spec::A11ySnapshot::default();
        };
        day_spec::A11ySnapshot {
            found: true,
            role: w.a11y.role,
            label: w.a11y.label.clone(),
            value: w.a11y.value.clone(),
            identifier: w.a11y.identifier.clone(),
        }
    }

    fn ui_idle(&mut self) -> bool {
        // No native transitions exist; idle is immediate — but log the poll so scripted runs
        // can assert dayscript's settle path touched it.
        self.state.borrow_mut().log("ui_idle".into());
        true
    }

    fn on_suspend(&mut self) {
        self.state.borrow_mut().log("on_suspend".into());
    }

    fn on_resume(&mut self) {
        self.state.borrow_mut().log("on_resume".into());
    }

    fn on_memory_warning(&mut self) {
        self.state.borrow_mut().log("on_memory_warning".into());
    }
}

/// A menu item's display title (submenus render as their title; separators as "—").
fn menu_title(item: &day_spec::MenuItem) -> String {
    match item {
        day_spec::MenuItem::Action { label, .. } => label.clone(),
        day_spec::MenuItem::Submenu { label, .. } => label.clone(),
        day_spec::MenuItem::Separator => "—".into(),
    }
}

impl Platform for MockToolkit {
    const TARGET: &'static str = "mock-mock";
    const TOOLKIT: &'static str = "mock";

    fn run(mut self, options: WindowOptions, ready: Box<dyn FnOnce(Self, MockHandle, Size)>) {
        // No native loop: create the root container, hand off, return. Tests drive via
        // MockProbe::emit + day_reactive::flush_sync.
        let root = self.realize(kinds::CONTAINER, &ContainerProps::default(), NodeId(0));
        ready(self, root, options.size);
    }

    fn post(f: Box<dyn FnOnce() + Send>) {
        // No loop to defer to: run immediately (tests are synchronous).
        f();
    }
}
