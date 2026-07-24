//! DynValue ⇄ JS conversion (docs/lite.md §5): typed object graphs across the boundary —
//! never stringified eval. Pieces and signals cross as integer handles carried by small
//! marker objects the bootstrap script wraps in classes (`__p` / `__s` keys); JS functions
//! cross as [`DynCallback`]s holding a `Persistent<Function>` plus the context, so Rust can
//! re-invoke them later (actions, reactive closures) from OUTSIDE any `Context::with`.

use std::rc::Rc;

use day_pieces::dynreg::{DynCallback, DynValue};
use rquickjs::{Array, Context, Ctx, Function, Object, Persistent, Value};

use crate::engine::{PIECES, SIGNALS, with_services};

/// JS → DynValue. `depth` guards cycles from adversarial scripts.
pub fn from_js<'js>(ctx: &Ctx<'js>, v: &Value<'js>, depth: u8) -> DynValue {
    if depth == 0 {
        return DynValue::Null;
    }
    if v.is_null() || v.is_undefined() {
        return DynValue::Null;
    }
    if let Some(b) = v.as_bool() {
        return DynValue::Bool(b);
    }
    if let Some(n) = v.as_float() {
        return DynValue::Num(n);
    }
    if let Some(n) = v.as_int() {
        return DynValue::Num(n as f64);
    }
    if let Some(s) = v.as_string() {
        return DynValue::Str(s.to_string().unwrap_or_default());
    }
    if let Some(f) = v.as_function() {
        let saved = Persistent::save(ctx, f.clone());
        if let Some(context) = crate::engine::current_context() {
            return DynValue::Fn(js_callback(context, saved));
        }
        return DynValue::Null;
    }
    if let Some(arr) = v.as_array() {
        let mut out = Vec::with_capacity(arr.len());
        for item in arr.iter::<Value>() {
            match item {
                Ok(item) => out.push(from_js(ctx, &item, depth - 1)),
                Err(_) => out.push(DynValue::Null),
            }
        }
        return DynValue::List(out);
    }
    if let Some(obj) = v.as_object() {
        // Handle-marker objects from the bootstrap classes.
        if let Ok(h) = obj.get::<_, u32>("__p") {
            if let Some(p) = PIECES.with(|s| s.borrow().get(h as usize).cloned()) {
                return DynValue::Piece(p);
            }
            return DynValue::Null;
        }
        if let Ok(h) = obj.get::<_, u32>("__s") {
            if let Some(s) = SIGNALS.with(|s| s.borrow().get(h as usize).cloned()) {
                return DynValue::Signal(s);
            }
            return DynValue::Null;
        }
        let mut out = Vec::new();
        if let Ok(keys) = obj.keys::<String>().collect::<Result<Vec<_>, _>>() {
            for k in keys {
                if let Ok(val) = obj.get::<_, Value>(k.as_str()) {
                    out.push((k, from_js(ctx, &val, depth - 1)));
                }
            }
        }
        return DynValue::Map(out);
    }
    DynValue::Null
}

/// DynValue → JS.
pub fn to_js<'js>(ctx: &Ctx<'js>, v: &DynValue) -> rquickjs::Result<Value<'js>> {
    Ok(match v {
        DynValue::Null => Value::new_null(ctx.clone()),
        DynValue::Bool(b) => Value::new_bool(ctx.clone(), *b),
        DynValue::Num(n) => Value::new_float(ctx.clone(), *n),
        DynValue::Str(s) => rquickjs::String::from_str(ctx.clone(), s)?.into_value(),
        DynValue::List(items) => {
            let arr = Array::new(ctx.clone())?;
            for (i, item) in items.iter().enumerate() {
                arr.set(i, to_js(ctx, item)?)?;
            }
            arr.into_value()
        }
        DynValue::Map(entries) => {
            let obj = Object::new(ctx.clone())?;
            for (k, val) in entries {
                obj.set(k.as_str(), to_js(ctx, val)?)?;
            }
            obj.into_value()
        }
        // Handles/functions coming back out are rare (bridge results are data); represent
        // them as null rather than materializing new wrappers.
        DynValue::Fn(_) | DynValue::Signal(_) | DynValue::Piece(_) => Value::new_null(ctx.clone()),
    })
}

/// Wrap a saved JS function as a host callback. Invocation enters the context, converts
/// arguments, calls, converts the result, then drives the microtask queue (a JS callback
/// may have queued promise reactions).
pub fn js_callback(context: Context, saved: Persistent<Function<'static>>) -> DynCallback {
    Rc::new(move |args: &[DynValue]| -> DynValue {
        let saved = saved.clone();
        context.with(|ctx| {
            let Ok(f) = saved.restore(&ctx) else {
                return DynValue::Null;
            };
            let mut jargs = rquickjs::function::Args::new(ctx.clone(), args.len());
            for a in args {
                match to_js(&ctx, a) {
                    Ok(v) => {
                        if jargs.push_arg(v).is_err() {
                            return DynValue::Null;
                        }
                    }
                    Err(_) => return DynValue::Null,
                }
            }
            let out: DynValue = match f.call_arg::<Value>(jargs) {
                Ok(v) => from_js(&ctx, &v, 16),
                Err(e) => {
                    report_js_error(&ctx, "callback", e);
                    DynValue::Null
                }
            };
            while ctx.execute_pending_job() {}
            out
        })
    })
}

/// Surface a JS exception in the host log (visible in `day launch` output) and to the
/// miniapp's `App.onError` when registered. The thrown value may be an Error object OR a
/// plain value (the host's own `throw()` throws strings) — stringify whichever arrived.
pub fn report_js_error(ctx: &Ctx<'_>, what: &str, e: rquickjs::Error) {
    let detail = if e.is_exception() {
        let caught = ctx.catch();
        if let Some(x) = caught.as_exception() {
            x.to_string()
        } else if let Some(s) = caught.as_string() {
            s.to_string().unwrap_or_else(|_| e.to_string())
        } else {
            format!("threw a {}", caught.type_name())
        }
    } else {
        e.to_string()
    };
    eprintln!("day-lite: {what}: {detail}");
    with_services(|s| s.report_error(&detail));
}
