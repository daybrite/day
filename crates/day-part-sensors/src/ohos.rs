// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// HarmonyOS SensorServiceKit event subscriptions (API 11+).

use core::ffi::c_int;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use super::{Calibration, Sensor, SensorKind, SensorReading, push};

// Sensor_Type (oh_sensor_type.h).
const SENSOR_TYPE_ACCELEROMETER: c_int = 1;
const SENSOR_TYPE_GYROSCOPE: c_int = 2;
const SENSOR_TYPE_MAGNETIC_FIELD: c_int = 6;

// Sensor_Result (oh_sensor_type.h).
const SENSOR_SUCCESS: c_int = 0;

/// Data reporting interval passed to the subscription, in nanoseconds (~UI rate).
const SAMPLING_INTERVAL_NS: i64 = 50_000_000;

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

    fn OH_Sensor_Unsubscribe(
        id: *const SensorSubscriptionId,
        subscriber: *const SensorSubscriber,
    ) -> c_int;

    fn OH_SensorEvent_GetType(event: *mut SensorEvent, sensor_type: *mut c_int) -> i32;
    fn OH_SensorEvent_GetData(
        event: *mut SensorEvent,
        data: *mut *mut f32,
        length: *mut u32,
    ) -> i32;
}

fn sensor_type(kind: Sensor) -> c_int {
    match kind.kind {
        SensorKind::Accelerometer => SENSOR_TYPE_ACCELEROMETER,
        SensorKind::Gyroscope => SENSOR_TYPE_GYROSCOPE,
        SensorKind::Magnetometer => SENSOR_TYPE_MAGNETIC_FIELD,
    }
}

// The API supplies a function pointer without user data. Lifecycle is serialized by the
// dispatcher; invalidate before unsubscribe, and install the new epoch only after it completes.
static GENERATIONS: [AtomicU32; 3] = [const { AtomicU32::new(0) }; 3];
static STOP_FAILED: [AtomicBool; 3] = [const { AtomicBool::new(false) }; 3];

/// Copy the native sample before returning; the service owns the event memory.
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
    let generation = GENERATIONS[idx].load(Ordering::Acquire);
    if generation == 0 {
        return;
    }
    let mut data: *mut f32 = std::ptr::null_mut();
    let mut len: u32 = 0;
    if unsafe { OH_SensorEvent_GetData(event, &mut data, &mut len) } != SENSOR_SUCCESS
        || data.is_null()
        || len < 3
    {
        return;
    }
    let xyz = unsafe { [*data as f64, *data.add(1) as f64, *data.add(2) as f64] };
    if let Some(kind) = Sensor::from_code(idx as u32) {
        push(
            kind,
            generation,
            SensorReading {
                x: xyz[0],
                y: xyz[1],
                z: xyz[2],
            },
        );
    }
}

pub struct Subscription {
    kind: Sensor,
    id: *mut SensorSubscriptionId,
    attr: *mut SensorSubscriptionAttribute,
    sub: *mut SensorSubscriber,
    active: bool,
}
impl Drop for Subscription {
    fn drop(&mut self) {
        GENERATIONS[self.kind.index()].store(0, Ordering::Release);
        unsafe {
            if self.active && OH_Sensor_Unsubscribe(self.id, self.sub) != SENSOR_SUCCESS {
                // The service may still hold these pointers. Preserve them, and prevent a new
                // source from accepting old events through the callback without user data.
                STOP_FAILED[self.kind.index()].store(true, Ordering::Release);
                eprintln!(
                    "day-part-sensors: native unsubscribe failed for {:?}",
                    self.kind
                );
                return;
            }
            if !self.sub.is_null() {
                OH_Sensor_DestroySubscriber(self.sub);
            }
            if !self.attr.is_null() {
                OH_Sensor_DestroySubscriptionAttribute(self.attr);
            }
            if !self.id.is_null() {
                OH_Sensor_DestroySubscriptionId(self.id);
            }
        }
    }
}
pub fn start(kind: Sensor, generation: u32) -> Option<Subscription> {
    if kind.calibration == Calibration::Uncalibrated {
        return None;
    }
    if STOP_FAILED[kind.index()].load(Ordering::Acquire) || !is_available(kind) {
        return None;
    }
    unsafe {
        let mut source = Subscription {
            kind,
            id: OH_Sensor_CreateSubscriptionId(),
            attr: OH_Sensor_CreateSubscriptionAttribute(),
            sub: OH_Sensor_CreateSubscriber(),
            active: false,
        };
        if source.id.is_null()
            || source.attr.is_null()
            || source.sub.is_null()
            || OH_SensorSubscriptionId_SetType(source.id, sensor_type(kind)) != SENSOR_SUCCESS
            || OH_SensorSubscriptionAttribute_SetSamplingInterval(source.attr, SAMPLING_INTERVAL_NS)
                != SENSOR_SUCCESS
            || OH_SensorSubscriber_SetCallback(source.sub, on_event) != SENSOR_SUCCESS
        {
            return None;
        }
        GENERATIONS[kind.index()].store(generation, Ordering::Release);
        if OH_Sensor_Subscribe(source.id, source.attr, source.sub) != SENSOR_SUCCESS {
            return None;
        }
        source.active = true;
        Some(source)
    }
}

/// The device's sensor types via OH_Sensor_GetInfos (count first, then the filled array), cached on
/// first success; the hardware set doesn't change at runtime.
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

pub fn is_available(kind: Sensor) -> bool {
    if kind.calibration == Calibration::Uncalibrated {
        return false;
    }
    available_types().contains(&sensor_type(kind))
}
