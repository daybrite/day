// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// Android: this crate's Java factory (`dev.daybrite.day.piece.datetime.DayDateTime`, bundled
// in src/DayDateTime.java and folded into the app's Gradle build via [package.metadata.day.android]).
// Compact = a value button launching the modal MaterialDatePicker / MaterialTimePicker through
// DayActivity's FragmentManager (the Material idiom: a dialog, not a popover); Inline = the
// framework DatePicker / TimePicker widgets. Picks come back through DayBridge.nativeOnEvent
// kind 12 → `Event::Custom { tag: "", num, .. }` carrying epoch days / seconds-of-day.
// ---------------------------------------------------------------------------

use crate::{AHandle, Android, java};
use day_pieces::datetime::*;
use day_spec::NodeId;

// This piece's Java class (src/DayDateTime.java, on the app classpath at build).
crate::java_class! {
    DayDateTime => dev.daybrite.day.piece.datetime.DayDateTime,
    methods {
        static fn makeDatePicker(id: jlong, inline: jboolean, days: jlong, has_min: jboolean,
            min: jlong, has_max: jboolean, max: jlong) -> android.view.View,
        static fn setDate(view: android.view.View, days: jlong),
        static fn makeTimePicker(id: jlong, inline: jboolean, seconds: jlong) -> android.view.View,
        static fn setTime(view: android.view.View, seconds: jlong),
    },
}

mod date_renderer {
    use super::*;

    fn make(_backend: &mut Android, p: &DateProps, id: NodeId) -> AHandle {
        crate::realize("date_picker", |env| {
            DayDateTime::makeDatePicker(
                env,
                id.0 as i64,
                p.style == Style::Inline,
                p.date.to_epoch_days(),
                p.min.is_some(),
                p.min.map_or(0, DayDate::to_epoch_days),
                p.max.is_some(),
                p.max.map_or(0, DayDate::to_epoch_days),
            )
        })
    }

    fn update(_backend: &mut Android, h: &AHandle, patch: &DatePatch) {
        let DatePatch::SetDate(d) = patch;
        let days = d.to_epoch_days();
        java(|env| DayDateTime::setDate(env, h.0.as_obj(), days));
    }

    day_pieces::renderer!(crate::RENDERERS, Android,
        kind: DATE_KIND, props: DateProps, patch: DatePatch,
        make: make, update: update);
}

mod time_renderer {
    use super::*;

    fn make(_backend: &mut Android, p: &TimeProps, id: NodeId) -> AHandle {
        // `seconds` is a documented no-op: neither Material nor the framework clock edits seconds.
        crate::realize("time_picker", |env| {
            DayDateTime::makeTimePicker(
                env,
                id.0 as i64,
                p.style == Style::Inline,
                p.time.seconds_of_day(),
            )
        })
    }

    fn update(_backend: &mut Android, h: &AHandle, patch: &TimePatch) {
        let TimePatch::SetTime(t) = patch;
        let secs = t.seconds_of_day();
        java(|env| DayDateTime::setTime(env, h.0.as_obj(), secs));
    }

    day_pieces::renderer!(crate::RENDERERS, Android,
        kind: TIME_KIND, props: TimeProps, patch: TimePatch,
        make: make, update: update);
}
