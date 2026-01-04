//! LM planner integration for probe plan generation.

use crate::schema::{ProbeBudget, ProbePlan, ProbeType, SelfReport, StopRules};
use anyhow::{anyhow, Result};
use serde::Serialize;
use std::collections::BTreeSet;
use std::io::Write;
use std::process::{Command, Stdio};

pub const PROBE_LIBRARY_VERSION: &str = "v1";

#[derive(Debug, Serialize)]
pub struct PlannerRequest {
    pub self_report: SelfReport,
    pub options: Vec<String>,
    pub probe_library: ProbeLibrary,
    pub budget: ProbeBudget,
    pub stop_rules: StopRules,
}

#[derive(Debug, Serialize)]
pub struct ProbeLibrary {
    pub version: String,
    pub probes: Vec<ProbeDefinition>,
}

#[derive(Debug, Serialize)]
pub struct ProbeDefinition {
    pub probe: ProbeType,
    pub description: String,
}

#[derive(Debug)]
pub struct PlannerOutput {
    pub plan: ProbePlan,
    pub raw_json: String,
}

pub fn probe_library() -> ProbeLibrary {
    ProbeLibrary {
        version: PROBE_LIBRARY_VERSION.to_string(),
        probes: vec![
            ProbeDefinition {
                probe: ProbeType::Existence,
                description: "Run <opt> --help and detect unrecognized or ambiguous responses.".to_string(),
            },
            ProbeDefinition {
                probe: ProbeType::InvalidValue,
                description: "Run <opt> with a dummy value and --help to detect argument binding.".to_string(),
            },
            ProbeDefinition {
                probe: ProbeType::OptionAtEnd,
                description: "Run <opt> at end (no --help) to detect missing-arg responses.".to_string(),
            },
        ],
    }
}

pub fn run_planner(request: &PlannerRequest) -> Result<PlannerOutput> {
    if let Ok(plan_path) = std::env::var("BVM_PLANNER_PLAN") {
        let raw_json = std::fs::read_to_string(plan_path)?;
        let plan = parse_plan(&raw_json)?;
        validate_plan(&plan, request)?;
        return Ok(PlannerOutput { plan, raw_json });
    }

    let cmd = std::env::var("BVM_PLANNER_CMD")
        .map_err(|_| anyhow!("BVM_PLANNER_CMD or BVM_PLANNER_PLAN is required"))?;
    let request_json = serde_json::to_string_pretty(request)?;
    let mut child = Command::new(cmd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(request_json.as_bytes())?;
    }

    let output = child.wait_with_output()?;
    if !output.status.success() {
        return Err(anyhow!(
            "planner command failed: exit={:?} stderr={}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    if !output.stderr.is_empty() {
        return Err(anyhow!(
            "planner command emitted stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let raw_json = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if raw_json.is_empty() {
        return Err(anyhow!("planner returned empty output"));
    }

    let plan = parse_plan(&raw_json)?;
    validate_plan(&plan, request)?;
    Ok(PlannerOutput { plan, raw_json })
}

fn parse_plan(raw_json: &str) -> Result<ProbePlan> {
    let plan: ProbePlan = serde_json::from_str(raw_json)?;
    if plan.planner_version.trim().is_empty() {
        return Err(anyhow!("planner_version is required"));
    }
    Ok(plan)
}

fn validate_plan(plan: &ProbePlan, request: &PlannerRequest) -> Result<()> {
    if plan.budget.max_total != request.budget.max_total
        || plan.budget.max_per_option != request.budget.max_per_option
    {
        return Err(anyhow!("planner budget does not match requested budget"));
    }

    if plan.stop_rules.stop_on_unrecognized != request.stop_rules.stop_on_unrecognized
        || plan.stop_rules.stop_on_binding_confirmed != request.stop_rules.stop_on_binding_confirmed
    {
        return Err(anyhow!("planner stop rules do not match requested stop rules"));
    }

    let allowed: BTreeSet<String> = request.options.iter().cloned().collect();
    let mut seen = BTreeSet::new();
    let mut total_probes = 0usize;

    for planned in &plan.options {
        if !allowed.contains(&planned.option) {
            return Err(anyhow!(
                "planner returned unknown option: {}",
                planned.option
            ));
        }
        if !seen.insert(planned.option.clone()) {
            return Err(anyhow!("planner returned duplicate option: {}", planned.option));
        }
        if planned.probes.is_empty() {
            return Err(anyhow!("planner returned empty probe list for {}", planned.option));
        }
        if planned.probes[0] != ProbeType::Existence {
            return Err(anyhow!(
                "planner must start probes with existence for {}",
                planned.option
            ));
        }
        if planned.probes.len() > plan.budget.max_per_option {
            return Err(anyhow!(
                "planner exceeded per-option probe budget for {}",
                planned.option
            ));
        }
        total_probes += planned.probes.len();
    }

    if total_probes > plan.budget.max_total {
        return Err(anyhow!("planner exceeded total probe budget"));
    }

    if seen.len() != allowed.len() {
        let missing: Vec<String> = allowed
            .difference(&seen)
            .cloned()
            .collect();
        return Err(anyhow!(
            "planner did not cover all options; missing {:?}",
            missing
        ));
    }

    Ok(())
}
