// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Canonical `flow:` documents. Transport, project expansion and execution stay with callers.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::Step;

/// Whether the host runner continues a flow after a failed step.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailurePolicy {
    #[default]
    Continue,
    Stop,
}

/// An invalid document or step, with a one-based flow position when applicable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormatError(String);

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for FormatError {}

/// A typed operation and its non-protocol fields (platform gates, gallery metadata, etc.).
/// Annotations survive document round trips but are not part of [`crate::Request`].
#[derive(Clone, Debug, PartialEq)]
pub struct ScriptStep {
    pub step: Step,
    pub annotations: Map<String, Value>,
}

impl ScriptStep {
    /// Accept an on-disk single-key entry or the flattened JSON wire spelling.
    /// Unknown parameter fields are retained as annotations for forward compatibility.
    pub fn from_value(value: Value) -> Result<Self, FormatError> {
        let mut fields = normalize(value)?;
        let step: Step = serde_json::from_value(Value::Object(fields.clone()))
            .map_err(|e| FormatError(e.to_string()))?;
        // Host sleeps must be representable durations; reject them before callers can panic.
        let duration = match &step {
            Step::Pause { secs } => Some(*secs),
            Step::ExpectExit { within } => *within,
            _ => None,
        };
        if let Some(secs) = duration
            && std::time::Duration::try_from_secs_f64(secs).is_err()
        {
            return Err(FormatError(format!(
                "{}: duration must be finite, non-negative and representable",
                step.op()
            )));
        }
        for key in ["skip_on", "only_on"] {
            if let Some(value) = fields.get(key)
                && !value
                    .as_array()
                    .is_some_and(|a| a.iter().all(Value::is_string))
            {
                return Err(FormatError(format!("{key} must be a sequence of strings")));
            }
        }
        if fields.get("animation").is_some_and(|v| !v.is_boolean()) {
            return Err(FormatError("animation must be a boolean".into()));
        }
        for key in protocol_fields(&step).keys() {
            fields.remove(key);
        }
        // This protocol field is omitted by serde when false, but is never an annotation.
        if matches!(step, Step::NavBack { .. }) {
            fields.remove("native");
        }
        Ok(Self {
            step,
            annotations: fields,
        })
    }

    /// Flatten the operation and annotations for host-side processing. Protocol fields win
    /// if a caller manually adds a conflicting annotation.
    pub fn to_value(&self) -> Value {
        let mut fields = self.annotations.clone();
        // Preserve the omission of a default `native: false`, even with conflicting metadata.
        if matches!(self.step, Step::NavBack { .. }) {
            fields.remove("native");
        }
        fields.extend(protocol_fields(&self.step));
        Value::Object(fields)
    }
}

impl From<Step> for ScriptStep {
    fn from(step: Step) -> Self {
        Self {
            step,
            annotations: Map::new(),
        }
    }
}

/// A script document, including host policy and author-supplied document metadata.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Script {
    pub steps: Vec<ScriptStep>,
    pub on_failure: FailurePolicy,
    pub metadata: Map<String, Value>,
}

impl Script {
    /// Parse and validate the entire flow before executing any of it. YAML comments and a
    /// UTF-8 BOM are accepted; every flow entry must have exactly one operation key.
    pub fn from_yaml(yaml: &str) -> Result<Self, FormatError> {
        // YAML's own Value rejects duplicate keys before conversion to a JSON map would
        // overwrite them. Reject non-finite numbers before JSON conversion, which would
        // silently turn them into null, including in optional durations.
        let yaml: serde_norway::Value = serde_norway::from_str(yaml.trim_start_matches('\u{feff}'))
            .map_err(|e| FormatError(e.to_string()))?;
        if !finite_numbers(&yaml) {
            return Err(FormatError("script contains a non-finite number".into()));
        }
        let doc = serde_json::to_value(yaml).map_err(|e| FormatError(e.to_string()))?;
        let Value::Object(mut metadata) = doc else {
            return Err(FormatError("script must be a mapping".into()));
        };
        let on_failure = match metadata.remove("on_failure") {
            None => FailurePolicy::Continue,
            Some(value) => serde_json::from_value(value)
                .map_err(|_| FormatError("on_failure must be stop or continue".into()))?,
        };
        let Some(Value::Array(flow)) = metadata.remove("flow") else {
            return Err(FormatError("script has no `flow:` sequence".into()));
        };
        let mut steps = Vec::with_capacity(flow.len());
        for (index, entry) in flow.into_iter().enumerate() {
            // Flattened entries are accepted by JSON driving, but a YAML flow has one
            // operation key; allowing both would make `op: tap` an ambiguous disk entry.
            if !entry.as_object().is_some_and(|m| m.len() == 1) {
                return Err(FormatError(format!(
                    "flow step {}: entries must be single-key mappings",
                    index + 1
                )));
            }
            let fields = normalize_entry(entry)
                .map_err(|e| FormatError(format!("flow step {}: {e}", index + 1)))?;
            let step = ScriptStep::from_value(Value::Object(fields))
                .map_err(|e| FormatError(format!("flow step {}: {e}", index + 1)))?;
            steps.push(step);
        }
        if steps
            .iter()
            .take(steps.len().saturating_sub(1))
            .any(|s| matches!(s.step, Step::ExpectExit { .. }))
        {
            return Err(FormatError("expect_exit must be the last step".into()));
        }
        Ok(Self {
            steps,
            on_failure,
            metadata,
        })
    }

    /// Serialize as the canonical single-key YAML form. Optional null parameters are omitted;
    /// author metadata and annotations are retained. The default failure policy is implicit.
    pub fn to_yaml(&self) -> String {
        let mut doc = self.metadata.clone();
        doc.remove("on_failure");
        if self.on_failure == FailurePolicy::Stop {
            doc.insert("on_failure".into(), Value::String("stop".into()));
        }
        let flow = self
            .steps
            .iter()
            .map(|step| {
                let Value::Object(mut fields) = step.to_value() else {
                    unreachable!("script step is an object")
                };
                fields.remove("op");
                fields.retain(|key, value| !value.is_null() || step.annotations.contains_key(key));
                let params = if fields.is_empty() {
                    Value::Null
                } else {
                    Value::Object(fields)
                };
                let mut entry = Map::new();
                entry.insert(step.step.op().into(), params);
                Value::Object(entry)
            })
            .collect();
        doc.insert("flow".into(), Value::Array(flow));
        // The document contains only JSON scalars, sequences and string-keyed maps.
        serde_norway::to_string(&doc).expect("JSON dayscript document serializes as YAML")
    }
}

/// Serialize operations without host annotations, as the recorder does.
pub fn steps_to_yaml(steps: &[Step]) -> String {
    Script {
        steps: steps.iter().cloned().map(ScriptStep::from).collect(),
        ..Script::default()
    }
    .to_yaml()
}

/// Read operations for in-process playback. Host annotations and failure policy are validated
/// but execution remains the caller's responsibility.
pub fn steps_from_yaml(yaml: &str) -> Result<Vec<Step>, String> {
    Script::from_yaml(yaml)
        .map(|script| script.steps.into_iter().map(|s| s.step).collect())
        .map_err(|e| e.to_string())
}

fn protocol_fields(step: &Step) -> Map<String, Value> {
    match serde_json::to_value(step).expect("dayscript step serializes as JSON") {
        Value::Object(fields) => fields,
        _ => unreachable!("internally tagged step is an object"),
    }
}

fn finite_numbers(value: &serde_norway::Value) -> bool {
    use serde_norway::Value as Yaml;
    match value {
        Yaml::Number(n) => n.is_finite(),
        Yaml::Sequence(values) => values.iter().all(finite_numbers),
        Yaml::Mapping(fields) => fields
            .iter()
            .all(|(key, value)| finite_numbers(key) && finite_numbers(value)),
        Yaml::Tagged(value) => finite_numbers(&value.value),
        Yaml::Null | Yaml::Bool(_) | Yaml::String(_) => true,
    }
}

fn normalize(value: Value) -> Result<Map<String, Value>, FormatError> {
    if let Some(fields) = value.as_object()
        && fields.contains_key("op")
    {
        return Ok(fields.clone());
    }
    normalize_entry(value)
}

fn normalize_entry(value: Value) -> Result<Map<String, Value>, FormatError> {
    let Value::Object(entry) = value else {
        return Err(FormatError("step must be a mapping".into()));
    };
    if entry.len() != 1 {
        return Err(FormatError("step must be a single-key mapping".into()));
    }
    let (op, params) = entry.into_iter().next().expect("exactly one key");
    let mut fields = match params {
        Value::Object(fields) => fields,
        Value::String(s) if op == "screenshot" => Map::from_iter([("name".into(), s.into())]),
        Value::Number(n) if op == "pause" => Map::from_iter([("secs".into(), n.into())]),
        Value::String(s) if op == "resize" && s == "auto" => {
            Map::from_iter([("restore".into(), true.into())])
        }
        Value::Null => Map::new(),
        other => {
            return Err(FormatError(format!(
                "step {op}: unsupported params {other}"
            )));
        }
    };
    if fields.contains_key("op") {
        return Err(FormatError(format!(
            "step {op}: params cannot override `op`"
        )));
    }
    fields.insert("op".into(), op.into());
    Ok(fields)
}
