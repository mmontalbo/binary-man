//! Schema types for surface extraction, planning, and reports.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BinaryIdentity {
    pub path: PathBuf,
    pub hash: Hash,
    pub platform: Platform,
    pub env: EnvSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hash {
    pub algo: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Platform {
    pub os: String,
    pub arch: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvSnapshot {
    pub locale: String,
    pub tz: String,
    pub term: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureOutput {
    pub args: Vec<String>,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelfReport {
    pub help: CaptureOutput,
    pub version: CaptureOutput,
    pub usage_error: CaptureOutput,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbePlan {
    pub planner_version: String,
    pub options: Vec<PlannedOption>,
    pub budget: ProbeBudget,
    pub stop_rules: StopRules,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedOption {
    pub option: String,
    pub probes: Vec<ProbeType>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeBudget {
    pub max_total: usize,
    pub max_per_option: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StopRules {
    pub stop_on_unrecognized: bool,
    pub stop_on_binding_confirmed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProbeType {
    Existence,
    InvalidValue,
    OptionAtEnd,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SurfaceReport {
    pub invoked_path: PathBuf,
    pub binary_identity: BinaryIdentity,
    pub planner: PlannerInfo,
    pub probe_library_version: String,
    pub timings_ms: Timings,
    pub self_report: SelfReport,
    pub options: Vec<OptionSurface>,
    pub higher_tiers: HigherTierStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannerInfo {
    pub version: String,
    pub plan_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Timings {
    pub planner_ms: u128,
    pub probes_ms: u128,
    pub total_ms: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptionSurface {
    pub option: String,
    pub existence: TierResult,
    pub binding: BindingResult,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TierResult {
    pub status: ValidationStatus,
    pub reason: Option<String>,
    pub evidence: Vec<Evidence>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BindingResult {
    pub status: ValidationStatus,
    pub kind: Option<BindingKind>,
    pub reason: Option<String>,
    pub evidence: Vec<Evidence>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BindingKind {
    NoValue,
    Required,
    Optional,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationStatus {
    Confirmed,
    Refuted,
    Undetermined,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub exit_code: Option<i32>,
    pub stdout: Option<String>,
    pub stderr: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HigherTierStatus {
    pub t2: TierStatus,
    pub t3: TierStatus,
    pub t4: TierStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TierStatus {
    NotEvaluated,
}

/// Compute binary identity using a provided environment snapshot.
pub fn compute_binary_identity_with_env(path: &Path, env: EnvSnapshot) -> Result<BinaryIdentity> {
    let abs_path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let bytes = std::fs::read(&abs_path)?;
    let hash = blake3::hash(&bytes).to_hex().to_string();

    Ok(BinaryIdentity {
        path: abs_path,
        hash: Hash {
            algo: "blake3".to_string(),
            value: hash,
        },
        platform: Platform {
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
        },
        env,
    })
}
