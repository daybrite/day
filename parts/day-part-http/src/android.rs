// ---------------------------------------------------------------------------
// Android: java.net.HttpURLConnection via this crate's own Java shim (DayHttp.java, staged by
// `day build` through [package.metadata.day.android]) — the platform HTTP stack: system
// ProxySelector, VPN routing, network security config + user CA store. The Java call BLOCKS the
// calling thread (day-android's `with_env` attaches ANY thread to the JVM), matching `fetch`'s
// contract; results cross JNI as one byte[] envelope (see DayHttp.java's header comment).
// ---------------------------------------------------------------------------

use std::path::Path;

use day_android::jni::objects::{JObject, JValue};
use day_android::{DayEnv, with_env};

use super::{Download, HttpError, Request, Response, Tier};

pub const TIER: Tier = Tier::NativeStack;

const CLASS: &str = "dev/daybrite/day/http/DayHttp";

pub fn fetch(req: &Request) -> Result<Response, HttpError> {
    let envelope = call(req, None)?;
    let (status, headers, payload) = unpack(&envelope)?;
    Ok(Response {
        status,
        headers,
        body: payload.to_vec(),
    })
}

pub fn fetch_to_file(req: &Request, dest: &Path) -> Result<Download, HttpError> {
    let envelope = call(req, Some(dest))?;
    let (status, headers, payload) = unpack(&envelope)?;
    let bytes_written = payload
        .try_into()
        .map(u64::from_be_bytes)
        .map_err(|_| HttpError::Io("malformed download envelope".into()))?;
    Ok(Download {
        status,
        headers,
        bytes_written,
    })
}

/// Invoke the Java shim; returns the raw envelope bytes.
fn call(req: &Request, dest: Option<&Path>) -> Result<Vec<u8>, HttpError> {
    let timeout_ms = i32::try_from(req.timeout.as_millis()).unwrap_or(i32::MAX);
    with_env(|env| -> Result<Vec<u8>, HttpError> {
        let jerr = |e: day_android::jni::errors::Error| HttpError::Io(format!("jni: {e}"));
        let method = env.new_string(req.method.as_str()).map_err(jerr)?;
        let url = env.new_string(&req.url).map_err(jerr)?;
        // Headers as a flat String[] of k,v pairs.
        let empty = env.new_string("").map_err(jerr)?;
        let string_class = env.dfind("java/lang/String").map_err(jerr)?;
        let kv = env
            .new_object_array((req.headers.len() * 2) as i32, &string_class, &empty)
            .map_err(jerr)?;
        for (i, (k, v)) in req.headers.iter().enumerate() {
            let jk = env.new_string(k).map_err(jerr)?;
            let jv = env.new_string(v).map_err(jerr)?;
            kv.set_element(env, i * 2, &jk).map_err(jerr)?;
            kv.set_element(env, i * 2 + 1, &jv).map_err(jerr)?;
        }
        let body = env
            .byte_array_from_slice(req.body.as_deref().unwrap_or(&[]))
            .map_err(jerr)?;

        let result = match dest {
            None => env
                .dcall_static(
                    CLASS,
                    "fetch",
                    "(Ljava/lang/String;Ljava/lang/String;[Ljava/lang/String;[BI)[B",
                    &[
                        JValue::Object(&method),
                        JValue::Object(&url),
                        JValue::Object(&kv),
                        JValue::Object(&body),
                        JValue::Int(timeout_ms),
                    ],
                )
                .map_err(jerr)?,
            Some(path) => {
                let jdest = env
                    .new_string(path.to_string_lossy().as_ref())
                    .map_err(jerr)?;
                env.dcall_static(
                    CLASS,
                    "fetchToFile",
                    "(Ljava/lang/String;Ljava/lang/String;[Ljava/lang/String;[BILjava/lang/String;)[B",
                    &[
                        JValue::Object(&method),
                        JValue::Object(&url),
                        JValue::Object(&kv),
                        JValue::Object(&body),
                        JValue::Int(timeout_ms),
                        JValue::Object(&jdest),
                    ],
                )
                .map_err(jerr)?
            }
        };
        let obj: JObject = result.l().map_err(jerr)?;
        // SAFETY: the shim's return type is byte[]; a JByteArray is a transparent jobject wrapper.
        let arr = unsafe { day_android::jni::objects::JByteArray::from_raw(env, obj.into_raw()) };
        env.convert_byte_array(&arr).map_err(jerr)
    })
}

/// The parsed pieces of a response envelope: (status, headers, payload).
type Unpacked = (u16, Vec<(String, String)>, Vec<u8>);

/// Split the envelope via the shared bridge convention (day_android::envelope) and map this
/// part's negative-status sentinels onto [`HttpError`].
fn unpack(bytes: &[u8]) -> Result<Unpacked, HttpError> {
    let e = day_android::envelope::Envelope::decode(bytes).map_err(|m| HttpError::Io(m.into()))?;
    if e.status < 0 {
        let msg = e.error_message();
        return Err(match e.status {
            -1 => HttpError::Timeout,
            -2 => HttpError::Dns,
            -3 => HttpError::Tls(msg),
            -4 => HttpError::Connect,
            -6 => HttpError::BadUrl(msg),
            _ => HttpError::Io(msg),
        });
    }
    Ok((e.status as u16, e.meta, e.payload))
}

/// Streaming: open the connection through the shim, then PULL 64 KiB chunks over JNI until EOF
/// (empty array). Abort (sink false/Err) closes the stream server-side via streamClose.
pub fn fetch_streamed(
    req: &Request,
    sink: &mut dyn super::StreamSink,
) -> Result<Download, HttpError> {
    let envelope = call_stream_open(req)?;
    let (status, headers, payload) = unpack(&envelope)?;
    let handle = i32::from_be_bytes(
        payload
            .try_into()
            .map_err(|_| HttpError::Io("malformed stream envelope".into()))?,
    );
    let close = |handle: i32| {
        with_env(|env| {
            let _ = env.dcall_static(CLASS, "streamClose", "(I)V", &[JValue::Int(handle)]);
        });
    };
    if !sink.head(status, &headers) {
        close(handle);
        return Err(HttpError::Io("aborted".into()));
    }
    let mut bytes_written = 0u64;
    loop {
        let chunk: Option<Vec<u8>> = with_env(|env| {
            let out = env
                .dcall_static(CLASS, "streamRead", "(I)[B", &[JValue::Int(handle)])
                .ok()?
                .l()
                .ok()?;
            if out.is_null() {
                return None; // read error — the shim already closed the stream
            }
            // SAFETY: streamRead returns byte[]; JByteArray is a transparent jobject wrapper.
            let arr =
                unsafe { day_android::jni::objects::JByteArray::from_raw(env, out.into_raw()) };
            env.convert_byte_array(&arr).ok()
        });
        match chunk {
            None => {
                close(handle);
                return Err(HttpError::Io("stream read failed".into()));
            }
            Some(c) if c.is_empty() => break, // EOF
            Some(c) => {
                if let Err(e) = sink.chunk(&c) {
                    close(handle);
                    return Err(e);
                }
                bytes_written += c.len() as u64;
            }
        }
    }
    close(handle);
    Ok(Download {
        status,
        headers,
        bytes_written,
    })
}

/// `streamOpen` with the same marshaling as `call`.
fn call_stream_open(req: &Request) -> Result<Vec<u8>, HttpError> {
    let timeout_ms = i32::try_from(req.timeout.as_millis()).unwrap_or(i32::MAX);
    with_env(|env| -> Result<Vec<u8>, HttpError> {
        let jerr = |e: day_android::jni::errors::Error| HttpError::Io(format!("jni: {e}"));
        let method = env.new_string(req.method.as_str()).map_err(jerr)?;
        let url = env.new_string(&req.url).map_err(jerr)?;
        let empty = env.new_string("").map_err(jerr)?;
        let string_class = env.dfind("java/lang/String").map_err(jerr)?;
        let kv = env
            .new_object_array((req.headers.len() * 2) as i32, &string_class, &empty)
            .map_err(jerr)?;
        for (i, (k, v)) in req.headers.iter().enumerate() {
            let jk = env.new_string(k).map_err(jerr)?;
            let jv = env.new_string(v).map_err(jerr)?;
            kv.set_element(env, i * 2, &jk).map_err(jerr)?;
            kv.set_element(env, i * 2 + 1, &jv).map_err(jerr)?;
        }
        let body = env
            .byte_array_from_slice(req.body.as_deref().unwrap_or(&[]))
            .map_err(jerr)?;
        let result = env
            .dcall_static(
                CLASS,
                "streamOpen",
                "(Ljava/lang/String;Ljava/lang/String;[Ljava/lang/String;[BI)[B",
                &[
                    JValue::Object(&method),
                    JValue::Object(&url),
                    JValue::Object(&kv),
                    JValue::Object(&body),
                    JValue::Int(timeout_ms),
                ],
            )
            .map_err(jerr)?;
        let obj: JObject = result.l().map_err(jerr)?;
        // SAFETY: byte[] return; transparent wrapper.
        let arr = unsafe { day_android::jni::objects::JByteArray::from_raw(env, obj.into_raw()) };
        env.convert_byte_array(&arr).map_err(jerr)
    })
}
