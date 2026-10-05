// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The native module the ArkTS host imports (`import native from 'libentry.so'`,
//! platform/harmony/types/Index.d.ts), through `napi-ohos`.
//!
//! Every `#[napi]` function here is one export of the `entry` module; the ArkTS pages call
//! them (docs/harmonyos.md). The duties HarmonyOS keeps in ArkTS (the navigation stack, file
//! and permission pickers, URL opening, secondary windows, the status bar, ArkTS-built piece
//! components) come back as JS callbacks the host registers, held here as function references
//! and called on the JS thread, which is the thread every entry point below runs on.

// Node handles are opaque runtime tokens (see node.rs), never dereferenced here.
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::{c_char, c_void};
use std::ptr;

use day_spec::bridge::BridgeKind as K;
use napi_derive_ohos::napi;
use napi_ohos::bindgen_prelude::{FnArgs, FunctionRef, Null, Object, ObjectRef, Unknown};
use napi_ohos::{Env, JsValue, sys};
use ohos_sys::arkui::native_node::{ArkUI_NodeDirtyFlag, OH_ArkUI_NodeContent_RemoveNode};
use ohos_sys::arkui::native_node_napi::{
    OH_ArkUI_GetNodeContentFromNapiValue, OH_ArkUI_GetNodeHandleFromNapiValue,
};
use ohos_sys::arkui::native_type::ArkUI_NodeContentHandle;

use crate::node::{self, Handle};

/// A JS callback the host registered, kept past the call that registered it.
type Callback<Args> = RefCell<Option<Registered<Args>>>;
/// A registered callback's reference, as the `#[napi]` exports receive it.
type Registered<Args> = FunctionRef<Args, Unknown<'static>>;
/// The file picker's `(req, mode, name, src, filters)` (docs/files.md).
type FilePickerArgs = FnArgs<(f64, i32, String, String, String)>;
/// The toolbar's five `\n`-joined parallel fields (docs/toolbars.md).
type MenuArgs = FnArgs<(String, String, String, String, String)>;

thread_local! {
    /// `(revision, window)`: RenderService-backed capture checkpoints.
    static CAPTURE: Callback<FnArgs<(f64, f64)>> = const { RefCell::new(None) };
    static CAPTURE_FENCES: RefCell<HashMap<u64, day_spec::capture::Fence>> = RefCell::new(HashMap::new());
    /// The NAPI environment, captured at the first export call, for calls made from native
    /// callbacks (an ArkUI event, a posted job) rather than from an export.
    static ENV: Cell<sys::napi_env> = const { Cell::new(ptr::null_mut()) };
    /// `(req, mode, name, src, filters)` (docs/files.md).
    static FILE_PICKER: Callback<FilePickerArgs> = const { RefCell::new(None) };
    /// `(req, names)` (docs/permissions.md).
    static PERMISSIONS: Callback<FnArgs<(f64, String)>> = const { RefCell::new(None) };
    static PERMISSION_WAITERS: RefCell<HashMap<u64, extern "C" fn(u64, u64)>> = RefCell::new(HashMap::new());
    /// `(url)`: the `link` piece's opener.
    static OPEN_URL: Callback<FnArgs<(String,)>> = const { RefCell::new(None) };
    /// `(hidden)`: the window's status bar (docs/cover.md).
    static STATUS_BAR: Callback<FnArgs<(bool,)>> = const { RefCell::new(None) };
    /// The last status-bar answer Day asked for, replayed when the host registers late.
    static STATUS_BAR_HIDDEN: Cell<bool> = const { Cell::new(false) };
    /// Show/hide the host's layer ABOVE Navigation, not its root-page content slot.
    static COVER_LAYER: Callback<FnArgs<(bool,)>> = const { RefCell::new(None) };
    /// `(node, title)` / `(node)`: the multiton window launchers (docs/windows.md).
    static WINDOW_OPEN: Callback<FnArgs<(f64, String)>> = const { RefCell::new(None) };
    static WINDOW_CLOSE: Callback<FnArgs<(f64,)>> = const { RefCell::new(None) };
    // The Navigation bridge (docs/navigation.md).
    static NAV_PUSH: Callback<FnArgs<(f64, String)>> = const { RefCell::new(None) };
    /// `()`: the pop takes no argument; `null` rides along, since an empty tuple has no NAPI form.
    static NAV_POP: Callback<FnArgs<(Null,)>> = const { RefCell::new(None) };
    static NAV_TITLE: Callback<FnArgs<(String,)>> = const { RefCell::new(None) };
    static NAV_GUARD: Callback<FnArgs<(bool,)>> = const { RefCell::new(None) };
    static NAV_MENU: Callback<MenuArgs> = const { RefCell::new(None) };
    static NAV_SEARCH: Callback<FnArgs<(i32, String, String)>> = const { RefCell::new(None) };
    /// A pushed page's slot: the NodeContent handle plus a strong reference on the JS object.
    /// The ArkTS side drops its own reference when the NavDestination disappears, so without
    /// the ref the content is GC'd while Rust may still detach the page from it; the
    /// RemoveNode-after-pop then walks freed FrameNodes.
    static NAV_CONTENTS: RefCell<HashMap<u64, (usize, ObjectRef<false>)>> = RefCell::new(HashMap::new());
    // ArkTS-built piece components (docs/extending.md).
    static PIECE_MAKE: Callback<FnArgs<(String, f64, String)>> = const { RefCell::new(None) };
    static PIECE_UPDATE: Callback<FnArgs<(f64, String, String)>> = const { RefCell::new(None) };
    static PIECE_DISPOSE: Callback<FnArgs<(f64,)>> = const { RefCell::new(None) };
    /// Every bridged crate's ArkTS arm by symbol (docs/bridge.md).
    static BRIDGES: RefCell<Option<ObjectRef<false>>> = const { RefCell::new(None) };
}

// Implemented by the app cdylib (`day::day_start_arkui!`): mount the root and run the loop,
// and take a deep link (docs/deep-links.md).
unsafe extern "C" {
    fn day_arkui_start(content: *mut c_void, w_vp: f64, h_vp: f64, density: f64);
    fn day_arkui_deeplink(uri: *const c_char);
}

fn remember(env: &Env) {
    ENV.with(|e| e.set(env.raw()));
    let _ = crate::main_thread::init(env);
}

/// The environment, once any export has run.
pub fn env() -> Option<Env> {
    let raw = ENV.with(|e| e.get());
    (!raw.is_null()).then(|| Env::from_raw(raw))
}

/// A handle scope for values created outside an export's own scope (an ArkUI event, a posted
/// job): released on drop.
pub struct Scope(sys::napi_env, sys::napi_handle_scope);

impl Scope {
    pub fn open(env: &Env) -> Scope {
        let mut scope = ptr::null_mut();
        // SAFETY: a live env.
        unsafe { sys::napi_open_handle_scope(env.raw(), &mut scope) };
        Scope(env.raw(), scope)
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        if !self.1.is_null() {
            // SAFETY: the scope this value opened.
            unsafe { sys::napi_close_handle_scope(self.0, self.1) };
        }
    }
}

/// Call a registered callback with `args`, handing its return value to `with` inside a handle
/// scope. `None` when nothing is registered (the host predates the seam) or the call failed.
fn call<Args, R>(
    slot: &'static std::thread::LocalKey<Callback<Args>>,
    args: Args,
    with: impl FnOnce(&Env, Unknown<'_>) -> R,
) -> Option<R>
where
    Args: napi_ohos::bindgen_prelude::JsValuesTupleIntoVec,
{
    let env = env()?;
    let _scope = Scope::open(&env);
    slot.with(|s| {
        let s = s.borrow();
        let f = s.as_ref()?;
        let f = f.borrow_back(&env).ok()?;
        match f.call(args) {
            Ok(ret) => Some(with(&env, ret)),
            Err(e) => {
                log::warn!("day-arkui: ArkTS callback failed: {e}");
                None
            }
        }
    })
}

fn store<Args>(
    slot: &'static std::thread::LocalKey<Callback<Args>>,
    f: FunctionRef<Args, Unknown<'static>>,
) where
    Args: napi_ohos::bindgen_prelude::JsValuesTupleIntoVec,
{
    slot.with(|s| *s.borrow_mut() = Some(f));
}

/// The bridge record, for `bridge::call_here`.
pub(crate) fn bridges<'env>(env: &'env Env) -> Option<Object<'env>> {
    BRIDGES.with(|b| b.borrow().as_ref().and_then(|r| r.get_value(env).ok()))
}

// ---- exports ------------------------------------------------------------------------------

#[napi(js_name = "registerCoverLayer")]
pub fn register_cover_layer(env: Env, content: Unknown, callback: Registered<FnArgs<(bool,)>>) {
    remember(&env);
    let mut handle: ArkUI_NodeContentHandle = ptr::null_mut();
    unsafe {
        OH_ArkUI_GetNodeContentFromNapiValue(env.raw().cast(), content.raw().cast(), &mut handle)
    };
    store(&COVER_LAYER, callback);
    crate::cover_layer_init(handle);
}

#[napi(js_name = "coverLayerResized")]
pub fn cover_layer_resized(env: Env, width_vp: f64, height_vp: f64) {
    remember(&env);
    crate::cover_layer_resized(width_vp, height_vp);
}

pub fn show_cover_layer(shown: bool) {
    call(&COVER_LAYER, FnArgs::from((shown,)), |_, _| ());
}

#[napi(js_name = "coverBackRequested")]
pub fn cover_back_requested() -> bool {
    crate::cover_back_requested()
}

/// `start(nodeContent, widthVp, heightVp, density)`: mount the Day tree into the host's slot.
#[napi(js_name = "start")]
pub fn start(env: Env, content: Unknown, width_vp: f64, height_vp: f64, density: f64) {
    remember(&env);
    let mut handle: ArkUI_NodeContentHandle = ptr::null_mut();
    // SAFETY: the value is the host's NodeContent; the opaque napi types are the same pointers.
    unsafe {
        OH_ArkUI_GetNodeContentFromNapiValue(env.raw().cast(), content.raw().cast(), &mut handle)
    };
    node::set_density(density);
    crate::events::install();
    // SAFETY: the app cdylib defines the export with this signature.
    unsafe { day_arkui_start(handle.cast(), width_vp, height_vp, node::density()) };
}

/// `resized(w, h)`: the root area changed after start (keyboard RESIZE avoidance, rotation).
#[napi(js_name = "resized")]
pub fn resized(env: Env, width_vp: f64, height_vp: f64) {
    remember(&env);
    crate::resized(width_vp, height_vp);
}

/// `setEnv(key, value)`: a process environment variable, before `start()`. The launcher hands
/// the app its dayscript engine port + token (and locale / autodrive) this way, the HarmonyOS
/// analogue of Android's intent-extra env delivery.
#[napi(js_name = "setEnv")]
pub fn set_env(key: String, value: String) {
    if !key.is_empty() {
        // SAFETY: called from the JS thread before the app's threads start reading the
        // environment, as the launcher contract requires.
        unsafe { std::env::set_var(key, value) };
    }
}

/// `deepLink(uri)`: a cold `want.uri` or a warm `onNewWant` one. Safe before `start`: the app
/// side buffers until the first mount.
#[napi(js_name = "deepLink")]
pub fn deep_link(env: Env, uri: String) {
    remember(&env);
    if uri.is_empty() {
        return;
    }
    let uri = node::cstr(&uri);
    // SAFETY: the app cdylib defines the export; the string outlives the call.
    unsafe { day_arkui_deeplink(uri.as_ptr()) };
}

/// Register the ArkTS render checkpoint (revision, window node; zero = primary).
#[napi(js_name = "registerCapture")]
pub fn register_capture(env: Env, callback: Registered<FnArgs<(f64, f64)>>) {
    remember(&env);
    store(&CAPTURE, callback);
}

#[napi(js_name = "captureReady")]
pub fn capture_ready(revision: u32, window: f64, error: String) {
    CAPTURE_FENCES.with(|f| {
        if let Some(fence) = f.borrow().get(&(window as u64)) {
            fence.complete(revision, if error.is_empty() { Ok(()) } else { Err(error) });
        }
    });
}

pub fn prepare_capture(revision: u32, window: u64) -> Result<day_spec::capture::Readiness, String> {
    let fence = CAPTURE_FENCES.with(|f| f.borrow_mut().entry(window).or_default().clone());
    if fence.begin(revision)
        && call(
            &CAPTURE,
            FnArgs::from((f64::from(revision), window as f64)),
            |_, _| (),
        )
        .is_none()
    {
        fence.complete(
            revision,
            Err("ArkTS render checkpoint is not registered".into()),
        );
    }
    fence.poll()
}

/// `registerFilePicker(cb, cacheDir)` (docs/files.md).
#[napi(js_name = "registerFilePicker")]
pub fn register_file_picker(env: Env, callback: Registered<FilePickerArgs>, cache_dir: String) {
    remember(&env);
    store(&FILE_PICKER, callback);
    if !cache_dir.is_empty() {
        crate::set_cache_dir(&cache_dir);
    }
}

/// `onFileResult(req, path)`: the picker's answer; an empty path is a cancel.
#[napi(js_name = "onFileResult")]
pub fn on_file_result(req: f64, path: String) {
    crate::on_event(req as u64, K::PresentFile as i32, 0.0, &path);
}

/// `registerPermissions(cb)` (docs/permissions.md).
#[napi(js_name = "registerPermissions")]
pub fn register_permissions(
    env: Env,
    callback: FunctionRef<FnArgs<(f64, String)>, Unknown<'static>>,
) {
    remember(&env);
    store(&PERMISSIONS, callback);
}

/// `onPermissionResult(req, mask)`: bit i set when the i-th name was granted.
#[napi(js_name = "onPermissionResult")]
pub fn on_permission_result(req: f64, mask: f64) {
    let waiter = PERMISSION_WAITERS.with(|w| w.borrow_mut().remove(&(req as u64)));
    if let Some(cb) = waiter {
        cb(req as u64, mask as u64);
    }
}

/// `registerOpenUrl(cb)`: the `link` piece's opener (an implicit viewData Want).
#[napi(js_name = "registerOpenUrl")]
pub fn register_open_url(env: Env, callback: FunctionRef<FnArgs<(String,)>, Unknown<'static>>) {
    remember(&env);
    store(&OPEN_URL, callback);
}

/// `registerStatusBar(cb)`: the window's status bar (docs/cover.md). A request made before the
/// host registered is replayed at once, so the order of `start()` and this call never loses one.
#[napi(js_name = "registerStatusBar")]
pub fn register_status_bar(env: Env, callback: FunctionRef<FnArgs<(bool,)>, Unknown<'static>>) {
    remember(&env);
    store(&STATUS_BAR, callback);
    if STATUS_BAR_HIDDEN.with(|h| h.get()) {
        set_status_bar_hidden(true);
    }
}

/// `registerWindows(open, close)` (docs/windows.md).
#[napi(js_name = "registerWindows")]
pub fn register_windows(
    env: Env,
    open: FunctionRef<FnArgs<(f64, String)>, Unknown<'static>>,
    close: FunctionRef<FnArgs<(f64,)>, Unknown<'static>>,
) {
    remember(&env);
    store(&WINDOW_OPEN, open);
    store(&WINDOW_CLOSE, close);
}

/// `windowStart(nodeContent, node, w, h)`: a secondary window's page connected; true when the
/// pending open completed (false = closed before connecting; the page's ability terminates).
#[napi(js_name = "windowStart")]
pub fn window_start(env: Env, content: Unknown, node: f64, width_vp: f64, height_vp: f64) -> bool {
    remember(&env);
    let mut handle: ArkUI_NodeContentHandle = ptr::null_mut();
    // SAFETY: the value is the page's NodeContent.
    unsafe {
        OH_ArkUI_GetNodeContentFromNapiValue(env.raw().cast(), content.raw().cast(), &mut handle)
    };
    crate::window_start(node as u64, handle, width_vp, height_vp)
}

#[napi(js_name = "windowResized")]
pub fn window_resized(node: f64, width_vp: f64, height_vp: f64) {
    crate::window_resized(node as u64, width_vp, height_vp);
}

#[napi(js_name = "windowClosed")]
pub fn window_closed(node: f64) {
    CAPTURE_FENCES.with(|f| f.borrow_mut().remove(&(node as u64)));
    crate::window_closed(node as u64);
}

#[napi(js_name = "windowFocused")]
pub fn window_focused(node: f64, active: f64) {
    crate::window_focused(node as u64, active != 0.0);
}

/// `lifecycle(phase)`: an app lifecycle phase from the entry ability, in `day_spec::Lifecycle`
/// order (docs/lifecycle.md): 2 DidBecomeActive … 7 WillTerminate.
#[napi(js_name = "lifecycle")]
pub fn lifecycle(phase: f64) {
    crate::lifecycle(phase as i32);
}

/// `registerResourceManager(resourceManager)`: the app's ArkTS resource manager, so the
/// rawfile opener (§18.3) can read staged data resources.
#[napi(js_name = "registerResourceManager")]
pub fn register_resource_manager(env: Env, resource_manager: Unknown) {
    remember(&env);
    // SAFETY: a live env and the host's resourceManager object.
    unsafe { crate::resources::register(env.raw(), resource_manager.raw()) };
}

/// `registerNav(push, pop, setTitle, setGuard, setMenu, setSearch)` (docs/navigation.md).
#[napi(js_name = "registerNav")]
#[allow(clippy::too_many_arguments)]
pub fn register_nav(
    env: Env,
    push: FunctionRef<FnArgs<(f64, String)>, Unknown<'static>>,
    pop: FunctionRef<FnArgs<(Null,)>, Unknown<'static>>,
    set_title: FunctionRef<FnArgs<(String,)>, Unknown<'static>>,
    set_guard: Option<Registered<FnArgs<(bool,)>>>,
    set_menu: Option<Registered<MenuArgs>>,
    set_search: Option<Registered<FnArgs<(i32, String, String)>>>,
) {
    remember(&env);
    store(&NAV_PUSH, push);
    store(&NAV_POP, pop);
    store(&NAV_TITLE, set_title);
    if let Some(f) = set_guard {
        store(&NAV_GUARD, f);
    }
    if let Some(f) = set_menu {
        store(&NAV_MENU, f);
    }
    if let Some(f) = set_search {
        store(&NAV_SEARCH, f);
    }
}

/// `navPopped(key)`: a NavDestination disappeared.
#[napi(js_name = "navPopped")]
pub fn nav_popped(key: f64) {
    crate::nav_popped(key as u64);
}

/// `navBackRequested()`: a guarded destination's back was pressed.
#[napi(js_name = "navBackRequested")]
pub fn nav_back_requested() {
    crate::nav_back_requested();
}

/// `navMenuAction(action, selection?)`: a title-bar action was tapped (docs/toolbars.md).
#[napi(js_name = "navMenuAction")]
pub fn nav_menu_action(action: f64, selection: Option<i32>) {
    crate::nav_menu_action(action as u64, selection.unwrap_or(-1));
}

/// `navSearchChanged(text)` (docs/search.md).
#[napi(js_name = "navSearchChanged")]
pub fn nav_search_changed(text: String) {
    crate::nav_search_changed(&text);
}

/// `navPageArea(key, w, h)`: a destination's content area, in vp.
#[napi(js_name = "navPageArea")]
pub fn nav_page_area(key: f64, width_vp: f64, height_vp: f64) {
    crate::nav_area(key as u64, width_vp, height_vp);
}

/// `registerPiece(make, update, dispose)` (docs/extending.md).
#[napi(js_name = "registerPiece")]
pub fn register_piece(
    env: Env,
    make: FunctionRef<FnArgs<(String, f64, String)>, Unknown<'static>>,
    update: FunctionRef<FnArgs<(f64, String, String)>, Unknown<'static>>,
    dispose: FunctionRef<FnArgs<(f64,)>, Unknown<'static>>,
) {
    remember(&env);
    store(&PIECE_MAKE, make);
    store(&PIECE_UPDATE, update);
    store(&PIECE_DISPOSE, dispose);
}

/// `pieceEvent(id, text, num?, kind?)`: an ArkTS-built component reporting back to its piece.
/// By default it rides the Custom channel (the payload is the whole event, `num` the piece's
/// own discriminator); `kind` delivers one of Day's own events instead, so a component standing
/// in for a control reports what the control would: a text edit (1, with `text`), a selection
/// (4, the index in `num`), a submit (17), a toggle (2).
#[napi(js_name = "pieceEvent")]
pub fn piece_event(id: f64, text: String, num: Option<f64>, kind: Option<i32>) {
    let asked = kind.unwrap_or(K::Custom as i32);
    // Only the events a stand-in control has any business sending.
    let kind = if [
        K::TextChanged as i32,
        K::SelectionChanged as i32,
        K::Submitted as i32,
        K::ToggleChanged as i32,
    ]
    .contains(&asked)
    {
        asked
    } else {
        K::Custom as i32
    };
    crate::on_event(id as u64, kind, num.unwrap_or(0.0), &text);
}

/// `registerDayBridges(record)` (docs/bridge.md "Callbacks").
#[napi(js_name = "registerDayBridges")]
pub fn register_day_bridges(env: Env, record: Object) {
    remember(&env);
    let old = BRIDGES.with(|b| b.borrow_mut().take());
    if let Some(old) = old {
        let _ = old.unref(&env);
    }
    if let Ok(r) = record.create_ref::<false>() {
        BRIDGES.with(|b| *b.borrow_mut() = Some(r));
    }
}

/// `dayBridgeComplete(symbol, done, status, value, message)`: an asynchronous arm's answer.
#[napi(js_name = "dayBridgeComplete")]
pub fn day_bridge_complete(
    env: Env,
    symbol: String,
    done: f64,
    status: f64,
    value: Option<Unknown>,
    message: Option<String>,
) {
    remember(&env);
    crate::bridge::complete(
        &symbol,
        done as u64,
        status as i32,
        value,
        message.as_deref().unwrap_or(""),
    );
}

// ---- Rust-facing calls into the host ------------------------------------------------------

/// Ask the ArkTS picker to open/save a file (docs/files.md). A missing picker answers an
/// immediate cancel.
pub fn present_file(req: u64, mode: i32, name: &str, src: &str, filters: &str) {
    let sent = call(
        &FILE_PICKER,
        FnArgs::from((
            req as f64,
            mode,
            name.to_owned(),
            src.to_owned(),
            filters.to_owned(),
        )),
        |_, _| (),
    );
    if sent.is_none() {
        crate::on_event(req, K::PresentFile as i32, 0.0, "");
    }
}

/// Ask the ArkTS prompter for `names` (0x1F-separated, docs/permissions.md). True when the
/// request went out, in which case `cb` is called on the JS thread with the request id and a
/// bit mask of the grants; false when no prompter is registered, and `cb` is never called.
pub fn request_permissions(req: u64, names: &str, cb: extern "C" fn(u64, u64)) -> bool {
    if PERMISSIONS.with(|p| p.borrow().is_none()) {
        return false;
    }
    PERMISSION_WAITERS.with(|w| w.borrow_mut().insert(req, cb));
    let sent = call(
        &PERMISSIONS,
        FnArgs::from((req as f64, names.to_owned())),
        |_, _| (),
    );
    if sent.is_none() {
        PERMISSION_WAITERS.with(|w| w.borrow_mut().remove(&req));
        return false;
    }
    true
}

/// Open `url` in the system's default handler. A no-op when the host registered no opener.
pub fn open_url(url: &str) {
    call(&OPEN_URL, FnArgs::from((url.to_owned(),)), |_, _| ());
}

/// Hide or show the window's status bar. Remembered when no host is registered yet.
pub fn set_status_bar_hidden(hidden: bool) {
    STATUS_BAR_HIDDEN.with(|h| h.set(hidden));
    call(&STATUS_BAR, FnArgs::from((hidden,)), |_, _| ());
}

/// Whether the host registered the window launchers (drives `Cap::MultiWindow`).
pub fn has_windows() -> bool {
    WINDOW_OPEN.with(|w| w.borrow().is_some())
}

/// Launch a secondary Day window; true when the request went out (the ability's page completes
/// the open).
pub fn open_window(node: u64, title: &str) -> bool {
    call(
        &WINDOW_OPEN,
        FnArgs::from((node as f64, title.to_owned())),
        |_, _| (),
    )
    .is_some()
}

pub fn close_window(node: u64) {
    call(&WINDOW_CLOSE, FnArgs::from((node as f64,)), |_, _| ());
}

/// Push one Day page into the ArkTS Navigation: ask the registered push callback for a fresh
/// NodeContent (it also pushes the NavDestination) and mount the page's node into it. 0 on
/// success.
pub fn nav_push(page: Handle, key: u64, title: &str) -> i32 {
    let mounted = call(
        &NAV_PUSH,
        FnArgs::from((key as f64, title.to_owned())),
        |env, ret| {
            let mut content: ArkUI_NodeContentHandle = ptr::null_mut();
            // SAFETY: the callback returned the destination's NodeContent.
            unsafe {
                OH_ArkUI_GetNodeContentFromNapiValue(
                    env.raw().cast(),
                    ret.raw().cast(),
                    &mut content,
                )
            };
            if content.is_null() {
                return false;
            }
            // The value is an object, kept alive by the reference.
            let keep = Object::from_raw(env.raw(), ret.raw()).create_ref::<false>();
            let Ok(keep) = keep else {
                return false;
            };
            NAV_CONTENTS.with(|c| c.borrow_mut().insert(key, (content as usize, keep)));
            node::content_add(content, page);
            // Re-homed subtrees keep their (already clean) layout/render state, and the fresh
            // NavDestination composes an EMPTY content layer over the previous page unless the
            // attached tree is explicitly re-marked for layout + paint.
            node::mark_dirty(page, ArkUI_NodeDirtyFlag::NODE_NEED_MEASURE);
            node::mark_dirty(page, ArkUI_NodeDirtyFlag::NODE_NEED_LAYOUT);
            node::mark_dirty(page, ArkUI_NodeDirtyFlag::NODE_NEED_RENDER);
            true
        },
    );
    if mounted == Some(true) { 0 } else { -1 }
}

/// Pop the top NavDestination (a Day-initiated route change).
pub fn nav_pop() {
    call(&NAV_POP, FnArgs::from((Null,)), |_, _| ());
}

pub fn nav_set_title(title: &str) {
    call(&NAV_TITLE, FnArgs::from((title.to_owned(),)), |_, _| ());
}

/// Arm/disarm the top destination's back guard. A host that predates the seam gets no guard.
pub fn nav_set_guard(on: bool) {
    call(&NAV_GUARD, FnArgs::from((on,)), |_, _| ());
}

/// The window toolbar's title-bar actions (docs/toolbars.md), five `\n`-joined parallel
/// fields.
pub fn nav_set_menu(icons: &str, labels: &str, actions: &str, scopes: &str, enabled: &str) {
    call(
        &NAV_MENU,
        FnArgs::from((
            icons.to_owned(),
            labels.to_owned(),
            actions.to_owned(),
            scopes.to_owned(),
            enabled.to_owned(),
        )),
        |_, _| (),
    );
}

/// Show (1), hide (0) or only re-text (-1) the navigation surface's search field.
pub fn nav_set_search(shown: i32, prompt: &str, text: &str) {
    call(
        &NAV_SEARCH,
        FnArgs::from((shown, prompt.to_owned(), text.to_owned())),
        |_, _| (),
    );
}

/// Unmount a page's node from its still-LIVE NodeContent (a Day-initiated pop detaches before
/// the destination's teardown) and release the slot.
pub fn nav_remove(key: u64, page: Handle) {
    let Some((content, keep)) = NAV_CONTENTS.with(|c| c.borrow_mut().remove(&key)) else {
        return;
    };
    // SAFETY: the content is alive: its destination has not disappeared yet.
    unsafe { OH_ArkUI_NodeContent_RemoveNode(content as ArkUI_NodeContentHandle, page) };
    if let Some(env) = env() {
        let _ = keep.unref(&env);
    }
}

/// Release a slot whose NavDestination already disappeared: the destination tore its content
/// down, so touching the nodes again would use freed memory; just drop the bookkeeping and the
/// keep-alive reference.
pub fn nav_forget(key: u64) {
    let Some((_, keep)) = NAV_CONTENTS.with(|c| c.borrow_mut().remove(&key)) else {
        return;
    };
    if let Some(env) = env() {
        let _ = keep.unref(&env);
    }
}

/// Build a piece's ArkTS component and return its FrameNode as a node Day can mount. Null when
/// nothing is registered or the factory declined the kind.
pub fn piece_make(kind: &str, id: u64, props: &str) -> Handle {
    call(
        &PIECE_MAKE,
        FnArgs::from((kind.to_owned(), id as f64, props.to_owned())),
        |env, ret| {
            let mut n: Handle = ptr::null_mut();
            // Returns non-zero for undefined/null (a factory that declined the kind), leaving `n`
            // untouched, hence the explicit null above.
            // SAFETY: the value is a FrameNode or undefined.
            unsafe {
                OH_ArkUI_GetNodeHandleFromNapiValue(env.raw().cast(), ret.raw().cast(), &mut n)
            };
            n
        },
    )
    .unwrap_or(ptr::null_mut())
}

/// Send a piece command to its ArkTS side.
pub fn piece_update(id: u64, cmd: &str, arg: &str) {
    call(
        &PIECE_UPDATE,
        FnArgs::from((id as f64, cmd.to_owned(), arg.to_owned())),
        |_, _| (),
    );
}

/// Release the ArkTS BuilderNode behind a piece node.
pub fn piece_dispose(id: u64) {
    call(&PIECE_DISPOSE, FnArgs::from((id as f64,)), |_, _| ());
}
