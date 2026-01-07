//! LM prompt assembly and Claude CLI invocation.

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::runner::run_direct;
use crate::scenario::ScenarioLimits;

const HELP_LIMITS: ScenarioLimits = ScenarioLimits {
    wall_time_ms: 2000,
    cpu_time_ms: 1000,
    memory_kb: 65536,
    file_size_kb: 1024,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LmCommandConfig {
    command: Vec<String>,
}

pub(crate) struct LmCommand {
    pub(crate) argv: Vec<String>,
}

pub(crate) struct HelpCapture {
    pub(crate) bytes: Vec<u8>,
    pub(crate) source: &'static str,
    pub(crate) flag: &'static str,
}

/// Capture help text for a binary using `--help`, falling back to `-h`.
pub(crate) fn capture_help(binary: &Path) -> Result<HelpCapture> {
    let cwd = std::env::current_dir().context("resolve cwd for help")?;
    let output = capture_help_with_arg(binary, "--help", &cwd)?;
    if !output.bytes.is_empty() {
        return Ok(output);
    }
    capture_help_with_arg(binary, "-h", &cwd)
}

/// Load the LM command configuration, falling back to Claude defaults.
pub(crate) fn load_lm_command() -> Result<LmCommand> {
    if let Ok(raw) = env::var("BMAN_LM_COMMAND") {
        let argv = parse_command_config(&raw)
            .context("parse BMAN_LM_COMMAND")?;
        return Ok(LmCommand { argv });
    }
    Ok(default_lm_command())
}

fn parse_command_config(raw: &str) -> Result<Vec<String>> {
    let config: LmCommandConfig =
        serde_json::from_str(raw).context("parse LM command JSON")?;
    if config.command.is_empty() {
        return Err(anyhow!("LM command is empty"));
    }
    Ok(config.command)
}

fn default_lm_command() -> LmCommand {
    LmCommand {
        argv: vec![
            "claude".to_string(),
            "--print".to_string(),
            "--output-format".to_string(),
            "json".to_string(),
            "--json-schema".to_string(),
            "{schema}".to_string(),
            "--no-session-persistence".to_string(),
            "--system-prompt".to_string(),
            "Return a single JSON object only. No prose or code fences.".to_string(),
            "--tools".to_string(),
            "".to_string(),
        ],
    }
}

fn capture_help_with_arg(binary: &Path, flag: &'static str, cwd: &Path) -> Result<HelpCapture> {
    let args = vec![flag.to_string()];
    let result = run_direct(binary, &args, cwd, HELP_LIMITS).context("run help command")?;
    if result.timed_out {
        return Err(anyhow!("help command timed out"));
    }
    if !result.stdout.is_empty() {
        return Ok(HelpCapture {
            bytes: result.stdout,
            source: "stdout",
            flag,
        });
    }
    Ok(HelpCapture {
        bytes: result.stderr,
        source: "stderr",
        flag,
    })
}

/// Load a UTF-8 file into a string for prompt assembly.
pub(crate) fn load_text(path: &Path) -> Result<String> {
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Build the LM prompt for scenario generation.
pub(crate) fn build_prompt(
    binary_path: &Path,
    help_text: &str,
    schema_text: &str,
    catalog_text: &str,
    example_text: Option<&str>,
) -> String {
    let mut prompt = String::new();
    prompt.push_str("Return a single JSON object that conforms to the schema below.\n");
    prompt.push_str(
        "Output JSON only. No prose, no code fences, and no markdown.\n",
    );
    prompt.push_str("Begin with '{' and end with '}'.\n\n");
    prompt.push_str("Target binary path (must match exactly):\n");
    prompt.push_str(&format!("{}\n\n", binary_path.display()));
    prompt.push_str("Fixture catalog (allowed fixture.id values):\n");
    prompt.push_str(catalog_text);
    prompt.push_str("\n\nSchema:\n");
    prompt.push_str(schema_text);
    if let Some(example) = example_text {
        prompt.push_str("\n\nExample (format only; replace values as needed):\n");
        prompt.push_str(example);
    }
    prompt.push_str("\n\nRaw help text:\n");
    prompt.push_str(help_text);
    prompt.push_str(
        "\n\nConstraints:\n\
 - Use the target binary path verbatim in binary.path.\n\
 - args must be an array of strings (no shell parsing).\n\
 - rationale must be short, plain text.\n\
 - fixture.id must come from the fixture catalog.\n\
 - artifacts.capture_exit_code must be true.\n\
 - limits must be present and within schema bounds.\n",
    );
    prompt
}

/// Invoke Claude CLI to obtain a scenario JSON response.
pub(crate) fn run_lm(prompt: &str, schema: &str, command: &LmCommand) -> Result<Vec<u8>> {
    if command.argv.is_empty() {
        return Err(anyhow!("LM command is empty"));
    }
    let mut argv = command.argv.clone();
    let mut has_placeholder = false;
    for arg in &mut argv {
        if arg == "{prompt}" {
            *arg = prompt.to_string();
            has_placeholder = true;
        }
        if arg == "{schema}" {
            *arg = schema.to_string();
        }
    }
    let program = argv.remove(0);
    let mut command = Command::new(program);
    command.args(argv);
    if has_placeholder {
        command.stdin(Stdio::null());
    } else {
        command.stdin(Stdio::piped());
    }
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let output = if has_placeholder {
        command.output().context("run LM command")?
    } else {
        let mut child = command.spawn().context("spawn LM command")?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(prompt.as_bytes())
                .context("write LM prompt")?;
        }
        child.wait_with_output().context("wait LM output")?
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("LM command failed: {}", stderr.trim()));
    }
    Ok(output.stdout)
}

/// Resolve paths for prompt assets.
pub(crate) fn scenario_schema_path(root: &Path) -> PathBuf {
    root.join("schema").join("scenario.v0.json")
}

pub(crate) fn lm_schema_path(root: &Path) -> PathBuf {
    root.join("schema").join("scenario.lm.json")
}

pub(crate) fn fixture_catalog_path(root: &Path) -> PathBuf {
    root.join("fixtures").join("catalog.json")
}

pub(crate) fn example_scenario_path(root: &Path) -> PathBuf {
    root.join("scenarios").join("examples").join("ls_help.json")
}
