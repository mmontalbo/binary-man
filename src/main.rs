//! M6 scenario runner entrypoint.

mod binary;
mod contract;
mod evidence;
mod fixture;
mod hashing;
mod limits;
mod paths;
mod runner;
mod scenario;

use anyhow::{Context, Result};
use clap::Parser;
use std::fs;
use std::path::PathBuf;

use crate::binary::{hash_binary, resolve_binary};
use crate::contract::env_contract;
use crate::evidence::{
    create_evidence_dir, write_meta, ArtifactsMeta, BinaryMeta, ErrorReport, FixtureMeta, Meta,
    Outcome, ResultMeta, SandboxMeta, TOOL_VERSION,
};
use crate::fixture::{fixture_root, prepare_fixture};
use crate::hashing::sha256_hex;
use crate::runner::{run_direct, run_sandboxed};
use crate::scenario::{validate_scenario, Scenario};

const DEFAULT_OUT_DIR: &str = "out";
const FIXTURES_DIR: &str = "fixtures";

/// CLI arguments for the scenario runner.
#[derive(Parser, Debug)]
#[command(
    name = "m6",
    version,
    about = "Run a single binary scenario in a sandbox"
)]
struct Args {
    /// Path to scenario JSON file
    scenario: PathBuf,

    /// Output directory root (evidence written under <dir>/evidence)
    #[arg(long, value_name = "DIR", default_value = DEFAULT_OUT_DIR)]
    out_dir: PathBuf,

    /// Run without bwrap (dev/debug only)
    #[arg(long)]
    direct: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();
    run(args)
}

/// Execute a single scenario and emit an evidence bundle.
fn run(args: Args) -> Result<()> {
    let env = env_contract();

    let scenario_bytes = match fs::read(&args.scenario) {
        Ok(bytes) => Some(bytes),
        Err(err) => {
            let evidence_dir = create_evidence_dir(&args.out_dir, None)?;
            write_meta(
                &evidence_dir,
                Meta {
                    tool_version: TOOL_VERSION.to_string(),
                    scenario_sha256: None,
                    scenario_id: None,
                    binary: None,
                    fixture: None,
                    env,
                    limits: None,
                    outcome: Outcome::SchemaInvalid,
                    error: Some(ErrorReport {
                        code: "schema_invalid".to_string(),
                        message: format!("failed to read scenario file: {err}"),
                        details: Vec::new(),
                    }),
                    result: None,
                    artifacts: None,
                    sandbox: None,
                },
            )?;
            return Ok(());
        }
    };

    let scenario_bytes = scenario_bytes.unwrap();
    let scenario_hash = sha256_hex(&scenario_bytes);
    let evidence_dir = create_evidence_dir(&args.out_dir, Some(&scenario_hash))?;
    fs::write(evidence_dir.join("scenario.json"), &scenario_bytes)
        .context("write scenario.json")?;

    let scenario: Scenario = match serde_json::from_slice(&scenario_bytes) {
        Ok(value) => value,
        Err(err) => {
            write_meta(
                &evidence_dir,
                Meta {
                    tool_version: TOOL_VERSION.to_string(),
                    scenario_sha256: Some(scenario_hash),
                    scenario_id: None,
                    binary: None,
                    fixture: None,
                    env,
                    limits: None,
                    outcome: Outcome::SchemaInvalid,
                    error: Some(ErrorReport {
                        code: "schema_invalid".to_string(),
                        message: "scenario JSON failed to parse".to_string(),
                        details: vec![err.to_string()],
                    }),
                    result: None,
                    artifacts: None,
                    sandbox: None,
                },
            )?;
            return Ok(());
        }
    };

    if let Some(errors) = validate_scenario(&scenario) {
        write_meta(
            &evidence_dir,
            Meta {
                tool_version: TOOL_VERSION.to_string(),
                scenario_sha256: Some(scenario_hash),
                scenario_id: Some(scenario.scenario_id.clone()),
                binary: None,
                fixture: None,
                env,
                limits: None,
                outcome: Outcome::SchemaInvalid,
                error: Some(ErrorReport {
                    code: "schema_invalid".to_string(),
                    message: "scenario validation failed".to_string(),
                    details: errors,
                }),
                result: None,
                artifacts: None,
                sandbox: None,
            },
        )?;
        return Ok(());
    }

    // Preserve argv[0] semantics while hashing the resolved target.
    let exec_binary = PathBuf::from(&scenario.binary.path);
    let resolved_binary = match resolve_binary(&exec_binary) {
        Ok(path) => path,
        Err(err) => {
            write_meta(
                &evidence_dir,
                Meta {
                    tool_version: TOOL_VERSION.to_string(),
                    scenario_sha256: Some(scenario_hash),
                    scenario_id: Some(scenario.scenario_id.clone()),
                    binary: Some(BinaryMeta {
                        path: scenario.binary.path.clone(),
                        sha256: None,
                    }),
                    fixture: None,
                    env,
                    limits: Some(scenario.limits),
                    outcome: Outcome::BinaryMissing,
                    error: Some(ErrorReport {
                        code: "binary_missing".to_string(),
                        message: format!("binary path invalid: {err}"),
                        details: Vec::new(),
                    }),
                    result: None,
                    artifacts: None,
                    sandbox: None,
                },
            )?;
            return Ok(());
        }
    };

    let binary_hash = match hash_binary(&resolved_binary) {
        Ok(hash) => hash,
        Err(err) => {
            write_meta(
                &evidence_dir,
                Meta {
                    tool_version: TOOL_VERSION.to_string(),
                    scenario_sha256: Some(scenario_hash),
                    scenario_id: Some(scenario.scenario_id.clone()),
                    binary: Some(BinaryMeta {
                        path: scenario.binary.path.clone(),
                        sha256: None,
                    }),
                    fixture: None,
                    env,
                    limits: Some(scenario.limits),
                    outcome: Outcome::BinaryMissing,
                    error: Some(ErrorReport {
                        code: "binary_missing".to_string(),
                        message: format!("failed to hash binary: {err}"),
                        details: Vec::new(),
                    }),
                    result: None,
                    artifacts: None,
                    sandbox: None,
                },
            )?;
            return Ok(());
        }
    };

    let fixtures_root = std::env::current_dir()
        .context("resolve current directory for fixtures")?
        .join(FIXTURES_DIR);
    let fixture_dir = match fixture_root(&fixtures_root, &scenario.fixture.id) {
        Ok(path) => path,
        Err(err) => {
            write_meta(
                &evidence_dir,
                Meta {
                    tool_version: TOOL_VERSION.to_string(),
                    scenario_sha256: Some(scenario_hash),
                    scenario_id: Some(scenario.scenario_id.clone()),
                    binary: Some(BinaryMeta {
                        path: scenario.binary.path.clone(),
                        sha256: Some(binary_hash.clone()),
                    }),
                    fixture: Some(FixtureMeta {
                        id: scenario.fixture.id.clone(),
                        sha256: None,
                    }),
                    env,
                    limits: Some(scenario.limits),
                    outcome: Outcome::FixtureMissing,
                    error: Some(ErrorReport {
                        code: "fixture_missing".to_string(),
                        message: format!("fixture id invalid: {err}"),
                        details: Vec::new(),
                    }),
                    result: None,
                    artifacts: None,
                    sandbox: None,
                },
            )?;
            return Ok(());
        }
    };

    let prepared_fixture = match prepare_fixture(&fixture_dir) {
        Ok(prepared) => prepared,
        Err(err) => {
            let outcome = if err.is_missing {
                Outcome::FixtureMissing
            } else {
                Outcome::FixtureInvalid
            };
            write_meta(
                &evidence_dir,
                Meta {
                    tool_version: TOOL_VERSION.to_string(),
                    scenario_sha256: Some(scenario_hash),
                    scenario_id: Some(scenario.scenario_id.clone()),
                    binary: Some(BinaryMeta {
                        path: scenario.binary.path.clone(),
                        sha256: Some(binary_hash.clone()),
                    }),
                    fixture: Some(FixtureMeta {
                        id: scenario.fixture.id.clone(),
                        sha256: None,
                    }),
                    env,
                    limits: Some(scenario.limits),
                    outcome,
                    error: Some(ErrorReport {
                        code: match outcome {
                            Outcome::FixtureMissing => "fixture_missing".to_string(),
                            _ => "fixture_invalid".to_string(),
                        },
                        message: err.message,
                        details: err.details,
                    }),
                    result: None,
                    artifacts: None,
                    sandbox: None,
                },
            )?;
            return Ok(());
        }
    };

    let run_result = if args.direct {
        run_direct(
            &exec_binary,
            &scenario.args,
            &prepared_fixture.fixture_root,
            scenario.limits,
        )
    } else {
        run_sandboxed(
            &exec_binary,
            &resolved_binary,
            &scenario.args,
            &prepared_fixture.fixture_root,
            scenario.limits,
        )
    };

    let run_result = match run_result {
        Ok(result) => result,
        Err(err) => {
            write_meta(
                &evidence_dir,
                Meta {
                    tool_version: TOOL_VERSION.to_string(),
                    scenario_sha256: Some(scenario_hash),
                    scenario_id: Some(scenario.scenario_id.clone()),
                    binary: Some(BinaryMeta {
                        path: scenario.binary.path.clone(),
                        sha256: Some(binary_hash.clone()),
                    }),
                    fixture: Some(FixtureMeta {
                        id: scenario.fixture.id.clone(),
                        sha256: Some(prepared_fixture.fixture_hash.clone()),
                    }),
                    env,
                    limits: Some(scenario.limits),
                    outcome: Outcome::SandboxFailed,
                    error: Some(error_report("sandbox_failed", &err)),
                    result: None,
                    artifacts: None,
                    sandbox: Some(SandboxMeta {
                        mode: if args.direct {
                            "direct".to_string()
                        } else {
                            "bwrap".to_string()
                        },
                    }),
                },
            )?;
            return Ok(());
        }
    };

    let stdout_hash = sha256_hex(&run_result.stdout);
    let stderr_hash = sha256_hex(&run_result.stderr);
    if scenario.artifacts.capture_stdout {
        fs::write(evidence_dir.join("stdout.txt"), &run_result.stdout)
            .context("write stdout.txt")?;
    }
    if scenario.artifacts.capture_stderr {
        fs::write(evidence_dir.join("stderr.txt"), &run_result.stderr)
            .context("write stderr.txt")?;
    }

    let outcome = if run_result.timed_out {
        Outcome::TimedOut
    } else {
        Outcome::Exited
    };

    let meta = Meta {
        tool_version: TOOL_VERSION.to_string(),
        scenario_sha256: Some(scenario_hash),
        scenario_id: Some(scenario.scenario_id.clone()),
        binary: Some(BinaryMeta {
            path: scenario.binary.path.clone(),
            sha256: Some(binary_hash),
        }),
        fixture: Some(FixtureMeta {
            id: scenario.fixture.id.clone(),
            sha256: Some(prepared_fixture.fixture_hash),
        }),
        env,
        limits: Some(scenario.limits),
        outcome,
        error: None,
        result: Some(ResultMeta {
            exit_code: run_result.exit_code,
            timed_out: run_result.timed_out,
            wall_time_ms: run_result.wall_time_ms,
        }),
        artifacts: Some(ArtifactsMeta {
            stdout_sha256: stdout_hash,
            stderr_sha256: stderr_hash,
            stdout_bytes: run_result.stdout.len() as u64,
            stderr_bytes: run_result.stderr.len() as u64,
        }),
        sandbox: Some(SandboxMeta {
            mode: if args.direct {
                "direct".to_string()
            } else {
                "bwrap".to_string()
            },
        }),
    };

    write_meta(&evidence_dir, meta)?;
    println!("evidence: {}", evidence_dir.display());
    Ok(())
}

fn error_report(code: &str, err: &anyhow::Error) -> ErrorReport {
    let details = err.chain().skip(1).map(|cause| cause.to_string()).collect();
    ErrorReport {
        code: code.to_string(),
        message: err.to_string(),
        details,
    }
}
