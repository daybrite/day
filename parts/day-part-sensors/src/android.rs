// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

use super::{Sensor, SensorReading, push};
use day_android::java;
use day_android::jni::{
    Env,
    objects::JClass,
    sys::{jdouble, jint},
};

day_android::java_class! {
    DaySensors => dev.daybrite.day.sensors.DaySensors,
    methods {
        static fn isAvailable(kind: jint) -> jboolean,
        static fn start(kind: jint, generation: jint) -> jboolean,
        static fn stop(kind: jint),
    },
    native_methods {
        static fn nativeSample(kind: jint, generation: jint, x: jdouble, y: jdouble, z: jdouble),
    },
}
pub fn is_available(kind: Sensor) -> bool {
    java(|env| DaySensors::isAvailable(env, kind.index() as i32)).unwrap_or(false)
}
pub struct Subscription(Sensor);
impl Drop for Subscription {
    fn drop(&mut self) {
        java(|env| DaySensors::stop(env, self.0.index() as i32));
    }
}
pub fn start(kind: Sensor, generation: u32) -> Option<Subscription> {
    java(|env| DaySensors::start(env, kind.index() as i32, generation as i32))
        .unwrap_or(false)
        .then_some(Subscription(kind))
}
impl DaySensorsNativeInterface for DaySensorsAPI {
    type Error = day_android::jni::errors::Error;
    fn nativeSample<'l>(
        _: &mut Env<'l>,
        _: JClass<'l>,
        kind: jint,
        generation: jint,
        x: jdouble,
        y: jdouble,
        z: jdouble,
    ) -> Result<(), Self::Error> {
        if let Some(kind) = Sensor::from_code(kind as u32) {
            push(kind, generation as u32, SensorReading { x, y, z });
        }
        Ok(())
    }
}
