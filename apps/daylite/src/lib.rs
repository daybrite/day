//! Daylite — the reference day-lite **superapp** (docs/lite.md §12): browse a catalog of
//! JS/TS miniapps, install them with permission disclosure, keep them updated, and run them
//! in a fullscreen cover — all through `day_lite::Host`, which any other superapp embeds
//! the same way. Mobile-first (iOS / Android / HarmonyOS).
//!
//! The three bundled samples (weather / todos / tic-tac-toe) are materialized to disk and
//! installed through the ordinary local-origin path, so the whole install/update pipeline
//! runs even offline; "Add from URL" installs from any https static host (a raw git branch
//! URL) with the same flow.

mod bundled;

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use day::prelude::*;
use day_lite::{Host, InstallPlan, Store};

/// Theme-neutral card surface (the showcase idiom): translucent mid-grey reads as a card
/// on BOTH light and dark platform themes, so default label colors stay legible.
const CARD: Color = Color::rgba(0.5, 0.5, 0.55, 0.16);

thread_local! {
    static HOST: RefCell<Option<Rc<Host>>> = const { RefCell::new(None) };
    static RUNNING: RefCell<Option<day_lite::LiteApp>> = const { RefCell::new(None) };
    /// The install plan awaiting the user's disclosure decision (not `Clone`; the signal
    /// carries only its display summary).
    static PENDING: RefCell<Option<InstallPlan>> = const { RefCell::new(None) };
    /// Background-job completions keyed by job id (see [`bg`]).
    static JOBS: RefCell<HashMap<u64, Job>> = RefCell::new(HashMap::new());
}

// ---- store root -------------------------------------------------------------------------

/// The persistent store root, per platform. iOS/macOS: the app container's Application
/// Support; Android: `Context.getFilesDir()` via day-android's JNI bridge (resolved ON the
/// main thread); OHOS: the sandbox files dir day-part-prefs also uses; elsewhere: a dotdir.
fn store_root() -> PathBuf {
    #[cfg(target_os = "android")]
    if let Some(dir) = android_files_dir() {
        return dir.join("daylite");
    }
    #[cfg(all(target_os = "linux", target_env = "ohos"))]
    {
        for var in ["OHOS_APP_FILES_DIR", "HOME", "TMPDIR"] {
            if let Some(dir) = std::env::var_os(var)
                && !dir.is_empty()
            {
                return PathBuf::from(dir).join("daylite");
            }
        }
        return PathBuf::from("/data/storage/el2/base/haps/entry/files/daylite");
    }
    #[allow(unreachable_code)]
    {
        if let Some(home) = std::env::var_os("HOME") {
            let base = PathBuf::from(home);
            #[cfg(any(target_os = "ios", target_os = "macos"))]
            return base.join("Library/Application Support/daylite");
            #[allow(unreachable_code)]
            base.join(".daylite")
        } else {
            std::env::temp_dir().join("daylite")
        }
    }
}

/// `Context.getFilesDir().getAbsolutePath()` through day-android's JNI bridge
/// (`DayBridge.filesDirPath`), using the `DayEnv` helpers whose class resolution works from
/// any thread. Called on the main thread at host init.
#[cfg(target_os = "android")]
fn android_files_dir() -> Option<PathBuf> {
    use day_android::{DayEnv, as_jstring, with_env};
    with_env(|env| {
        let obj = env
            .dcall_static(
                "dev/daybrite/day/bridge/DayBridge",
                "filesDirPath",
                "()Ljava/lang/String;",
                &[],
            )
            .ok()?
            .l()
            .ok()?;
        if obj.is_null() {
            return None;
        }
        let path = env.dstr(&as_jstring(obj)).ok()?;
        if path.is_empty() {
            None
        } else {
            Some(PathBuf::from(path))
        }
    })
}

fn host() -> Rc<Host> {
    HOST.with(|h| {
        h.borrow_mut()
            .get_or_insert_with(|| {
                Rc::new(
                    Host::builder()
                        .store(Store::at(store_root().join("store")))
                        .bridge(day_lite::net())
                        .bridge(day_lite::prefs())
                        .build(),
                )
            })
            .clone()
    })
}

// ---- background jobs --------------------------------------------------------------------

type Job = Box<dyn FnOnce()>;

/// Run `work` off the main thread, then `done(result)` back ON it. Delivery rides a tick
/// signal's `Setter`; completions are keyed so coalesced ticks still run every job.
fn bg<T: Send + 'static>(
    tick: Signal<f64>,
    work: impl FnOnce() -> T + Send + 'static,
    done: impl FnOnce(T) + 'static,
) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let (tx, rx) = std::sync::mpsc::channel::<T>();
    JOBS.with(|j| {
        j.borrow_mut().insert(
            id,
            Box::new(move || {
                if let Ok(v) = rx.try_recv() {
                    done(v);
                }
            }),
        )
    });
    let setter = tick.setter();
    std::thread::spawn(move || {
        let v = work();
        let _ = tx.send(v);
        setter.set(id as f64);
    });
}

/// Drain every ready job (tick payloads coalesce; readiness is per-job channel state).
fn drain_jobs() {
    let ready: Vec<Job> = JOBS.with(|j| {
        let mut map = j.borrow_mut();
        let keys: Vec<u64> = map.keys().copied().collect();
        let mut out = Vec::new();
        for k in keys {
            if let Some(job) = map.remove(&k) {
                out.push(job);
            }
        }
        out
    });
    for job in ready {
        job();
    }
}

// ---- install / open flows ---------------------------------------------------------------

/// The disclosure sheet's display model.
#[derive(Clone, PartialEq)]
struct Disclosure {
    name: String,
    version: String,
    origin: String,
    permissions: Vec<(String, String)>,
}

fn permission_label(id: &str) -> String {
    id.strip_prefix("day.permission.").unwrap_or(id).to_string()
}

struct Ui {
    open: Signal<Option<String>>,
    trace: Signal<String>,
    rev: Signal<f64>,
    tick: Signal<f64>,
    status: Signal<String>,
    disclosure: Signal<Option<Disclosure>>,
    add_url: Signal<String>,
}

impl Ui {
    fn bump(&self) {
        self.rev.set(self.rev.get_untracked() + 1.0);
    }
}

fn begin_install(ui: &Ui, origin: String) {
    let status = ui.status;
    let disclosure = ui.disclosure;
    status.set(format!("Fetching {origin}…"));
    let store = host().store().clone();
    bg(
        ui.tick,
        move || store.install(&origin),
        move |result| match result {
            Ok(plan) => {
                status.set(String::new());
                let d = Disclosure {
                    name: plan.manifest.name.clone(),
                    version: plan.manifest.version.name.clone(),
                    origin: plan.origin.as_str(),
                    permissions: plan
                        .manifest
                        .req_permissions
                        .iter()
                        .map(|p| (permission_label(&p.name), p.reason.clone()))
                        .collect(),
                };
                PENDING.with(|p| *p.borrow_mut() = Some(plan));
                disclosure.set(Some(d));
            }
            Err(e) => status.set(format!("Install failed: {e}")),
        },
    );
}

fn confirm_install(ui: &Ui) {
    let Some(plan) = PENDING.with(|p| p.borrow_mut().take()) else {
        return;
    };
    ui.disclosure.set(None);
    let granted: Vec<String> = plan
        .manifest
        .req_permissions
        .iter()
        .map(|p| p.name.clone())
        .collect();
    let status = ui.status;
    match plan.confirm(&granted) {
        Ok(m) => status.set(format!("Installed {}", m.name)),
        Err(e) => status.set(format!("Install failed: {e}")),
    }
    // Refresh the app lists on a FRESH main-loop turn: this handler runs inside the
    // disclosure sheet's arm, which the `disclosure.set(None)` above is about to unmount,
    // and subtree rebuilds triggered from a dying arm's flush must not parent into it.
    let rev = ui.rev;
    day::task(async move {
        day_lite::sleep_ms(30).await;
        rev.set(rev.get_untracked() + 1.0);
    });
}

fn open_app(ui: &Ui, app_id: &str) {
    ui.trace.set(format!("open requested: {app_id}"));
    open_attempt(ui.open, ui.trace, ui.tick, app_id.to_string(), 20);
}

/// Launch, retrying briefly while the previous app's teardown (which rides its cover's
/// disposal — see `run_cover`) is still releasing the runtime slot.
fn open_attempt(
    open: Signal<Option<String>>,
    trace: Signal<String>,
    tick: Signal<f64>,
    app_id: String,
    tries: u8,
) {
    match host().launch(&app_id) {
        Ok(app) => {
            RUNNING.with(|r| *r.borrow_mut() = Some(app));
            trace.set(format!("launched {app_id}"));
            open.set(Some(app_id));
        }
        Err(e) if e.contains("already running") && tries > 0 => {
            trace.set(format!("waiting for teardown ({tries})"));
            // Retry over the bg/Setter channel (the delivery path every install flow
            // already exercises), not a task-held timer.
            bg(
                tick,
                || std::thread::sleep(std::time::Duration::from_millis(300)),
                move |_| open_attempt(open, trace, tick, app_id, tries - 1),
            );
        }
        Err(e) => trace.set(format!("open failed: {e}")),
    }
}

fn close_app(ui: &Ui) {
    // The runtime teardown rides the cover's disposal (see `run_cover`): when the hidden
    // cover's presentation scope is cleaned up, `RUNNING` drops with no live bindings.
    ui.open.set(None);
}

fn update_app(ui: &Ui, app_id: String) {
    let status = ui.status;
    let rev = ui.rev;
    status.set(format!("Checking {app_id}…"));
    let store = host().store().clone();
    bg(
        ui.tick,
        move || match store.check_update(&app_id) {
            Ok(None) => Ok("Already up to date".to_string()),
            // Auto-grant nothing new: updates that ADD permissions keep them ungranted
            // until the user revisits the app's disclosure (docs/lite.md §8).
            Ok(Some(plan)) => plan
                .apply(&[])
                .map(|m| format!("Updated to {}", m.version.name))
                .map_err(|e| e.to_string()),
            Err(e) => Err(e.to_string()),
        },
        move |result| {
            match result {
                Ok(msg) => status.set(msg),
                Err(e) => status.set(format!("Update failed: {e}")),
            }
            rev.set(rev.get_untracked() + 1.0);
        },
    );
}

// ---- UI ---------------------------------------------------------------------------------

fn section_title(text: &'static str) -> AnyPiece {
    label(text).font(Font::Headline).any()
}

fn card(content: AnyPiece) -> AnyPiece {
    content.padding(12.0).background(CARD).corner_radius(12.0)
}

fn home(ui: Rc<Ui>) -> AnyPiece {
    let installed = {
        let rev = ui.rev;
        move || {
            rev.get();
            host()
                .store()
                .installed()
                .into_iter()
                .map(|(id, m)| (id, m.name, m.version.name))
                .collect::<Vec<_>>()
        }
    };
    let u1 = ui.clone();
    let u2 = ui.clone();
    let u3 = ui.clone();
    let u4 = ui.clone();
    let status = ui.status;
    let trace = ui.trace;

    scroll(
        column((
            label("Daylite").font(Font::LargeTitle),
            label("Miniapps on day — installed from the web, run native").font(Font::Footnote),
            when(
                move || !status.get().is_empty(),
                move || {
                    label(move || status.get())
                        .font(Font::Callout)
                        .id("dl-status")
                },
            ),
            when(
                move || !trace.get().is_empty(),
                move || {
                    label(move || trace.get())
                        .font(Font::Caption)
                        .id("dl-trace")
                },
            ),
            section_title("My apps"),
            each(
                installed,
                |(id, _, _): &(String, String, String)| id.clone(),
                move |slot| {
                    let (id, name, version) = slot.get();
                    let open_ui = u1.clone();
                    let update_ui = u2.clone();
                    let remove_ui = u3.clone();
                    let (id_o, id_u, id_r) = (id.clone(), id.clone(), id.clone());
                    card(
                        row((
                            column((
                                label(name).font(Font::Headline),
                                label(version).font(Font::Caption),
                            ))
                            .align(HAlign::Leading)
                            .any()
                            .grow_w(),
                            button("Open")
                                .action(move || open_app(&open_ui, &id_o))
                                .id_keyed("dl-open", &id),
                            button("Update").action(move || update_app(&update_ui, id_u.clone())),
                            button("Remove").action(move || {
                                let _ = host().store().remove(&id_r);
                                remove_ui.bump();
                            }),
                        ))
                        .spacing(8.0)
                        .align(VAlign::Center)
                        .any(),
                    )
                },
            ),
            section_title("Catalog"),
            catalog_rows(ui.clone()),
            section_title("Add from URL"),
            card(
                column((
                    label("Any https static host works — e.g. a raw git branch URL")
                        .font(Font::Caption),
                    row((
                        text_field(ui.add_url)
                            .placeholder("https://raw.githubusercontent.com/…/main")
                            .id("dl-add-url"),
                        button("Add")
                            .action(move || {
                                let url = u4.add_url.get_untracked().trim().to_string();
                                if !url.is_empty() {
                                    begin_install(&u4, url);
                                }
                            })
                            .id("dl-add"),
                    ))
                    .spacing(8.0)
                    .any(),
                ))
                .spacing(8.0)
                .align(HAlign::Leading)
                .any(),
            ),
        ))
        .spacing(12.0)
        .align(HAlign::Leading)
        .any(),
    )
    .any()
    .padding(16.0)
    .id("dl-root")
}

/// The bundled samples ARE the default catalog: real packages at local origins, installed
/// through the normal disclosure flow the moment the user taps Install.
fn catalog_rows(ui: Rc<Ui>) -> AnyPiece {
    let entries: Vec<(String, PathBuf)> = bundled::materialize(&store_root());
    let rev = ui.rev;
    let rows = {
        let entries = entries.clone();
        move || {
            rev.get();
            let installed: Vec<String> = host()
                .store()
                .installed()
                .into_iter()
                .map(|(id, _)| id)
                .collect();
            entries
                .iter()
                .map(|(id, dir)| {
                    let manifest = host()
                        .store()
                        .install(&dir.display().to_string())
                        .map(|p| p.manifest)
                        .ok();
                    let (name, desc, version) = manifest
                        .map(|m| (m.name, m.description, m.version.name))
                        .unwrap_or_else(|| (id.clone(), String::new(), String::new()));
                    (
                        id.clone(),
                        dir.display().to_string(),
                        name,
                        desc,
                        version,
                        installed.contains(id),
                    )
                })
                .collect::<Vec<_>>()
        }
    };
    // Rows key by app id ONLY (stable across installs); the Install↔Open switch is a
    // reactive `when` pair inside the row, so an install patches the row rather than
    // re-keying it (mid-list re-keys are also where uikit's child-diff is least exercised).
    each(
        rows,
        |(id, _, _, _, _, _): &(String, String, String, String, String, bool)| id.clone(),
        move |slot| {
            let (id, origin, name, desc, version, _) = slot.get();
            let ui = ui.clone();
            let rev = ui.rev;
            let is_installed = {
                let id = id.clone();
                move || {
                    rev.get();
                    host().store().record(&id).is_some()
                }
            };
            let not_installed = {
                let is_installed = is_installed.clone();
                move || !is_installed()
            };
            let open_ui = ui.clone();
            let id_open = id.clone();
            card(
                row((
                    column((
                        label(name).font(Font::Headline),
                        label(desc).font(Font::Caption),
                        label(version).font(Font::Caption2),
                    ))
                    .align(HAlign::Leading)
                    .any()
                    .grow_w(),
                    when(is_installed.clone(), move || {
                        let open_ui = open_ui.clone();
                        let id = id_open.clone();
                        let id_action = id.clone();
                        button("Open")
                            .action(move || open_app(&open_ui, &id_action))
                            .id_keyed("dl-catalog-open", &id)
                    }),
                    when(not_installed, move || {
                        let ui = ui.clone();
                        let origin = origin.clone();
                        button("Install")
                            .action(move || begin_install(&ui, origin.clone()))
                            .id_keyed("dl-install", &id)
                    }),
                ))
                .spacing(8.0)
                .align(VAlign::Center)
                .any(),
            )
        },
    )
}

/// The permission-disclosure sheet: nothing installs until "Install" here.
fn disclosure_sheet(ui: Rc<Ui>) -> AnyPiece {
    let disclosure = ui.disclosure;
    when(
        move || disclosure.get().is_some(),
        move || {
            let Some(d) = disclosure.get() else {
                return spacer().any();
            };
            // Modal presentation over live content needs BOTH an OPAQUE, theme-tracking
            // panel surface (a translucent card composites its text over the page's) and
            // a scrim that mutes what's underneath. `day::dark_mode()` picks the surface
            // the platform's default text colors are legible on.
            let (panel, scrim) = if day::dark_mode() {
                (Color::hex(0x2C_2C_2E), Color::rgba(0.0, 0.0, 0.0, 0.55))
            } else {
                (Color::hex(0xFF_FF_FF), Color::rgba(0.0, 0.0, 0.0, 0.45))
            };
            let cancel_scrim = ui.disclosure;
            let ui_ok = ui.clone();
            let cancel = ui.disclosure;
            let perms: Vec<AnyPiece> = if d.permissions.is_empty() {
                vec![label("No permissions requested").font(Font::Callout).any()]
            } else {
                d.permissions
                    .iter()
                    .map(|(name, reason)| {
                        row((
                            label(name.clone()).font(Font::Headline),
                            label(reason.clone()).font(Font::Caption).grow_w(),
                        ))
                        .spacing(10.0)
                        .any()
                    })
                    .collect()
            };
            let sheet = column((
                label(format!("Install {}?", d.name)).font(Font::Title2),
                label(format!("Version {} — from {}", d.version, d.origin)).font(Font::Caption),
                label("This app can:").font(Font::Callout),
                column(PieceVec(perms))
                    .spacing(6.0)
                    .align(HAlign::Leading)
                    .any(),
                row((
                    button("Cancel")
                        .action(move || {
                            PENDING.with(|p| p.borrow_mut().take());
                            cancel.set(None);
                        })
                        .id("dl-cancel"),
                    button("Install")
                        .action(move || confirm_install(&ui_ok))
                        .id("dl-confirm"),
                ))
                .spacing(12.0)
                .any(),
            ))
            .spacing(10.0)
            .align(HAlign::Leading)
            .any()
            .padding(16.0)
            .background(panel)
            .corner_radius(16.0)
            .id("dl-disclosure");
            // Scrim behind, sheet in front; tapping the scrim cancels (the standard
            // modal affordance, and it keeps taps from reaching the page beneath).
            zstack((
                spacer()
                    .any()
                    .grow()
                    .background(scrim)
                    .on_tap(move || {
                        PENDING.with(|p| p.borrow_mut().take());
                        cancel_scrim.set(None);
                    })
                    .id("dl-scrim"),
                sheet.padding(24.0),
            ))
            .any()
            .grow()
        },
    )
    .any()
}

/// The running miniapp, fullscreen with the X-to-exit affordance (docs/cover.md).
fn run_cover(ui: Rc<Ui>) -> AnyPiece {
    let open = ui.open;
    cover(open, move |app_id: &String| {
        let surface = RUNNING.with(|r| r.borrow().as_ref().map(|a| a.surface()));
        let body = surface.unwrap_or_else(|| label("This app is no longer running").any());
        // The build runs inside the cover's presentation scope, which is disposed exactly
        // when the dismissed cover finishes hiding — the one moment the JS runtime can be
        // dropped with no live piece bindings left to call into it.
        let status = ui.status;
        Scope::current().on_cleanup(move || {
            RUNNING.with(|r| r.borrow_mut().take());
            status.set(String::new());
        });
        let close_ui = ui.clone();
        // `.id` directly on the tap-handling node (before the padding wrapper): scripted
        // taps dispatch against the id, and the handler lives on the label itself.
        let close = label("✕")
            .font(Font::Title2)
            .on_tap(move || close_app(&close_ui))
            .id("dl-close")
            .padding(14.0);
        let _ = app_id;
        body.grow()
            .overlay_aligned(Alignment::TopLeading, close)
            .any()
    })
    .any()
}

pub fn root() -> AnyPiece {
    // Scripted runs start from a clean store (`day launch --env DAYLITE_RESET=1`);
    // normal launches keep installed apps and their data.
    if std::env::var_os("DAYLITE_RESET").is_some() {
        let _ = std::fs::remove_dir_all(store_root());
    }
    let ui = Rc::new(Ui {
        open: Signal::new(None),
        trace: Signal::new(String::new()),
        rev: Signal::new(0.0),
        tick: Signal::new(0.0),
        status: Signal::new(String::new()),
        disclosure: Signal::new(None),
        add_url: Signal::new(String::new()),
    });
    // Background-job delivery: any tick drains every ready completion (see `bg`).
    let tick = ui.tick;
    watch(move || tick.get(), |_, _| drain_jobs());
    zstack((
        home(ui.clone()),
        disclosure_sheet(ui.clone()),
        run_cover(ui),
    ))
    .any()
}

day::ios_main!("Daylite", root);
day::android_main!(root);
day::arkui_main!(root);
