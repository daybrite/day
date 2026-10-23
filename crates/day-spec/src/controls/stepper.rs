// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

pub const KIND: &str = "day.piece.stepper";

/// The tag every in-process native arm reports a value under. Across a native boundary the
/// tag arrives empty and only the payload matters (§8.2); the front-end reads the text
/// either way.
pub const VALUE_TAG: &str = "stepper:value";

/// Full props (realize) for the native leaf. Everything but `value` is set once at build.
#[derive(Clone, Debug, PartialEq)]
pub struct StepperProps {
    pub value: f64,
    pub min: f64,
    pub max: f64,
    pub step: f64,
    /// Fraction digits the field shows (and the native widget's display precision).
    pub decimals: u32,
}

impl Default for StepperProps {
    fn default() -> Self {
        StepperProps {
            value: 0.0,
            min: 0.0,
            max: 100.0,
            step: 1.0,
            decimals: 0,
        }
    }
}

/// The single imperative update: show `value` in the field.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StepperPatch {
    SetValue(f64),
}
