// HarmonyOS / OpenHarmony: the native SensorServiceKit C API (`libohsensor.so`, `oh_sensor.h`,
// API 11+). Pure FFI, like iOS — no ArkTS bridge or Day runtime needed (unlike Android's
// SensorManager, which rides day-android's JVM/Context). The API is push-only: the first `read` for
// a kind lazily creates a subscription (SubscriptionId + Attribute + Subscriber, all kept alive for
// the process) whose callback caches the newest sample in a static; Rust polls the cache. Values are
// already SI (m/s², rad/s, µT). NOTE: subscribing to the accelerometer requires the
// `ohos.permission.ACCELEROMETER` permission and the gyroscope `ohos.permission.GYROSCOPE` (declare
// them in the app's module.json5 `requestPermissions`); the magnetometer needs none.

use core::ffi::c_int;
use std::sync::{Mutex, OnceLock};

use super::{SensorKind, SensorReading};

// Sensor_Type (oh_sensor_type.h).
const SENSOR_TYPE_ACCELEROMETER: c_int = 1;
const SENSOR_TYPE_GYROSCOPE: c_int = 2;
const SENSOR_TYPE_MAGNETIC_FIELD: c_int = 6;

// Sensor_Result (oh_sensor_type.h).
const SENSOR_SUCCESS: c_int = 0;

/// Data reporting interval passed to the subscription, in nanoseconds (~UI rate).
const SAMPLING_INTERVAL_NS: i64 = 60_000_000;

// Opaque SensorServiceKit handles.
#[repr(C)]
struct SensorInfo([u8; 0]);
#[repr(C)]
struct SensorEvent([u8; 0]);
#[repr(C)]
struct SensorSubscriptionId([u8; 0]);
#[repr(C)]
struct SensorSubscriptionAttribute([u8; 0]);
#[repr(C)]
struct SensorSubscriber([u8; 0]);

type SensorEventCallback = unsafe extern "C" fn(event: *mut SensorEvent);

#[link(name = "ohsensor")]
unsafe extern "C" {
    fn OH_Sensor_GetInfos(infos: *mut *mut SensorInfo, count: *mut u32) -> c_int;
    fn OH_Sensor_CreateInfos(count: u32) -> *mut *mut SensorInfo;
    fn OH_Sensor_DestroyInfos(sensors: *mut *mut SensorInfo, count: u32) -> i32;
    fn OH_SensorInfo_GetType(sensor: *mut SensorInfo, sensor_type: *mut c_int) -> i32;

    fn OH_Sensor_CreateSubscriptionId() -> *mut SensorSubscriptionId;
    fn OH_Sensor_DestroySubscriptionId(id: *mut SensorSubscriptionId) -> i32;
    fn OH_SensorSubscriptionId_SetType(id: *mut SensorSubscriptionId, sensor_type: c_int) -> i32;

    fn OH_Sensor_CreateSubscriptionAttribute() -> *mut SensorSubscriptionAttribute;
    fn OH_Sensor_DestroySubscriptionAttribute(attribute: *mut SensorSubscriptionAttribute) -> i32;
    fn OH_SensorSubscriptionAttribute_SetSamplingInterval(
        attribute: *mut SensorSubscriptionAttribute,
        sampling_interval: i64,
    ) -> i32;

    fn OH_Sensor_CreateSubscriber() -> *mut SensorSubscriber;
    fn OH_Sensor_DestroySubscriber(subscriber: *mut SensorSubscriber) -> i32;
    fn OH_SensorSubscriber_SetCallback(
        subscriber: *mut SensorSubscriber,
        callback: SensorEventCallback,
    ) -> i32;

    fn OH_Sensor_Subscribe(
        id: *const SensorSubscriptionId,
        attribute: *const SensorSubscriptionAttribute,
        subscriber: *const SensorSubscriber,
    ) -> c_int;

    fn OH_SensorEvent_GetType(event: *mut SensorEvent, sensor_type: *mut c_int) -> i32;
    fn OH_SensorEvent_GetData(
        event: *mut SensorEvent,
        data: *mut *mut f32,
        length: *mut u32,
    ) -> i32;
}

fn sensor_type(kind: SensorKind) -> c_int {
    match kind {
        SensorKind::Accelerometer => SENSOR_TYPE_ACCELEROMETER,
        SensorKind::Gyroscope => SENSOR_TYPE_GYROSCOPE,
        SensorKind::Magnetometer => SENSOR_TYPE_MAGNETIC_FIELD,
    }
}

fn kind_index(kind: SensorKind) -> usize {
    match kind {
        SensorKind::Accelerometer => 0,
        SensorKind::Gyroscope => 1,
        SensorKind::Magnetometer => 2,
    }
}

/// Latest sample per kind, written by the subscription callback, read by `read`.
static LATEST: Mutex<[Option<[f64; 3]>; 3]> = Mutex::new([None; 3]);
/// Whether the per-kind subscription is active (its handles are intentionally kept for the process
/// lifetime — the API has no `stop`, and the service requires them alive while subscribed).
static SUBSCRIBED: Mutex<[bool; 3]> = Mutex::new([false; 3]);

/// The subscription callback: identify the sensor by type and cache its x/y/z triple.
unsafe extern "C" fn on_event(event: *mut SensorEvent) {
    let mut ty: c_int = 0;
    if unsafe { OH_SensorEvent_GetType(event, &mut ty) } != SENSOR_SUCCESS {
        return;
    }
    let idx = match ty {
        SENSOR_TYPE_ACCELEROMETER => 0,
        SENSOR_TYPE_GYROSCOPE => 1,
        SENSOR_TYPE_MAGNETIC_FIELD => 2,
        _ => return,
    };
    let mut data: *mut f32 = std::ptr::null_mut();
    let mut len: u32 = 0;
    if unsafe { OH_SensorEvent_GetData(event, &mut data, &mut len) } != SENSOR_SUCCESS
        || data.is_null()
        || len < 3
    {
        return;
    }
    let xyz = unsafe { [*data as f64, *data.add(1) as f64, *data.add(2) as f64] };
    if let Ok(mut latest) = LATEST.lock() {
        latest[idx] = Some(xyz);
    }
}

/// Subscribe to `kind` if not yet subscribed. Failures (e.g. a missing ACCELEROMETER/GYROSCOPE
/// permission) release the handles and leave the flag unset, so a later read retries.
fn ensure_subscribed(kind: SensorKind) {
    let Ok(mut subscribed) = SUBSCRIBED.lock() else {
        return;
    };
    let idx = kind_index(kind);
    if subscribed[idx] {
        return;
    }
    unsafe {
        let id = OH_Sensor_CreateSubscriptionId();
        let attr = OH_Sensor_CreateSubscriptionAttribute();
        let sub = OH_Sensor_CreateSubscriber();
        let ok = !id.is_null()
            && !attr.is_null()
            && !sub.is_null()
            && OH_SensorSubscriptionId_SetType(id, sensor_type(kind)) == SENSOR_SUCCESS
            && OH_SensorSubscriptionAttribute_SetSamplingInterval(attr, SAMPLING_INTERVAL_NS)
                == SENSOR_SUCCESS
            && OH_SensorSubscriber_SetCallback(sub, on_event) == SENSOR_SUCCESS
            && OH_Sensor_Subscribe(id, attr, sub) == SENSOR_SUCCESS;
        if ok {
            // Deliberately leak id/attr/sub: the service needs them for as long as we're subscribed,
            // which is the rest of the process.
            subscribed[idx] = true;
        } else {
            if !sub.is_null() {
                OH_Sensor_DestroySubscriber(sub);
            }
            if !attr.is_null() {
                OH_Sensor_DestroySubscriptionAttribute(attr);
            }
            if !id.is_null() {
                OH_Sensor_DestroySubscriptionId(id);
            }
        }
    }
}

/// The device's sensor types via OH_Sensor_GetInfos (count first, then the filled array), cached on
/// first success — the hardware set doesn't change at runtime.
fn available_types() -> &'static [c_int] {
    static TYPES: OnceLock<Vec<c_int>> = OnceLock::new();
    if let Some(types) = TYPES.get() {
        return types;
    }
    let Some(types) = query_types() else {
        return &[]; // transient service failure: report nothing, retry next call
    };
    TYPES.get_or_init(|| types)
}

fn query_types() -> Option<Vec<c_int>> {
    unsafe {
        let mut count: u32 = 0;
        if OH_Sensor_GetInfos(std::ptr::null_mut(), &mut count) != SENSOR_SUCCESS || count == 0 {
            return None;
        }
        let infos = OH_Sensor_CreateInfos(count);
        if infos.is_null() {
            return None;
        }
        let mut types = None;
        if OH_Sensor_GetInfos(infos, &mut count) == SENSOR_SUCCESS {
            let mut found = Vec::with_capacity(count as usize);
            for i in 0..count as usize {
                let mut ty: c_int = 0;
                if OH_SensorInfo_GetType(*infos.add(i), &mut ty) == SENSOR_SUCCESS {
                    found.push(ty);
                }
            }
            types = Some(found);
        }
        OH_Sensor_DestroyInfos(infos, count);
        types
    }
}

pub fn is_available(kind: SensorKind) -> bool {
    available_types().contains(&sensor_type(kind))
}

pub fn read(kind: SensorKind) -> Option<SensorReading> {
    if !is_available(kind) {
        return None;
    }
    ensure_subscribed(kind);
    let xyz = LATEST.lock().ok()?[kind_index(kind)]?;
    Some(SensorReading {
        x: xyz[0],
        y: xyz[1],
        z: xyz[2],
    })
}
