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

## Parameter Surface Tiers

Option parameters are evaluated as a tiered surface:

- T0: Option existence.
- T1: Parameter binding (required vs optional value).
- T2: Parameter form (attachment style, repeatability).
- T3: Parameter domain/type (enum, numeric, path-like).
- T4: Behavioral semantics.

Only T0 and T1 are in scope today. Higher tiers may remain not evaluated indefinitely.

## What "Comprehensive" Means

A man page is comprehensive when every user-visible surface is either validated at its tier or
explicitly marked as undetermined/not evaluated.

Requirements:

- Tiered surface coverage: report % confirmed/undetermined for T0/T1; higher tiers are marked not evaluated.
- Large parameter spaces are accounted for via coverage + unknowns, not exhaustive enumeration.
- Behavioral semantics (T4) only included when validated; otherwise out of scope.
- Observational grounding: every statement traceable to evidence or marked unknown.
- Negative space: document limits, variability, and untested cases.

## Source of Truth and Claims

- Binary identity is recorded (path, hash, platform, env).
- Documentation inputs are non-authoritative claims until validated.

## Validation and Outputs

- Validation runs under controlled env and fixtures when needed.
- Outputs include a regenerated man page and a machine-readable validation report.

## Environment Contract

Validation is tied to a controlled execution contract:

- LC_ALL=C
- TZ=UTC
- TERM=dumb
- temp fs fixtures (when required)

Results are valid only under this contract. Environment-sensitive behavior is classified as
undetermined.

## Scope

- Initial target: a single coreutils-style binary (e.g. ls).
- Current validation scope: T0 option existence and T1 parameter binding.
- Stop when tiered surface completeness is reached and remaining gaps are documented.

See `docs/MILESTONES.md` for the current plan and status and `docs/SCHEMAS.md` for schema
definitions and the tiered surface model.

## Evaluation Criteria

- Only observed behaviors are documented.
- Defaults are explicit.
- Discrepancies are justified with evidence.
- Unknowns are clearly marked.
- Output is tied to a specific binary hash.
