// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! daybridge's ArkTS arms (docs/bridge.md "Callbacks") through `napi-ohos`.
//!
//! The host hands over the record `registerDayBridges()` built (`{ 'day_bridge_<crate>_<fn>':
//! Function }`, kept in src/host_api.rs), and generated Rust reaches an arm through
//! [`invoke`], which runs it on the JS thread: inline from that thread, posted and awaited from
//! any other. An asynchronous arm gets its `Done` token as a trailing number and answers
//! through `dayBridgeComplete`, or by returning a promise, which is settled into the same
//! completion. The completion is one uniform C export per function in the app's own cdylib
//! (`day_bridge_complete_arkts_<crate>_<fn>`), resolved here by name.

use std::ffi::{CStr, c_char, c_void};
use std::ptr;
use std::sync::{Arc, Condvar, Mutex};

use napi_ohos::bindgen_prelude::{
    ArrayBuffer, BigInt, FromNapiValue, Object, PromiseRaw, ToNapiValue, Uint8Array, Unknown,
};
use napi_ohos::{Env, JsValue, ValueType, sys};

/// One argument crossing from Rust, `day_bridge::arkts::Arg`'s layout: `kind` selects the
/// field (0 bool, 1 i32, 2 i64, 3 f64, 4 utf-8 string, 5 bytes).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Arg {
    pub kind: i32,
    pub i: i64,
    pub f: f64,
    pub ptr: *const u8,
    pub len: usize,
}

impl Arg {
    const NONE: Arg = Arg {
        kind: -1,
        i: 0,
        f: 0.0,
        ptr: ptr::null(),
        len: 0,
    };
}

/// An argument copied for a call posted from another thread: the caller's buffers are only
/// valid until it returns, and it waits.
#[derive(Clone)]
enum Owned {
    Bool(bool),
    Num(f64),
    Str(String),
    Bytes(Vec<u8>),
}

impl Owned {
    fn from_arg(a: &Arg) -> Owned {
        // SAFETY: per the Arg contract, `ptr`/`len` describe live bytes for kinds 4 and 5.
        unsafe {
            match a.kind {
                0 => Owned::Bool(a.i != 0),
                1 | 2 => Owned::Num(a.i as f64),
                3 => Owned::Num(a.f),
                4 => Owned::Str(
                    String::from_utf8_lossy(std::slice::from_raw_parts(a.ptr, a.len)).into_owned(),
                ),
                _ => Owned::Bytes(std::slice::from_raw_parts(a.ptr, a.len).to_vec()),
            }
        }
    }

    unsafe fn to_napi(&self, env: sys::napi_env) -> napi_ohos::Result<sys::napi_value> {
        // SAFETY: a live env; each conversion is napi-ohos's own.
        unsafe {
            match self {
                Owned::Bool(b) => ToNapiValue::to_napi_value(env, *b),
                Owned::Num(n) => ToNapiValue::to_napi_value(env, *n),
                Owned::Str(s) => ToNapiValue::to_napi_value(env, s.as_str()),
                // A fresh typed array the arm may keep.
                Owned::Bytes(b) => ToNapiValue::to_napi_value(env, Uint8Array::from(b.clone())),
            }
        }
    }
}

type CompleteFn = unsafe extern "C" fn(u64, i32, i64, f64, *const u8, usize, *const u8, usize);

/// Resolve a completion: marshal the JS value into the uniform shape and call the Rust export.
pub fn complete(symbol: &str, done: u64, status: i32, value: Option<Unknown<'_>>, message: &str) {
    let name = crate::node::cstr(symbol);
    let sym = day_bridge::arkts::lookup(&name);
    if sym.is_null() {
        log::warn!("day-bridge: no completion export {symbol}");
        return;
    }
    // SAFETY: the export has exactly this signature (day-bridge generates it).
    let f: CompleteFn = unsafe { std::mem::transmute::<*mut c_void, CompleteFn>(sym) };
    let mut num = 0i64;
    let mut flt = 0f64;
    let mut bytes: Vec<u8> = Vec::new();
    let mut have_bytes = false;
    if let Some(value) = value {
        // SAFETY: each cast follows the type the runtime reports for the value.
        unsafe {
            match value.get_type().unwrap_or(ValueType::Undefined) {
                ValueType::Boolean => {
                    num = i64::from(value.cast::<bool>().unwrap_or(false));
                }
                ValueType::Number => {
                    flt = value.cast::<f64>().unwrap_or(0.0);
                    num = flt as i64;
                }
                ValueType::BigInt => {
                    num = value.cast::<BigInt>().map(|b| b.get_i64().0).unwrap_or(0);
                    flt = num as f64;
                }
                ValueType::String => {
                    if let Ok(s) = value.cast::<String>() {
                        bytes = s.into_bytes();
                        have_bytes = true;
                    }
                }
                ValueType::Object => {
                    if value.is_arraybuffer().unwrap_or(false) {
                        if let Ok(ab) = value.cast::<ArrayBuffer>() {
                            bytes = ab.to_vec();
                            have_bytes = true;
                        }
                    } else if value.is_typedarray().unwrap_or(false)
                        && let Ok(arr) = value.cast::<Uint8Array>()
                    {
                        bytes = arr.to_vec();
                        have_bytes = true;
                    }
                }
                _ => {}
            }
        }
    }
    let (ptr, len) = if have_bytes {
        (bytes.as_ptr(), bytes.len())
    } else {
        (ptr::null(), 0)
    };
    // SAFETY: the export reads its byte arguments during the call only.
    unsafe {
        f(
            done,
            status,
            num,
            flt,
            ptr,
            len,
            message.as_ptr(),
            message.len(),
        )
    };
}

/// `err.message` when a rejection is an Error, else its string form.
fn error_message(err: &Unknown<'_>) -> String {
    let message = if err.get_type().ok() == Some(ValueType::Object) {
        // SAFETY: the value is an object.
        unsafe { err.cast::<Object>() }
            .ok()
            .and_then(|o| o.get::<Unknown>("message").ok().flatten())
    } else {
        None
    };
    message
        .unwrap_or(*err)
        .coerce_to_string()
        .and_then(|s| s.into_utf8())
        .and_then(|s| s.into_owned())
        .unwrap_or_default()
}

/// Run one registered arm on the JS thread. 0 = the arm returned (or its promise is being
/// settled), 1 = it threw, 2 = no such arm is registered. A scalar return lands in `out`.
fn call_here(env: &Env, symbol: &str, args: &[Owned], done: u64, out: &mut Arg) -> i32 {
    *out = Arg::NONE;
    let Some(record) = crate::host_api::bridges(env) else {
        return 2;
    };
    let Ok(Some(fn_value)) = record.get::<Unknown>(symbol) else {
        return 2;
    };
    if fn_value.get_type().ok() != Some(ValueType::Function) {
        return 2;
    }
    let raw_env = env.raw();
    // SAFETY: a live env; every value built here belongs to the caller's handle scope.
    unsafe {
        let mut argv: Vec<sys::napi_value> = Vec::with_capacity(args.len() + 1);
        for a in args {
            match a.to_napi(raw_env) {
                Ok(v) => argv.push(v),
                Err(_) => return 1,
            }
        }
        if done != 0 {
            match ToNapiValue::to_napi_value(raw_env, done as f64) {
                Ok(v) => argv.push(v),
                Err(_) => return 1,
            }
        }
        let mut undefined = ptr::null_mut();
        sys::napi_get_undefined(raw_env, &mut undefined);
        let mut ret = ptr::null_mut();
        let status = sys::napi_call_function(
            raw_env,
            undefined,
            fn_value.raw(),
            argv.len(),
            argv.as_ptr(),
            &mut ret,
        );
        let mut pending = false;
        sys::napi_is_exception_pending(raw_env, &mut pending);
        if status != sys::Status::napi_ok || pending {
            let mut err = ptr::null_mut();
            sys::napi_get_and_clear_last_exception(raw_env, &mut err);
            log::error!("day-bridge: {symbol} threw");
            return 1;
        }
        if ret.is_null() {
            return 0;
        }
        let Ok(value) = Unknown::from_napi_value(raw_env, ret) else {
            return 0;
        };
        // A scalar the arm returned, as the Rust side reads it: booleans and integers in `i`,
        // numbers in `f` (with `i` its truncation), anything else left as kind -1.
        match value.get_type().unwrap_or(ValueType::Undefined) {
            ValueType::Boolean => {
                out.kind = 0;
                out.i = i64::from(value.cast::<bool>().unwrap_or(false));
            }
            ValueType::Number => {
                out.kind = 3;
                out.f = value.cast::<f64>().unwrap_or(0.0);
                out.i = out.f as i64;
            }
            ValueType::BigInt => {
                out.kind = 2;
                out.i = value.cast::<BigInt>().map(|b| b.get_i64().0).unwrap_or(0);
                out.f = out.i as f64;
            }
            _ => {}
        }
        // A promise: settle it into the same completion the explicit path uses.
        if done != 0
            && value.is_promise().unwrap_or(false)
            && let Ok(promise) = PromiseRaw::<Unknown>::from_napi_value(raw_env, ret)
        {
            let ok_symbol = symbol.to_owned();
            let err_symbol = symbol.to_owned();
            let chained = promise.then(move |ctx| {
                complete(&ok_symbol, done, 0, Some(ctx.value), "");
                Ok(())
            });
            if let Ok(chained) = chained {
                let _ = chained.catch(
                    move |ctx: napi_ohos::bindgen_prelude::CallbackContext<Unknown>| {
                        let message = error_message(&ctx.value);
                        complete(&err_symbol, done, 1, None, &message);
                        Ok(())
                    },
                );
            }
        }
        0
    }
}

/// A call posted from another thread: the caller parks until the JS thread answers.
struct Job {
    symbol: String,
    args: Vec<Owned>,
    done: u64,
    result: Mutex<Option<(i32, i32, i64, f64)>>,
    cv: Condvar,
}

/// Rust-facing (`day_bridge::arkts::invoke`): run the arm registered under `symbol`. On the JS
/// thread the call is inline; from any other thread it is posted and awaited.
///
/// # Safety
/// `symbol` is a C string and `args` points to `n` live [`Arg`]s; `ret`, when non-null,
/// receives the scalar the arm returned.
pub unsafe fn invoke(
    symbol: *const c_char,
    args: *const Arg,
    n: usize,
    done: u64,
    ret: *mut Arg,
) -> i32 {
    if symbol.is_null() {
        return 2;
    }
    // SAFETY: per the caller's contract.
    let (symbol, args) = unsafe {
        (
            CStr::from_ptr(symbol).to_string_lossy().into_owned(),
            if args.is_null() {
                &[][..]
            } else {
                std::slice::from_raw_parts(args, n)
            },
        )
    };
    let owned: Vec<Owned> = args.iter().map(Owned::from_arg).collect();
    if crate::main_thread::on_js_thread() {
        let Some(env) = crate::host_api::env() else {
            return 2;
        };
        let _scope = crate::host_api::Scope::open(&env);
        let mut out = Arg::NONE;
        let r = call_here(&env, &symbol, &owned, done, &mut out);
        if !ret.is_null() {
            // SAFETY: per the caller's contract.
            unsafe { *ret = out };
        }
        return r;
    }
    if !crate::main_thread::ready() {
        return 2;
    }
    let job = Arc::new(Job {
        symbol,
        args: owned,
        done,
        result: Mutex::new(None),
        cv: Condvar::new(),
    });
    let posted = job.clone();
    crate::main_thread::post(Box::new(move || {
        let mut out = Arg::NONE;
        let r = match crate::host_api::env() {
            Some(env) => {
                let _scope = crate::host_api::Scope::open(&env);
                call_here(&env, &posted.symbol, &posted.args, posted.done, &mut out)
            }
            None => 2,
        };
        *posted.result.lock().unwrap_or_else(|e| e.into_inner()) =
            Some((r, out.kind, out.i, out.f));
        posted.cv.notify_all();
    }));
    let mut guard = job.result.lock().unwrap_or_else(|e| e.into_inner());
    while guard.is_none() {
        guard = job.cv.wait(guard).unwrap_or_else(|e| e.into_inner());
    }
    let (r, kind, i, f) = guard.unwrap_or((2, -1, 0, 0.0));
    if !ret.is_null() {
        // SAFETY: per the caller's contract.
        unsafe {
            *ret = Arg {
                kind,
                i,
                f,
                ptr: ptr::null(),
                len: 0,
            }
        };
    }
    r
}
