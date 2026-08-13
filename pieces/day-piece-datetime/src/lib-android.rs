// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// Android: this crate's OWN Java factory (`dev.daybrite.day.piece.datetime.DayDateTime`, bundled
// under android/java and folded into the app's Gradle build via [package.metadata.day.android]).
// Compact = a value button launching the modal MaterialDatePicker / MaterialTimePicker through
// DayActivity's FragmentManager (the Material idiom — a dialog, not a popover); Inline = the
// framework DatePicker / TimePicker widgets. Picks come back through DayBridge.nativeOnEvent
// kind 12 → `Event::Custom { tag: "", num, .. }` carrying epoch days / seconds-of-day.
// ---------------------------------------------------------------------------

use super::*;
use day_android::DayEnv;
use day_android::jni::objects::JValue;
use day_android::{AHandle, Android, with_env};
use day_spec::NodeId;

/// This piece's OWN Java class (in the crate's android/java, on the app classpath at build).
const CLASS: &str = "dev/daybrite/day/piece/datetime/DayDateTime";

mod date_renderer {
    use super::*;

    fn make(_backend: &mut Android, p: &DateProps, id: NodeId) -> AHandle {
        with_env(|env| {
            // A Java throw must not panic inside realize (the panic unwinds the JNI
            // up-call and aborts the process); degrade to the visible placeholder.
            let made = day_android::try_make_view_on(
                env,
                CLASS,
                "makeDatePicker",
                "(JZJZJZJ)Landroid/view/View;",
                &[
                    JValue::Long(id.0 as i64),
                    JValue::Bool(p.style == Style::Inline),
                    JValue::Long(p.date.to_epoch_days()),
                    JValue::Bool(p.min.is_some()),
                    JValue::Long(p.min.map_or(0, DayDate::to_epoch_days)),
                    JValue::Bool(p.max.is_some()),
                    JValue::Long(p.max.map_or(0, DayDate::to_epoch_days)),
                ],
            );
            AHandle(made.unwrap_or_else(|_| {
                eprintln!(
                    "day-piece-datetime: DayDateTime.makeDatePicker failed; substituting a placeholder"
                );
                day_android::placeholder_view(env, "date_picker")
            }))
        })
    }

    fn update(_backend: &mut Android, h: &AHandle, patch: &DatePatch) {
        let DatePatch::SetDate(d) = patch;
        let days = d.to_epoch_days();
        with_env(|env| {
            let _ = env.dcall_static(
                CLASS,
                "setDate",
                "(Landroid/view/View;J)V",
                &[JValue::Object(h.0.as_obj()), JValue::Long(days)],
            );
        });
    }

    day_pieces::renderer!(day_android::RENDERERS, Android,
        kind: DATE_KIND, props: DateProps, patch: DatePatch,
        make: make, update: update);
}

mod time_renderer {
    use super::*;

    fn make(_backend: &mut Android, p: &TimeProps, id: NodeId) -> AHandle {
        // `seconds` is a documented no-op: neither Material nor the framework clock edits seconds.
        with_env(|env| {
            // Same rule as the date picker: no panic inside realize, placeholder instead.
            let made = day_android::try_make_view_on(
                env,
                CLASS,
                "makeTimePicker",
                "(JZJ)Landroid/view/View;",
                &[
                    JValue::Long(id.0 as i64),
                    JValue::Bool(p.style == Style::Inline),
                    JValue::Long(p.time.seconds_of_day()),
                ],
            );
            AHandle(made.unwrap_or_else(|_| {
                eprintln!(
                    "day-piece-datetime: DayDateTime.makeTimePicker failed; substituting a placeholder"
                );
                day_android::placeholder_view(env, "time_picker")
            }))
        })
    }

    fn update(_backend: &mut Android, h: &AHandle, patch: &TimePatch) {
        let TimePatch::SetTime(t) = patch;
        let secs = t.seconds_of_day();
        with_env(|env| {
            let _ = env.dcall_static(
                CLASS,
                "setTime",
                "(Landroid/view/View;J)V",
                &[JValue::Object(h.0.as_obj()), JValue::Long(secs)],
            );
        });
    }

    day_pieces::renderer!(day_android::RENDERERS, Android,
        kind: TIME_KIND, props: TimeProps, patch: TimePatch,
        make: make, update: update);
}
