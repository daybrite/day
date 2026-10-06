// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The built-in pieces' conformance cases (docs/testing.md): one `#[day::test]` per aspect,
//! each a page and a drive, next to the constructors they prove. Compiled only under the
//! `conformance` feature, so a shipping app links none of it; `day test` runs them on a real
//! toolkit through the conformance app, and `cargo test` runs the same functions on the mock.

use day_core::conformance::{Case, Drive};
use day_core::*;
use day_reactive::Signal;
use day_spec::{Font, Role, kinds};

use crate::*;

/// A button fires its action, counts its presses, and a disabled one takes no press.
#[day_macros::test(day_core)]
pub fn button_status() -> Case {
    let presses = Signal::new(0i64);
    Case::new()
        .proves(kinds::BUTTON)
        .page(move || {
            column((
                button("Press")
                    .action(move || presses.update(|n| *n += 1))
                    .id("btn-press"),
                button("Off")
                    .enabled(false)
                    .action(move || presses.update(|n| *n += 100))
                    .id("btn-off"),
                label(move || presses.get().to_string()).id("btn-count"),
            ))
            .spacing(8.0)
        })
        .shot("default")
        .drive(|d: Drive| async move {
            d.tap("btn-press").await?;
            d.assert_text("btn-count", "1").await?;
            d.tap("btn-press").await?;
            d.assert_text("btn-count", "2").await?;
            d.tap("btn-off").await?;
            d.assert_text("btn-count", "2").await
        })
}

/// A toggle reports its flips to its signal, and a write to the signal reaches it.
#[day_macros::test(day_core)]
pub fn toggle_binding() -> Case {
    let on = Signal::new(false);
    Case::new()
        .proves(kinds::TOGGLE)
        .page(move || {
            column((
                toggle(on).id("tgl-switch"),
                label(move || if on.get() { "on" } else { "off" }.to_owned()).id("tgl-state"),
                button("Set on").action(move || on.set(true)).id("tgl-set"),
            ))
            .spacing(8.0)
        })
        .drive(|d: Drive| async move {
            d.toggle("tgl-switch", true).await?;
            d.assert_text("tgl-state", "on").await?;
            d.toggle("tgl-switch", false).await?;
            d.assert_text("tgl-state", "off").await?;
            d.tap("tgl-set").await?;
            d.assert_on("tgl-switch", true).await
        })
}

/// A text field writes what is typed to its signal, and shows what the signal is set to.
#[day_macros::test(day_core)]
pub fn text_field_binding() -> Case {
    let name = Signal::new(String::new());
    Case::new()
        .proves(kinds::TEXT_FIELD)
        .page(move || {
            column((
                text_field(name).placeholder("Name").id("tf-field"),
                label(move || name.get()).id("tf-echo"),
                button("Set")
                    .action(move || name.set("Ada".into()))
                    .id("tf-set"),
            ))
            .spacing(8.0)
        })
        .drive(|d: Drive| async move {
            d.input("tf-field", "Grace").await?;
            d.assert_text("tf-echo", "Grace").await?;
            d.tap("tf-set").await?;
            d.assert_text("tf-field", "Ada").await
        })
}

/// A secure field keeps its text and its focus across a show-password flip: on AppKit and
/// WinUI the flip rebuilds the widget as its other native class (docs/textfield.md).
#[day_macros::test(day_core)]
pub fn text_field_secure() -> Case {
    let password = Signal::new(String::new());
    let shown = Signal::new(false);
    Case::new()
        .proves(kinds::TEXT_FIELD)
        .proves_duty("set_input_traits")
        .page(move || {
            column((
                secure_field(password)
                    .secure(move || !shown.get())
                    .placeholder("Password")
                    .id("tfs-pass"),
                toggle(shown).id("tfs-show"),
                label(move || password.get().chars().count().to_string()).id("tfs-len"),
            ))
            .spacing(8.0)
        })
        .shot("masked")
        .drive(|d: Drive| async move {
            d.input("tfs-pass", "correct horse").await?;
            d.assert_text("tfs-len", "13").await?;
            d.focus("tfs-pass").await?;
            d.toggle("tfs-show", true).await?;
            d.wait_idle().await?;
            d.shot("shown").await?;
            d.assert_focused("tfs-pass", true).await?;
            d.input("tfs-pass", "battery").await?;
            d.assert_text("tfs-len", "7").await?;
            d.toggle("tfs-show", false).await?;
            d.wait_idle().await?;
            d.assert_text("tfs-pass", "battery").await
        })
}

/// A field held to four characters cuts what is typed past them, on every toolkit alike.
#[day_macros::test(day_core)]
pub fn text_field_max_length() -> Case {
    let pin = Signal::new(String::new());
    Case::new()
        .proves(kinds::TEXT_FIELD)
        .page(move || {
            column((
                text_field(pin).max_length(4).id("tfm-pin"),
                label(move || pin.get()).id("tfm-echo"),
            ))
            .spacing(8.0)
        })
        .drive(|d: Drive| async move {
            d.input("tfm-pin", "123456").await?;
            d.assert_text("tfm-echo", "1234").await?;
            d.assert_text("tfm-pin", "1234").await
        })
}

/// A slider's value reaches its signal on the step grid the piece declares, whatever the
/// native control's own stepping.
#[day_macros::test(day_core)]
pub fn slider_range() -> Case {
    let value = Signal::new(40.0f64);
    Case::new()
        .proves(kinds::SLIDER)
        .page(move || {
            column((
                slider(value).range(0.0..=100.0).step(10.0).id("sl-slider"),
                label(move || format!("{:.0}", value.get())).id("sl-value"),
            ))
            .spacing(8.0)
        })
        .drive(|d: Drive| async move {
            d.set_value("sl-slider", 72.0).await?;
            d.assert_text("sl-value", "70").await?;
            d.assert_value("sl-slider", 70.0).await
        })
}

/// A label shows its text, follows a signal, and reports its explicit role natively where
/// the toolkit reads its tree back.
#[day_macros::test(day_core)]
pub fn label_heading() -> Case {
    let text = Signal::new("Hello".to_owned());
    Case::new()
        .proves(kinds::LABEL)
        .proves_duty("set_a11y")
        .page(move || {
            column((
                label(move || text.get())
                    .font(Font::Headline)
                    .a11y(|a| a.role(Role::Heading(2)))
                    .id("lbl-heading"),
                button("Rename")
                    .action(move || text.set("Renamed".into()))
                    .id("lbl-rename"),
            ))
            .spacing(8.0)
        })
        .drive(|d: Drive| async move {
            d.assert_text("lbl-heading", "Hello").await?;
            d.tap("lbl-rename").await?;
            d.assert_text("lbl-heading", "Renamed").await?;
            d.a11y_audit(Some("lbl-heading")).await
        })
}

/// A headless test: no page, app state checked on the app's own thread, in its own process.
#[day_macros::test(day_core)]
pub fn headless_sanity() -> Case {
    Case::headless().run(|t: Drive| async move {
        let count = Signal::new(1i64);
        count.update(|n| *n += 1);
        t.check_eq(count.get(), 2)?;
        day_core::sleep(1).await;
        t.check_eq(count.get(), 2)
    })
}

day_core::tests! {
    button_status,
    toggle_binding,
    text_field_binding,
    text_field_secure,
    text_field_max_length,
    slider_range,
    label_heading,
    headless_sanity,
}

/// The conformance app's content: a nav over every registered GUI case, one route per case,
/// so a person can open any case on any toolkit and look at it. A run does not go through it:
/// the app roots it in [`test_host`](crate::test_host), which shows each driven case's page
/// alone while it runs. An app with its own tests can browse them the same way.
pub fn test_pages() -> impl Piece {
    let selection = Signal::new(String::new());
    nav(selection)
        .items(
            || {
                day_core::conformance::cases()
                    .into_iter()
                    .filter(|c| c.kind() == day_core::conformance::TestKind::Gui)
                    .map(|c| c.name().to_owned())
                    .collect::<Vec<String>>()
            },
            |name: &String| item(name.clone(), name.clone()),
        )
        .destination(
            |key: &String| match day_core::conformance::case_named(key) {
                Some(case) => guarded_page(case),
                None => label(format!("no test {key:?}")).any(),
            },
        )
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    /// The roster (what wasm registers) names exactly the functions the link-time slice holds,
    /// under the same names.
    #[test]
    fn roster_matches_the_link_time_registry() {
        let entries = |tests: Vec<day_core::conformance::TestFn>| {
            let mut v: Vec<(String, usize)> = tests
                .iter()
                .map(|t| (t.name(), t.build_fn() as usize))
                .collect();
            v.sort();
            v
        };
        assert_eq!(
            entries(day_core::conformance::TESTS.to_vec()),
            entries(super::roster()),
            "every #[day::test] here must also be in `day_core::tests!`"
        );
    }
}
