//! Validation helpers for surface-level option claims.
//!
//! Validation is intentionally conservative: it runs bounded, low-impact probes
//! and relies on explicit error markers rather than exit status alone.
//!
//! ## Pipeline summary
//! - **Existence**: run `<opt> --help` and look for unrecognized/ambiguous markers.
//! - **Binding**: run missing-arg and with-arg probes and compare responses.
//! - **Evidence**: store hashed stdout/stderr plus marker notes.
//!
//! ## Example walkthroughs
//! Existence claim (`--all`) confirmed when no unrecognized marker is present:
//! ```text
//! $ tool --all --help
//! (exit 0, no "unrecognized option" marker)
//! -> confirmed
//! ```
//! Existence claim (`--nope`) refuted when the error mentions the option:
//! ```text
//! $ tool --nope --help
//! error: unrecognized option '--nope'
//! -> refuted
//! ```
//! Required binding confirmed by missing-arg response:
//! ```text
//! $ tool --size --help
//! option '--size' requires an argument
//! $ tool --size __bvm__ --help
//! (no missing-arg error)
//! -> confirmed (required)
//! ```
//! Optional binding confirmed when missing-arg is OK but invalid arg is rejected:
//! ```text
//! $ tool --color --help
//! (exit 0)
//! $ tool --color __bvm__ --help
//! invalid argument '__bvm__' for '--color'
//! -> confirmed (optional)
//! ```

use crate::schema::{
    Claim, Determinism, EnvSnapshot, Evidence, ValidationMethod, ValidationResult, ValidationStatus,
};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BindingExpectation {
    Required,
    Optional,
}

#[derive(Debug, Clone)]
struct BindingSpec {
    option: String,
    expectation: Option<BindingExpectation>,
    form: Option<String>,
}

#[derive(Debug)]
struct AttemptAnalysis {
    unrecognized: bool,
    missing_arg: bool,
    arg_not_allowed: bool,
    invalid_arg: bool,
    ambiguous: bool,
    exit_code: Option<i32>,
    notes: Vec<String>,
}

/// Default validation environment (`LC_ALL=C`, `TZ=UTC`, `TERM=dumb`).
pub fn validation_env() -> EnvSnapshot {
    EnvSnapshot {
        locale: "C".to_string(),
        tz: "UTC".to_string(),
        term: "dumb".to_string(),
    }
}

/// Extract the canonical option token from an option existence claim ID.
///
/// # Examples
/// ```ignore
/// use crate::validate::option_from_claim_id;
/// assert_eq!(
///     option_from_claim_id("claim:option:opt=--all:exists"),
///     Some("--all".to_string())
/// );
/// assert_eq!(option_from_claim_id("claim:option:opt=--all:binding"), None);
/// ```
pub fn option_from_claim_id(id: &str) -> Option<String> {
    const PREFIX: &str = "claim:option:opt=";
    const SUFFIX: &str = ":exists";
    if !id.starts_with(PREFIX) || !id.ends_with(SUFFIX) {
        return None;
    }
    let option = &id[PREFIX.len()..id.len().saturating_sub(SUFFIX.len())];
    if option.is_empty() {
        None
    } else {
        Some(option.to_string())
    }
}

/// Extract the canonical option token from a parameter binding claim ID.
///
/// # Examples
/// ```ignore
/// use crate::validate::option_from_binding_claim_id;
/// assert_eq!(
///     option_from_binding_claim_id("claim:option:opt=--size:binding"),
///     Some("--size".to_string())
/// );
/// assert_eq!(option_from_binding_claim_id("claim:option:opt=--size:exists"), None);
/// ```
pub fn option_from_binding_claim_id(id: &str) -> Option<String> {
    const PREFIX: &str = "claim:option:opt=";
    const SUFFIX: &str = ":binding";
    if !id.starts_with(PREFIX) || !id.ends_with(SUFFIX) {
        return None;
    }
    let option = &id[PREFIX.len()..id.len().saturating_sub(SUFFIX.len())];
    if option.is_empty() {
        None
    } else {
        Some(option.to_string())
    }
}

/// Execute a harmless invocation and classify the option existence claim.
///
/// The probe always appends `--help` to minimize side effects.
///
/// # Examples
/// ```ignore
/// # use crate::validate::validate_option_existence;
/// # use crate::validate::validation_env;
/// # use std::path::Path;
/// let env = validation_env();
/// let result = validate_option_existence(Path::new("tool"), "claim:option:opt=--all:exists", "--all", &env);
/// assert!(matches!(result.status, crate::schema::ValidationStatus::Confirmed | crate::schema::ValidationStatus::Undetermined));
/// ```
pub fn validate_option_existence(
    binary: &Path,
    claim_id: &str,
    option: &str,
    env: &EnvSnapshot,
) -> ValidationResult {
    let args = vec![option.to_string(), "--help".to_string()];
    let output = Command::new(binary)
        .args(&args)
        .env_clear()
        .env("LC_ALL", &env.locale)
        .env("TZ", &env.tz)
        .env("TERM", &env.term)
        .output();

    let attempt = match output {
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
    };

    let (status, notes) = classify_attempt(option, &attempt);
    let evidence = Evidence {
        args: attempt.args.clone(),
        env: env_map(env),
        exit_code: attempt.exit_code,
        stdout: Some(hash_bytes(&attempt.stdout)),
        stderr: Some(hash_bytes(&attempt.stderr)),
        notes,
    };

    ValidationResult {
        claim_id: claim_id.to_string(),
        status,
        method: ValidationMethod::AcceptanceTest,
        determinism: Some(Determinism::Deterministic),
        attempts: vec![evidence],
        observed: None,
        reason: None,
    }
}

/// Execute controlled invocations and classify the parameter binding claim.
///
/// The validator runs two probes:
/// - **Missing arg**: `<opt> --help`
/// - **With arg**: `<opt> __bvm__ --help` (or `--opt=__bvm__` when attached)
///
/// # Examples
/// ```ignore
/// # use crate::validate::validate_option_binding;
/// # use crate::validate::validation_env;
/// # use crate::schema::Claim;
/// # use std::path::Path;
/// let env = validation_env();
/// let claim = Claim {
///     id: "claim:option:opt=--size:binding".to_string(),
///     text: "Option --size requires a value in `--size=SIZE` form.".to_string(),
///     kind: crate::schema::ClaimKind::Option,
///     source: crate::schema::ClaimSource { source_type: crate::schema::ClaimSourceType::Help, path: "<captured:--help>".to_string(), line: None },
///     status: crate::schema::ClaimStatus::Unvalidated,
///     extractor: "parse:help:v1".to_string(),
///     raw_excerpt: "--size=SIZE".to_string(),
///     confidence: Some(0.7),
/// };
/// let result = validate_option_binding(Path::new("tool"), &claim, &env);
/// assert!(matches!(result.status, crate::schema::ValidationStatus::Confirmed | crate::schema::ValidationStatus::Undetermined | crate::schema::ValidationStatus::Refuted));
/// ```
pub fn validate_option_binding(
    binary: &Path,
    claim: &Claim,
    env: &EnvSnapshot,
) -> ValidationResult {
    let Some(spec) = binding_spec_from_claim(claim) else {
        return ValidationResult {
            claim_id: claim.id.clone(),
            status: ValidationStatus::Undetermined,
            method: ValidationMethod::AcceptanceTest,
            determinism: Some(Determinism::Deterministic),
            attempts: Vec::new(),
            observed: None,
            reason: Some("unrecognized parameter binding claim".to_string()),
        };
    };

    let missing_args = vec![spec.option.clone(), "--help".to_string()];
    let missing_attempt = run_attempt(binary, missing_args, env);
    let missing_analysis = analyze_binding_attempt(&spec.option, &missing_attempt);

    let mut with_arg_args = build_with_arg_args(spec.form.as_deref(), &spec.option);
    with_arg_args.push("--help".to_string());
    let with_arg_attempt = run_attempt(binary, with_arg_args, env);
    let with_arg_analysis = analyze_binding_attempt(&spec.option, &with_arg_attempt);

    let (status, reason) = match spec.expectation {
        Some(expectation) => {
            classify_binding_attempts(expectation, &missing_analysis, &with_arg_analysis)
        }
        None => (
            ValidationStatus::Undetermined,
            Some("parameter binding expectation missing".to_string()),
        ),
    };

    let attempts = vec![
        evidence_for_attempt(missing_attempt, env, "missing_arg", &missing_analysis),
        evidence_for_attempt(with_arg_attempt, env, "with_arg", &with_arg_analysis),
    ];

    ValidationResult {
        claim_id: claim.id.clone(),
        status,
        method: ValidationMethod::AcceptanceTest,
        determinism: Some(Determinism::Deterministic),
        attempts,
        observed: None,
        reason,
    }
}

fn classify_attempt(
    option: &str,
    attempt: &ExecutionAttempt,
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

    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&attempt.stdout),
        String::from_utf8_lossy(&attempt.stderr)
    );
    let output_lower = output.to_lowercase();

    if let Some(marker) = find_marker(&output_lower, UNRECOGNIZED_MARKERS) {
        let reported = extract_reported_options(&output);
        if reported
            .iter()
            .any(|reported| option_matches(reported, option))
        {
            return (ValidationStatus::Refuted, None);
        }
        let note = if reported.is_empty() {
            format!("unrecognized option marker ({marker}) without option attribution")
        } else {
            format!("unrecognized option marker ({marker}) for {:?}", reported)
        };
        return (ValidationStatus::Undetermined, Some(note));
    }

    if let Some(marker) = find_marker(&output_lower, AMBIGUOUS_MARKERS) {
        return (
            ValidationStatus::Undetermined,
            Some(format!("ambiguous option response ({marker})")),
        );
    }

    let mut notes = Vec::new();
    if exit_code != 0 {
        notes.push(format!(
            "nonzero exit ({exit_code}) without unrecognized option marker"
        ));
    }
    if contains_any(&output_lower, ARGUMENT_ERROR_MARKERS) {
        notes.push("argument error reported".to_string());
    }

    let note = if notes.is_empty() {
        None
    } else {
        Some(notes.join("; "))
    };

    (ValidationStatus::Confirmed, note)
}

fn binding_spec_from_claim(claim: &Claim) -> Option<BindingSpec> {
    let option = option_from_binding_claim_id(&claim.id)?;
    let expectation = parse_binding_expectation(&claim.text, &claim.raw_excerpt);
    let form = extract_form_text(&claim.text);
    Some(BindingSpec {
        option,
        expectation,
        form,
    })
}

fn parse_binding_expectation(text: &str, raw_excerpt: &str) -> Option<BindingExpectation> {
    let lower = text.to_lowercase();
    if lower.contains("optional value") {
        return Some(BindingExpectation::Optional);
    }
    if lower.contains("requires a value") {
        return Some(BindingExpectation::Required);
    }
    if raw_excerpt.contains("[=") {
        return Some(BindingExpectation::Optional);
    }
    if raw_excerpt.contains('=') {
        return Some(BindingExpectation::Required);
    }
    None
}

fn extract_form_text(text: &str) -> Option<String> {
    let start = text.find('`')?;
    let rest = &text[start + 1..];
    let end = rest.find('`')?;
    Some(rest[..end].to_string())
}

fn build_with_arg_args(form: Option<&str>, option: &str) -> Vec<String> {
    if let Some(form) = form {
        if let Some(idx) = form.find("[=") {
            return vec![format!("{}={}", &form[..idx], BINDING_DUMMY_VALUE)];
        }
        if let Some(idx) = form.find('=') {
            return vec![format!("{}={}", &form[..idx], BINDING_DUMMY_VALUE)];
        }
    }
    if option.starts_with("--") {
        vec![format!("{option}={BINDING_DUMMY_VALUE}")]
    } else {
        vec![option.to_string(), BINDING_DUMMY_VALUE.to_string()]
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

fn analyze_binding_attempt(option: &str, attempt: &ExecutionAttempt) -> AttemptAnalysis {
    let mut notes = Vec::new();
    if let Some(err) = &attempt.spawn_error {
        notes.push(format!("spawn failed: {err}"));
        return AttemptAnalysis {
            unrecognized: false,
            missing_arg: false,
            arg_not_allowed: false,
            invalid_arg: false,
            ambiguous: false,
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
    let missing_arg = missing_options
        .iter()
        .any(|reported| option_matches(reported, option));
    if !missing_options.is_empty() && !missing_arg {
        notes.push(format!("missing argument marker for {:?}", missing_options));
    } else if missing_options.is_empty() && contains_any(&output_lower, MISSING_ARGUMENT_MARKERS) {
        notes.push("missing argument marker without option attribution".to_string());
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
    let invalid_arg = invalid_options
        .iter()
        .any(|reported| option_matches(reported, option));
    if !invalid_options.is_empty() && !invalid_arg {
        notes.push(format!("invalid argument marker for {:?}", invalid_options));
    } else if invalid_options.is_empty() && contains_any(&output_lower, INVALID_ARGUMENT_MARKERS) {
        notes.push("invalid argument marker without option attribution".to_string());
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
        exit_code: attempt.exit_code,
        notes,
    }
}

fn classify_binding_attempts(
    expectation: BindingExpectation,
    missing: &AttemptAnalysis,
    with_arg: &AttemptAnalysis,
) -> (ValidationStatus, Option<String>) {
    if missing.unrecognized || with_arg.unrecognized {
        return (
            ValidationStatus::Refuted,
            Some("unrecognized option response".to_string()),
        );
    }

    let ambiguous_note = if missing.ambiguous || with_arg.ambiguous {
        Some("ambiguous option response".to_string())
    } else {
        None
    };

    match expectation {
        BindingExpectation::Required => {
            if missing.missing_arg {
                (
                    ValidationStatus::Confirmed,
                    Some("missing argument response observed".to_string()),
                )
            } else if with_arg.arg_not_allowed {
                (
                    ValidationStatus::Refuted,
                    Some("argument not allowed response observed".to_string()),
                )
            } else {
                (ValidationStatus::Undetermined, ambiguous_note)
            }
        }
        BindingExpectation::Optional => {
            if missing.missing_arg {
                (
                    ValidationStatus::Refuted,
                    Some("missing argument response observed".to_string()),
                )
            } else if with_arg.arg_not_allowed {
                (
                    ValidationStatus::Refuted,
                    Some("argument not allowed response observed".to_string()),
                )
            } else if with_arg.invalid_arg {
                (
                    ValidationStatus::Confirmed,
                    Some("argument accepted with invalid value".to_string()),
                )
            } else if missing.exit_code == Some(0) && with_arg.exit_code == Some(0) {
                (
                    ValidationStatus::Confirmed,
                    Some("no argument errors detected".to_string()),
                )
            } else {
                (ValidationStatus::Undetermined, ambiguous_note)
            }
        }
    }
}

fn evidence_for_attempt(
    attempt: ExecutionAttempt,
    env: &EnvSnapshot,
    probe: &str,
    analysis: &AttemptAnalysis,
) -> Evidence {
    Evidence {
        args: attempt.args,
        env: env_map(env),
        exit_code: attempt.exit_code,
        stdout: Some(hash_bytes(&attempt.stdout)),
        stderr: Some(hash_bytes(&attempt.stderr)),
        notes: build_attempt_notes(probe, analysis),
    }
}

fn build_attempt_notes(probe: &str, analysis: &AttemptAnalysis) -> Option<String> {
    let mut parts = Vec::new();
    parts.push(format!("probe={probe}"));
    parts.extend(analysis.notes.iter().cloned());
    Some(parts.join("; "))
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

fn find_marker<'a>(output: &'a str, markers: &[&'a str]) -> Option<&'a str> {
    markers
        .iter()
        .copied()
        .find(|marker| output.contains(marker))
}

fn contains_any(output: &str, markers: &[&str]) -> bool {
    markers.iter().any(|marker| output.contains(marker))
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

    #[test]
    fn refutes_when_unrecognized_option_matches_claim() {
        let attempt = attempt_from_output("error: unrecognized option '--nope'");
        let (status, note) = classify_attempt("--nope", &attempt);
        assert!(matches!(status, ValidationStatus::Refuted));
        assert!(note.is_none());
    }

    #[test]
    fn undetermined_when_unrecognized_option_is_different() {
        let attempt = attempt_from_output("error: unrecognized option '--help'");
        let (status, note) = classify_attempt("--all", &attempt);
        assert!(matches!(status, ValidationStatus::Undetermined));
        assert!(note.is_some());
    }

    #[test]
    fn confirms_when_argument_missing() {
        let attempt = attempt_from_output("option '--block-size' requires an argument");
        let (status, note) = classify_attempt("--block-size", &attempt);
        assert!(matches!(status, ValidationStatus::Confirmed));
        assert!(note.is_some());
    }

    #[test]
    fn refutes_getopt_short_option_error() {
        let attempt = attempt_from_output("invalid option -- 'i'");
        let (status, note) = classify_attempt("-i", &attempt);
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
        let (status, note) = classify_attempt("--all", &attempt);
        assert!(matches!(status, ValidationStatus::Undetermined));
        assert_eq!(note.as_deref(), Some("spawn failed: boom"));
    }

    #[test]
    fn confirms_required_binding_on_missing_argument() {
        let missing = attempt_from_output("option '--size' requires an argument");
        let with_arg = attempt_from_output("");
        let missing_analysis = analyze_binding_attempt("--size", &missing);
        let with_arg_analysis = analyze_binding_attempt("--size", &with_arg);
        let (status, _) = classify_binding_attempts(
            BindingExpectation::Required,
            &missing_analysis,
            &with_arg_analysis,
        );
        assert!(matches!(status, ValidationStatus::Confirmed));
    }

    #[test]
    fn refutes_optional_binding_on_missing_argument() {
        let missing = attempt_from_output("option '--size' requires an argument");
        let with_arg = attempt_from_output("");
        let missing_analysis = analyze_binding_attempt("--size", &missing);
        let with_arg_analysis = analyze_binding_attempt("--size", &with_arg);
        let (status, _) = classify_binding_attempts(
            BindingExpectation::Optional,
            &missing_analysis,
            &with_arg_analysis,
        );
        assert!(matches!(status, ValidationStatus::Refuted));
    }

    #[test]
    fn confirms_optional_binding_on_invalid_argument() {
        let missing = attempt_from_output_with_code("", 0);
        let with_arg = attempt_from_output("invalid argument 'nope' for '--color'");
        let missing_analysis = analyze_binding_attempt("--color", &missing);
        let with_arg_analysis = analyze_binding_attempt("--color", &with_arg);
        let (status, _) = classify_binding_attempts(
            BindingExpectation::Optional,
            &missing_analysis,
            &with_arg_analysis,
        );
        assert!(matches!(status, ValidationStatus::Confirmed));
    }

    #[test]
    fn refutes_optional_binding_on_argument_not_allowed() {
        let missing = attempt_from_output_with_code("", 0);
        let with_arg = attempt_from_output("option '--all' doesn't allow an argument");
        let missing_analysis = analyze_binding_attempt("--all", &missing);
        let with_arg_analysis = analyze_binding_attempt("--all", &with_arg);
        let (status, _) = classify_binding_attempts(
            BindingExpectation::Optional,
            &missing_analysis,
            &with_arg_analysis,
        );
        assert!(matches!(status, ValidationStatus::Refuted));
    }

    #[test]
    fn refutes_required_binding_on_unrecognized_option() {
        let missing = attempt_from_output("error: unrecognized option '--ghost'");
        let with_arg = attempt_from_output("");
        let missing_analysis = analyze_binding_attempt("--ghost", &missing);
        let with_arg_analysis = analyze_binding_attempt("--ghost", &with_arg);
        let (status, _) = classify_binding_attempts(
            BindingExpectation::Required,
            &missing_analysis,
            &with_arg_analysis,
        );
        assert!(matches!(status, ValidationStatus::Refuted));
    }
}
