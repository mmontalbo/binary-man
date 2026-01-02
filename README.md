# Binary-Validated Man Pages

Generate accurate man pages by validating documentation claims against a specific binary.

The binary on disk is the source of truth. Man pages, --help output, and source excerpts are
treated as claims and validated through controlled execution. The output is a regenerated man page
that records confirmed facts and explicit unknowns.

## Motivation

Man pages drift from actual behavior, omit defaults, and diverge across versions. When docs are
wrong or incomplete, users and models must guess. This project replaces guesswork with measured
validation.

## Goal

- Parse documentation inputs into claims with provenance.
- Execute the binary under controlled environments to validate claims.
- Classify each claim as confirmed, refuted, or undetermined.
- Regenerate a man page tied to a specific binary identity.

## What "Comprehensive" Means

A man page is comprehensive when every user-visible behavior is either documented and validated or
explicitly marked as undetermined.

Requirements:

- Surface coverage: options, argument forms, env vars, and IO surfaces.
- Behavioral coverage: outputs, errors, and exit status meanings.
- Observational grounding: every statement traceable to evidence or marked unknown.
- Negative space: document limits, variability, and untested cases.

## Source of Truth and Claims

- Binary identity is recorded (path, hash, platform, env).
- Documentation inputs are non-authoritative claims until validated.

## Validation and Outputs

- Validation runs under controlled env and fixtures when needed.
- Outputs include a regenerated man page and a machine-readable validation report.

## Scope

- Initial target: a single coreutils-style binary (e.g. ls).
- Stop when surface completeness is reached and remaining gaps are documented.

See `docs/MILESTONES.md` for the current plan and status and `docs/SCHEMAS.md` for schema
definitions.

## Evaluation Criteria

- Only observed behaviors are documented.
- Defaults are explicit.
- Discrepancies are justified with evidence.
- Unknowns are clearly marked.
- Output is tied to a specific binary hash.
