use day_pieces::pager::*;

use crate::jni::objects::JString;
use crate::{AHandle, Android, java};
use day_spec::NodeId;

// This piece's Java class (src/DayPager.java, on the app classpath at build).
crate::java_class! {
    DayPager => dev.daybrite.day.piece.pager.DayPager,
    methods {
        static fn make(id: jlong, slots: jint) -> android.view.View,
        static fn configure(view: android.view.View, order: JString, selected: jint, epoch: jlong),
    },
}

fn make(_: &mut Android, p: &PagerProps, id: NodeId) -> AHandle {
    let slots = p.slots as i32;
    crate::realize("pager", |env| DayPager::make(env, id.0 as i64, slots))
}

fn update(_: &mut Android, h: &AHandle, patch: &PagerPatch) {
    let p = &patch.0;
    let order = p
        .order
        .iter()
        .map(usize::to_string)
        .collect::<Vec<_>>()
        .join(",");
    java(|env| {
        let order = JString::from_str(env, &order)?;
        DayPager::configure(env, h.0.as_obj(), &order, p.selected as i32, p.epoch as i64)
    });
}

day_pieces::renderer!(crate::RENDERERS, Android, kind:KIND, props:PagerProps, patch:PagerPatch, make:make, update:update, measure:day_pieces::fill_measure);
