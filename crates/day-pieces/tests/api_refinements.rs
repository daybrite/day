// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Regression fixtures for scope ownership, numeric builders, and text reservation.
use day_mock::{MockProbe, MockToolkit};
use day_pieces::prelude::*;
use day_spec::{Event, NodeId, WindowOptions};
use std::{cell::RefCell, rc::Rc};

fn boot(root: impl FnOnce() -> AnyPiece + 'static) -> MockProbe {
    day_core::uninstall_tree();
    let (toolkit, probe) = MockToolkit::new();
    day_core::launch_with(toolkit, WindowOptions::default(), root);
    probe
}

fn node(id: &str) -> NodeId {
    day_core::with_tree(|tree| day_core::rnode_to_id(tree.find_by_id(id).unwrap()))
}

#[test]
fn numeric_builders_survive_layout_and_submit_after_committing() {
    let value = Signal::new(0.4);
    let enabled = Signal::new(true);
    let read_only = Signal::new(false);
    let hint = Signal::new("Fixture percent".to_owned());
    let submitted = Rc::new(RefCell::new(Vec::new()));
    let observed = submitted.clone();
    let probe = boot(move || {
        column((
            number_field(value)
                .padding(8.0)
                .range(0.0..=1.0)
                .format(NumberFormat::percent())
                .enabled(enabled)
                .read_only(read_only)
                .placeholder(hint)
                .input_purpose(InputPurpose::Decimal)
                .submit_label(SubmitLabel::Done)
                .on_submit(move || {
                    let text = day_core::with_tree(|tree| {
                        tree.node_probe(tree.find_by_id("fixture_number").unwrap())
                            .unwrap()
                            .text
                            .clone()
                    });
                    assert_eq!(text, NumberFormat::percent().format(value.get()));
                    observed.borrow_mut().push(value.get());
                })
                .id(day_core::id!(fixture_number)),
            stepper(value)
                .padding(4.0)
                .range(0.0..=1.0)
                .step(0.01)
                .format(NumberFormat::percent())
                .composed()
                .id(day_core::id!(fixture_stepper)),
        ))
        .any()
    });
    let number = node("fixture_number");
    let stepper = node("fixture_stepper");
    assert_ne!(number, stepper);
    assert_eq!(probe.find_by_kind("day.text_field").len(), 2);
    probe.emit(number, Event::TextChanged("73%".into()));
    assert_eq!(
        value.get(),
        0.4,
        "an edit remains a draft before submission"
    );
    probe.emit(number, Event::Submitted);
    assert_eq!(&*submitted.borrow(), &[0.73]);
    assert_eq!(value.get(), 0.73);
    probe.emit(number, Event::TextChanged("invalid fixture".into()));
    probe.emit(number, Event::Submitted);
    assert_eq!(&*submitted.borrow(), &[0.73, 0.73]);
    let restored = day_core::with_tree(|tree| {
        tree.node_probe(tree.find_by_id("fixture_number").unwrap())
            .unwrap()
            .text
            .clone()
    });
    assert_eq!(restored, "73%");
    probe.emit(number, Event::TextChanged("42%".into()));
    probe.emit(number, Event::FocusChanged(false));
    assert_eq!(value.get(), 0.42);
    assert_eq!(
        submitted.borrow().len(),
        2,
        "focus loss commits without submitting"
    );
    probe.emit(stepper, Event::ValueCommitted(0.85));
    assert_eq!(
        value.get(),
        0.85,
        "the decorated stepper id addresses its field"
    );
    hint.set("Fixture updated hint".into());
    enabled.set(false);
    read_only.set(true);
    day_reactive::flush_sync();
    let field = probe
        .find_by_kind("day.text_field")
        .into_iter()
        .find(|(_, w)| w.node == number.0)
        .unwrap()
        .1;
    assert!(!field.enabled);
    assert_eq!(field.placeholder, "Fixture updated hint");
    let traits = field.input_traits.unwrap();
    assert!(traits.read_only);
    assert_eq!(traits.purpose, InputPurpose::Decimal);
    assert_eq!(traits.submit_label, SubmitLabel::Done);
    probe.emit(number, Event::Submitted);
    assert_eq!(
        submitted.borrow().len(),
        2,
        "disabled/read-only fields do not submit callbacks"
    );
}

#[test]
fn reservation_uses_final_font_and_updates_its_sample_without_replacing_content() {
    let sample = Signal::new("888".to_owned());
    let value = Signal::new("1".to_owned());
    let probe = boot(move || {
        label(value)
            .id(day_core::id!(fixture_value))
            .reserving(sample)
            .font(Font::Title2)
            .weight(FontWeight::Bold)
            .font_scale(1.5)
            .tabular()
            .monospace()
            .wrapper_id(day_core::id!(fixture_reservation))
            .any()
    });
    let original = node("fixture_value");
    let labels = probe.find_by_kind("day.label");
    let content = &labels.iter().find(|(_, w)| w.text == "1").unwrap().1;
    let measurement = &labels.iter().find(|(_, w)| w.text == "888").unwrap().1;
    assert_eq!(content.font, measurement.font);
    let font = measurement.font.unwrap();
    assert_eq!(font.style, Font::Title2);
    assert_eq!(font.weight, Some(FontWeight::Bold));
    assert_eq!(font.scale, 1.5);
    assert!(font.tabular && font.monospace);
    assert!(measurement.a11y.hidden && measurement.a11y.decorative);
    assert_eq!(measurement.opacity, Some(0.0));
    assert_eq!(measurement.frame.size, day_spec::Size::ZERO);
    sample.set("888888".into());
    value.set("22".into());
    day_reactive::flush_sync();
    assert_eq!(node("fixture_value"), original);
    let labels = probe.find_by_kind("day.label");
    assert_eq!(labels.len(), 2);
    assert!(labels.iter().any(|(_, w)| w.text == "888888"));
    assert!(labels.iter().any(|(_, w)| w.text == "22"));
}

#[test]
fn reactive_labels_resolve_their_original_environment() {
    #[derive(Clone)]
    struct FixtureContext(&'static str);
    let source = Signal::new(0);
    let probe = boot(move || {
        column((
            with_environment(FixtureContext("first"), move || {
                label(move || {
                    format!(
                        "{} {}",
                        environment::<FixtureContext>().unwrap().0,
                        source.get()
                    )
                })
            }),
            with_environment(FixtureContext("second"), move || {
                label(move || {
                    format!(
                        "{} {}",
                        environment::<FixtureContext>().unwrap().0,
                        source.get()
                    )
                })
            }),
        ))
        .any()
    });
    source.set(1);
    day_reactive::flush_sync();
    let texts: Vec<_> = probe
        .find_by_kind("day.label")
        .into_iter()
        .map(|(_, w)| w.text)
        .collect();
    assert!(texts.contains(&"first 1".into()));
    assert!(texts.contains(&"second 1".into()));
}

#[test]
fn reservation_preserves_the_content_baseline_through_padding() {
    let probe = boot(|| {
        row((
            label("Fixture").reserving("Fixture widest").padding(10.0),
            text_field(Signal::new("Fixture field".to_owned())),
        ))
        .align(VAlign::FirstBaseline)
        .any()
    });
    let label = probe
        .find_by_kind("day.label")
        .into_iter()
        .find(|(_, w)| w.text == "Fixture")
        .unwrap()
        .1;
    let field = probe.find_by_kind("day.text_field")[0].1.clone();
    // The mock's label baseline is 12; fields center that text in their control height.
    let label_baseline = label.frame.origin.y + 12.0;
    let field_baseline = field.frame.origin.y + (field.frame.size.height - 16.0) / 2.0 + 12.0;
    assert!((label_baseline - field_baseline).abs() < 0.01);
}
