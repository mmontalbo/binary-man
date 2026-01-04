//! Fast binary-only surface extraction (T0/T1).

use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

mod help;
mod planner;
mod schema;
mod validate;

use crate::help::{extract_help_options, BindingHint, HelpOption};
use crate::planner::{probe_library, run_planner, PlannerRequest, PROBE_LIBRARY_VERSION};
use crate::schema::{
    compute_binary_identity_with_env, BindingKind, BindingResult, CaptureOutput, EnvSnapshot,
    HigherTierStatus, OptionSurface, ProbeBudget, ProbePlan, SelfReport, SurfaceReport, TierResult,
    TierStatus, Timings,
};
use crate::validate::{
    infer_binding, run_existence_probe, run_invalid_value_probe, run_option_at_end_probe,
    validation_env, ProbeRun,
};

const DEFAULT_OUT_DIR: &str = "out";
const USAGE_ERROR_ARG: &str = "--__bvm_unknown__";

#[derive(Parser, Debug)]
#[command(name = "bvm", version, about = "Binary-validated surface extractor")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Fast T0/T1 surface extraction
    Surface(SurfaceArgs),
}

#[derive(Parser, Debug)]
struct SurfaceArgs {
    /// Path to the binary under test
    binary: PathBuf,

    /// Output directory for cached artifacts
    #[arg(long, value_name = "DIR", default_value = DEFAULT_OUT_DIR)]
    out_dir: PathBuf,

    /// Max probes per option (default: 3)
    #[arg(long, default_value_t = 3)]
    max_per_option: usize,

    /// Max total probes (default: options.len() * max_per_option)
    #[arg(long)]
    max_total: Option<usize>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Surface(args) => cmd_surface(args),
    }
}

fn cmd_surface(args: SurfaceArgs) -> Result<()> {
    let started = Instant::now();
    let env = validation_env();
    let binary_identity = compute_binary_identity_with_env(&args.binary, env.clone())?;

    let self_report = capture_self_report(&args.binary, &env)?;
    let help_text = select_help_text(&self_report.help).ok_or_else(|| {
        anyhow!(
            "--help capture produced no output: exit={:?}",
            self_report.help.exit_code
        )
    })?;

    let help_options = extract_help_options(help_text);
    if help_options.is_empty() {
        return Err(anyhow!("no options detected in help output"));
    }

    let option_names: Vec<String> = help_options
        .iter()
        .map(|opt| opt.option.clone())
        .collect();

    let max_total = args
        .max_total
        .unwrap_or_else(|| option_names.len() * args.max_per_option);

    let budget = ProbeBudget {
        max_total,
        max_per_option: args.max_per_option,
    };
    let stop_rules = schema::StopRules {
        stop_on_unrecognized: true,
        stop_on_binding_confirmed: true,
    };

    let planner_request = PlannerRequest {
        self_report: self_report.clone(),
        options: option_names.clone(),
        probe_library: probe_library(),
        budget,
        stop_rules,
    };

    let planner_start = Instant::now();
    let planner_output = run_planner(&planner_request)?;
    let planner_ms = planner_start.elapsed().as_millis();

    let plan = planner_output.plan;
    let plan_hash = hash_text(&planner_output.raw_json);

    let cache_key = cache_key(
        &binary_identity,
        &plan.planner_version,
        PROBE_LIBRARY_VERSION,
    );
    let cache_dir = args.out_dir.join("surface").join(cache_key);
    std::fs::create_dir_all(&cache_dir)?;

    let plan_path = cache_dir.join("plan.json");
    let request_path = cache_dir.join("planner_request.json");
    let surface_path = cache_dir.join("surface.json");
    let view_path = cache_dir.join("surface.md");

    write_json(&plan_path, &plan)?;
    write_json(&request_path, &planner_request)?;

    if surface_path.exists() && view_path.exists() {
        println!("cache hit: {}", cache_dir.display());
        println!("surface: {}", surface_path.display());
        println!("view: {}", view_path.display());
        return Ok(());
    }

    let probe_start = Instant::now();
    let options = execute_plan(&args.binary, &env, &plan, &help_options)?;
    let probes_ms = probe_start.elapsed().as_millis();

    let total_ms = started.elapsed().as_millis();

    let report = SurfaceReport {
        invoked_path: args.binary.clone(),
        binary_identity,
        planner: schema::PlannerInfo {
            version: plan.planner_version.clone(),
            plan_hash,
        },
        probe_library_version: PROBE_LIBRARY_VERSION.to_string(),
        timings_ms: Timings {
            planner_ms,
            probes_ms,
            total_ms,
        },
        self_report,
        options,
        higher_tiers: HigherTierStatus {
            t2: TierStatus::NotEvaluated,
            t3: TierStatus::NotEvaluated,
            t4: TierStatus::NotEvaluated,
        },
    };

    write_json(&surface_path, &report)?;
    let view = render_markdown(&report);
    std::fs::write(&view_path, view)?;

    println!("surface: {}", surface_path.display());
    println!("view: {}", view_path.display());

    Ok(())
}

fn execute_plan(
    binary: &Path,
    env: &EnvSnapshot,
    plan: &ProbePlan,
    help_options: &[HelpOption],
) -> Result<Vec<OptionSurface>> {
    let mut hint_map: BTreeMap<String, Option<BindingHint>> = BTreeMap::new();
    for opt in help_options {
        hint_map.insert(opt.option.clone(), opt.binding);
    }

    let mut surfaces = Vec::new();
    for planned in &plan.options {
        let option = planned.option.clone();
        let binding_hint = hint_map.get(&option).copied().flatten();
        let form_hint = binding_hint.map(|hint| hint.form);

        let mut existence: Option<(schema::ValidationStatus, Option<String>, ProbeRun)> = None;
        let mut invalid_value: Option<ProbeRun> = None;
        let mut option_at_end: Option<ProbeRun> = None;

        for probe in &planned.probes {
            match probe {
                schema::ProbeType::Existence => {
                    existence = Some(run_existence_probe(binary, &option, env));
                }
                schema::ProbeType::InvalidValue => {
                    invalid_value = Some(run_invalid_value_probe(binary, &option, env, form_hint));
                }
                schema::ProbeType::OptionAtEnd => {
                    option_at_end = Some(run_option_at_end_probe(binary, &option, env));
                }
            }

            if plan.stop_rules.stop_on_unrecognized {
                if let Some((status, _, _)) = &existence {
                    if matches!(status, schema::ValidationStatus::Refuted) {
                        break;
                    }
                }
            }

            if plan.stop_rules.stop_on_binding_confirmed {
                if let (Some((exist_status, _, exist_run)), Some(invalid_run)) =
                    (&existence, &invalid_value)
                {
                    if matches!(exist_status, schema::ValidationStatus::Confirmed) {
                        let (status, kind, _) = infer_binding(
                            &exist_run.analysis,
                            &invalid_run.analysis,
                            option_at_end.as_ref().map(|run| &run.analysis),
                            form_hint,
                        );
                        if matches!(status, schema::ValidationStatus::Confirmed) && kind.is_some() {
                            break;
                        }
                    }
                }
            }
        }

        let exist_status = existence
            .as_ref()
            .map(|(status, _, _)| status.clone())
            .unwrap_or(schema::ValidationStatus::Undetermined);
        let exist_reason = match existence.as_ref() {
            Some((_, reason, _)) => reason.clone(),
            None => Some("existence probe not run".to_string()),
        };
        let exist_evidence = existence
            .as_ref()
            .map(|(_, _, run)| vec![run.evidence.clone()])
            .unwrap_or_default();
        let exist_analysis = existence.as_ref().map(|(_, _, run)| &run.analysis);

        let existence_result = TierResult {
            status: exist_status.clone(),
            reason: exist_reason,
            evidence: exist_evidence,
        };

        let mut binding_evidence = Vec::new();
        if let Some((_, _, run)) = &existence {
            binding_evidence.push(run.evidence.clone());
        }
        if let Some(run) = &invalid_value {
            binding_evidence.push(run.evidence.clone());
        }
        if let Some(run) = &option_at_end {
            binding_evidence.push(run.evidence.clone());
        }

        let (binding_status, binding_kind, binding_reason) = if !matches!(
            exist_status,
            schema::ValidationStatus::Confirmed
        ) {
            (
                schema::ValidationStatus::Undetermined,
                None,
                Some("option existence not confirmed".to_string()),
            )
        } else if let (Some(missing), Some(invalid)) = (exist_analysis, invalid_value.as_ref()) {
            infer_binding(
                missing,
                &invalid.analysis,
                option_at_end.as_ref().map(|run| &run.analysis),
                form_hint,
            )
        } else {
            (
                schema::ValidationStatus::Undetermined,
                None,
                Some("invalid-value probe not run".to_string()),
            )
        };

        let binding_result = BindingResult {
            status: binding_status,
            kind: binding_kind,
            reason: binding_reason,
            evidence: binding_evidence,
        };

        surfaces.push(OptionSurface {
            option,
            existence: existence_result,
            binding: binding_result,
        });
    }

    Ok(surfaces)
}

fn render_markdown(report: &SurfaceReport) -> String {
    let mut out = String::new();
    let name = report
        .invoked_path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .or_else(|| {
            report
                .binary_identity
                .path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
        })
        .unwrap_or_else(|| "binary".to_string());

    push_line(&mut out, &format!("# {} surface", name));
    push_line(&mut out, "");
    push_line(&mut out, "## Binary Identity");
    push_line(
        &mut out,
        &format!("- Invoked Path: {}", report.invoked_path.display()),
    );
    push_line(
        &mut out,
        &format!("- Resolved Path: {}", report.binary_identity.path.display()),
    );
    push_line(
        &mut out,
        &format!(
            "- Hash: {}:{}",
            report.binary_identity.hash.algo, report.binary_identity.hash.value
        ),
    );
    push_line(
        &mut out,
        &format!(
            "- Platform: {}/{}",
            report.binary_identity.platform.os, report.binary_identity.platform.arch
        ),
    );
    push_line(
        &mut out,
        &format!(
            "- Environment: LC_ALL={} TZ={} TERM={}",
            report.binary_identity.env.locale,
            report.binary_identity.env.tz,
            report.binary_identity.env.term
        ),
    );
    if let Some(version) = first_nonempty_line(&report.self_report.version) {
        push_line(&mut out, &format!("- Version: {}", version));
    }
    push_line(&mut out, "");
    push_line(&mut out, "## T0 Option Existence");
    render_status_list(&mut out, &report.options, |opt| &opt.existence);
    push_line(&mut out, "");
    push_line(&mut out, "## T1 Parameter Binding");
    render_binding_list(&mut out, &report.options);
    push_line(&mut out, "");
    push_line(&mut out, "## Higher Tiers");
    push_line(&mut out, "- T2 parameter form: not evaluated");
    push_line(&mut out, "- T3 parameter domain/type: not evaluated");
    push_line(&mut out, "- T4 behavior semantics: not evaluated");

    out
}

fn render_status_list(
    out: &mut String,
    options: &[OptionSurface],
    selector: impl Fn(&OptionSurface) -> &TierResult,
) {
    for opt in options {
        let result = selector(opt);
        let label = format!("{} ({})", opt.option, status_label(&result.status));
        if let Some(reason) = &result.reason {
            push_line(out, &format!("- {} — {}", label, reason));
        } else {
            push_line(out, &format!("- {}", label));
        }
    }
}

fn render_binding_list(out: &mut String, options: &[OptionSurface]) {
    for opt in options {
        let binding = &opt.binding;
        let kind = binding
            .kind
            .as_ref()
            .map(|kind| binding_label(kind));
        let label = match kind {
            Some(kind) => format!("{} ({}, {})", opt.option, kind, status_label(&binding.status)),
            None => format!("{} ({})", opt.option, status_label(&binding.status)),
        };
        if let Some(reason) = &binding.reason {
            push_line(out, &format!("- {} — {}", label, reason));
        } else {
            push_line(out, &format!("- {}", label));
        }
    }
}

fn binding_label(kind: &BindingKind) -> &'static str {
    match kind {
        BindingKind::NoValue => "no_value",
        BindingKind::Required => "required",
        BindingKind::Optional => "optional",
    }
}

fn status_label(status: &schema::ValidationStatus) -> &'static str {
    match status {
        schema::ValidationStatus::Confirmed => "confirmed",
        schema::ValidationStatus::Refuted => "refuted",
        schema::ValidationStatus::Undetermined => "undetermined",
    }
}

fn push_line(out: &mut String, line: &str) {
    out.push_str(line);
    out.push('\n');
}

fn first_nonempty_line(capture: &CaptureOutput) -> Option<String> {
    capture
        .stdout
        .lines()
        .find(|line| !line.trim().is_empty())
        .or_else(|| capture.stderr.lines().find(|line| !line.trim().is_empty()))
        .map(|line| line.trim().to_string())
}

fn capture_self_report(binary: &Path, env: &EnvSnapshot) -> Result<SelfReport> {
    let help = capture_help(binary, env)?;
    let version = capture_output(binary, &["--version"], env)?;
    let usage_error = capture_output(binary, &[USAGE_ERROR_ARG], env)?;
    Ok(SelfReport {
        help,
        version,
        usage_error,
    })
}

fn capture_help(binary: &Path, env: &EnvSnapshot) -> Result<CaptureOutput> {
    let primary = capture_output(binary, &["--help"], env)?;
    if select_help_text(&primary).is_some() {
        return Ok(primary);
    }
    let fallback = capture_output(binary, &["-h"], env)?;
    if select_help_text(&fallback).is_some() {
        return Ok(fallback);
    }
    Ok(primary)
}

fn select_help_text(capture: &CaptureOutput) -> Option<&str> {
    if !capture.stdout.trim().is_empty() {
        Some(capture.stdout.as_str())
    } else if !capture.stderr.trim().is_empty() {
        Some(capture.stderr.as_str())
    } else {
        None
    }
}

fn capture_output(binary: &Path, args: &[&str], env: &EnvSnapshot) -> Result<CaptureOutput> {
    let output = Command::new(binary)
        .args(args)
        .env_clear()
        .env("LC_ALL", &env.locale)
        .env("TZ", &env.tz)
        .env("TERM", &env.term)
        .output()?;

    Ok(CaptureOutput {
        args: args.iter().map(|arg| arg.to_string()).collect(),
        exit_code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

fn cache_key(identity: &schema::BinaryIdentity, planner_version: &str, probe_version: &str) -> String {
    let material = format!(
        "hash={};os={};arch={};lc_all={};tz={};term={};planner={};probe_lib={}",
        identity.hash.value,
        identity.platform.os,
        identity.platform.arch,
        identity.env.locale,
        identity.env.tz,
        identity.env.term,
        planner_version,
        probe_version
    );
    hash_text(&material)
}

fn hash_text(input: &str) -> String {
    blake3::hash(input.as_bytes()).to_hex().to_string()
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let json = serde_json::to_string_pretty(value)?;
    std::fs::write(path, json)?;
    Ok(())
}
