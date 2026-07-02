//! day.yaml — the project manifest (DESIGN.md §17.3), v0 subset.

use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub day: u32,
    pub app: App,
    // Parsed for schema validation (deny_unknown_fields); the app scaffold consumes these,
    // the CLI does not yet (§17.3).
    #[serde(default)]
    #[allow(dead_code)]
    pub targets: Vec<String>,
    #[serde(default)]
    #[allow(dead_code)]
    pub window: Window,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct App {
    pub name: String,
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default = "default_version")]
    pub version: String,
    #[serde(default = "default_build")]
    pub build: u64,
}

fn default_version() -> String {
    "0.1.0".into()
}
fn default_build() -> u64 {
    1
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)] // see Manifest::window
pub struct Window {
    #[serde(default = "default_w")]
    pub width: f64,
    #[serde(default = "default_h")]
    pub height: f64,
}

impl Default for Window {
    fn default() -> Self {
        Window {
            width: default_w(),
            height: default_h(),
        }
    }
}

fn default_w() -> f64 {
    480.0
}
fn default_h() -> f64 {
    640.0
}

pub struct Project {
    pub root: PathBuf,
    pub manifest: Manifest,
}

/// Find the nearest ancestor directory containing day.yaml (from `start` or cwd).
pub fn find_project(start: Option<&Path>) -> Result<Project, String> {
    let mut dir = match start {
        Some(p) => p.to_path_buf(),
        None => std::env::current_dir().map_err(|e| e.to_string())?,
    };
    loop {
        let candidate = dir.join("day.yaml");
        if candidate.exists() {
            let text = std::fs::read_to_string(&candidate).map_err(|e| e.to_string())?;
            let manifest: Manifest =
                serde_norway::from_str(&text).map_err(|e| format!("day.yaml: {e}"))?;
            if manifest.day != 1 {
                return Err(format!(
                    "day.yaml: unsupported schema version {}",
                    manifest.day
                ));
            }
            return Ok(Project {
                root: dir,
                manifest,
            });
        }
        if !dir.pop() {
            return Err("no day.yaml found in this directory or any ancestor".into());
        }
    }
}
