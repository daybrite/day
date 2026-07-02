//! The day showcase (DESIGN.md Appendix A, staged): every implemented piece, with state and
//! ids throughout. Fluent localization joins at M6; combo_box at M3; canvas gauge at M8a;
//! image at M8b — exactly the milestone staging of §21.2.

use day::prelude::*;
use day_piece_combobox::combo_box;

pub fn root() -> AnyPiece {
    install_locales(
        "en",
        &[
            ("en", include_str!("../locales/en/app.ftl")),
            ("fr", include_str!("../locales/fr/app.ftl")),
        ],
    );
    let count = Signal::new(0i64);
    let name = Signal::new(String::new());
    let volume = Signal::new(40.0f64);
    let subscribed = Signal::new(false);
    let flavors = Signal::new(vec!["vanilla".to_string(), "chocolate".into(), "pistachio".into()]);
    let flavor = Signal::new(Some(0usize));

    scroll(
        column((
            row((
                image("day-logo.png").frame(28.0, 28.0),
                label(tr("app-title")).font(Font::Title).id("controls-title"),
            ))
            .spacing(8.0),
            // — state: counter —
            row((
                button(tr("decrement")).action(move || count.update(|c| *c -= 1)).id("decrement-button"),
                label(tr("counter-value").arg("count", count)).id("counter-label"),
                button(tr("increment")).action(move || count.update(|c| *c += 1)).id("increment-button"),
            ))
            .spacing(8.0),
            divider(),
            // — text input + conditional —
            text_field(name).placeholder(tr("name-placeholder")).id("name-field"),
            when(
                move || !name.with(|s| s.is_empty()),
                move || label(tr("greeting").arg("name", name)).id("greeting-label"),
            ),
            // — slider with live readout —
            row((
                label(tr("volume-label")),
                slider(volume).range(0.0..=100.0).id("volume-slider"),
                label(move || format!("{:.0}", volume.get())).id("volume-value"),
            ))
            .spacing(8.0),
            toggle(subscribed)
                .id("subscribe-toggle")
                .a11y(|a| a.label("Subscribe to updates")), // a11y strings localize at M6.5 (IntoText a11y)
            // — an EXTERNAL Day Piece, registered like any built-in (§8.2, DP-21) —
            row((
                label(tr("flavor-label")),
                combo_box(flavors, flavor).id("flavor-combo"),
                label(move || {
                    let names = flavors.get();
                    flavor.get().and_then(|i| names.get(i).cloned()).unwrap_or_default()
                })
                .id("flavor-value"),
            ))
            .spacing(8.0),
            // — canvas gauge bound to the slider (§11) —
            gauge(volume),
            divider(),
            // — keyed collection (watch + monotonic keys, §5.4 / A.1) —
            history(count),
        ))
        .spacing(12.0)
        .align(HAlign::Leading)
        .padding(16.0),
    )
    .any()
}

fn gauge(value: Signal<f64>) -> AnyPiece {
    canvas(move |d, size| {
        if size.width <= 0.0 {
            return;
        }
        let r = Rect::from_size(size).inset(8.0);
        let track = Color::rgba(0.5, 0.5, 0.55, 0.35);
        let accent = Color::hex(0x2F6FDE);
        d.stroke(Shape::Arc { rect: r, start_deg: 135.0, sweep_deg: 270.0 }, track, 6.0);
        let frac = (value.get() / 100.0).clamp(0.0, 1.0);
        if frac > 0.0 {
            d.stroke(
                Shape::Arc { rect: r, start_deg: 135.0, sweep_deg: 270.0 * frac },
                accent,
                6.0,
            );
        }
        d.text(
            &format!("{:.0}", value.get()),
            Point::new(size.width / 2.0, size.height / 2.0),
            22.0,
            accent,
            true,
        );
    })
    .frame(110.0, 110.0)
    .id("gauge")
}

fn history(count: Signal<i64>) -> AnyPiece {
    let entries = Signal::new(Vec::<(u64, i64)>::new());
    let next_id = Signal::new(0u64);
    watch(
        move || count.get(),
        move |new, _old| {
            let id = next_id.get_untracked();
            next_id.set(id + 1);
            let v = *new;
            entries.update(|e| {
                e.push((id, v));
                if e.len() > 8 {
                    e.remove(0);
                }
            });
        },
    );
    column((
        label(tr("history-title")).font(Font::Headline),
        each(
            move || entries.get(),
            |e| e.0,
            move |slot: ItemSlot<(u64, i64), u64>| {
                label(move || {
                    tr("history-entry").arg("value", slot.field(|t| t.1)).format()
                })
            },
        ),
    ))
    .spacing(4.0)
    .align(HAlign::Leading)
    .any()
}

/// iOS entry: the Runner's main.swift calls this from the staticlib (DESIGN.md §17.4).
#[cfg(all(feature = "uikit", target_os = "ios"))]
#[unsafe(no_mangle)]
pub extern "C" fn day_main() {
    day::launch(
        day::WindowOptions {
            title: "Day Showcase".into(),
            size: day::prelude::Size::new(0.0, 0.0), // ignored: iOS uses the screen bounds
            min_size: None,
        },
        root,
    );
}

/// Android entries: DayBridge's natives resolve to these exports from the app cdylib.
#[cfg(all(feature = "widget", target_os = "android"))]
mod android_glue {
    use day::android::jni::JNIEnv;
    use day::android::jni::objects::{JClass, JObject, JString};
    use day::android::jni::sys::{jdouble, jfloat, jint, jlong};

    fn opt_string(env: &mut JNIEnv, s: &JString) -> Option<String> {
        if s.is_null() { None } else { env.get_string(s).ok().map(|v| v.into()) }
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn Java_dev_day_bridge_DayBridge_nativeStart(
        mut env: JNIEnv,
        _class: JClass,
        root: JObject,
        density: jfloat,
        w: jint,
        h: jint,
        autodrive: JString,
        locale: JString,
        env_blob: JString,
    ) {
        let a = opt_string(&mut env, &autodrive);
        let l = opt_string(&mut env, &locale);
        let e = opt_string(&mut env, &env_blob);
        day::android::start(&mut env, root, density, w, h, a, l, e, crate::root);
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn Java_dev_day_bridge_DayBridge_nativeOnEvent(
        mut env: JNIEnv,
        _class: JClass,
        id: jlong,
        kind: jint,
        num: jdouble,
        s: JString,
    ) {
        day::android::dispatch_event(&mut env, id, kind, num, &s);
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn Java_dev_day_bridge_DayBridge_nativeRunPosted(
        _env: JNIEnv,
        _class: JClass,
        token: jlong,
    ) {
        day::android::run_posted(token);
    }
}
