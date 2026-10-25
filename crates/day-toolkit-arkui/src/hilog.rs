// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The hilog sink (docs/logging.md): std's stderr goes nowhere in an OHOS ability, so Day's
//! logger routes every line through `OH_LOG_Print` under the `day` tag.

use ohos_sys::hilog::{LogLevel, LogType, OH_LOG_Print};

/// Day's hilog domain.
const DOMAIN: u32 = 0xDA11;

pub fn print(level: log::Level, line: &str) {
    let level = match level {
        log::Level::Error => LogLevel::LOG_ERROR,
        log::Level::Warn => LogLevel::LOG_WARN,
        log::Level::Info => LogLevel::LOG_INFO,
        log::Level::Debug | log::Level::Trace => LogLevel::LOG_DEBUG,
    };
    let line = crate::node::cstr(line);
    // SAFETY: a fixed format with one string argument, both valid for the call.
    unsafe {
        OH_LOG_Print(
            LogType::LOG_APP,
            level,
            DOMAIN,
            c"day".as_ptr(),
            c"%{public}s".as_ptr(),
            line.as_ptr(),
        );
    }
}
