//! Evidence bundle metadata and output helpers.

use anyhow::{Context, Result};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::contract::EnvContract;
use crate::scenario::ScenarioLimits;

/// Tool version emitted in evidence metadata.
pub(crate) const TOOL_VERSION: &str = "0.7.0";

/// Top-level metadata file written for each run.
#[derive(Serialize)]
pub(crate) struct Meta {
    pub(crate) tool_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) scenario_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) scenario_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) binary: Option<BinaryMeta>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) fixture: Option<FixtureMeta>,
    pub(crate) env: EnvContract,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) limits: Option<ScenarioLimits>,
    pub(crate) outcome: Outcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<ErrorReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) result: Option<ResultMeta>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) artifacts: Option<ArtifactsMeta>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) sandbox: Option<SandboxMeta>,
}

/// Binary identity recorded in metadata.
#[derive(Serialize)]
pub(crate) struct BinaryMeta {
    pub(crate) path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) sha256: Option<String>,
}

/// Fixture identity recorded in metadata.
#[derive(Serialize)]
pub(crate) struct FixtureMeta {
    pub(crate) id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) sha256: Option<String>,
}

/// Execution outcome details for a completed run.
#[derive(Serialize)]
pub(crate) struct ResultMeta {
    pub(crate) exit_code: Option<i32>,
    pub(crate) timed_out: bool,
    pub(crate) wall_time_ms: u64,
}

/// Artifact hashes and sizes recorded in metadata.
#[derive(Serialize)]
pub(crate) struct ArtifactsMeta {
    pub(crate) stdout_sha256: String,
    pub(crate) stderr_sha256: String,
    pub(crate) stdout_bytes: u64,
    pub(crate) stderr_bytes: u64,
}

/// Sandbox mode indicator recorded in metadata.
#[derive(Serialize)]
pub(crate) struct SandboxMeta {
    pub(crate) mode: String,
}

/// Error report recorded when execution fails early.
#[derive(Serialize)]
pub(crate) struct ErrorReport {
    pub(crate) code: String,
    pub(crate) message: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) details: Vec<String>,
}

/// Outcome classification for a scenario run.
#[derive(Serialize, Copy, Clone)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Outcome {
    SchemaInvalid,
    BinaryMissing,
    FixtureMissing,
    FixtureInvalid,
    SandboxFailed,
    TimedOut,
    Exited,
}

/// Evidence directory name used under the output root.
pub(crate) const EVIDENCE_DIR: &str = "evidence";

/// Create a unique evidence directory for this run.
pub(crate) fn create_evidence_dir(
    out_dir: &Path,
    scenario_hash: Option<&str>,
    label: Option<&str>,
) -> Result<PathBuf> {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let label = label.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    });
    let label = label.map(sanitize_label).unwrap_or_else(|| "run".to_string());
    let hash = scenario_hash.unwrap_or("unknown");
    let short_hash = if hash.len() > 12 { &hash[..12] } else { hash };
    let run_id = format!("{label}-{short_hash}-{ts}");
    let path = out_dir.join(EVIDENCE_DIR).join(run_id);
    fs::create_dir_all(&path).context("create evidence dir")?;
    Ok(path)
}

fn sanitize_label(label: &str) -> String {
    let mut sanitized = String::new();
    let mut last_sep = false;
    for ch in label.chars() {
        let mapped = if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            ch.to_ascii_lowercase()
        } else {
            '_'
        };
        if mapped == '_' {
            if last_sep {
                continue;
            }
            last_sep = true;
        } else {
            last_sep = false;
        }
        sanitized.push(mapped);
        if sanitized.len() >= 32 {
            break;
        }
    }
    while sanitized.ends_with('_') {
        sanitized.pop();
    }
    if sanitized.is_empty() {
        "run".to_string()
    } else {
        sanitized
    }
}

/// Serialize and write `meta.json` into the evidence directory.
pub(crate) fn write_meta(path: &Path, meta: Meta) -> Result<()> {
    let json = serde_json::to_vec_pretty(&meta).context("serialize meta.json")?;
    fs::write(path.join("meta.json"), json).context("write meta.json")?;
    Ok(())
}
