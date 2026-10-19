// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// Android: android.widget.ProgressBar: the default style is a circular indeterminate spinner, so
// this piece adds no Gradle dependencies and no permissions. The Java factory
// (`dev.daybrite.day.piece.activity.DayActivity`) is bundled with this crate in `src/DayActivity.java`
// and pulled into the app's Gradle build via `[package.metadata.day.android]`, using only
// day-android's public Java surface (DayBridge.ctx). See docs/extending.md.
// ---------------------------------------------------------------------------

use super::*;
use day_android::{AHandle, Android, java};
use day_spec::NodeId;

// This piece's Java class (src/DayActivity.java, on the app classpath at build).
day_android::java_class! {
    DayActivity => dev.daybrite.day.piece.activity.DayActivity,
    methods {
        static fn makeActivity(animating: jboolean, large: jboolean) -> android.view.View,
        static fn setActivityAnimating(view: android.view.View, on: jboolean),
    },
}

fn make(_backend: &mut Android, p: &ActivityProps, _id: NodeId) -> AHandle {
    let (animating, large) = (p.animating, p.large);
    day_android::realize("activity", |env| DayActivity::makeActivity(env, animating, large))
}

fn update(_backend: &mut Android, h: &AHandle, patch: &ActivityPatch) {
    match patch {
        ActivityPatch::Animating(on) => {
            java(|env| DayActivity::setActivityAnimating(env, h.0.as_obj(), *on));
        }
    }
}

day_pieces::renderer!(day_android::RENDERERS, Android,
    kind: KIND, props: ActivityProps, patch: ActivityPatch,
    make: make, update: update);
