//! The miniapp runtime (docs/lite.md §1, §5–§7): one QuickJS context on the main thread,
//! the `day.*` API installed over the dyn registry, and the module loader that strips
//! TypeScript on the way in. Everything here is single-threaded; async work (net, timers)
//! goes through `day_core::task` and re-enters the context from the main thread.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use day_pieces::dynreg::{self, DynCallback, DynSignal, DynValue};
use day_pieces::prelude::*;
use day_reactive::Signal;
use rquickjs::loader::{Loader, Resolver};
use rquickjs::{CatchResultExt, Context, Ctx, Function, Module, Runtime, Value};

use crate::db::{Cell as DbCell, Db};
use crate::fsx::Sandbox;
use crate::store::{Manifest, Store};
use crate::value::from_js;
use crate::{Bridge, PermissionSet};

thread_local! {
    /// Piece handles held by JS (`__p` markers). Cleared when the app tears down.
    pub(crate) static PIECES: RefCell<Vec<dynreg::DynPiece>> = const { RefCell::new(Vec::new()) };
    /// Signal handles held by JS (`__s` markers).
    pub(crate) static SIGNALS: RefCell<Vec<DynSignal>> = const { RefCell::new(Vec::new()) };
    /// The running app's services. One miniapp runs at a time (the superapp presents one
    /// cover); nesting is prevented at launch.
    static SERVICES: RefCell<Option<Rc<Services>>> = const { RefCell::new(None) };
}

/// One pushed page: its route and the JSON-encoded params `navigateTo` supplied.
#[derive(Clone, PartialEq)]
pub struct NavEntry {
    pub route: String,
    pub params: String,
}

impl day_pieces::Route for NavEntry {
    fn key(&self) -> String {
        if self.params.is_empty() {
            self.route.clone()
        } else {
            format!("{}|{}", self.route, self.params)
        }
    }
    fn from_key(key: &str) -> Option<Self> {
        let (route, params) = key.split_once('|').unwrap_or((key, ""));
        Some(NavEntry {
            route: route.into(),
            params: params.into(),
        })
    }
    fn title(&self) -> String {
        self.route.clone()
    }
}

struct PageDef {
    builder: DynCallback,
    hooks: HashMap<String, DynCallback>,
}

/// Everything a running miniapp's host functions reach for. Held in a thread-local while
/// the app runs so plain `fn` bridge entries can find it.
pub struct Services {
    pub app_id: String,
    pub manifest: Manifest,
    pub store: Store,
    pub db: Db,
    pub fs: Sandbox,
    pub permissions: PermissionSet,
    pages: RefCell<HashMap<String, PageDef>>,
    app_hooks: RefCell<HashMap<String, DynCallback>>,
    pub nav: Signal<Vec<NavEntry>>,
    /// Log lines (tests assert on them; `day launch` streams stderr anyway).
    pub log: RefCell<Vec<String>>,
    i18n: crate::i18n::I18n,
    context: RefCell<Option<Context>>,
}

impl Services {
    pub fn report_error(&self, detail: &str) {
        if let Some(cb) = self.app_hooks.borrow().get("onError").cloned() {
            cb(&[DynValue::Str(detail.into())]);
        }
    }

    fn page(&self, route: &str) -> Option<(DynCallback, HashMap<String, DynCallback>)> {
        self.pages
            .borrow()
            .get(route)
            .map(|p| (p.builder.clone(), p.hooks.clone()))
    }

    fn app_hook(&self, name: &str) -> Option<DynCallback> {
        self.app_hooks.borrow().get(name).cloned()
    }
}

pub fn with_services<T>(f: impl FnOnce(&Services) -> T) -> Option<T> {
    SERVICES
        .with(|s| s.borrow().as_ref().cloned())
        .map(|s| f(&s))
}

fn services() -> Rc<Services> {
    SERVICES
        .with(|s| s.borrow().as_ref().cloned())
        .expect("day-lite host function called with no running miniapp")
}

// ---- module loading ---------------------------------------------------------------------

/// Resolves `./`-relative imports against the importing module's path; bare and absolute
/// specifiers are refused (a miniapp imports only its own files).
struct PkgResolver;

impl Resolver for PkgResolver {
    fn resolve<'js>(
        &mut self,
        _ctx: &Ctx<'js>,
        base: &str,
        name: &str,
        _attributes: Option<rquickjs::loader::ImportAttributes<'js>>,
    ) -> rquickjs::Result<String> {
        let joined = if let Some(rest) = name.strip_prefix("./") {
            match base.rsplit_once('/') {
                Some((dir, _)) => format!("{dir}/{rest}"),
                None => rest.to_string(),
            }
        } else if name.starts_with("../") {
            let mut dir: Vec<&str> = match base.rsplit_once('/') {
                Some((dir, _)) => dir.split('/').collect(),
                None => Vec::new(),
            };
            let mut rest = name;
            while let Some(r) = rest.strip_prefix("../") {
                if dir.pop().is_none() {
                    return Err(rquickjs::Error::new_resolving(base, name));
                }
                rest = r;
            }
            if dir.is_empty() {
                rest.to_string()
            } else {
                format!("{}/{}", dir.join("/"), rest)
            }
        } else {
            return Err(rquickjs::Error::new_resolving(base, name));
        };
        // Normalize implicit extensions: `./util` finds util.ts then util.js.
        let s = services();
        for cand in [
            joined.clone(),
            format!("{joined}.ts"),
            format!("{joined}.js"),
        ] {
            if s.store.read_file(&s.app_id, &cand).is_ok() {
                return Ok(cand);
            }
        }
        Err(rquickjs::Error::new_resolving(base, name))
    }
}

struct PkgLoader;

impl Loader for PkgLoader {
    fn load<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        name: &str,
        _attributes: Option<rquickjs::loader::ImportAttributes<'js>>,
    ) -> rquickjs::Result<Module<'js, rquickjs::module::Declared>> {
        let s = services();
        let bytes = s
            .store
            .read_file(&s.app_id, name)
            .map_err(|_| rquickjs::Error::new_loading(name))?;
        let source = String::from_utf8_lossy(&bytes).into_owned();
        let js = if name.ends_with(".ts") {
            crate::ts::strip(name, &source).map_err(|e| {
                eprintln!("day-lite: {e}");
                rquickjs::Error::new_loading(name)
            })?
        } else {
            source
        };
        Module::declare(ctx.clone(), name, js)
    }
}

// ---- the running app --------------------------------------------------------------------

/// A running miniapp: the QuickJS runtime plus its UI surface.
pub struct LiteApp {
    services: Rc<Services>,
    // Kept alive for the app's lifetime; dropped (in order) at teardown.
    context: Context,
    _runtime: Runtime,
}

impl LiteApp {
    /// Boot `app_id` from the store: build the runtime, install the API + bridges, and
    /// evaluate the entry module (which registers `App` and its pages).
    pub fn boot(
        store: Store,
        app_id: &str,
        bridges: &[Bridge],
        permissions: PermissionSet,
    ) -> Result<LiteApp, String> {
        let manifest = store.manifest(app_id).map_err(|e| e.to_string())?;
        let db = Db::open(store.app_dir(app_id).join("app.sqlite")).map_err(|e| e.to_string())?;
        let fs = Sandbox::at(store.app_dir(app_id).join("fs"));
        let services = Rc::new(Services {
            app_id: app_id.to_string(),
            manifest: manifest.clone(),
            store,
            db,
            fs,
            permissions,
            pages: RefCell::new(HashMap::new()),
            app_hooks: RefCell::new(HashMap::new()),
            nav: Signal::new(Vec::new()),
            log: RefCell::new(Vec::new()),
            i18n: crate::i18n::I18n::default(),
            context: RefCell::new(None),
        });
        let prior = SERVICES.with(|s| s.borrow().is_some());
        if prior {
            return Err("a miniapp is already running".into());
        }
        SERVICES.with(|s| *s.borrow_mut() = Some(services.clone()));

        let runtime = Runtime::new().map_err(|e| e.to_string())?;
        runtime.set_loader(PkgResolver, PkgLoader);
        let context = Context::full(&runtime).map_err(|e| e.to_string())?;
        *services.context.borrow_mut() = Some(context.clone());

        let boot = context.with(|ctx| -> Result<(), String> {
            install_api(&ctx, bridges).map_err(|e| format!("api install: {e}"))?;
            let entry = manifest.entry().to_string();
            let promise = Module::evaluate(ctx.clone(), entry.clone(), load_entry(&entry)?)
                .catch(&ctx)
                .map_err(|e| format!("{entry}: {e}"))?;
            promise
                .finish::<Value>()
                .catch(&ctx)
                .map_err(|e| format!("{entry}: {e}"))?;
            while ctx.execute_pending_job() {}
            Ok(())
        });
        if let Err(e) = boot {
            teardown();
            return Err(e);
        }

        if let Some(cb) = services.app_hook("onLaunch") {
            let first = services.manifest.pages.first().cloned().unwrap_or_default();
            cb(&[DynValue::Map(vec![("path".into(), DynValue::Str(first))])]);
        }
        Ok(LiteApp {
            services,
            context,
            _runtime: runtime,
        })
    }

    pub fn manifest(&self) -> &Manifest {
        &self.services.manifest
    }

    /// The miniapp's UI: a nav stack whose root is `pages[0]` and whose pushed entries are
    /// looked up in the page registry. Place it anywhere (the daylite superapp puts it in a
    /// fullscreen cover).
    pub fn surface(&self) -> AnyPiece {
        let root_route = self
            .services
            .manifest
            .pages
            .first()
            .cloned()
            .unwrap_or_default();
        let nav = self.services.nav;
        let root = build_page(&root_route, "");
        stack(nav, root)
            .destination(|entry: &NavEntry| build_page(&entry.route, &entry.params))
            .any()
    }
}

impl LiteApp {
    /// Evaluate test modules and run their collected `test()` registrations (docs/lite.md
    /// §11). Returns `(module, test name, failure detail)` per test; `None` = passed.
    /// Used by the `day lite test` runner, not by superapps.
    pub fn run_test_modules(
        &self,
        modules: &[String],
        bootstrap: &str,
    ) -> Result<Vec<(String, String, Option<String>)>, String> {
        let mut out = Vec::new();
        for module in modules {
            let results =
                self.context
                    .with(|ctx| -> Result<Vec<(String, Option<String>)>, String> {
                        ctx.eval::<(), _>(bootstrap)
                            .catch(&ctx)
                            .map_err(|e| format!("test bootstrap: {e}"))?;
                        let source = load_module_source(module)?;
                        let promise = Module::evaluate(ctx.clone(), module.clone(), source)
                            .catch(&ctx)
                            .map_err(|e| format!("{module}: {e}"))?;
                        promise
                            .finish::<Value>()
                            .catch(&ctx)
                            .map_err(|e| format!("{module}: {e}"))?;
                        while ctx.execute_pending_job() {}
                        let run: Function = ctx
                            .globals()
                            .get("__day_run")
                            .map_err(|e| format!("__day_run: {e}"))?;
                        let results: Value = run
                            .call(())
                            .catch(&ctx)
                            .map_err(|e| format!("{module}: {e}"))?;
                        let DynValue::List(items) = from_js(&ctx, &results, 8) else {
                            return Err(format!("{module}: __day_run returned no list"));
                        };
                        Ok(items
                            .into_iter()
                            .map(|item| {
                                let (mut name, mut error) = (String::new(), None);
                                if let DynValue::Map(entries) = item {
                                    for (k, v) in entries {
                                        match (k.as_str(), v) {
                                            ("name", DynValue::Str(s)) => name = s,
                                            ("error", DynValue::Str(s)) => error = Some(s),
                                            _ => {}
                                        }
                                    }
                                }
                                (name, error)
                            })
                            .collect())
                    })?;
            for (name, error) in results {
                out.push((module.clone(), name, error));
            }
        }
        Ok(out)
    }
}

fn load_module_source(module: &str) -> Result<String, String> {
    let s = services();
    let bytes = s
        .store
        .read_file(&s.app_id, module)
        .map_err(|e| e.to_string())?;
    let source = String::from_utf8_lossy(&bytes).into_owned();
    if module.ends_with(".ts") {
        crate::ts::strip(module, &source)
    } else {
        Ok(source)
    }
}

impl Drop for LiteApp {
    fn drop(&mut self) {
        if let Some(cb) = self.services.app_hook("onHide") {
            cb(&[]);
        }
        teardown();
    }
}

fn teardown() {
    SERVICES.with(|s| *s.borrow_mut() = None);
    PIECES.with(|s| s.borrow_mut().clear());
    SIGNALS.with(|s| s.borrow_mut().clear());
}

fn load_entry(entry: &str) -> Result<String, String> {
    let s = services();
    let bytes = s
        .store
        .read_file(&s.app_id, entry)
        .map_err(|e| e.to_string())?;
    let source = String::from_utf8_lossy(&bytes).into_owned();
    if entry.ends_with(".ts") {
        crate::ts::strip(entry, &source)
    } else {
        Ok(source)
    }
}

/// Build one page's piece tree by invoking its JS builder (outside any `Context::with` —
/// the callback enters the context itself). Unknown routes render a plain error label so a
/// script bug degrades visibly.
fn build_page(route: &str, params_json: &str) -> AnyPiece {
    let Some((builder, hooks)) = with_services(|s| s.page(route)).flatten() else {
        return label(format!("day-lite: page `{route}` is not registered")).any();
    };
    let params = if params_json.is_empty() {
        DynValue::Null
    } else {
        json_to_dyn(&serde_json::from_str(params_json).unwrap_or(serde_json::Value::Null))
    };
    if let Some(on_load) = hooks.get("onLoad") {
        on_load(std::slice::from_ref(&params));
    }
    if let Some(on_unload) = hooks.get("onUnload").cloned() {
        day_reactive::Scope::current().on_cleanup(move || {
            on_unload(&[]);
        });
    }
    let built = builder(&[params]);
    let piece = match built {
        DynValue::Piece(p) => p.into_any(),
        _ => label(format!("day-lite: page `{route}` returned no piece")).any(),
    };
    if let Some(on_ready) = hooks.get("onReady") {
        on_ready(&[]);
    }
    piece
}

// ---- JSON bridge ------------------------------------------------------------------------

pub fn json_to_dyn(v: &serde_json::Value) -> DynValue {
    match v {
        serde_json::Value::Null => DynValue::Null,
        serde_json::Value::Bool(b) => DynValue::Bool(*b),
        serde_json::Value::Number(n) => DynValue::Num(n.as_f64().unwrap_or(0.0)),
        serde_json::Value::String(s) => DynValue::Str(s.clone()),
        serde_json::Value::Array(items) => DynValue::List(items.iter().map(json_to_dyn).collect()),
        serde_json::Value::Object(map) => DynValue::Map(
            map.iter()
                .map(|(k, v)| (k.clone(), json_to_dyn(v)))
                .collect(),
        ),
    }
}

pub fn dyn_to_json(v: &DynValue) -> serde_json::Value {
    match v {
        DynValue::Null | DynValue::Fn(_) | DynValue::Signal(_) | DynValue::Piece(_) => {
            serde_json::Value::Null
        }
        DynValue::Bool(b) => serde_json::Value::Bool(*b),
        DynValue::Num(n) => serde_json::Number::from_f64(*n)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        DynValue::Str(s) => serde_json::Value::String(s.clone()),
        DynValue::List(items) => serde_json::Value::Array(items.iter().map(dyn_to_json).collect()),
        DynValue::Map(entries) => serde_json::Value::Object(
            entries
                .iter()
                .map(|(k, v)| (k.clone(), dyn_to_json(v)))
                .collect(),
        ),
    }
}

// ---- API installation -------------------------------------------------------------------

/// The JS bootstrap: wraps the host hooks in the ergonomic classes and globals
/// (docs/lite.md §5–§7). Kept as plain JS so it needs no stripping.
const BOOTSTRAP: &str = include_str!("bootstrap.js");

fn day_construct<'js>(ctx: Ctx<'js>, name: String, args: Vec<Value<'js>>) -> rquickjs::Result<u32> {
    let dargs: Vec<DynValue> = args.iter().map(|v| from_js(&ctx, v, 16)).collect();
    let piece = dynreg::construct(&name, &dargs).map_err(|e| throw(&ctx, &e.to_string()))?;
    Ok(PIECES.with(|s| {
        s.borrow_mut().push(piece);
        (s.borrow().len() - 1) as u32
    }))
}

fn day_modify<'js>(
    ctx: Ctx<'js>,
    h: u32,
    name: String,
    args: Vec<Value<'js>>,
) -> rquickjs::Result<()> {
    let piece = PIECES
        .with(|s| s.borrow().get(h as usize).cloned())
        .ok_or_else(|| throw(&ctx, "stale piece handle"))?;
    let dargs: Vec<DynValue> = args.iter().map(|v| from_js(&ctx, v, 16)).collect();
    piece
        .modify(&name, &dargs)
        .map_err(|e| throw(&ctx, &e.to_string()))
}

fn day_signal<'js>(ctx: Ctx<'js>, init: Value<'js>) -> rquickjs::Result<u32> {
    let sig = match from_js(&ctx, &init, 4) {
        DynValue::Bool(b) => DynSignal::Bool(Signal::new(b)),
        DynValue::Num(n) => DynSignal::Num(Signal::new(n)),
        DynValue::Str(s) => DynSignal::Str(Signal::new(s)),
        _ => return Err(throw(&ctx, "signal(initial): bool, number, or string")),
    };
    Ok(SIGNALS.with(|s| {
        s.borrow_mut().push(sig);
        (s.borrow().len() - 1) as u32
    }))
}

fn day_sig_get(ctx: Ctx<'_>, h: u32) -> rquickjs::Result<String> {
    let sig = SIGNALS
        .with(|s| s.borrow().get(h as usize).cloned())
        .ok_or_else(|| throw(&ctx, "stale signal handle"))?;
    Ok(dyn_to_json(&sig.get()).to_string())
}

fn day_sig_set<'js>(ctx: Ctx<'js>, h: u32, v: Value<'js>) -> rquickjs::Result<()> {
    let sig = SIGNALS
        .with(|s| s.borrow().get(h as usize).cloned())
        .ok_or_else(|| throw(&ctx, "stale signal handle"))?;
    let dv = from_js(&ctx, &v, 8);
    sig.set(&dv).map_err(|e| throw(&ctx, &e.to_string()))
}

fn day_page<'js>(
    ctx: Ctx<'js>,
    route: String,
    builder: Value<'js>,
    hooks: Value<'js>,
) -> rquickjs::Result<()> {
    let DynValue::Fn(builder) = from_js(&ctx, &builder, 2) else {
        return Err(throw(
            &ctx,
            "page(route, builder): builder must be a function",
        ));
    };
    let mut hmap = HashMap::new();
    if let DynValue::Map(entries) = from_js(&ctx, &hooks, 4) {
        for (k, v) in entries {
            if let DynValue::Fn(f) = v {
                hmap.insert(k, f);
            }
        }
    }
    services().pages.borrow_mut().insert(
        route,
        PageDef {
            builder,
            hooks: hmap,
        },
    );
    Ok(())
}

fn day_app<'js>(ctx: Ctx<'js>, obj: Value<'js>) -> rquickjs::Result<()> {
    if let DynValue::Map(entries) = from_js(&ctx, &obj, 4) {
        let s = services();
        for (k, v) in entries {
            if let DynValue::Fn(f) = v {
                s.app_hooks.borrow_mut().insert(k, f);
            }
        }
    }
    Ok(())
}

fn day_nav<'js>(
    ctx: Ctx<'js>,
    op: String,
    route: String,
    params: Value<'js>,
) -> rquickjs::Result<()> {
    let s = services();
    let params = dyn_to_json(&from_js(&ctx, &params, 8));
    let params = if params.is_null() {
        String::new()
    } else {
        params.to_string()
    };
    let mut path = s.nav.get_untracked();
    match op.as_str() {
        "to" => path.push(NavEntry { route, params }),
        "back" => {
            path.pop();
        }
        "relaunch" => path.clear(),
        _ => return Err(throw(&ctx, "unknown nav op")),
    }
    s.nav.set(path);
    Ok(())
}

fn day_db<'js>(
    ctx: Ctx<'js>,
    op: String,
    sql: Value<'js>,
    params: Value<'js>,
) -> rquickjs::Result<String> {
    let s = services();
    let out: DynValue = match op.as_str() {
        "migrate" => {
            let steps: Vec<String> = match from_js(&ctx, &sql, 4) {
                DynValue::List(items) => items
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect(),
                _ => return Err(throw(&ctx, "db.migrate([...ddl])")),
            };
            let n = s.db.migrate(&steps).map_err(|e| throw(&ctx, &e.0))?;
            DynValue::Num(n as f64)
        }
        "exec" => {
            let sql = sql
                .as_string()
                .and_then(|s| s.to_string().ok())
                .ok_or_else(|| throw(&ctx, "db.exec(sql, params?)"))?;
            let p = db_params(&ctx, &params);
            let (changes, rowid) = s.db.exec(&sql, &p).map_err(|e| throw(&ctx, &e.0))?;
            DynValue::Map(vec![
                ("changes".into(), DynValue::Num(changes as f64)),
                ("lastInsertRowId".into(), DynValue::Num(rowid as f64)),
            ])
        }
        "query" => {
            let sql = sql
                .as_string()
                .and_then(|s| s.to_string().ok())
                .ok_or_else(|| throw(&ctx, "db.query(sql, params?)"))?;
            let p = db_params(&ctx, &params);
            let rows = s.db.query(&sql, &p).map_err(|e| throw(&ctx, &e.0))?;
            DynValue::List(
                rows.into_iter()
                    .map(|row| {
                        DynValue::Map(
                            row.into_iter()
                                .map(|(k, c)| {
                                    (
                                        k,
                                        match c {
                                            DbCell::Null => DynValue::Null,
                                            DbCell::Int(i) => DynValue::Num(i as f64),
                                            DbCell::Real(f) => DynValue::Num(f),
                                            DbCell::Text(t) => DynValue::Str(t),
                                        },
                                    )
                                })
                                .collect(),
                        )
                    })
                    .collect(),
            )
        }
        _ => return Err(throw(&ctx, "unknown db op")),
    };
    Ok(dyn_to_json(&out).to_string())
}

fn day_fs<'js>(
    ctx: Ctx<'js>,
    op: String,
    path: String,
    arg: Value<'js>,
) -> rquickjs::Result<String> {
    let s = services();
    let fs = &s.fs;
    let out = match op.as_str() {
        "read" => fs
            .read(&path)
            .map(|b| DynValue::Str(String::from_utf8_lossy(&b).into_owned())),
        "write" => {
            let data = arg
                .as_string()
                .and_then(|v| v.to_string().ok())
                .unwrap_or_default();
            fs.write(&path, data.as_bytes()).map(|_| DynValue::Null)
        }
        "mkdir" => fs.mkdir(&path).map(|_| DynValue::Null),
        "size" => fs.size(&path).map(|n| DynValue::Num(n as f64)),
        "entries" => fs.entries(&path).map(|items| {
            DynValue::List(
                items
                    .into_iter()
                    .map(|(name, kind)| {
                        DynValue::Map(vec![
                            ("name".into(), DynValue::Str(name)),
                            (
                                "kind".into(),
                                DynValue::Str(
                                    match kind {
                                        crate::fsx::EntryKind::File => "file",
                                        crate::fsx::EntryKind::Directory => "directory",
                                    }
                                    .into(),
                                ),
                            ),
                        ])
                    })
                    .collect(),
            )
        }),
        "remove" => {
            let recursive = arg.as_bool().unwrap_or(false);
            fs.remove(&path, recursive).map(|_| DynValue::Null)
        }
        "exists" => fs.exists(&path).map(|k| match k {
            None => DynValue::Null,
            Some(crate::fsx::EntryKind::File) => DynValue::Str("file".into()),
            Some(crate::fsx::EntryKind::Directory) => DynValue::Str("directory".into()),
        }),
        _ => return Err(throw(&ctx, "unknown fs op")),
    };
    match out {
        Ok(v) => Ok(dyn_to_json(&v).to_string()),
        Err(e) => Err(throw(&ctx, &e.to_string())),
    }
}

fn day_t<'js>(ctx: Ctx<'js>, key: String, args: Value<'js>) -> rquickjs::Result<String> {
    let s = services();
    let mut fargs: Vec<(String, String)> = Vec::new();
    if let DynValue::Map(entries) = from_js(&ctx, &args, 4) {
        for (k, v) in entries {
            fargs.push((k, v.display()));
        }
    }
    Ok(s.i18n.t(&s.store, &s.app_id, &key, &fargs))
}

fn day_timeout<'js>(ctx: Ctx<'js>, f: Value<'js>, ms: f64) -> rquickjs::Result<()> {
    let DynValue::Fn(cb) = from_js(&ctx, &f, 2) else {
        return Err(throw(&ctx, "setTimeout(fn, ms)"));
    };
    day_core::task(async move {
        crate::sleep::sleep_ms(ms.max(0.0) as u64).await;
        cb(&[]);
    });
    Ok(())
}

fn install_api(ctx: &Ctx<'_>, bridges: &[Bridge]) -> rquickjs::Result<()> {
    let g = ctx.globals();

    // Piece construction/modification over the dyn registry.
    g.set(
        "__day_construct",
        Function::new(ctx.clone(), day_construct)?,
    )?;
    g.set("__day_modify", Function::new(ctx.clone(), day_modify)?)?;
    let names: Vec<String> = dynreg::catalog()
        .iter()
        .filter(|e| e.kind == dynreg::SpecKind::Constructor)
        .map(|e| e.name.to_string())
        .collect();
    g.set("__day_ctors", names)?;
    let names: Vec<String> = dynreg::catalog()
        .iter()
        .filter(|e| e.kind != dynreg::SpecKind::Constructor)
        .map(|e| e.name.to_string())
        .collect();
    g.set("__day_modifiers", names)?;

    // Signals (docs/lite.md §5): typed at creation from the initial value.
    g.set("__day_signal", Function::new(ctx.clone(), day_signal)?)?;
    g.set("__day_sig_get", Function::new(ctx.clone(), day_sig_get)?)?;
    g.set("__day_sig_set", Function::new(ctx.clone(), day_sig_set)?)?;

    // App/page registration + navigation (docs/lite.md §6).
    g.set("__day_page", Function::new(ctx.clone(), day_page)?)?;
    g.set("__day_app", Function::new(ctx.clone(), day_app)?)?;
    g.set("__day_nav", Function::new(ctx.clone(), day_nav)?)?;

    // Storage: sqlite + sandboxed fs (docs/lite.md §7) — sync, app-scoped.
    g.set("__day_db", Function::new(ctx.clone(), day_db)?)?;
    g.set("__day_fs", Function::new(ctx.clone(), day_fs)?)?;

    // Permission probe + log + system info.
    g.set(
        "__day_can",
        Function::new(ctx.clone(), |perm: String| -> bool {
            with_services(|s| s.permissions.granted(&perm)).unwrap_or(false)
        })?,
    )?;
    g.set(
        "__day_log",
        Function::new(ctx.clone(), |level: String, msg: String| {
            eprintln!("day-lite[{level}]: {msg}");
            let _ = with_services(|s| s.log.borrow_mut().push(format!("[{level}] {msg}")));
        })?,
    )?;
    g.set(
        "__day_sysinfo",
        Function::new(ctx.clone(), || -> String {
            let out = with_services(|s| {
                DynValue::Map(vec![
                    (
                        "platform".into(),
                        DynValue::Str(std::env::consts::OS.into()),
                    ),
                    ("appId".into(), DynValue::Str(s.app_id.clone())),
                    (
                        "version".into(),
                        DynValue::Str(s.manifest.version.name.clone()),
                    ),
                ])
            })
            .unwrap_or(DynValue::Null);
            dyn_to_json(&out).to_string()
        })?,
    )?;

    // Timers: a thread parks for the duration, then the callback re-enters on the main
    // thread through `day_core::task`'s waker (docs/lite.md §7).
    g.set("__day_timeout", Function::new(ctx.clone(), day_timeout)?)?;

    // Fluent localization over the package's i18n/<locale>.ftl files (docs/lite.md §7).
    g.set("__day_t", Function::new(ctx.clone(), day_t)?)?;

    // The ergonomic layer, BEFORE bridges: their installers attach under `day.`.
    ctx.eval::<(), _>(BOOTSTRAP)?;

    // Host bridges (net, sensors, custom): each installs behind its permission gate.
    for bridge in bridges {
        let granted = with_services(|s| {
            s.permissions.granted(bridge.permission) && s.manifest_declares(bridge.permission)
        })
        .unwrap_or(false);
        (bridge.install)(ctx, granted)?;
    }
    Ok(())
}

impl Services {
    fn manifest_declares(&self, permission: &str) -> bool {
        self.manifest
            .req_permissions
            .iter()
            .any(|p| p.name == permission)
    }
}

fn db_params<'js>(ctx: &Ctx<'js>, v: &Value<'js>) -> Vec<DbCell> {
    match from_js(ctx, v, 4) {
        DynValue::List(items) => items
            .iter()
            .map(|v| match v {
                DynValue::Null => DbCell::Null,
                DynValue::Bool(b) => DbCell::Int(*b as i64),
                DynValue::Num(n) if n.fract() == 0.0 && n.abs() < 9e15 => DbCell::Int(*n as i64),
                DynValue::Num(n) => DbCell::Real(*n),
                DynValue::Str(s) => DbCell::Text(s.clone()),
                _ => DbCell::Null,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Throw a JS exception carrying `msg`; the returned error propagates it.
pub fn throw(ctx: &Ctx<'_>, msg: &str) -> rquickjs::Error {
    let _ = ctx.throw(
        rquickjs::String::from_str(ctx.clone(), msg)
            .map(|s| s.into_value())
            .unwrap_or_else(|_| Value::new_null(ctx.clone())),
    );
    rquickjs::Error::Exception
}

/// The running app's `Context` (for re-entering JS from async completions).
pub(crate) fn current_context() -> Option<Context> {
    SERVICES.with(|s| s.borrow().as_ref().and_then(|s| s.context.borrow().clone()))
}
