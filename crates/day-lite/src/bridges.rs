//! Host bridges (docs/lite.md §7, §10): native capabilities exposed to scripts, each
//! behind a permission id. A bridge installs its namespace whether or not the permission
//! was granted — ungranted entry points REJECT with `PermissionError` so feature detection
//! is `day.can('NETWORK')`, never try/catch guessing.

use rquickjs::{Ctx, Function, Value};

use day_pieces::dynreg::DynValue;

use crate::engine::{throw, with_services};
use crate::value::from_js;

/// One native capability. `install` runs at context setup with `granted` resolved from the
/// manifest + the user's grants; it must install BOTH the granted and the rejecting shape.
pub struct Bridge {
    pub namespace: &'static str,
    pub permission: &'static str,
    pub install: fn(&Ctx<'_>, bool) -> rquickjs::Result<()>,
}

/// `day.net.fetch(url, opts?)` over day-part-http (docs/lite.md §7). Wants NETWORK; URLs
/// must match a `day.net_origins` prefix from the manifest.
pub fn net() -> Bridge {
    Bridge {
        namespace: "net",
        permission: crate::permission::NETWORK,
        install: install_net,
    }
}

fn origin_allowed(url: &str) -> bool {
    with_services(|s| {
        s.manifest
            .day
            .net_origins
            .iter()
            .any(|prefix| url.starts_with(prefix.as_str()))
    })
    .unwrap_or(false)
}

fn day_fetch<'js>(
    ctx: Ctx<'js>,
    url: String,
    opts: Value<'js>,
    resolve: Function<'js>,
    reject: Function<'js>,
) -> rquickjs::Result<()> {
    {
        {
            {
                if !ctx
                    .globals()
                    .get::<_, bool>("__day_net_granted")
                    .unwrap_or(false)
                {
                    return Err(throw(&ctx, "PermissionError: NETWORK is not granted"));
                }
                if !origin_allowed(&url) {
                    return Err(throw(
                        &ctx,
                        "NetError: url is outside the manifest's day.net_origins",
                    ));
                }
                let opts = from_js(&ctx, &opts, 8);
                let mut method = "GET".to_string();
                let mut body: Option<Vec<u8>> = None;
                let mut headers: Vec<(String, String)> = Vec::new();
                if let DynValue::Map(entries) = &opts {
                    for (k, v) in entries {
                        match (k.as_str(), v) {
                            ("method", DynValue::Str(m)) => method = m.to_uppercase(),
                            ("body", DynValue::Str(b)) => body = Some(b.clone().into_bytes()),
                            ("headers", DynValue::Map(hs)) => {
                                for (hk, hv) in hs {
                                    if let DynValue::Str(hv) = hv {
                                        headers.push((hk.clone(), hv.clone()));
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                let mut req = match method.as_str() {
                    "GET" => day_part_http::Request::get(&url),
                    "POST" => day_part_http::Request::post(&url, body.take().unwrap_or_default()),
                    "PUT" => day_part_http::Request::put(&url, body.take().unwrap_or_default()),
                    "DELETE" => day_part_http::Request::delete(&url),
                    "HEAD" => day_part_http::Request::head(&url),
                    _ => return Err(throw(&ctx, "NetError: unsupported method")),
                };
                for (k, v) in &headers {
                    req = req.header(k, v);
                }

                let resolve = rquickjs::Persistent::save(&ctx, resolve);
                let reject = rquickjs::Persistent::save(&ctx, reject);
                let Some(context) = crate::engine::current_context() else {
                    return Err(throw(&ctx, "NetError: no running miniapp"));
                };
                day_core::task(async move {
                    let result = day_part_http::fetch_future(req).await;
                    context.with(|ctx| {
                        match result {
                            Ok(resp) => {
                                let headers = DynValue::Map(
                                    resp.headers
                                        .iter()
                                        .map(|(k, v)| (k.to_lowercase(), DynValue::Str(v.clone())))
                                        .collect(),
                                );
                                let out = DynValue::Map(vec![
                                    ("ok".into(), DynValue::Bool(resp.status < 400)),
                                    ("status".into(), DynValue::Num(resp.status as f64)),
                                    ("headers".into(), headers),
                                    ("body".into(), DynValue::Str(resp.text().into_owned())),
                                ]);
                                let json = crate::engine::dyn_to_json(&out).to_string();
                                if let Ok(resolve) = resolve.restore(&ctx) {
                                    let _ = resolve.call::<_, ()>((json,));
                                }
                            }
                            Err(e) => {
                                if let Ok(reject) = reject.restore(&ctx) {
                                    let _ = reject.call::<_, ()>((format!("NetError: {e:?}"),));
                                }
                            }
                        }
                        while ctx.execute_pending_job() {}
                    });
                });
                Ok(())
            }
        }
    }
}

fn install_net(ctx: &Ctx<'_>, granted: bool) -> rquickjs::Result<()> {
    ctx.globals().set("__day_net_granted", granted)?;
    ctx.globals()
        .set("__day_fetch", Function::new(ctx.clone(), day_fetch)?)?;
    // The ergonomic wrapper: response.text()/json() like the web fetch.
    ctx.eval::<(), _>(
        r#"day.net = { fetch: (url, opts) => new Promise((res, rej) => {
             __day_fetch(url, opts ?? null, (json) => {
               const r = JSON.parse(json);
               res({ ok: r.ok, status: r.status, headers: r.headers,
                     text: () => r.body, json: () => JSON.parse(r.body) });
             }, (e) => rej(new Error(e)));
           }) };"#,
    )?;
    Ok(())
}

/// `day.prefs` over day-part-prefs, app-scoped by key prefix (docs/lite.md §7).
pub fn prefs() -> Bridge {
    Bridge {
        namespace: "prefs",
        permission: crate::permission::PREFS,
        install: install_prefs,
    }
}

fn scoped(key: &str) -> String {
    let app = with_services(|s| s.app_id.clone()).unwrap_or_default();
    format!("lite.{app}.{key}")
}

fn install_prefs(ctx: &Ctx<'_>, granted: bool) -> rquickjs::Result<()> {
    ctx.globals().set("__day_prefs_granted", granted)?;
    ctx.globals().set(
        "__day_prefs",
        Function::new(
            ctx.clone(),
            |ctx: Ctx<'_>, op: String, key: String, value: String| -> rquickjs::Result<String> {
                if !ctx
                    .globals()
                    .get::<_, bool>("__day_prefs_granted")
                    .unwrap_or(false)
                {
                    return Err(throw(&ctx, "PermissionError: PREFS is not granted"));
                }
                let k = scoped(&key);
                let out = match op.as_str() {
                    "get" => day_part_prefs::get(&k)
                        .map(DynValue::Str)
                        .unwrap_or(DynValue::Null),
                    "set" => {
                        day_part_prefs::set(&k, &value);
                        DynValue::Null
                    }
                    "remove" => {
                        day_part_prefs::remove(&k);
                        DynValue::Null
                    }
                    _ => return Err(throw(&ctx, "unknown prefs op")),
                };
                Ok(crate::engine::dyn_to_json(&out).to_string())
            },
        )?,
    )?;
    ctx.eval::<(), _>(
        r#"day.prefs = { get: (k) => JSON.parse(__day_prefs("get", k, "")),
                         set: (k, v) => { __day_prefs("set", k, String(v)); },
                         remove: (k) => { __day_prefs("remove", k, ""); } };"#,
    )?;
    Ok(())
}
