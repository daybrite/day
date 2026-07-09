//! Pack settings: CLI options + `${ENV}` interpolation for day.yaml `signing:` values (§17.3).
//! Interpolation happens at USE time, never at parse time, and missing variables are reported by
//! NAME only — secret values must never appear in output or errors (§16.5).

/// Options resolved from the `day pack` command line.
pub struct PackOptions {
    pub profile: String,
    /// Explicit format list (`--formats dmg,flatpak`); None = the target's defaults.
    pub formats: Option<Vec<String>>,
    /// Skip all signing stages (artifacts are marked unsigned).
    pub no_sign: bool,
    /// Sign but skip notarization (macOS).
    pub no_notarize: bool,
    /// Submit for notarization but do not wait for the verdict.
    pub no_wait: bool,
}

impl Default for PackOptions {
    fn default() -> Self {
        PackOptions {
            profile: "release".into(),
            formats: None,
            no_sign: false,
            no_notarize: false,
            no_wait: false,
        }
    }
}

/// A `${VAR}` reference that couldn't resolve. `MissingEnv` is DEGRADABLE per the §20 CI
/// contract: absent secrets lower the signing tier loudly, they never fail the pack.
/// `Malformed` is a day.yaml mistake and always an error.
pub enum InterpolateError {
    MissingEnv(String),
    Malformed(String),
}

impl InterpolateError {
    pub fn message(&self) -> String {
        match self {
            InterpolateError::MissingEnv(name) => format!(
                "environment variable {name} is not set (referenced as ${{{name}}} in day.yaml signing config)"
            ),
            InterpolateError::Malformed(m) => m.clone(),
        }
    }
}

/// Replace every `${VAR}` in `raw` with the value of the environment variable `VAR`.
/// A missing variable is an error naming the VARIABLE (never echoing any resolved value).
/// An env var that is set-but-empty counts as missing: CI materializes absent repository
/// secrets as empty strings (`${{ secrets.X }}`), and the §20 contract wants those to degrade.
pub fn interpolate_full(raw: &str) -> Result<String, InterpolateError> {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            return Err(InterpolateError::Malformed(format!(
                "unterminated ${{…}} reference in {raw:?}"
            )));
        };
        let name = &after[..end];
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(InterpolateError::Malformed(format!(
                "invalid environment variable name ${{{name}}}"
            )));
        }
        match std::env::var(name) {
            Ok(v) if !v.is_empty() => out.push_str(&v),
            _ => return Err(InterpolateError::MissingEnv(name.to_string())),
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// String-error variant for callers that treat every failure alike (`day sign --check`).
pub fn interpolate(raw: &str) -> Result<String, String> {
    interpolate_full(raw).map_err(|e| e.message())
}

/// Interpolate an optional value.
pub fn interpolate_opt(raw: Option<&String>) -> Result<Option<String>, String> {
    raw.map(|s| interpolate(s)).transpose()
}

/// Degradable resolve for pack (§20): a missing/empty env var warns (naming `what` and the
/// variable) and yields None — the caller falls back to its dev tier. Malformed syntax errors.
pub fn resolve_degradable(raw: &str, what: &str) -> Result<Option<String>, String> {
    match interpolate_full(raw) {
        Ok(v) => Ok(Some(v)),
        Err(InterpolateError::MissingEnv(name)) => {
            crate::ops::status(
                "Warning",
                &format!("{what}: ${{{name}}} is not set — degrading to the dev signing tier"),
            );
            Ok(None)
        }
        Err(e) => Err(e.message()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpolate_passthrough_and_vars() {
        assert_eq!(interpolate("plain").unwrap(), "plain");
        // SAFETY: test-local env mutation; tests touching distinct vars.
        unsafe { std::env::set_var("DAY_TEST_INTERP", "value") };
        assert_eq!(interpolate("a-${DAY_TEST_INTERP}-b").unwrap(), "a-value-b");
        assert_eq!(
            interpolate("${DAY_TEST_INTERP}${DAY_TEST_INTERP}").unwrap(),
            "valuevalue"
        );
    }

    #[test]
    fn interpolate_missing_names_variable_only() {
        let err = interpolate("literal-${DAY_TEST_UNSET_VAR}-tail").unwrap_err();
        assert!(err.contains("DAY_TEST_UNSET_VAR"));
        assert!(!err.contains("literal-")); // surrounding content must not leak into the error
    }

    #[test]
    fn interpolate_rejects_malformed() {
        assert!(interpolate("${unclosed").is_err());
        assert!(interpolate("${bad name}").is_err());
        assert!(interpolate("${}").is_err());
    }

    #[test]
    fn resolve_degradable_missing_and_empty_env() {
        // Unset → degrade to None.
        assert_eq!(
            resolve_degradable("${DAY_TEST_UNSET_VAR2}", "test").unwrap(),
            None
        );
        // Set-but-empty (CI's absent-secret shape) → also degrade.
        // SAFETY: test-local env mutation; tests touch distinct vars.
        unsafe { std::env::set_var("DAY_TEST_EMPTY", "") };
        assert_eq!(
            resolve_degradable("${DAY_TEST_EMPTY}", "test").unwrap(),
            None
        );
        // Malformed still errors.
        assert!(resolve_degradable("${broken", "test").is_err());
    }
}
