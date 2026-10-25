// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
//! Coverage gates consume explicit reports, never a glob of possibly stale artifacts.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DayscriptConfig {
    pub coverage: CoveragePolicy,
}
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CoveragePolicy {
    pub enabled: bool,
    pub minimum: Option<f64>,
    pub targets: BTreeMap<String, TargetPolicy>,
    pub exclusions: Vec<Exclusion>,
}
impl CoveragePolicy {
    pub fn active(&self) -> bool {
        self.enabled || self.minimum.is_some() || !self.targets.is_empty()
    }
}
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetPolicy {
    pub minimum: Option<f64>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Exclusion {
    pub id: String,
    pub reason: String,
    #[serde(default)]
    pub targets: Vec<String>,
}
#[derive(Debug, Serialize)]
pub struct Summary {
    pub target: String,
    pub registered: usize,
    pub excluded: BTreeSet<String>,
    pub eligible: usize,
    pub exercised: BTreeSet<String>,
    pub interacted: BTreeSet<String>,
    pub asserted: BTreeSet<String>,
    pub uncovered: BTreeSet<String>,
    pub percent: Option<f64>,
    pub minimum: Option<f64>,
    pub passed: bool,
}

pub fn summarize(reports: &[Value], policy: &CoveragePolicy) -> Result<Summary, String> {
    let first = reports.first().ok_or("coverage suite has no reports")?;
    let catalog: day_script_protocol::IdCatalog =
        serde_json::from_value(first["id_catalog"].clone())
            .map_err(|e| format!("missing/invalid compiled registry: {e}"))?;
    catalog.validate()?;
    let target = first["target"].as_str().ok_or("missing target")?.to_owned();
    let minimum = policy
        .targets
        .get(&target)
        .and_then(|p| p.minimum)
        .or(policy.minimum);
    if minimum.is_some_and(|v| !v.is_finite() || !(0.0..=100.0).contains(&v)) {
        return Err("coverage minimum must be finite and between 0 and 100".into());
    }
    let names: BTreeSet<_> = catalog
        .declarations
        .iter()
        .map(|d| d.family.clone())
        .collect();
    let mut excluded = BTreeSet::new();
    for e in &policy.exclusions {
        if !e.targets.is_empty() && !e.targets.contains(&target) {
            continue;
        }
        if e.reason.trim().is_empty() || !names.contains(&e.id) {
            return Err(format!(
                "invalid coverage exclusion {:?}: requires a declared family and reason",
                e.id
            ));
        }
        excluded.insert(e.id.clone());
    }
    let eligible: BTreeSet<_> = names.difference(&excluded).cloned().collect();
    let mut interacted = BTreeSet::new();
    let mut asserted = BTreeSet::new();
    let mut runs = BTreeSet::new();
    let mut scripts = BTreeSet::new();
    for report in reports {
        for field in [
            "target",
            "variant",
            "locale",
            "flavor",
            "device_slug",
            "id_catalog",
            "id_suite",
        ] {
            if report[field] != first[field] {
                return Err(format!(
                    "coverage reports differ in {field}; use a separate gate per target/profile/build invocation"
                ));
            }
        }
        if report["id_suite"].as_str().is_none() {
            return Err("missing coverage suite identity".into());
        }
        let run = report["run_id"].as_str().ok_or("missing report identity")?;
        if !runs.insert(run) {
            return Err("duplicate coverage report".into());
        }
        let script = report["script"].as_str().ok_or("missing script identity")?;
        if !scripts.insert(script) {
            return Err("multiple attempts of a script cannot be unioned".into());
        }
        if report["status"] != "passed"
            || report["steps"]["failed"].as_u64() != Some(0)
            || report["steps"]["aborted"].as_u64() != Some(0)
        {
            return Err(format!("incomplete/failed coverage script: {script}"));
        }
        let steps = report["step_results"]
            .as_array()
            .ok_or("missing step evidence")?;
        if Some(steps.len() as u64) != report["steps"]["planned"].as_u64() {
            return Err(format!("missing step results for {script}"));
        }
        for step in steps {
            if step["status"] != "passed" {
                continue;
            }
            for evidence in step["id_evidence"].as_array().into_iter().flatten() {
                let family = evidence["family"]
                    .as_str()
                    .ok_or("missing evidence family")?;
                if !names.contains(family) {
                    return Err(format!("evidence names an undeclared family: {family}"));
                }
                if !eligible.contains(family) {
                    continue;
                }
                match evidence["class"].as_str() {
                    Some("interaction") => {
                        interacted.insert(family.to_owned());
                    }
                    Some("assertion") => {
                        asserted.insert(family.to_owned());
                    }
                    Some("observed" | "negative") => {}
                    _ => return Err("unknown evidence class".into()),
                }
            }
        }
    }
    let exercised: BTreeSet<_> = interacted.union(&asserted).cloned().collect();
    let percent = if eligible.is_empty() {
        None
    } else {
        Some(100.0 * exercised.len() as f64 / eligible.len() as f64)
    };
    // Compare counts, not a rounded display percentage. Empty denominators never earn 100%.
    let passed = minimum.is_none_or(|min| {
        !eligible.is_empty() && exercised.len() as f64 * 100.0 >= min * eligible.len() as f64
    });
    Ok(Summary {
        target,
        registered: names.len(),
        excluded,
        eligible: eligible.len(),
        uncovered: eligible.difference(&exercised).cloned().collect(),
        exercised,
        interacted,
        asserted,
        percent,
        minimum,
        passed,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SuiteManifest {
    schema_version: u32,
    target: String,
    /// Exactly one final report for each expected script. Paths are relative to this manifest.
    reports: BTreeMap<String, std::path::PathBuf>,
    #[serde(default)]
    policy: CoveragePolicy,
}
pub fn check_manifest(path: &Path) -> Result<(), String> {
    let manifest: SuiteManifest =
        serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    if manifest.schema_version != 1 || manifest.reports.is_empty() {
        return Err("unsupported/empty coverage suite manifest".into());
    }
    let mut reports = Vec::new();
    for (script, report_path) in manifest.reports {
        let p = path.parent().unwrap_or(Path::new(".")).join(report_path);
        let report: Value = serde_json::from_slice(
            &std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))?,
        )
        .map_err(|e| e.to_string())?;
        if report["script"] != script || report["target"] != manifest.target {
            return Err(format!(
                "report does not match expected script/target: {}",
                p.display()
            ));
        }
        reports.push(report);
    }
    let summary = summarize(&reports, &manifest.policy)?;
    println!("{}", serde_json::to_string_pretty(&summary).unwrap());
    if summary.passed {
        Ok(())
    } else {
        Err("element ID coverage is below the required minimum".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn report(script: &str, evidence: Value) -> Value {
        // Synthetic protocol fixture; these are not app-owned UI declarations.
        json!({"run_id":script,"script":script,"target":"fixture-mock","id_suite":"same-build-session",
            "status":"passed","steps":{"planned":1,"failed":0,"aborted":0},
            "id_catalog":{"schema_version":1,"declarations":[
                {"family":"save","keyed":false,"package":"fixture","version":"1","locations":[]},
                {"family":"row","keyed":true,"package":"fixture","version":"1","locations":[]}]},
            "step_results":[{"status":"passed","id_evidence":evidence}]})
    }
    #[test]
    fn union_and_threshold_count_families_not_actions_or_keys() {
        let a = report(
            "a",
            json!([{"family":"save","class":"interaction"},{"family":"save","class":"assertion"}]),
        );
        let b = report(
            "b",
            json!([{"family":"row","class":"observed"},{"family":"row","class":"negative"}]),
        );
        let mut policy = CoveragePolicy {
            enabled: true,
            minimum: Some(50.1),
            ..Default::default()
        };
        let summary = summarize(&[a.clone(), b.clone()], &policy).unwrap();
        assert_eq!(summary.exercised.len(), 1);
        assert_eq!(summary.percent, Some(50.0));
        assert!(!summary.passed);
        policy.minimum = Some(50.0);
        assert!(summarize(&[a, b], &policy).unwrap().passed);
    }
    #[test]
    fn refuses_missing_mixed_failed_and_retried_evidence() {
        let good = report("a", json!([]));
        assert!(summarize(&[], &CoveragePolicy::default()).is_err());
        assert!(summarize(&[good.clone(), good.clone()], &CoveragePolicy::default()).is_err());
        for (field, value) in [
            ("status", json!("failed")),
            ("id_suite", json!("other-build")),
            ("target", json!("other-target")),
            ("step_results", json!([])),
        ] {
            let mut bad = report("b", json!([]));
            bad[field] = value;
            assert!(
                summarize(&[good.clone(), bad], &CoveragePolicy::default()).is_err(),
                "{field}"
            );
        }
    }
    #[test]
    fn empty_denominator_and_exclusion_validation() {
        let mut empty = report("a", json!([]));
        empty["id_catalog"]["declarations"] = json!([]);
        let mut policy = CoveragePolicy {
            enabled: true,
            minimum: Some(0.0),
            ..Default::default()
        };
        let summary = summarize(&[empty], &policy).unwrap();
        assert!(!summary.passed);
        assert_eq!(summary.percent, None);
        policy.exclusions.push(Exclusion {
            id: "typo".into(),
            reason: "fixture".into(),
            targets: vec![],
        });
        assert!(summarize(&[report("a", json!([]))], &policy).is_err());
    }
}
