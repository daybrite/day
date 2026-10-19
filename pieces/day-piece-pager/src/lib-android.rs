use super::*;
use day_android::jni::objects::JValue;
use day_android::{AHandle, Android, DayEnv, with_env};
use day_spec::NodeId;
const CLASS: &str = "dev/daybrite/day/piece/pager/DayPager";
fn make(_: &mut Android, p: &PagerProps, id: NodeId) -> AHandle {
    with_env(|env| {
        let view = env
            .dcall_static(
                CLASS,
                "make",
                "(JI)Landroid/view/View;",
                &[JValue::Long(id.0 as i64), JValue::Int(p.slots as i32)],
            )
            .expect("DayPager.make")
            .l()
            .expect("View");
        AHandle(std::sync::Arc::new(
            env.new_global_ref(view).expect("pager reference"),
        ))
    })
}
fn update(_: &mut Android, h: &AHandle, patch: &PagerPatch) {
    let p = &patch.0;
    with_env(|env| {
        let order = p
            .order
            .iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let order = env.new_string(order).expect("pager order");
        let _ = env.dcall_static(
            CLASS,
            "configure",
            "(Landroid/view/View;Ljava/lang/String;IJ)V",
            &[
                JValue::Object(h.0.as_obj()),
                JValue::Object(order.as_ref()),
                JValue::Int(p.selected as i32),
                JValue::Long(p.epoch as i64),
            ],
        );
    });
}
day_pieces::renderer!(day_android::RENDERERS, Android, kind:KIND, props:PagerProps, patch:PagerPatch, make:make, update:update, measure:day_pieces::fill_measure);
