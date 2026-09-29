// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! HarmonyOS: the Location Kit's `geoLocationManager`, which exists only in ArkTS, through this
//! crate's daybridge arm (docs/bridge.md). Its `locationChange` subscription is already the shape
//! this crate wants: the arm streams each fix (or failure) back as one line, and a new watch
//! replaces the platform subscription.
//!
//! The permission is the app's (`ohos.permission.APPROXIMATELY_LOCATION` plus `LOCATION`,
//! requested through day-part-permissions); without it the subscription fails with 201, which
//! arrives as [`LocationError::PermissionDenied`].

use std::sync::Mutex;

use day_bridge::{Item, Support};

use crate::{Accuracy, LocationError};

/// The live platform stream's token, so `stop` can end it.
static STREAM: Mutex<Option<u64>> = Mutex::new(None);

pub fn is_available() -> bool {
    watch_native_support() != Support::Unsupported
}

pub fn start(acc: Accuracy) {
    stop();
    let mode = match acc {
        Accuracy::Coarse => 0,
        Accuracy::Balanced => 1,
        Accuracy::Best => 2,
    };
    let started = watch_native_stream(mode, |item| match item {
        Item::Value(line) => crate::deliver(crate::ohos_line::parse(&line)),
        Item::Failed(e) => crate::deliver(Err(LocationError::Io(e.to_string()))),
        Item::End => {}
    });
    match started {
        Ok(token) => *STREAM.lock().unwrap_or_else(|p| p.into_inner()) = Some(token),
        Err(e) => crate::deliver(Err(LocationError::Io(e.to_string()))),
    }
}

pub fn stop() {
    if let Some(token) = STREAM.lock().unwrap_or_else(|p| p.into_inner()).take() {
        watch_native_stop(token);
        stop_native();
    }
}

day_bridge::bridge! {
    #[day_bridge::declare]
    extern "day" {
        /// Subscribe to `locationChange` at `mode` (0 coarse, 1 balanced, 2 best); each fix or
        /// failure is one line (see `ohos_line::parse`).
        fn watch_native(mode: i32, emit: day_bridge::Emit<String>) -> Result<(), day_bridge::Error>;
        /// End the platform subscription.
        fn stop_native();
    }

    #[day_bridge::impl(arkts, platforms = [ohos])]
    arkts!(
        prelude = r#"
            import { geoLocationManager } from '@kit.LocationKit';
            import { BusinessError } from '@kit.BasicServicesKit';
        "#,
        body = r#"
            let dayListening: boolean = false;

            function dayNum(v: number | undefined): string {
              return v === undefined || v === null ? 'NaN' : `${v}`;
            }

            export function watch_native(mode: number, emit: number): void {
              stop_native();
              if (!geoLocationManager.isLocationEnabled()) {
                watch_native_emit(emit, 'error,3301100');
                return;
              }
              const request: geoLocationManager.LocationRequest = {
                priority: mode === 2 ? geoLocationManager.LocationRequestPriority.ACCURACY
                  : mode === 1 ? geoLocationManager.LocationRequestPriority.FIRST_FIX
                  : geoLocationManager.LocationRequestPriority.LOW_POWER,
                scenario: geoLocationManager.LocationRequestScenario.UNSET,
                timeInterval: 1,
                distanceInterval: 0,
                maxAccuracy: 0
              };
              try {
                geoLocationManager.on('locationChange', request, (l: geoLocationManager.Location): void => {
                  watch_native_emit(emit, ['fix', l.latitude, l.longitude, dayNum(l.altitude),
                    dayNum(l.accuracy), dayNum(l.altitudeAccuracy), dayNum(l.speed),
                    dayNum(l.direction), dayNum(l.timeStamp)].join(','));
                });
                dayListening = true;
              } catch (e) {
                watch_native_emit(emit, `error,${(e as BusinessError).code}`);
              }
            }

            export function stop_native(): void {
              if (!dayListening) {
                return;
              }
              dayListening = false;
              try {
                geoLocationManager.off('locationChange');
              } catch (e) {
                console.info(`day-part-location: ${(e as BusinessError).code}`);
              }
            }
        "#,
    );

    #[day_bridge::impl(rust, platforms = [other])]
    fn watch_native(_mode: i32, _emit: day_bridge::Emit<String>) -> Result<(), day_bridge::Error> {
        Err(day_bridge::Error::Unsupported)
    }

    #[day_bridge::impl(rust, platforms = [other])]
    fn stop_native() {}
}
