// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Tests that run inside a built Day app (docs/testing.md): a [`Case`] is a page plus a drive,
//! or a headless run, declared next to the code it tests with `#[day::test]` and collected in a
//! registry. The dayscript engine runs them on a real toolkit under `day test`; the mock
//! harness runs the same functions under `cargo test`. Nothing here knows how a step is
//! executed: a [`DriveBackend`] does, and the engine supplies one.
//!
//! A test's name is its function's, with hyphens for underscores (`button_status` runs as
//! `button-status`): the registry entry carries the identifier, so a tool reading the source
//! (`day test --list`, an editor) and the app agree without either one running the other.

use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

use day_spec::Cap;

use crate::AnyPiece;

/// A failed assertion, with the sentence the report shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fail(pub String);

impl std::fmt::Display for Fail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for Fail {
    fn from(s: String) -> Self {
        Fail(s)
    }
}

impl From<&str> for Fail {
    fn from(s: &str) -> Self {
        Fail(s.to_owned())
    }
}

/// What a test's body answers.
pub type TestResult = Result<(), Fail>;

/// What a drive asks of the app, in dayscript's vocabulary. The engine maps each to the step
/// a script would send, so a drive and a script mean the same thing by the same words.
#[derive(Clone, Debug, PartialEq)]
pub enum DriveOp {
    Tap(String),
    Input(String, String),
    Toggle(String, bool),
    SetValue(String, f64),
    Select(String, usize),
    Focus(String),
    Submit(String),
    Navigate(String),
    WaitIdle,
    Pause(f64),
    /// Capture the window under this name into the test's record.
    Shot(String),
    AssertText(String, String),
    AssertVisible(String),
    AssertMissing(String),
    AssertValue(String, f64),
    /// A toggle's state.
    AssertOn(String, bool),
    AssertFocused(String, bool),
    AssertRoute(String),
    A11yAudit(Option<String>),
}

/// The future one drive op resolves to.
pub type OpFuture = Pin<Box<dyn Future<Output = TestResult>>>;

/// Executes [`DriveOp`]s. The engine's backend runs each as the matching dayscript step on the
/// main thread and waits, asynchronously, through the step's retry window; the mock harness
/// uses the same backend against the mock toolkit.
pub trait DriveBackend {
    fn run(&self, op: DriveOp) -> OpFuture;
}

/// The handle a test drives the app with. Every method awaits the op and fails the test with
/// the step's own message when the op fails after its retry window.
#[derive(Clone)]
pub struct Drive {
    backend: Rc<dyn DriveBackend>,
}

impl Drive {
    /// A drive over `backend`; the engine and the mock harness build one per test.
    pub fn new(backend: Rc<dyn DriveBackend>) -> Self {
        Drive { backend }
    }

    fn op(&self, op: DriveOp) -> OpFuture {
        self.backend.run(op)
    }

    pub fn tap(&self, id: &str) -> OpFuture {
        self.op(DriveOp::Tap(id.into()))
    }
    /// Type `text` into the field, replacing what it holds.
    pub fn input(&self, id: &str, text: &str) -> OpFuture {
        self.op(DriveOp::Input(id.into(), text.into()))
    }
    pub fn toggle(&self, id: &str, on: bool) -> OpFuture {
        self.op(DriveOp::Toggle(id.into(), on))
    }
    pub fn set_value(&self, id: &str, value: f64) -> OpFuture {
        self.op(DriveOp::SetValue(id.into(), value))
    }
    pub fn select(&self, id: &str, index: usize) -> OpFuture {
        self.op(DriveOp::Select(id.into(), index))
    }
    pub fn focus(&self, id: &str) -> OpFuture {
        self.op(DriveOp::Focus(id.into()))
    }
    pub fn submit(&self, id: &str) -> OpFuture {
        self.op(DriveOp::Submit(id.into()))
    }
    pub fn navigate(&self, route: &str) -> OpFuture {
        self.op(DriveOp::Navigate(route.into()))
    }
    /// Let native transitions settle.
    pub fn wait_idle(&self) -> OpFuture {
        self.op(DriveOp::WaitIdle)
    }
    pub fn pause(&self, secs: f64) -> OpFuture {
        self.op(DriveOp::Pause(secs))
    }
    /// Capture the window as `name`; the capture rides the test's record to the runner.
    pub fn shot(&self, name: &str) -> OpFuture {
        self.op(DriveOp::Shot(name.into()))
    }
    pub fn assert_text(&self, id: &str, text: &str) -> OpFuture {
        self.op(DriveOp::AssertText(id.into(), text.into()))
    }
    pub fn assert_visible(&self, id: &str) -> OpFuture {
        self.op(DriveOp::AssertVisible(id.into()))
    }
    pub fn assert_missing(&self, id: &str) -> OpFuture {
        self.op(DriveOp::AssertMissing(id.into()))
    }
    pub fn assert_value(&self, id: &str, value: f64) -> OpFuture {
        self.op(DriveOp::AssertValue(id.into(), value))
    }
    /// A toggle's on/off state.
    pub fn assert_on(&self, id: &str, on: bool) -> OpFuture {
        self.op(DriveOp::AssertOn(id.into(), on))
    }
    pub fn assert_focused(&self, id: &str, focused: bool) -> OpFuture {
        self.op(DriveOp::AssertFocused(id.into(), focused))
    }
    pub fn assert_route(&self, route: &str) -> OpFuture {
        self.op(DriveOp::AssertRoute(route.into()))
    }
    /// Diff the native accessibility tree against Day's expectation for `id` (all id'd
    /// nodes when `None`), where the toolkit can read its tree back.
    pub fn a11y_audit(&self, id: Option<&str>) -> OpFuture {
        self.op(DriveOp::A11yAudit(id.map(str::to_owned)))
    }

    /// A plain assertion for a headless test: fails with `what` when `ok` is false.
    pub fn check(&self, ok: bool, what: &str) -> TestResult {
        if ok { Ok(()) } else { Err(Fail(what.into())) }
    }
    /// Equality for a headless test, naming both sides on a mismatch.
    pub fn check_eq<T: PartialEq + std::fmt::Debug>(&self, got: T, want: T) -> TestResult {
        if got == want {
            Ok(())
        } else {
            Err(Fail(format!("{got:?} ≠ expected {want:?}")))
        }
    }
}

/// Whether a case shows a page and drives it, or runs with no page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TestKind {
    Gui,
    Headless,
}

type Body = Rc<dyn Fn(Drive) -> Pin<Box<dyn Future<Output = TestResult>>>>;

/// One test: what it proves, what it needs, the page it shows and the body that drives it.
/// Built by a `#[day::test]` function each time the test runs, so the signals a page and its
/// drive share are fresh per run; the registry names it after that function.
#[derive(Clone)]
pub struct Case {
    name: String,
    kind: TestKind,
    proves: Vec<String>,
    requires: Vec<Cap>,
    page: Option<Rc<dyn Fn() -> AnyPiece>>,
    shots: Vec<String>,
    body: Option<Body>,
    timeout_secs: Option<f64>,
}

impl Default for Case {
    fn default() -> Self {
        Case::new()
    }
}

impl Case {
    /// A GUI case: a page the app's test host shows while the case runs, and a drive against it.
    pub fn new() -> Self {
        Case {
            name: String::new(),
            kind: TestKind::Gui,
            proves: Vec::new(),
            requires: Vec::new(),
            page: None,
            shots: Vec::new(),
            body: None,
            timeout_secs: None,
        }
    }
    /// A test with no page: app logic run inside the app's own environment.
    pub fn headless() -> Self {
        Case {
            kind: TestKind::Headless,
            ..Case::new()
        }
    }
    /// How long the case may take, page and drive together, before it fails as timed out.
    /// Unset, the run's default applies (30 s, or what `day test --case-timeout` says).
    pub fn timeout(mut self, secs: f64) -> Self {
        self.timeout_secs = Some(secs);
        self
    }
    /// The piece kind this case is the proof of (`kinds::BUTTON`); a pass marks the kind
    /// on this toolkit in the coverage tables.
    pub fn proves(mut self, kind: &str) -> Self {
        self.proves.push(format!("kind:{kind}"));
        self
    }
    /// The capability this case proves.
    pub fn proves_cap(mut self, cap: Cap) -> Self {
        self.proves.push(format!("cap:{cap:?}"));
        self
    }
    /// The toolkit duty this case proves (`"set_input_traits"`).
    pub fn proves_duty(mut self, duty: &str) -> Self {
        self.proves.push(format!("duty:{duty}"));
        self
    }
    /// A capability the case needs; where the toolkit answers `Unsupported` the case is
    /// skipped with that reason rather than failed. The one way a case adapts to a toolkit.
    pub fn requires(mut self, cap: Cap) -> Self {
        self.requires.push(cap);
        self
    }
    /// The page the app shows for this case; any piece.
    pub fn page<P: crate::Piece>(mut self, page: impl Fn() -> P + 'static) -> Self {
        self.page = Some(Rc::new(move || AnyPiece::new(page())));
        self
    }
    /// A capture taken before the drive runs, once the page is shown.
    pub fn shot(mut self, name: &str) -> Self {
        self.shots.push(name.into());
        self
    }
    /// The drive: an async body over a [`Drive`], awaiting each step so the app's main loop
    /// runs between them (a transition settles, a capture paints) as it does between a
    /// script's steps.
    pub fn drive<F>(mut self, body: impl Fn(Drive) -> F + 'static) -> Self
    where
        F: Future<Output = TestResult> + 'static,
    {
        self.body = Some(Rc::new(move |d| Box::pin(body(d))));
        self
    }
    /// [`Case::drive`] under the name a headless test reads better with.
    pub fn run<F>(self, body: impl Fn(Drive) -> F + 'static) -> Self
    where
        F: Future<Output = TestResult> + 'static,
    {
        self.drive(body)
    }

    /// The name the registry gave the case: its function's, hyphenated. Empty for a case
    /// built outside the registry.
    pub fn name(&self) -> &str {
        &self.name
    }
    /// The case's own time limit, if it set one.
    pub fn timeout_secs(&self) -> Option<f64> {
        self.timeout_secs
    }
    pub fn kind(&self) -> TestKind {
        self.kind
    }
    /// The `kind:`, `cap:` and `duty:` keys this case proves, in the matrices' own spelling.
    pub fn proves_keys(&self) -> &[String] {
        &self.proves
    }
    pub fn requirements(&self) -> &[Cap] {
        &self.requires
    }
    /// The captures to take before the drive.
    pub fn shots(&self) -> &[String] {
        &self.shots
    }
    /// Build the case's page, if it has one.
    pub fn build_page(&self) -> Option<AnyPiece> {
        self.page.as_ref().map(|p| p())
    }
    /// Run the body over `drive`; a case with no body passes once its page showed.
    pub fn run_body(&self, drive: Drive) -> Pin<Box<dyn Future<Output = TestResult>>> {
        match &self.body {
            Some(body) => body(drive),
            None => Box::pin(async { Ok(()) }),
        }
    }
}

/// A registered test: the function that builds its [`Case`], and that function's identifier,
/// which is where the test's name comes from. What `#[day::test]` and the [`tests!`] roster
/// both register.
#[derive(Clone, Copy)]
pub struct TestFn {
    ident: &'static str,
    build: fn() -> Case,
}

impl TestFn {
    /// An entry for `build`, named by `ident` (`stringify!` of the function or its path).
    pub const fn new(ident: &'static str, build: fn() -> Case) -> Self {
        TestFn { ident, build }
    }
    /// The test's name: the identifier's last path segment, with hyphens for underscores.
    pub fn name(&self) -> String {
        let last = self.ident.rsplit("::").next().unwrap_or(self.ident);
        last.trim().replace('_', "-")
    }
    /// The function that builds the case.
    pub fn build_fn(&self) -> fn() -> Case {
        self.build
    }
    /// A fresh case, named.
    pub fn case(&self) -> Case {
        let mut case = (self.build)();
        case.name = self.name();
        case
    }
}

/// Every `#[day::test]` in the binary, collected at link time. Not on wasm: `linkme` has no
/// `wasm32-unknown-unknown` support (the renderer registry has the same gap), so there a
/// crate registers its tests at run time with [`register`] through the [`tests!`] roster.
#[cfg(not(target_arch = "wasm32"))]
#[linkme::distributed_slice]
pub static TESTS: [TestFn];

/// What a test host does when the active case changes: rebuild its content.
type HostNotify = Rc<dyn Fn()>;

thread_local! {
    static RUNTIME: RefCell<Vec<TestFn>> = const { RefCell::new(Vec::new()) };
    /// The case the engine is driving, which the test host shows; its page shares its signals
    /// with the drive.
    static ACTIVE: RefCell<Option<Case>> = const { RefCell::new(None) };
    /// The mounted test host's rebuild, when the app shows one (`day::test_host`).
    static HOST: RefCell<Option<HostNotify>> = const { RefCell::new(None) };
    /// A panic raised while the host built the active case's page, for the runner to report.
    static PAGE_PANIC: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Register a test at run time: what the [`tests!`] roster expands to on wasm, where the
/// link-time slice does not exist. Also how a harness adds a case of its own.
pub fn register(test: TestFn) {
    RUNTIME.with(|r| {
        let mut r = r.borrow_mut();
        if !r.iter().any(|t| std::ptr::fn_addr_eq(t.build, test.build)) {
            r.push(test);
        }
    });
}

/// Every registered test, link-time and run-time ones together.
pub fn tests() -> Vec<TestFn> {
    let mut all: Vec<TestFn> = Vec::new();
    #[cfg(not(target_arch = "wasm32"))]
    all.extend(TESTS.iter().copied());
    RUNTIME.with(|r| {
        for t in r.borrow().iter() {
            if !all.iter().any(|a| std::ptr::fn_addr_eq(a.build, t.build)) {
                all.push(*t);
            }
        }
    });
    all
}

/// Every registered test as a fresh, named [`Case`], sorted by name.
pub fn cases() -> Vec<Case> {
    let mut cases: Vec<Case> = tests().iter().map(TestFn::case).collect();
    cases.sort_by(|a, b| a.name.cmp(&b.name));
    cases
}

/// Show `case` in the app's test host, built fresh, until the next call or [`clear_active`].
/// Answers false when the app mounts no test host, so the runner can say so.
pub fn set_active(case: Case) -> bool {
    ACTIVE.with(|a| *a.borrow_mut() = Some(case));
    PAGE_PANIC.with(|p| *p.borrow_mut() = None);
    notify_host()
}

/// Return the test host to the app's own content.
pub fn clear_active() {
    ACTIVE.with(|a| *a.borrow_mut() = None);
    notify_host();
}

fn notify_host() -> bool {
    let host = HOST.with(|h| h.borrow().clone());
    match host {
        Some(rebuild) => {
            rebuild();
            true
        }
        None => false,
    }
}

/// The active case, for the test host to build.
pub fn active() -> Option<Case> {
    ACTIVE.with(|a| a.borrow().clone())
}

/// Install the mounted test host's rebuild (`day::test_host` does, for the life of its
/// scope); `None` when it goes.
pub fn set_host(rebuild: Option<Rc<dyn Fn()>>) {
    HOST.with(|h| *h.borrow_mut() = rebuild);
}

/// Record a panic the test host caught while building the active case's page.
pub fn note_page_panic(message: String) {
    PAGE_PANIC.with(|p| *p.borrow_mut() = Some(message));
}

/// The panic, if any, the active case's page raised as it was built.
pub fn take_page_panic() -> Option<String> {
    PAGE_PANIC.with(|p| p.borrow_mut().take())
}

/// The sentence a caught panic's payload reads as.
pub fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_owned()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "a panic with no message".to_owned()
    }
}

/// A fresh case for the test called `name`, for a person browsing an app's tests (the
/// conformance app's navigation does).
pub fn case_named(name: &str) -> Option<Case> {
    tests()
        .into_iter()
        .find(|t| t.name() == name)
        .map(|t| t.case())
}

/// The roster of a crate's tests, for the targets with no link-time registry (wasm): expands
/// to a `register_tests()` function the app calls at launch. On every other target the slice
/// already holds them, and the function is a no-op, so an app calls it unconditionally.
#[macro_export]
macro_rules! tests {
    ($($test:path),* $(,)?) => {
        /// Register this crate's `#[day::test]` functions where no link-time registry exists.
        pub fn register_tests() {
            #[cfg(target_arch = "wasm32")]
            {
                $( $crate::conformance::register(
                    $crate::conformance::TestFn::new(stringify!($test), $test),
                ); )*
            }
        }
        /// The roster as written, for the lint that holds it equal to the link-time slice.
        pub fn roster() -> Vec<$crate::conformance::TestFn> {
            vec![$( $crate::conformance::TestFn::new(stringify!($test), $test) ),*]
        }
    };
}
