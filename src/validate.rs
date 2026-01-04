//! Probe execution and analysis for T0/T1 surface validation.

use crate::help::ValueForm;
use crate::schema::{BindingKind, EnvSnapshot, Evidence, ValidationStatus};
use regex::Regex;
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

const UNRECOGNIZED_MARKERS: &[&str] = &[
    "unrecognized option",
    "unknown option",
    "invalid option",
    "illegal option",
    "unknown flag",
    "unrecognized flag",
    "invalid flag",
    "unknown switch",
    "invalid switch",
];

const AMBIGUOUS_MARKERS: &[&str] = &["ambiguous option", "option is ambiguous"];

const ARGUMENT_ERROR_MARKERS: &[&str] = &[
    "requires an argument",
    "requires a value",
    "option requires an argument",
    "option requires a value",
    "missing argument",
    "missing value",
    "invalid argument",
];

const MISSING_ARGUMENT_MARKERS: &[&str] = &[
    "requires an argument",
    "requires a value",
    "option requires an argument",
    "option requires a value",
    "missing argument",
    "missing value",
];

const ARGUMENT_NOT_ALLOWED_MARKERS: &[&str] = &[
    "doesn't allow an argument",
    "does not allow an argument",
    "doesn't allow a value",
    "does not allow a value",
    "doesn't take an argument",
    "does not take an argument",
    "doesn't take a value",
    "does not take a value",
    "doesn't accept an argument",
    "does not accept an argument",
    "takes no argument",
    "takes no value",
    "argument not allowed",
    "value not allowed",
];

const INVALID_ARGUMENT_MARKERS: &[&str] = &["invalid argument", "invalid value"];

const BINDING_DUMMY_VALUE: &str = "__bvm__";

struct ExecutionAttempt {
    args: Vec<String>,
    exit_code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    spawn_error: Option<String>,
}

#[derive(Debug)]
pub struct ProbeRun {
    pub analysis: AttemptAnalysis,
    pub evidence: Evidence,
}

#[derive(Debug)]
pub struct AttemptAnalysis {
    pub unrecognized: bool,
    pub missing_arg: bool,
    pub arg_not_allowed: bool,
    pub invalid_arg: bool,
    pub ambiguous: bool,
    pub help_like: bool,
    pub argument_error: bool,
    pub exit_code: Option<i32>,
    pub notes: Vec<String>,
}

/// Default validation environment (`LC_ALL=C`, `TZ=UTC`, `TERM=dumb`).
pub fn validation_env() -> EnvSnapshot {
    EnvSnapshot {
        locale: "C".to_string(),
        tz: "UTC".to_string(),
        term: "dumb".to_string(),
    }
}

pub fn run_existence_probe(
    binary: &Path,
    option: &str,
    env: &EnvSnapshot,
) -> (ValidationStatus, Option<String>, ProbeRun) {
    let args = vec![option.to_string(), "--help".to_string()];
    let attempt = run_attempt(binary, args, env);
    let analysis = attempt_signals(option, &attempt);
    let (status, reason) = classify_attempt(&attempt, &analysis);
    let evidence = evidence_for_attempt(attempt, env, build_attempt_notes("existence", &analysis, None));
    (
        status,
        reason,
        ProbeRun {
            analysis,
            evidence,
        },
    )
}

pub fn run_invalid_value_probe(
    binary: &Path,
    option: &str,
    env: &EnvSnapshot,
    form: Option<ValueForm>,
) -> ProbeRun {
    let mut args = build_with_value_args(form, option, BINDING_DUMMY_VALUE);
    args.push("--help".to_string());
    let attempt = run_attempt(binary, args, env);
    let analysis = attempt_signals(option, &attempt);
    let form_note = form.map(|form| match form {
        ValueForm::Attached => "value_form=attached",
        ValueForm::Trailing => "value_form=trailing",
    });
    let evidence = evidence_for_attempt(attempt, env, build_attempt_notes("invalid_value", &analysis, form_note));
    ProbeRun { analysis, evidence }
}

pub fn run_option_at_end_probe(binary: &Path, option: &str, env: &EnvSnapshot) -> ProbeRun {
    let args = vec![option.to_string()];
    let attempt = run_attempt(binary, args, env);
    let analysis = attempt_signals(option, &attempt);
    let evidence = evidence_for_attempt(attempt, env, build_attempt_notes("option_at_end", &analysis, None));
    ProbeRun { analysis, evidence }
}

pub fn infer_binding(
    missing: &AttemptAnalysis,
    invalid: &AttemptAnalysis,
    option_at_end: Option<&AttemptAnalysis>,
    value_form: Option<ValueForm>,
) -> (ValidationStatus, Option<BindingKind>, Option<String>) {
    if missing.unrecognized || invalid.unrecognized {
        return (
            ValidationStatus::Undetermined,
            None,
            Some("unrecognized option response".to_string()),
        );
    }

    if missing.ambiguous || invalid.ambiguous {
        return (
            ValidationStatus::Undetermined,
            None,
            Some("ambiguous option response".to_string()),
        );
    }

    if missing.missing_arg {
        return (
            ValidationStatus::Confirmed,
            Some(BindingKind::Required),
            Some("missing argument response observed".to_string()),
        );
    }

    if missing.invalid_arg {
        return (
            ValidationStatus::Confirmed,
            Some(BindingKind::Required),
            Some("invalid argument response observed for missing probe".to_string()),
        );
    }

    if invalid.arg_not_allowed {
        return (
            ValidationStatus::Confirmed,
            Some(BindingKind::NoValue),
            Some("argument not allowed response observed".to_string()),
        );
    }

    let help_consumed = !missing.help_like && invalid.help_like;

    if invalid.invalid_arg {
        if help_consumed {
            return (
                ValidationStatus::Confirmed,
                Some(BindingKind::Required),
                Some("missing probe likely consumed --help; invalid argument observed".to_string()),
            );
        }
        return (
            ValidationStatus::Confirmed,
            Some(BindingKind::Optional),
            Some("invalid argument response observed".to_string()),
        );
    }

    if missing.exit_code == Some(0)
        && invalid.exit_code == Some(0)
        && !missing.argument_error
        && !invalid.argument_error
    {
        if matches!(value_form, Some(ValueForm::Attached)) {
            return (
                ValidationStatus::Confirmed,
                Some(BindingKind::Optional),
                Some("no argument errors detected with attached value".to_string()),
            );
        }
        return (
            ValidationStatus::Undetermined,
            None,
            Some("no argument errors detected with trailing value".to_string()),
        );
    }

    if let Some(end) = option_at_end {
        if end.missing_arg && end.exit_code.unwrap_or(0) != 0 {
            return (
                ValidationStatus::Confirmed,
                Some(BindingKind::Required),
                Some("missing argument response observed (option at end probe)".to_string()),
            );
        }
    }

    (
        ValidationStatus::Undetermined,
        None,
        Some("insufficient binding evidence".to_string()),
    )
}

fn classify_attempt(
    attempt: &ExecutionAttempt,
    analysis: &AttemptAnalysis,
) -> (ValidationStatus, Option<String>) {
    if let Some(err) = &attempt.spawn_error {
        return (
            ValidationStatus::Undetermined,
            Some(format!("spawn failed: {err}")),
        );
    }

    let Some(exit_code) = attempt.exit_code else {
        return (
            ValidationStatus::Undetermined,
            Some("terminated without exit code".to_string()),
        );
    };

    if analysis.unrecognized {
        return (ValidationStatus::Refuted, None);
    }
    if let Some(note) = find_note_with_prefix(&analysis.notes, "unrecognized option marker") {
        return (ValidationStatus::Undetermined, Some(note));
    }
    if analysis.ambiguous {
        let note = find_note_with_prefix(&analysis.notes, "ambiguous option response")
            .or_else(|| Some("ambiguous option response".to_string()));
        return (ValidationStatus::Undetermined, note);
    }

    let mut notes = Vec::new();
    if exit_code != 0 {
        notes.push(format!(
            "nonzero exit ({exit_code}) without unrecognized option marker"
        ));
    }
    if analysis.argument_error {
        notes.push("argument error reported".to_string());
    }

    let note = if notes.is_empty() {
        None
    } else {
        Some(notes.join("; "))
    };

    (ValidationStatus::Confirmed, note)
}

fn find_note_with_prefix(notes: &[String], prefix: &str) -> Option<String> {
    notes.iter().find(|note| note.starts_with(prefix)).cloned()
}

fn build_attempt_notes(
    probe: &str,
    analysis: &AttemptAnalysis,
    extra: Option<&str>,
) -> Option<String> {
    let mut parts = Vec::new();
    parts.push(format!("probe={probe}"));
    if let Some(extra) = extra {
        parts.push(extra.to_string());
    }
    parts.extend(analysis.notes.iter().cloned());
    Some(parts.join("; "))
}

fn build_with_value_args(form: Option<ValueForm>, option: &str, value: &str) -> Vec<String> {
    if let Some(form) = form {
        match form {
            ValueForm::Attached => {
                return vec![format!("{option}={value}")];
            }
            ValueForm::Trailing => {
                return vec![option.to_string(), value.to_string()];
            }
        }
    }
    if option.starts_with("--") {
        vec![format!("{option}={value}")]
    } else {
        vec![option.to_string(), value.to_string()]
    }
}

fn run_attempt(binary: &Path, args: Vec<String>, env: &EnvSnapshot) -> ExecutionAttempt {
    let output = Command::new(binary)
        .args(&args)
        .env_clear()
        .env("LC_ALL", &env.locale)
        .env("TZ", &env.tz)
        .env("TERM", &env.term)
        .output();

    match output {
        Ok(output) => ExecutionAttempt {
            args,
            exit_code: output.status.code(),
            stdout: output.stdout,
            stderr: output.stderr,
            spawn_error: None,
        },
        Err(err) => ExecutionAttempt {
            args,
            exit_code: None,
            stdout: Vec::new(),
            stderr: Vec::new(),
            spawn_error: Some(err.to_string()),
        },
    }
}

fn attempt_signals(option: &str, attempt: &ExecutionAttempt) -> AttemptAnalysis {
    let mut notes = Vec::new();
    if let Some(err) = &attempt.spawn_error {
        notes.push(format!("spawn failed: {err}"));
        return AttemptAnalysis {
            unrecognized: false,
            missing_arg: false,
            arg_not_allowed: false,
            invalid_arg: false,
            ambiguous: false,
            help_like: false,
            argument_error: false,
            exit_code: attempt.exit_code,
            notes,
        };
    }

    if attempt.exit_code.is_none() {
        notes.push("terminated without exit code".to_string());
    }

    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&attempt.stdout),
        String::from_utf8_lossy(&attempt.stderr)
    );
    let output_lower = output.to_lowercase();
    let help_like = is_help_like_output(&output_lower);
    let argument_error = contains_any(&output_lower, ARGUMENT_ERROR_MARKERS);

    let mut unrecognized = false;
    if let Some(marker) = find_marker(&output_lower, UNRECOGNIZED_MARKERS) {
        let reported = extract_reported_options(&output);
        if reported
            .iter()
            .any(|reported| option_matches(reported, option))
        {
            unrecognized = true;
        } else if reported.is_empty() {
            notes.push(format!(
                "unrecognized option marker ({marker}) without option attribution"
            ));
        } else {
            notes.push(format!(
                "unrecognized option marker ({marker}) for {:?}",
                reported
            ));
        }
    }

    let missing_options = extract_missing_argument_options(&output);
    let mut missing_arg = missing_options
        .iter()
        .any(|reported| option_matches(reported, option));
    if !missing_options.is_empty() && !missing_arg {
        notes.push(format!("missing argument marker for {:?}", missing_options));
    } else if missing_options.is_empty() && contains_any(&output_lower, MISSING_ARGUMENT_MARKERS) {
        notes.push(
            "missing argument marker without option attribution; attributed to tested option"
                .to_string(),
        );
        missing_arg = true;
    }

    let not_allowed_options = extract_argument_not_allowed_options(&output);
    let arg_not_allowed = not_allowed_options
        .iter()
        .any(|reported| option_matches(reported, option));
    if !not_allowed_options.is_empty() && !arg_not_allowed {
        notes.push(format!(
            "argument not allowed marker for {:?}",
            not_allowed_options
        ));
    } else if not_allowed_options.is_empty()
        && contains_any(&output_lower, ARGUMENT_NOT_ALLOWED_MARKERS)
    {
        notes.push("argument not allowed marker without option attribution".to_string());
    }

    let invalid_options = extract_invalid_argument_options(&output);
    let mut invalid_arg = invalid_options
        .iter()
        .any(|reported| option_matches(reported, option));
    if !invalid_options.is_empty() && !invalid_arg {
        notes.push(format!("invalid argument marker for {:?}", invalid_options));
    }
    if !invalid_arg {
        if infer_invalid_argument_for_option(option, &output_lower) {
            invalid_arg = true;
            notes.push(
                "invalid argument marker without option attribution; attributed to tested option"
                    .to_string(),
            );
        } else if invalid_options.is_empty()
            && contains_any(&output_lower, INVALID_ARGUMENT_MARKERS)
        {
            notes.push("invalid argument marker without option attribution".to_string());
        }
    }

    let mut ambiguous = false;
    if let Some(marker) = find_marker(&output_lower, AMBIGUOUS_MARKERS) {
        ambiguous = true;
        notes.push(format!("ambiguous option response ({marker})"));
    }

    AttemptAnalysis {
        unrecognized,
        missing_arg,
        arg_not_allowed,
        invalid_arg,
        ambiguous,
        help_like,
        argument_error,
        exit_code: attempt.exit_code,
        notes,
    }
}

fn env_map(env: &EnvSnapshot) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    map.insert("LC_ALL".to_string(), env.locale.clone());
    map.insert("TZ".to_string(), env.tz.clone());
    map.insert("TERM".to_string(), env.term.clone());
    map
}

fn hash_bytes(bytes: &[u8]) -> String {
    format!("blake3:{}", blake3::hash(bytes).to_hex())
}

fn evidence_for_attempt(
    attempt: ExecutionAttempt,
    env: &EnvSnapshot,
    notes: Option<String>,
) -> Evidence {
    Evidence {
        args: attempt.args,
        env: env_map(env),
        exit_code: attempt.exit_code,
        stdout: Some(hash_bytes(&attempt.stdout)),
        stderr: Some(hash_bytes(&attempt.stderr)),
        notes,
    }
}

fn find_marker<'a>(output: &'a str, markers: &[&'a str]) -> Option<&'a str> {
    markers
        .iter()
        .copied()
        .find(|marker| output.contains(marker))
}

fn contains_any(output: &str, markers: &[&str]) -> bool {
    markers.iter().any(|marker| output.contains(marker))
}

fn is_help_like_output(output_lower: &str) -> bool {
    output_lower.contains("usage:")
}

fn infer_invalid_argument_for_option(option: &str, output_lower: &str) -> bool {
    match option {
        "--tabsize" => output_lower.contains("invalid tab size"),
        "--width" => output_lower.contains("invalid line width"),
        _ => false,
    }
}

fn extract_reported_options(output: &str) -> Vec<String> {
    let mut options = Vec::new();
    let direct = Regex::new(
        r#"(?i)(?:unrecognized|unknown|invalid|illegal)\s+(?:option|flag|switch)(?:\s+|[:=])\s*['"`]?([^\s'"`]+)"#,
    )
    .expect("regex for direct option errors");
    for cap in direct.captures_iter(output) {
        let token = cap.get(1).map(|m| m.as_str()).unwrap_or_default();
        let cleaned = clean_option_token(token);
        if cleaned.is_empty() || cleaned == "-" || cleaned == "--" {
            continue;
        }
        options.push(cleaned);
    }

    let getopt = Regex::new(
        r#"(?i)(?:invalid|illegal|unknown|unrecognized)\s+option\s+--\s*['"]?([A-Za-z0-9])['"]?"#,
    )
    .expect("regex for getopt option errors");
    for cap in getopt.captures_iter(output) {
        let ch = cap.get(1).map(|m| m.as_str()).unwrap_or_default();
        if !ch.is_empty() {
            options.push(format!("-{}", ch));
        }
    }

    options
}

fn extract_missing_argument_options(output: &str) -> Vec<String> {
    let mut options = Vec::new();
    let direct = Regex::new(
        r#"(?i)(?:option|flag|switch)\s+['"`]?([^\s'"`]+)['"`]?\s+requires\s+(?:an?\s+)?(?:argument|value)"#,
    )
    .expect("regex for required argument errors");
    for cap in direct.captures_iter(output) {
        let token = cap.get(1).map(|m| m.as_str()).unwrap_or_default();
        let cleaned = clean_option_token(token);
        if cleaned.is_empty() || cleaned == "-" || cleaned == "--" {
            continue;
        }
        options.push(cleaned);
    }

    let missing = Regex::new(r#"(?i)missing\s+(?:argument|value)\s+for\s+['"`]?([^\s'"`]+)['"`]?"#)
        .expect("regex for missing argument errors");
    for cap in missing.captures_iter(output) {
        let token = cap.get(1).map(|m| m.as_str()).unwrap_or_default();
        let cleaned = clean_option_token(token);
        if cleaned.is_empty() || cleaned == "-" || cleaned == "--" {
            continue;
        }
        options.push(cleaned);
    }

    let required_for = Regex::new(
        r#"(?i)requires\s+(?:an?\s+)?(?:argument|value)\s+for\s+['"`]?([^\s'"`]+)['"`]?"#,
    )
    .expect("regex for required argument for option errors");
    for cap in required_for.captures_iter(output) {
        let token = cap.get(1).map(|m| m.as_str()).unwrap_or_default();
        let cleaned = clean_option_token(token);
        if cleaned.is_empty() || cleaned == "-" || cleaned == "--" {
            continue;
        }
        options.push(cleaned);
    }

    let getopt = Regex::new(
        r#"(?i)option\s+requires\s+(?:an?\s+)?(?:argument|value)\s+--\s*['"]?([A-Za-z0-9])['"]?"#,
    )
    .expect("regex for getopt missing argument errors");
    for cap in getopt.captures_iter(output) {
        let ch = cap.get(1).map(|m| m.as_str()).unwrap_or_default();
        if !ch.is_empty() {
            options.push(format!("-{}", ch));
        }
    }

    options
}

fn extract_argument_not_allowed_options(output: &str) -> Vec<String> {
    let mut options = Vec::new();
    let direct = Regex::new(
        r#"(?i)option\s+['"`]?([^\s'"`]+)['"`]?\s+does(?:n't| not)\s+(?:allow|take|accept)\s+(?:an?\s+)?(?:argument|value)"#,
    )
    .expect("regex for argument not allowed errors");
    for cap in direct.captures_iter(output) {
        let token = cap.get(1).map(|m| m.as_str()).unwrap_or_default();
        let cleaned = clean_option_token(token);
        if cleaned.is_empty() || cleaned == "-" || cleaned == "--" {
            continue;
        }
        options.push(cleaned);
    }

    let takes_no =
        Regex::new(r#"(?i)option\s+['"`]?([^\s'"`]+)['"`]?\s+takes?\s+no\s+(?:argument|value)"#)
            .expect("regex for takes no argument errors");
    for cap in takes_no.captures_iter(output) {
        let token = cap.get(1).map(|m| m.as_str()).unwrap_or_default();
        let cleaned = clean_option_token(token);
        if cleaned.is_empty() || cleaned == "-" || cleaned == "--" {
            continue;
        }
        options.push(cleaned);
    }

    let not_allowed =
        Regex::new(r#"(?i)(?:argument|value)\s+not\s+allowed\s+for\s+['"`]?([^\s'"`]+)['"`]?"#)
            .expect("regex for argument not allowed for option errors");
    for cap in not_allowed.captures_iter(output) {
        let token = cap.get(1).map(|m| m.as_str()).unwrap_or_default();
        let cleaned = clean_option_token(token);
        if cleaned.is_empty() || cleaned == "-" || cleaned == "--" {
            continue;
        }
        options.push(cleaned);
    }

    options
}

fn extract_invalid_argument_options(output: &str) -> Vec<String> {
    let mut options = Vec::new();
    let invalid = Regex::new(
        r#"(?i)invalid\s+(?:argument|value)\s+['"`]?[^'"`]+['"`]?\s+for\s+['"`]?([^\s'"`]+)['"`]?"#,
    )
    .expect("regex for invalid argument errors");
    for cap in invalid.captures_iter(output) {
        let token = cap.get(1).map(|m| m.as_str()).unwrap_or_default();
        let cleaned = clean_option_token(token);
        if cleaned.is_empty() || cleaned == "-" || cleaned == "--" {
            continue;
        }
        options.push(cleaned);
    }

    let invalid_option =
        Regex::new(r#"(?i)invalid\s+([^\s'"`]+)\s+(?:argument|value)\s+['"`]?[^'"`]+['"`]?"#)
            .expect("regex for invalid option argument errors");
    for cap in invalid_option.captures_iter(output) {
        let token = cap.get(1).map(|m| m.as_str()).unwrap_or_default();
        if !token.starts_with('-') {
            continue;
        }
        let cleaned = clean_option_token(token);
        if cleaned.is_empty() || cleaned == "-" || cleaned == "--" {
            continue;
        }
        options.push(cleaned);
    }

    options
}

fn clean_option_token(token: &str) -> String {
    token
        .trim_matches(|c: char| matches!(c, ',' | ';' | ':' | '.' | ')' | ']' | '('))
        .to_string()
}

fn option_matches(reported: &str, tested: &str) -> bool {
    let reported = reported.to_lowercase();
    let tested = tested.to_lowercase();
    if reported == tested {
        return true;
    }
    if let Some((prefix, _)) = reported.split_once('=') {
        if prefix == tested {
            return true;
        }
    }
    if reported.len() == 1 && tested.len() == 2 && tested.starts_with('-') {
        return tested.chars().nth(1) == reported.chars().next();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attempt_from_output(stderr: &str) -> ExecutionAttempt {
        ExecutionAttempt {
            args: Vec::new(),
            exit_code: Some(2),
            stdout: Vec::new(),
            stderr: stderr.as_bytes().to_vec(),
            spawn_error: None,
        }
    }

    fn attempt_from_output_with_code(stderr: &str, exit_code: i32) -> ExecutionAttempt {
        ExecutionAttempt {
            args: Vec::new(),
            exit_code: Some(exit_code),
            stdout: Vec::new(),
            stderr: stderr.as_bytes().to_vec(),
            spawn_error: None,
        }
    }

    fn attempt_from_output_with_stdout(
        stdout: &str,
        stderr: &str,
        exit_code: i32,
    ) -> ExecutionAttempt {
        ExecutionAttempt {
            args: Vec::new(),
            exit_code: Some(exit_code),
            stdout: stdout.as_bytes().to_vec(),
            stderr: stderr.as_bytes().to_vec(),
            spawn_error: None,
        }
    }

    #[test]
    fn refutes_when_unrecognized_option_matches_claim() {
        let attempt = attempt_from_output("error: unrecognized option '--nope'");
        let analysis = attempt_signals("--nope", &attempt);
        let (status, note) = classify_attempt(&attempt, &analysis);
        assert!(matches!(status, ValidationStatus::Refuted));
        assert!(note.is_none());
    }

    #[test]
    fn undetermined_when_unrecognized_option_is_different() {
        let attempt = attempt_from_output("error: unrecognized option '--help'");
        let analysis = attempt_signals("--all", &attempt);
        let (status, note) = classify_attempt(&attempt, &analysis);
        assert!(matches!(status, ValidationStatus::Undetermined));
        assert!(note.is_some());
    }

    #[test]
    fn confirms_when_argument_missing() {
        let attempt = attempt_from_output("option '--block-size' requires an argument");
        let analysis = attempt_signals("--block-size", &attempt);
        let (status, note) = classify_attempt(&attempt, &analysis);
        assert!(matches!(status, ValidationStatus::Confirmed));
        assert!(note.is_some());
    }

    #[test]
    fn refutes_getopt_short_option_error() {
        let attempt = attempt_from_output("invalid option -- 'i'");
        let analysis = attempt_signals("-i", &attempt);
        let (status, note) = classify_attempt(&attempt, &analysis);
        assert!(matches!(status, ValidationStatus::Refuted));
        assert!(note.is_none());
    }

    #[test]
    fn undetermined_on_spawn_error() {
        let attempt = ExecutionAttempt {
            args: Vec::new(),
            exit_code: None,
            stdout: Vec::new(),
            stderr: Vec::new(),
            spawn_error: Some("boom".to_string()),
        };
        let analysis = attempt_signals("--all", &attempt);
        let (status, note) = classify_attempt(&attempt, &analysis);
        assert!(matches!(status, ValidationStatus::Undetermined));
        assert_eq!(note.as_deref(), Some("spawn failed: boom"));
    }

    #[test]
    fn infers_required_binding_on_missing_argument() {
        let missing = attempt_from_output("option '--size' requires an argument");
        let invalid = attempt_from_output("");
        let missing_analysis = attempt_signals("--size", &missing);
        let invalid_analysis = attempt_signals("--size", &invalid);
        let (status, kind, _) = infer_binding(&missing_analysis, &invalid_analysis, None, None);
        assert!(matches!(status, ValidationStatus::Confirmed));
        assert!(matches!(kind, Some(BindingKind::Required)));
    }

    #[test]
    fn infers_no_value_on_argument_not_allowed() {
        let missing = attempt_from_output_with_code("", 0);
        let invalid = attempt_from_output("option '--all' doesn't allow an argument");
        let missing_analysis = attempt_signals("--all", &missing);
        let invalid_analysis = attempt_signals("--all", &invalid);
        let (status, kind, _) = infer_binding(&missing_analysis, &invalid_analysis, None, None);
        assert!(matches!(status, ValidationStatus::Confirmed));
        assert!(matches!(kind, Some(BindingKind::NoValue)));
    }

    #[test]
    fn infers_optional_binding_on_invalid_argument() {
        let missing = attempt_from_output_with_code("", 0);
        let invalid = attempt_from_output("invalid argument 'nope' for '--color'");
        let missing_analysis = attempt_signals("--color", &missing);
        let invalid_analysis = attempt_signals("--color", &invalid);
        let (status, kind, _) = infer_binding(&missing_analysis, &invalid_analysis, None, None);
        assert!(matches!(status, ValidationStatus::Confirmed));
        assert!(matches!(kind, Some(BindingKind::Optional)));
    }

    #[test]
    fn attributes_invalid_tab_size_to_tabsize() {
        let attempt = attempt_from_output("ls: invalid tab size: '--help'");
        let analysis = attempt_signals("--tabsize", &attempt);
        assert!(analysis.invalid_arg);
        assert!(analysis
            .notes
            .iter()
            .any(|note| note.contains("attributed to tested option")));
    }

    #[test]
    fn attributes_invalid_line_width_to_width() {
        let attempt = attempt_from_output("ls: invalid line width: '--help'");
        let analysis = attempt_signals("--width", &attempt);
        assert!(analysis.invalid_arg);
        assert!(analysis
            .notes
            .iter()
            .any(|note| note.contains("attributed to tested option")));
    }

    #[test]
    fn option_matches_attached_value() {
        assert!(option_matches("--sort=__bvm__", "--sort"));
    }

    #[test]
    fn infers_required_binding_when_help_consumed_and_invalid_value() {
        let missing = attempt_from_output_with_stdout("file1\nfile2", "", 0);
        let invalid = attempt_from_output_with_stdout(
            "Usage: tool [OPTION]",
            "invalid argument '__bvm__' for '--hide'",
            1,
        );
        let missing_analysis = attempt_signals("--hide", &missing);
        let invalid_analysis = attempt_signals("--hide", &invalid);
        let (status, kind, _) = infer_binding(&missing_analysis, &invalid_analysis, None, None);
        assert!(matches!(status, ValidationStatus::Confirmed));
        assert!(matches!(kind, Some(BindingKind::Required)));
    }
}
